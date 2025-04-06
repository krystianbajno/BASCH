#[cfg(windows)]
use std::io::{self, Read, Write};
#[cfg(windows)]
use std::net::TcpStream;
#[cfg(windows)]
use std::process::{Command, Stdio};
#[cfg(windows)]
use std::sync::Mutex;
#[cfg(windows)]
use std::thread;
#[cfg(windows)]
use std::ptr;

#[cfg(windows)]
use crate::net_mini_shell::{set_current_child, CURRENT_CHILD};

// For Windows ConPTY usage:
#[cfg(windows)]
use windows::Win32::{
    Foundation::HANDLE,
    System::Console::{
        CreatePseudoConsole, ClosePseudoConsole, HPCON, COORD,
        PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE,
    },
    System::Pipes::{
        CreatePipe, SetHandleInformation, HANDLE_FLAG_INHERIT,
    },
    System::Threading::{
        InitializeProcThreadAttributeList, UpdateProcThreadAttribute,
        DeleteProcThreadAttributeList, PROCESS_INFORMATION, STARTUPINFOEXW,
        CreateProcessW, WaitForSingleObject, GetExitCodeProcess, INFINITE
    },
};

#[cfg(windows)]
use widestring::U16CString;

#[cfg(windows)]
pub fn run_in_pty(shell_path: &str, shell_args: &[&str], stream: &mut TcpStream) -> io::Result<()> {
    // Build the command line
    let mut cmd_line = format!("{} ", shell_path);
    for arg in shell_args {
        if arg.contains(' ') {
            cmd_line.push_str(&format!("\"{}\" ", arg));
        } else {
            cmd_line.push_str(arg);
            cmd_line.push(' ');
        }
    }
    let cmd_line = cmd_line.trim().to_string();

    // Create pipes for ConPTY
    let (mut pipe_in_read, mut pipe_in_write) = (HANDLE(0), HANDLE(0));
    let (mut pipe_out_read, mut pipe_out_write) = (HANDLE(0), HANDLE(0));
    unsafe {
        let mut sa = Default::default();
        if !CreatePipe(&mut pipe_in_read, &mut pipe_in_write, &sa, 0).as_bool() {
            return Err(io::Error::last_os_error());
        }
        if !CreatePipe(&mut pipe_out_read, &mut pipe_out_write, &sa, 0).as_bool() {
            return Err(io::Error::last_os_error());
        }
        SetHandleInformation(pipe_in_write, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT);
        SetHandleInformation(pipe_out_read, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT);
    }

    // Create the ConPTY
    let mut size = COORD { X: 80, Y: 24 };
    let mut hpc: HPCON = HPCON(ptr::null_mut());
    unsafe {
        let hr = CreatePseudoConsole(size, pipe_out_read, pipe_in_write, 0, &mut hpc);
        if hr.0 != 0 {
            return Err(io::Error::from_raw_os_error(hr.0));
        }
    }

    // Prepare STARTUPINFOEX
    let mut si_ex: STARTUPINFOEXW = unsafe { std::mem::zeroed() };
    si_ex.StartupInfo.cb = std::mem::size_of::<STARTUPINFOEXW>() as u32;

    // Allocate attribute list
    let mut attr_size: usize = 0;
    unsafe {
        // first call just to get required size
        InitializeProcThreadAttributeList(ptr::null_mut(), 1, 0, &mut attr_size);
    }
    let mut attr_list = vec![0u8; attr_size];
    si_ex.lpAttributeList = attr_list.as_mut_ptr() as *mut _;

    unsafe {
        if !InitializeProcThreadAttributeList(si_ex.lpAttributeList, 1, 0, &mut attr_size).as_bool() {
            return Err(io::Error::last_os_error());
        }
        if !UpdateProcThreadAttribute(
            si_ex.lpAttributeList,
            0,
            PROC_THREAD_ATTRIBUTE_PSEUDOCONSOLE,
            hpc.0 as *mut _,
            std::mem::size_of::<HPCON>(),
            ptr::null_mut(),
            ptr::null_mut(),
        ).as_bool()
        {
            return Err(io::Error::last_os_error());
        }
    }

    let cmd_line_w = U16CString::from_str(&cmd_line).unwrap();
    let mut pi: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };

    let success = unsafe {
        CreateProcessW(
            None,
            Some(cmd_line_w.as_pwstr()),
            None,
            None,
            true, // inherit handles
            0x00080000, // EXTENDED_STARTUPINFO_PRESENT
            None,
            None,
            &mut si_ex.StartupInfo,
            &mut pi
        ).as_bool()
    };

    if !success {
        unsafe {
            DeleteProcThreadAttributeList(si_ex.lpAttributeList);
            ClosePseudoConsole(hpc);
        }
        return Err(io::Error::last_os_error());
    }

    // We have a child process. We can store the handle if needed. 
    // For demonstration, we store None or something:
    {
        let mut guard = CURRENT_CHILD.lock().unwrap();
        *guard = None; // We can't store the Windows handle easily as Child
    }

    // We must close thread handle
    unsafe {
        // keep pi.hProcess if we want to wait. Close thread handle:
        windows::Win32::Foundation::CloseHandle(pi.hThread);
    }

    // Now bridge pipe_in_read => network (child stdout => network)
    //        and network => pipe_out_write (network => child stdin)

    // The easiest approach is to convert these HANDLEs to `File`.
    let mut read_file = unsafe { std::fs::File::from_raw_handle(pipe_in_read.0 as *mut _) };
    let mut write_file = unsafe { std::fs::File::from_raw_handle(pipe_out_write.0 as *mut _) };

    // 1) Child STDOUT => network
    let mut net_writer = stream.try_clone()?;
    thread::spawn(move || {
        let mut buf = [0u8; 1024];
        loop {
            match read_file.read(&mut buf) {
                Ok(0) => {
                    let _ = net_writer.shutdown(std::net::Shutdown::Write);
                    break;
                }
                Ok(n) => {
                    if net_writer.write_all(&buf[..n]).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        // handle closed
    });

    // 2) Network => child STDIN
    let mut net_reader = stream.try_clone()?;
    thread::spawn(move || {
        let mut buf = [0u8; 1024];
        loop {
            match net_reader.read(&mut buf) {
                Ok(0) => {
                    // net closed
                    let _ = write_file.shutdown(std::net::Shutdown::Write);
                    break;
                }
                Ok(n) => {
                    if write_file.write_all(&buf[..n]).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    });

    // Optionally wait for the child in a separate thread
    thread::spawn(move || {
        unsafe {
            WaitForSingleObject(pi.hProcess, INFINITE);
            let mut code: u32 = 0;
            GetExitCodeProcess(pi.hProcess, &mut code);
            eprintln!("(shell) ConPTY child exited with code: {}", code);

            // Cleanup
            ClosePseudoConsole(hpc);
            DeleteProcThreadAttributeList(si_ex.lpAttributeList);
            windows::Win32::Foundation::CloseHandle(pi.hProcess);
        }
    });

    Ok(())
}
