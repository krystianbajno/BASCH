use std::io;
use std::net::TcpStream;
use std::sync::Mutex;
use lazy_static::lazy_static;
use std::process::Child;

lazy_static! {
    pub static ref CURRENT_CHILD: Mutex<Option<Child>> = Mutex::new(None);
}

pub fn set_current_child(child: Child) {
    let mut guard = CURRENT_CHILD.lock().unwrap();
    *guard = Some(child);
}

#[cfg(unix)]
fn candidate_shells() -> Vec<&'static str> {
    vec!["/bin/zsh", "/bin/bash", "/bin/sh", "/bin/ksh"]
}

#[cfg(windows)]
fn candidate_shells() -> Vec<&'static str> {
    vec!["powershell.exe", "cmd.exe"]
}

pub fn spawn_system_shell(stream: &mut TcpStream) -> io::Result<()> {
    let shells = candidate_shells();

    for shell in shells {
        match try_spawn_shell(shell, stream) {
            Ok(_) => return Ok(()),
            Err(e) => {
                eprintln!("Failed to spawn shell `{}`: {}", shell, e);
            }
        }
    }

    Err(io::Error::new(io::ErrorKind::NotFound, "No shell found"))
}

#[cfg(unix)]
fn try_spawn_shell(shell_path: &str, stream: &mut TcpStream) -> io::Result<()> {
    crate::net_mini::unix_pty::run_in_pty(shell_path, &[], stream)
}

#[cfg(windows)]
fn try_spawn_shell(shell_path: &str, stream: &mut TcpStream) -> io::Result<()> {
    crate::net_mini::win_pty::run_in_pty(shell_path, &[], stream)
}
