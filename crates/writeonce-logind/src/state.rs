// state.rs — in-memory registry of sessions, seats, and inhibitors.
//
// writeonce-logind is a single-process daemon; all state lives here
// behind an Arc<Mutex<>>. Sessions are created by writeonce-login (or
// any other PAM-aware login program) calling Manager.CreateSession.

use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

/// One logged-in session. Maps 1:1 to a logind session in the
/// org.freedesktop.login1.Session sense.
#[derive(Debug, Clone)]
pub struct SessionInfo {
    /// Stable identifier (e.g. "c1", "c2"). Used in object paths.
    pub id: String,
    /// User ID of the session owner.
    pub uid: u32,
    /// Username (resolved from /etc/passwd at CreateSession time).
    pub user_name: String,
    /// Seat the session is attached to. Always "seat0" for us.
    pub seat: String,
    /// Virtual terminal number, or 0 if not on a VT.
    pub vtnr: u32,
    /// $DISPLAY for X sessions, empty otherwise.
    pub display: String,
    /// True if this session is remote (e.g. ssh). False for local console.
    pub remote: bool,
    /// PAM service name that created the session ("writeonce-login").
    pub service: String,
    /// Class: user | greeter | lock-screen | manager.
    pub class: String,
    /// Type: tty | x11 | wayland | unspecified.
    pub session_type: String,
    /// State: online | active | closing.
    pub state: String,
    /// Idle hint set by clients (i3more-idle, etc.).
    pub idle_hint: bool,
    /// Monotonic timestamp of last idle-hint change.
    pub idle_since: u64,
    /// PID of the session leader process.
    pub leader_pid: u32,
    /// Wall-clock time the session was created (microseconds since epoch).
    pub timestamp_usec: u64,
}

/// One inhibitor handle. systemd-logind returns an FD whose close ends
/// the inhibition; for us the FD is just one half of a pipe we hold
/// open as long as the inhibitor is alive.
#[derive(Debug, Clone)]
pub struct Inhibitor {
    #[allow(dead_code)] // surfaced via ListInhibitors in a later round
    pub id: u32,
    /// What is being inhibited: comma-separated list of
    /// "shutdown", "sleep", "idle", "handle-power-key", etc.
    pub what: String,
    /// Caller-supplied identifier ("i3more-power-manager").
    pub who: String,
    /// Caller-supplied reason ("user confirmation needed").
    pub why: String,
    /// Mode: "block" or "delay".
    pub mode: String,
    /// UID of the inhibitor's owner.
    pub uid: u32,
    /// PID of the inhibitor's owner.
    pub pid: u32,
}

/// All state shared between D-Bus interface impls.
#[derive(Debug)]
pub struct AppState {
    pub sessions: HashMap<String, SessionInfo>,
    pub next_session_id: u64,
    pub next_inhibitor_id: u32,
    pub inhibitors: Vec<Inhibitor>,
    pub preparing_for_shutdown: bool,
    pub preparing_for_sleep: bool,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            sessions: HashMap::new(),
            next_session_id: 1,
            next_inhibitor_id: 1,
            inhibitors: Vec::new(),
            preparing_for_shutdown: false,
            preparing_for_sleep: false,
        }
    }

    pub fn allocate_session_id(&mut self) -> String {
        let id = format!("c{}", self.next_session_id);
        self.next_session_id += 1;
        id
    }

    pub fn allocate_inhibitor_id(&mut self) -> u32 {
        let id = self.next_inhibitor_id;
        self.next_inhibitor_id += 1;
        id
    }

    /// Look up the session whose leader_pid matches the given pid, or
    /// any of the pid's ancestors (caller may be the session's grandchild).
    pub fn find_session_by_pid(&self, pid: u32) -> Option<&SessionInfo> {
        // Walk the PID up via /proc/<pid>/status PPid field until we
        // find a leader_pid match or reach pid 1.
        let mut current = pid;
        for _ in 0..32 {
            if current == 0 || current == 1 {
                return None;
            }
            if let Some(s) = self.sessions.values().find(|s| s.leader_pid == current) {
                return Some(s);
            }
            current = match read_ppid(current) {
                Some(p) => p,
                None => return None,
            };
        }
        None
    }

    /// Build the active-sessions tuple list returned by Manager.ListSessions.
    pub fn list_sessions_tuples(&self) -> Vec<(String, u32, String, String, zbus::zvariant::OwnedObjectPath)> {
        self.sessions
            .values()
            .map(|s| {
                let path = session_object_path(&s.id);
                (
                    s.id.clone(),
                    s.uid,
                    s.user_name.clone(),
                    s.seat.clone(),
                    path,
                )
            })
            .collect()
    }
}

/// Read the parent PID of `pid` from /proc/<pid>/status. Returns None
/// if the proc entry has gone away (process exited).
fn read_ppid(pid: u32) -> Option<u32> {
    let path = format!("/proc/{pid}/status");
    let content = std::fs::read_to_string(&path).ok()?;
    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("PPid:") {
            return rest.trim().parse().ok();
        }
    }
    None
}

pub fn session_object_path(session_id: &str) -> zbus::zvariant::OwnedObjectPath {
    let s = format!("/org/freedesktop/login1/session/{session_id}");
    zbus::zvariant::ObjectPath::try_from(s).unwrap().into()
}

pub fn seat_object_path(seat_id: &str) -> zbus::zvariant::OwnedObjectPath {
    let s = format!("/org/freedesktop/login1/seat/{seat_id}");
    zbus::zvariant::ObjectPath::try_from(s).unwrap().into()
}

pub fn user_object_path(uid: u32) -> zbus::zvariant::OwnedObjectPath {
    let s = format!("/org/freedesktop/login1/user/_{uid}");
    zbus::zvariant::ObjectPath::try_from(s).unwrap().into()
}

pub fn now_usec() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_micros() as u64)
        .unwrap_or(0)
}

/// Look up a username from /etc/passwd. Returns the UID's string form
/// on lookup failure so the daemon never panics on bad input.
pub fn lookup_username(uid: u32) -> String {
    let content = match std::fs::read_to_string("/etc/passwd") {
        Ok(c) => c,
        Err(_) => return uid.to_string(),
    };
    for line in content.lines() {
        let mut fields = line.split(':');
        let name = fields.next().unwrap_or("");
        let _passwd = fields.next();
        let uid_str = fields.next().unwrap_or("");
        if uid_str.parse::<u32>().ok() == Some(uid) {
            return name.to_string();
        }
    }
    uid.to_string()
}
