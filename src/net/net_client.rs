use std::env;
use std::fs;
use std::io::{self, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::Path;

use net_utils::user_shell;
use net_utils::common;

fn connect_and_run(address: &str) -> io::Result<()> {
    let mut stream = TcpStream::connect(address)?;
    println!("Connected to {}", address);

    let mut buf_reader = BufReader::new(stream.try_clone()?);
    common::print_banner(&mut buf_reader)?; 
    user_shell::setup_signal_handler(&stream)?;

    common::command_loop(|trimmed_command_line| {
        if let Some((command, _redir_op, filename)) = user_shell::parse_redirect(trimmed_command_line) {
            stream.write_all(command.as_bytes())?;
            stream.write_all(b"\n")?;
            stream.flush()?;

            let output = user_shell::capture_until_prompt(&mut stream)?;
            if trimmed_command_line.contains(" >") {
                fs::write(filename, output)?;
            } else {
                use std::fs::OpenOptions;
                let mut file = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(filename)?;
                file.write_all(output.as_bytes())?;
            }
            println!("Output written to {}", filename);
        } 
        else if let Some(exec_line) = trimmed_command_line.strip_prefix("exec ") {
            let mut parts = exec_line.split_whitespace();
            if let Some(local_binary) = parts.next() {
                let args: Vec<String> = parts.map(|s| s.to_string()).collect();

                let binary_data = fs::read(local_binary)?;
                let file_size = binary_data.len();

                let file_name_only = Path::new(local_binary)
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .to_string();

                let mut upload_cmd = format!("EXEC_UPLOAD {} {}",
                                             file_name_only, file_size);
                for a in &args {
                    upload_cmd.push(' ');
                    upload_cmd.push_str(a);
                }

                stream.write_all(upload_cmd.as_bytes())?;
                stream.write_all(b"\n")?;
                stream.flush()?;


                stream.write_all(&binary_data)?;
                stream.flush()?;

                let output = user_shell::capture_until_prompt(&mut stream)?;
                print!("{}", output);
            } else {
                println!("Usage: exec <local_binary_path> [args...]");
            }
        }
        else {
            stream.write_all(trimmed_command_line.as_bytes())?;
            stream.write_all(b"\n")?;
            stream.flush()?;

            let output = user_shell::capture_until_prompt(&mut stream)?;
            print!("{}", output);
        }

        io::stdout().flush()?;
        Ok(())
    })?;

    println!("Disconnecting...");
    Ok(())
}

fn main() {
    let address = env::var("CONNECT_ADDRESS").unwrap_or_else(|_| "127.0.0.1:8080".to_string());
    if let Err(e) = connect_and_run(&address) {
        eprintln!("Connection error: {}", e);
    }
}
