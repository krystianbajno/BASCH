use std::env;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::thread;
use signal_hook::iterator::Signals;

fn connect_and_run(address: &str) -> io::Result<()> {
    let mut stream = TcpStream::connect(address)?;
    println!("Connected to {}", address);

    let mut banner = String::new();
    let mut buf_reader = BufReader::new(stream.try_clone()?);

    buf_reader.read_line(&mut banner)?;
    print!("{}", banner);
    banner.clear();
    buf_reader.read_line(&mut banner)?;
    print!("{}", banner);

    let mut stream_clone = stream.try_clone()?;
    thread::spawn(move || {
        let mut buffer = [0; 1024];
        loop {
            match stream_clone.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => {
                    let _ = io::stdout().write_all(&buffer[..n]);
                    let _ = io::stdout().flush();
                }
                Err(_) => break,
            }
        }
    });

    let mut signals = Signals::new(&[signal_hook::consts::SIGINT, signal_hook::consts::SIGTSTP])?;
    let mut stream_for_signal = stream.try_clone()?;
    thread::spawn(move || {
        for signal in signals.forever() {
            match signal {
                signal_hook::consts::SIGINT => {
                    let _ = stream_for_signal.write_all(b"SIGNAL SIGINT\n");
                }
                signal_hook::consts::SIGTSTP => {
                    let _ = stream_for_signal.write_all(b"SIGNAL SIGTSTP\n");
                }
                _ => {}
            }
        }
    });

    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim() == "quit" {
            println!("Disconnecting...");
            break;
        }
        stream.write_all(line.as_bytes())?;
        stream.write_all(b"\n")?;
    }

    Ok(())
}

fn main() {
    let address = env::var("CONNECT_ADDRESS")
        .unwrap_or_else(|_| "127.0.0.1:8080".to_string());

    match connect_and_run(&address) {
        Ok(_) => println!("Disconnected normally."),
        Err(e) => eprintln!("Connection error: {}", e),
    }
}
