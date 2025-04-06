#[cfg(unix)]
use std::io::{self, Read, Write};
#[cfg(unix)]
use std::net::TcpStream;
#[cfg(unix)]
use std::os::fd::{AsRawFd, FromRawFd};
#[cfg(unix)]
use std::fs::File;
#[cfg(unix)]
use std::process::Command;
#[cfg(unix)]
use std::thread;

#[cfg(unix)]
use nix::fcntl::OFlag;
#[cfg(unix)]
use nix::pty::{openpty, Winsize};
#[cfg(unix)]
use nix::sys::termios;
#[cfg(unix)]
use nix::unistd::{dup, close};

#[cfg(unix)]
use crate::exports::{CommandSpec, CURRENT_CHILD};

/// Spawns the given command in a fresh PTY on Unix-like systems,
/// then bridges I/O between that PTY and the given `TcpStream`.
#[cfg(unix)]
pub fn run_in_pty(cmdspec: &CommandSpec, stream: &mut TcpStream) -> io::Result<()> {
    // Convert CommandSpec into command line
    let program = &cmdspec.argv[0];
    let args = &cmdspec.argv[1..];

    // Create a new PTY with default settings
    let pty = openpty(
        Some(&Winsize {
            ws_row: 24,
            ws_col: 80,
            ws_xpixel: 0,
            ws_ypixel: 0,
        }),
        None, // no special termios here
    ).map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

    // pty.master and pty.slave are now `OwnedFd`s in nix 0.29+
    // Configure termios if needed
    {
        let mut term = termios::tcgetattr(&pty.slave)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        // Adjust any terminal modes if you want
        termios::tcsetattr(&pty.slave, termios::SetArg::TCSANOW, &term)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    }

    // Spawn child with slave as stdio
    let mut child_cmd = Command::new(program);
    child_cmd.args(args);

    // If you want to honor < > redirects for "interactive" commands, do so.
    // But many interactive programs ignore them. Example:
    if let Some(ref infile) = cmdspec.redirect_in {
        child_cmd.stdin(File::open(infile)?);
    } else {
        // Convert the `OwnedFd` into a File for stdin
        // We do "into_raw_fd()" because we only want one copy for child stdin.
        let slave_fd = pty.slave.as_raw_fd();
        // SAFE: We transfer ownership to Stdio
        let slave_file = unsafe { File::from_raw_fd(dup(slave_fd)?) };
        child_cmd.stdin(slave_file);
    }

    if let Some(ref outfile) = cmdspec.redirect_out {
        child_cmd.stdout(File::create(outfile)?);
    } else {
        let slave_fd = pty.slave.as_raw_fd();
        let slave_file = unsafe { File::from_raw_fd(dup(slave_fd)?) };
        child_cmd.stdout(slave_file);
    }

    if let Some(ref errfile) = cmdspec.redirect_err {
        child_cmd.stderr(File::create(errfile)?);
    } else {
        let slave_fd = pty.slave.as_raw_fd();
        let slave_file = unsafe { File::from_raw_fd(dup(slave_fd)?) };
        child_cmd.stderr(slave_file);
    }

    let child = child_cmd.spawn()?;

    {
        let mut guard = CURRENT_CHILD.lock().unwrap();
        *guard = Some(child);
    }

    // Close the slave in the parent so we don't hold it open
    // (The child has it already.)
    // We just drop the `pty.slave` OwnedFd:
    drop(pty.slave);

    // Now set up bridging from pty.master <-> TCP stream

    // We'll read from one cloned FD in one thread, and write to a second clone in another thread.
    // That means we need two duplicates of the master fd.
    let master_read_fd = dup(pty.master.as_raw_fd())?;
    let master_write_fd = dup(pty.master.as_raw_fd())?;

    // Convert them to File for convenient .read() and .write().
    let mut master_for_read = unsafe { File::from_raw_fd(master_read_fd) };
    let mut master_for_write = unsafe { File::from_raw_fd(master_write_fd) };

    // Child => Network
    let mut stream_writer = stream.try_clone()?;
    thread::spawn(move || {
        let mut buf = [0u8; 1024];
        loop {
            match master_for_read.read(&mut buf) {
                Ok(0) => {
                    // EOF from child
                    let _ = stream_writer.shutdown(std::net::Shutdown::Write);
                    break;
                }
                Ok(n) => {
                    if stream_writer.write_all(&buf[..n]).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        // optional close
        let _ = close(master_read_fd);
    });

    // Network => Child
    let mut stream_reader = stream.try_clone()?;
    thread::spawn(move || {
        let mut buf = [0u8; 1024];
        loop {
            match stream_reader.read(&mut buf) {
                Ok(0) => {
                    // network closed
                    let _ = close(master_write_fd);
                    break;
                }
                Ok(n) => {
                    if master_for_write.write_all(&buf[..n]).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        let _ = close(master_write_fd);
    });

    // We can wait for the child or let the caller handle that
    let status = {
        let mut guard = CURRENT_CHILD.lock().unwrap();
        if let Some(child) = guard.as_mut() {
            child.wait()?
        } else {
            return Ok(());
        }
    };
    eprintln!("(shell) PTY child exited with: {}", status);

    // Also close the main pty.master if you like:
    drop(pty.master);

    Ok(())
}
