use std::env;
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;

fn process_data(data: &[u8]) -> Vec<u8> {
    // if let Ok(text) = std::str::from_utf8(data) {
    //     Replace "foo" with "bar" in the text.
    //    let replaced = text.replace("foo", "bar");
    //    replaced.into_bytes()
    // } else {
    //    // If data is not valid UTF‑8, return it unchanged.
    //    data.to_vec()
    // }

    data.to_vec()
}


fn handle_client(mut client: TcpStream, remote_addr: &str, show: bool) -> io::Result<()> {
    let mut remote = TcpStream::connect(remote_addr)?;
    
    let client_to_remote = thread::spawn({
        let mut client = client.try_clone()?;
        let mut remote = remote.try_clone()?;
        move || -> io::Result<()> {
            let mut buffer = [0; 4096];
            loop {
                let n = client.read(&mut buffer)?;
                if n == 0 { break; }
                let data = process_data(&buffer[..n]);
                if show {
                    println!("C -> R ({} bytes): {:?}", data.len(), String::from_utf8_lossy(&data));
                }
                remote.write_all(&data)?;
            }
            Ok(())
        }
    });

    let remote_to_client = thread::spawn({
        let mut remote = remote;
        let mut client = client;
        move || -> io::Result<()> {
            let mut buffer = [0; 4096];
            loop {
                let n = remote.read(&mut buffer)?;
                if n == 0 { break; }
                let data = process_data(&buffer[..n]);
                if show {
                    println!("R -> C ({} bytes): {:?}", data.len(), String::from_utf8_lossy(&data));
                }
                client.write_all(&data)?;
            }
            Ok(())
        }
    });

    client_to_remote.join().unwrap()?;
    remote_to_client.join().unwrap()?;
    Ok(())
}

fn main() -> io::Result<()> {
    // Usage: tcp_proxy [show] <local_addr> <remote_addr>

    let args: Vec<String> = env::args().collect();

    let (show, local_addr, remote_addr) = if args.len() == 4 && args[1] == "show" {
        (true, args[2].clone(), args[3].clone())
    } else if args.len() == 3 {
        (false, args[1].clone(), args[2].clone())
    } else {
        eprintln!("Usage: {} [show] <local_addr> <remote_addr>", args[0]);
        std::process::exit(1);
    };

    let listener = TcpListener::bind(&local_addr)?;
    println!("TCP proxy listening on {} forwarding to {}", local_addr, remote_addr);
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let remote_addr = remote_addr.clone();
                thread::spawn(move || {
                    if let Err(e) = handle_client(stream, &remote_addr, show) {
                        eprintln!("Connection error: {}", e);
                    }
                });
            }
            Err(e) => {
                eprintln!("Accept error: {}", e);
            }
        }
    }
    Ok(())
}
