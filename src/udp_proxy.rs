use std::env;
use std::net::{UdpSocket, SocketAddr};
use std::io;
use std::time::Duration;

fn process_data(data: &[u8]) -> Vec<u8> {
        // if let Ok(text) = std::str::from_utf8(data) {
    //     Replace "foo" with "bar" in the text.
    //    let replaced = text.replace("foo", "bar");
    //    replaced.into_bytes()
    // } else {
    //    // If data is not valid UTF‑8, return it unchanged.
    //    data.to_vec()
    // }

    // Modify or inspect data here if needed.
    data.to_vec()
}

fn main() -> io::Result<()> {
    let args: Vec<String> = env::args().collect();
    // Usage: udp_proxy [show] <local_addr> <remote_addr>
    let (show, local_addr, remote_addr_str) = if args.len() == 4 && args[1] == "show" {
        (true, args[2].clone(), args[3].clone())
    } else if args.len() == 3 {
        (false, args[1].clone(), args[2].clone())
    } else {
        eprintln!("Usage: {} [show] <local_addr> <remote_addr>", args[0]);
        std::process::exit(1);
    };

    let remote_addr: SocketAddr = remote_addr_str.parse().expect("Invalid remote address");
    let socket = UdpSocket::bind(&local_addr)?;
    socket.set_read_timeout(Some(Duration::from_secs(1)))?;
    println!("UDP proxy listening on {} forwarding to {}", local_addr, remote_addr);

    let mut last_client: Option<SocketAddr> = None;
    let mut buffer = [0u8; 4096];
    loop {
        match socket.recv_from(&mut buffer) {
            Ok((n, src)) => {
                let data = process_data(&buffer[..n]);
                if show {
                    println!("Received {} bytes from {}: {:?}", data.len(), src, String::from_utf8_lossy(&data));
                }
                if src == remote_addr {
                    if let Some(client_addr) = last_client {
                        socket.send_to(&data, client_addr)?;
                        if show {
                            println!("Forwarded {} bytes from remote to client {}", data.len(), client_addr);
                        }
                    }
                } else {
                    last_client = Some(src);
                    socket.send_to(&data, remote_addr)?;
                    if show {
                        println!("Forwarded {} bytes from client {} to remote", data.len(), src);
                    }
                }
            }
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
            }
            Err(e) => {
                eprintln!("Error receiving UDP packet: {}", e);
            }
        }
    }
}
