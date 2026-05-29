//! `writeonce-kerngen` CLI.
//!
//! Usage:
//!     writeonce-kerngen probe [--output PATH]
//!
//! `--output -` (or omitting) writes pretty-printed JSON to stdout.
//! Anything else is taken as a path; the file is created (or
//! truncated) and the JSON written there.
//!
//! Phase 7b will add `resolve` (consume a probe JSON + kernel source
//! tree, emit a Kconfig fragment). For now this binary only probes.

use std::fs;
use std::io::{self, Write};
use std::process;

use writeonce_kerngen::probe;

fn main() {
    let argv: Vec<String> = std::env::args().collect();
    let result = match argv.get(1).map(String::as_str) {
        Some("probe")        => run_probe(&argv[2..]),
        Some("-h") | Some("--help") | Some("help") | None => {
            print_usage();
            Ok(())
        }
        Some(other) => {
            eprintln!("writeonce-kerngen: unknown subcommand: {other}");
            print_usage();
            process::exit(2);
        }
    };
    if let Err(e) = result {
        eprintln!("writeonce-kerngen: error: {e}");
        process::exit(1);
    }
}

fn print_usage() {
    eprintln!("usage:");
    eprintln!("  writeonce-kerngen probe [--output PATH | -o PATH]");
    eprintln!("  writeonce-kerngen help");
    eprintln!();
    eprintln!("`probe` walks /sys, /proc and emits hardware probe JSON.");
    eprintln!("Future: `resolve <probe.json>` will derive a kernel .config fragment.");
}

fn run_probe(args: &[String]) -> io::Result<()> {
    let mut output: Option<String> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-o" | "--output" => {
                i += 1;
                if i >= args.len() {
                    return Err(io::Error::other("--output requires a value"));
                }
                output = Some(args[i].clone());
            }
            "-h" | "--help" => {
                print_usage();
                return Ok(());
            }
            other => {
                return Err(io::Error::other(format!(
                    "probe: unknown argument: {other}")));
            }
        }
        i += 1;
    }

    let probe = probe::collect();
    let json = serde_json::to_string_pretty(&probe)
        .map_err(io::Error::other)?;

    match output.as_deref() {
        None | Some("-") => {
            let mut out = io::stdout().lock();
            out.write_all(json.as_bytes())?;
            out.write_all(b"\n")?;
        }
        Some(path) => {
            fs::write(path, json + "\n")?;
        }
    }
    Ok(())
}
