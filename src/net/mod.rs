#[cfg(unix)]
pub mod unix_pty;

#[cfg(windows)]
pub mod windows_pty;