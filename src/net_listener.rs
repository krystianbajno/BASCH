use std::env;
use std::io::{self, BufRead, Read, Write};
use std::net::TcpListener;
use std::thread;

fn main() -> io::Result<()> {
    let address = env::var("LISTENER_ADDRESS")
        .unwrap_or_else(|_| "127.0.0.1:8080".to_string());

    let listener: TcpListener = TcpListener::bind(&address)?;
    println!("Listener started on {}. Waiting for reverse shell connection...", address);

    let (mut stream, addr) = listener.accept()?;
    println!("Reverse shell connected from {}", addr);

    let mut stream_clone = stream.try_clone()?;
    thread::spawn(move || {
        let mut buffer = [0; 1024];
        loop {
            match stream_clone.read(&mut buffer) {
                Ok(0) => {
                    eprintln!("Connection closed by remote host.");
                    break;
                }
                Ok(n) => {
                    print!("{}", String::from_utf8_lossy(&buffer[..n]));
                    let _ = io::stdout().flush();
                }
                Err(e) => {
                    eprintln!("Error reading from connection: {}", e);
                    break;
                }
            }
        }
    });

    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        let command = line?;
        if command.trim() == "quit" {
            println!("Exiting listener.");
            break;
        }
        // Write command and newline; if BrokenPipe is encountered, exit gracefully.
        if let Err(e) = stream.write_all(command.as_bytes()) {
            if e.kind() == io::ErrorKind::BrokenPipe {
                eprintln!("Broken pipe encountered, exiting listener.");
                break;
            } else {
                return Err(e);
            }
        }
        if let Err(e) = stream.write_all(b"\n") {
            if e.kind() == io::ErrorKind::BrokenPipe {
                eprintln!("Broken pipe encountered, exiting listener.");
                break;
            } else {
                return Err(e);
            }
        }
    }
    Ok(())
}
