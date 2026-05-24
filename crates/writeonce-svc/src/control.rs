//! Unix-socket control plane for `wo-ctl`.
//!
//! Wire protocol: line-based ASCII. The client sends one request line
//! (terminated by `\n`); the server replies with zero or more response
//! lines and closes the socket. Multi-line responses are terminated by
//! the underlying socket close, not by a sentinel — clients read until
//! EOF.
//!
//! Commands:
//!
//! ```text
//!   LIST                    → one line per unit:  <name> <state>
//!   STATUS  <unit>          → key/value pairs, one per line
//!   START   <unit>          → start the unit (and its closure)
//!   STOP    <unit>          → stop the unit (and units depending on it)
//!   RESTART <unit>          → stop then start
//!   SHUTDOWN                → initiate orderly supervisor shutdown
//! ```
//!
//! Every command's response includes a final `ok` or `err: <msg>` line
//! so the client can exit with a clean status code.

use std::io::{BufRead, BufReader, Write};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;

use crate::state::SupervisorState;

pub const DEFAULT_SOCKET: &str = "/run/writeonce/control.sock";

pub struct ControlListener {
    pub listener: UnixListener,
    pub path:     String,
}

impl ControlListener {
    /// Bind the listener. If `path` already exists (stale socket from a
    /// previous run), unlink and re-bind.
    pub fn bind(path: &str) -> std::io::Result<Self> {
        if Path::new(path).exists() {
            std::fs::remove_file(path)?;
        }
        if let Some(parent) = Path::new(path).parent() {
            std::fs::create_dir_all(parent).ok();
        }
        let listener = UnixListener::bind(path)?;
        listener.set_nonblocking(true)?;
        Ok(Self { listener, path: path.to_string() })
    }

    pub fn as_raw_fd(&self) -> RawFd { self.listener.as_raw_fd() }
}

impl Drop for ControlListener {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Accept any waiting connection and serve it synchronously. Returns
/// `true` if the supervisor should begin shutdown (the client issued
/// `SHUTDOWN`).
pub fn handle_ready(listener: &ControlListener, state: &mut SupervisorState) -> bool {
    let mut shutdown_requested = false;
    loop {
        match listener.listener.accept() {
            Ok((stream, _)) => {
                if handle_one(stream, state) {
                    shutdown_requested = true;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
            Err(e) => {
                eprintln!("writeonce-svc: accept failed: {e}");
                break;
            }
        }
    }
    shutdown_requested
}

fn handle_one(stream: UnixStream, state: &mut SupervisorState) -> bool {
    // Read one request line.
    stream.set_nonblocking(false).ok();
    let mut reader = BufReader::new(&stream);
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() {
        let _ = writeln!(&stream, "err: read failed");
        return false;
    }
    let request = line.trim_end_matches(['\r', '\n']).to_string();
    drop(reader); // release borrow on &stream

    let parts: Vec<&str> = request.split_whitespace().collect();
    if parts.is_empty() {
        let _ = writeln!(&stream, "err: empty request");
        return false;
    }

    let mut shutdown_requested = false;
    let result: Result<(), String> = match parts[0].to_ascii_uppercase().as_str() {
        "LIST"     => list(&stream, state),
        "STATUS"   => status(&stream, state, parts.get(1).copied()),
        "START"    => start(&stream, state, parts.get(1).copied()),
        "STOP"     => stop(&stream, state, parts.get(1).copied()),
        "RESTART"  => restart(&stream, state, parts.get(1).copied()),
        "SHUTDOWN" => {
            let _ = writeln!(&stream, "writeonce-svc: shutdown initiated");
            shutdown_requested = true;
            Ok(())
        }
        other => Err(format!("unknown command: {other}")),
    };

    match result {
        Ok(()) => { let _ = writeln!(&stream, "ok"); }
        Err(e) => { let _ = writeln!(&stream, "err: {e}"); }
    }
    shutdown_requested
}

fn list(mut stream: &UnixStream, state: &SupervisorState) -> Result<(), String> {
    for (id, name) in state.iter_loaded() {
        let u = state.unit(id);
        let pid = u.pid.map(|p| p.to_string()).unwrap_or_else(|| "—".to_string());
        writeln!(stream, "{name:32}  {:>12?}  pid={pid}", u.state)
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn status(mut stream: &UnixStream, state: &SupervisorState, name: Option<&str>) -> Result<(), String> {
    let name = name.ok_or("STATUS requires a unit name")?;
    let id = state.registry.lookup(name).ok_or_else(|| format!("unknown unit: {name}"))?;
    let u = state.unit(id);
    let unit = state.registry.get(id);
    writeln!(stream, "name:        {}", &unit.name).map_err(|e| e.to_string())?;
    writeln!(stream, "description: {}", &unit.file.unit.description).map_err(|e| e.to_string())?;
    writeln!(stream, "state:       {:?}", u.state).map_err(|e| e.to_string())?;
    if let Some(p) = u.pid {
        writeln!(stream, "pid:         {p}").map_err(|e| e.to_string())?;
    }
    if let Some(svc) = &unit.file.service {
        writeln!(stream, "type:        {:?}", svc.kind).map_err(|e| e.to_string())?;
        writeln!(stream, "restart:     {:?}", svc.restart).map_err(|e| e.to_string())?;
        writeln!(stream, "exec-start:  {}", svc.exec_start).map_err(|e| e.to_string())?;
    }
    Ok(())
}

fn start(mut stream: &UnixStream, state: &mut SupervisorState, name: Option<&str>) -> Result<(), String> {
    let name = name.ok_or("START requires a unit name")?;
    writeln!(stream, "writeonce-svc: starting {name}").map_err(|e| e.to_string())?;
    state.start_named(name)
}

fn stop(mut stream: &UnixStream, state: &mut SupervisorState, name: Option<&str>) -> Result<(), String> {
    let name = name.ok_or("STOP requires a unit name")?;
    writeln!(stream, "writeonce-svc: stopping {name}").map_err(|e| e.to_string())?;
    state.stop_unit(name)
}

fn restart(mut stream: &UnixStream, state: &mut SupervisorState, name: Option<&str>) -> Result<(), String> {
    let name = name.ok_or("RESTART requires a unit name")?;
    let _ = writeln!(stream, "writeonce-svc: restarting {name}");
    state.stop_unit(name)?;
    state.start_named(name)
}
