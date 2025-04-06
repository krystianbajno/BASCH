// src/common.rs
use std::io::{self, BufRead, Write};

pub fn print_banner<R: BufRead>(reader: &mut R) -> io::Result<()> {
    let mut line = String::new();
    // read SERVER_OS line
    reader.read_line(&mut line)?;
    print!("{}", line);
    line.clear();
    // read BANNER line
    reader.read_line(&mut line)?;
    print!("{}", line);
    io::stdout().flush()?;
    Ok(())
}

pub fn command_loop<F>(mut process_command: F) -> io::Result<()>
where
    F: FnMut(&str) -> io::Result<()>,
{
    let stdin = std::io::stdin();
    let mut input = String::new();
    loop {
        print!("$ ");
        io::stdout().flush()?;
        input.clear();
        if stdin.read_line(&mut input)? == 0 {
            break;
        }
        let trimmed = input.trim();
        if trimmed.eq_ignore_ascii_case("quit") {
            break;
        }
        process_command(trimmed)?;
    }
    Ok(())
}
