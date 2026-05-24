//! `wo-ctl` — client for the WriteOnce supervisor's control plane.
//!
//! Connects to the supervisor's Unix socket (`/run/writeonce/control.sock`
//! by default, overridable with `WO_CTL_SOCKET=…` or `--socket …`),
//! sends a single command line, and prints the response. Exits 0 if the
//! supervisor's final response line is `ok`, otherwise 1.
//!
//! Usage:
//!     wo-ctl [--socket PATH] <command> [args...]
//!
//! Commands:
//!     list                              show every unit + its state
//!     status   <unit>                   show details for one unit
//!     start    <unit>                   start a unit (and its closure)
//!     stop     <unit>                   stop a unit (and its dependents)
//!     restart  <unit>                   stop then start
//!     shutdown                          tell the supervisor to exit

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::process;

const DEFAULT_SOCKET: &str = "/run/writeonce/control.sock";

fn usage() -> ! {
    eprintln!("usage: wo-ctl [--socket PATH] <list | status <unit> | start <unit> | stop <unit> | restart <unit> | shutdown>");
    process::exit(2);
}

fn main() {
    let argv: Vec<String> = std::env::args().collect();
    let mut socket = std::env::var("WO_CTL_SOCKET")
        .unwrap_or_else(|_| DEFAULT_SOCKET.to_string());
    let mut cmd_args: Vec<&str> = Vec::new();
    let mut i = 1;
    while i < argv.len() {
        match argv[i].as_str() {
            "--socket" => {
                i += 1;
                if i >= argv.len() { usage(); }
                socket = argv[i].clone();
            }
            "-h" | "--help" => usage(),
            other => cmd_args.push(other),
        }
        i += 1;
    }

    if cmd_args.is_empty() {
        usage();
    }

    // Build the wire command line.
    let request_line = cmd_args.iter()
        .map(|s| s.to_string())
        .collect::<Vec<_>>()
        .join(" ");

    let mut stream = match UnixStream::connect(&socket) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("wo-ctl: connect {}: {}", socket, e);
            process::exit(1);
        }
    };

    if let Err(e) = writeln!(stream, "{request_line}") {
        eprintln!("wo-ctl: write: {e}");
        process::exit(1);
    }
    // Tell the supervisor we're done sending.
    let _ = stream.shutdown(std::net::Shutdown::Write);

    let reader = BufReader::new(&stream);
    let mut last_line = String::new();
    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                eprintln!("wo-ctl: read: {e}");
                process::exit(1);
            }
        };
        println!("{line}");
        last_line = line;
    }

    if last_line == "ok" {
        process::exit(0);
    } else if last_line.starts_with("err:") {
        process::exit(1);
    } else {
        // Closed without sentinel — treat as a soft success (server may
        // have been mid-shutdown).
        process::exit(0);
    }
}
