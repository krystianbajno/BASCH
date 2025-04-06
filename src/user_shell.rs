// src/user_shell.rs
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::thread;
use signal_hook::iterator::Signals;
use signal_hook::consts::signal::{SIGINT, SIGTSTP};

#[cfg(unix)]
use nix::sys::signal::{kill, Signal};
#[cfg(unix)]
use nix::unistd::Pid;

/// Parses a command line for a redirection operator (">" or ">>").
/// If found, returns: (command_without_redirection, operator, filename)
/// Otherwise returns None.
pub fn parse_redirect(line: &str) -> Option<(String, &str, &str)> {
    let parts: Vec<&str> = line.split_whitespace().collect();
    for (i, part) in parts.iter().enumerate() {
        if *part == ">" || *part == ">>" {
            if i == 0 || i + 1 >= parts.len() {
                return None;
            }
            let command = parts[..i].join(" ");
            let filename = parts[i + 1];
            return Some((command, *part, filename));
        }
    }
    None
}

/// Reads from the given stream until a prompt is detected:
/// we assume the prompt ends with "bash> " or "PS> ".
pub fn capture_until_prompt(stream: &mut impl Read) -> io::Result<String> {
    let mut output = String::new();
    let mut buffer = [0u8; 1024];
    loop {
        let n = stream.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        let chunk = String::from_utf8_lossy(&buffer[..n]);
        output.push_str(&chunk);
        if output.contains("bash> ") || output.contains("PS> ") {
            break;
        }
    }
    if let Some(pos) = output.rfind("bash> ") {
        output.truncate(pos);
    } else if let Some(pos) = output.rfind("PS> ") {
        output.truncate(pos);
    }
    Ok(output)
}

/// On the client side, set up SIGINT/SIGTSTP so that pressing Ctrl+C or Ctrl+Z
/// sends "SIGNAL SIGINT" or "SIGNAL SIGTSTP" over the TCP stream.
pub fn setup_signal_handler(stream: &TcpStream) -> io::Result<()> {
    let mut stream_clone = stream.try_clone()?;
    let mut signals = Signals::new(&[SIGINT, SIGTSTP])?;
    thread::spawn(move || {
        for signal in signals.forever() {
            match signal {
                SIGINT => {
                    let _ = stream_clone.write_all(b"SIGNAL SIGINT\n");
                }
                SIGTSTP => {
                    let _ = stream_clone.write_all(b"SIGNAL SIGTSTP\n");
                }
                _ => {}
            }
        }
    });
    Ok(())
}

/// On the server side, handle "SIGNAL" commands by sending the specified signal
/// to a running child process (tracked in a global).
#[cfg(unix)]
pub fn process_signal_command(
    sig_str: &str,
    stream: &mut impl Write,
    current_child: &std::sync::Mutex<Option<std::process::Child>>,
) -> io::Result<()> {
    let mut child_opt = current_child.lock().unwrap();
    if let Some(child) = child_opt.as_mut() {
        let pid = Pid::from_raw(child.id() as i32);
        let signal = match sig_str {
            "SIGINT" => Signal::SIGINT,
            "SIGTSTP" => Signal::SIGTSTP,
            _ => {
                writeln!(stream, "Unknown signal")?;
                return Ok(());
            }
        };
        let _ = kill(pid, signal);
    } else {
        writeln!(stream, "No running process to signal")?;
    }
    Ok(())
}

#[cfg(windows)]
pub fn process_signal_command(
    _sig_str: &str,
    stream: &mut impl Write,
    current_child: &std::sync::Mutex<Option<std::process::Child>>,
) -> io::Result<()> {
    let mut child_opt = current_child.lock().unwrap();
    if let Some(child) = child_opt.as_mut() {
        // On Windows, just kill the process for now
        let _ = child.kill();
    } else {
        writeln!(stream, "No running process to signal")?;
    }
    Ok(())
}
