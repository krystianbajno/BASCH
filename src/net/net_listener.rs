use std::env;
use std::io::{self, BufRead, BufReader, Write};
use std::net::TcpListener;

fn main() -> io::Result<()> {
    let address: String = env::var("LISTENER_ADDRESS").unwrap_or_else(|_| "0.0.0.0:8080".to_string());
    let listener = TcpListener::bind(&address)?;
    println!("net_listener: listening on {}", address);
    println!("Wait for reverse shell connection...");

    let (mut stream, addr) = listener.accept()?;
    println!("Reverse shell connected from {}", addr);

    let mut remote_reader = BufReader::new(stream.try_clone()?);
    std::thread::spawn(move || {
        let mut line = String::new();
        while let Ok(n) = remote_reader.read_line(&mut line) {
            if n == 0 {
                println!("Remote shell disconnected.");
                break;
            }
            print!("{}", line);
            io::stdout().flush().ok();
            line.clear();
        }
    });

    // Thread B: read from local stdin -> write to remote
    let mut remote_writer = stream;
    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        let line = line.unwrap();
        if line.trim().eq_ignore_ascii_case("quit") {
            break;
        }
        writeln!(remote_writer, "{}", line)?;
        remote_writer.flush()?;
    }

    println!("Exiting net_listener.");
    Ok(())
}
