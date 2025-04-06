#[cfg(unix)]
use std::io::{self, Read, Write};
#[cfg(unix)]
use std::net::TcpStream;
#[cfg(unix)]
use std::os::fd::{AsRawFd, FromRawFd};
#[cfg(unix)]
use std::fs::File;
#[cfg(unix)]
use std::process::{Command, Stdio};
#[cfg(unix)]
use std::thread;

#[cfg(unix)]
use nix::pty::{openpty, Winsize};
#[cfg(unix)]
use nix::sys::termios;
#[cfg(unix)]
use nix::unistd::{dup, close};

#[cfg(unix)]
use crate::net_mini::net_mini_shell::set_current_child;


#[cfg(unix)]
pub fn run_in_pty(shell_path: &str, shell_args: &[&str], stream: &mut TcpStream) -> io::Result<()> {
    let pty = openpty(
        Some(&Winsize {
            ws_row: 24,
            ws_col: 80,
            ws_xpixel: 0,
            ws_ypixel: 0,
        }),
        None,
    ).map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;

    // (pty.master, pty.slave) are OwnedFd in nix 0.29+
    // Optionally set up termios on pty.slave
    {
        let mut term = termios::tcgetattr(&pty.slave)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
        termios::tcsetattr(&pty.slave, termios::SetArg::TCSANOW, &term)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e))?;
    }

    // Spawn the child
    let mut cmd = Command::new(shell_path);
    cmd.args(shell_args);

    // Child stdin => slave
    // We do a dup of pty.slave FD so we can wrap it in Stdio.
    let slave_fd = pty.slave.as_raw_fd();
    let slave_in = unsafe { File::from_raw_fd(dup(slave_fd)?) };
    cmd.stdin(slave_in);

    // Child stdout => slave
    let slave_out = unsafe { File::from_raw_fd(dup(slave_fd)?) };
    cmd.stdout(slave_out);

    // Child stderr => slave
    let slave_err = unsafe { File::from_raw_fd(dup(slave_fd)?) };
    cmd.stderr(slave_err);

    let child = cmd.spawn()?;
    set_current_child(child);

    // Close slave in parent
    drop(pty.slave);

    // Bridge PTY master <--> TCP Stream
    let master_read_fd = dup(pty.master.as_raw_fd())?;
    let master_write_fd = dup(pty.master.as_raw_fd())?;

    let mut master_for_read = unsafe { File::from_raw_fd(master_read_fd) };
    let mut master_for_write = unsafe { File::from_raw_fd(master_write_fd) };

    // 1) PTY => Network
    let mut stream_writer = stream.try_clone()?;
    thread::spawn(move || {
        let mut buf = [0u8; 1024];
        while let Ok(n) = master_for_read.read(&mut buf) {
            if n == 0 {
                let _ = stream_writer.shutdown(std::net::Shutdown::Write);
                break;
            }
            if stream_writer.write_all(&buf[..n]).is_err() {
                break;
            }
        }
        let _ = close(master_read_fd);
    });

    // 2) Network => PTY
    let mut stream_reader = stream.try_clone()?;
    thread::spawn(move || {
        let mut buf = [0u8; 1024];
        while let Ok(n) = stream_reader.read(&mut buf) {
            if n == 0 {
                // network closed
                let _ = close(master_write_fd);
                break;
            }
            if master_for_write.write_all(&buf[..n]).is_err() {
                break;
            }
        }
        let _ = close(master_write_fd);
    });

    // If you want to wait on the child here, do so; or let the main code handle it
    // (We skip waiting, so the shell runs until the network is closed).
    Ok(())
}
