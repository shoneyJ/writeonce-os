// manager.rs — org.freedesktop.login1.Manager interface.
//
// Implements the subset of systemd-logind's Manager methods that
// i3more-lock + writeonce-login + the i3more session-aware applets
// require. Methods we don't need (CanSuspend, ScheduleShutdown,
// SetUserLinger, etc.) are deliberately absent — clients that probe
// for them get an UnknownMethod error, which is the documented
// fallback path.

use std::sync::{Arc, Mutex};

use log::{info, warn};
use zbus::object_server::SignalEmitter;
use zbus::zvariant::{OwnedFd, OwnedObjectPath};
use zbus::{fdo, interface};

use crate::inhibitor::{make_pipe_cloexec, InhibitorWatcher};
use crate::session::Session;
use crate::state::{
    lookup_username, now_usec, seat_object_path, session_object_path,
    user_object_path, AppState, Inhibitor, SessionInfo,
};

pub struct Manager {
    pub state: Arc<Mutex<AppState>>,
    pub watcher: Arc<InhibitorWatcher>,
}

impl Manager {
    pub fn new(state: Arc<Mutex<AppState>>, watcher: Arc<InhibitorWatcher>) -> Self {
        Self { state, watcher }
    }
}

/// Tuple shape returned by Manager.CreateSession matching the
/// systemd-logind ABI exactly. Clients (pam_systemd, our login)
/// destructure this.
type CreateSessionReply = (
    String,            // session id ("c1")
    OwnedObjectPath,   // session object path
    String,            // runtime path ($XDG_RUNTIME_DIR)
    OwnedFd,           // fifo fd — caller must hold open for session lifetime
    u32,               // session uid (echoed)
    String,            // seat id ("seat0")
    u32,               // VT number
    bool,              // existing? (true if CreateSession was called twice for the same uid+pid)
);

/// Tuple shape returned by Manager.ListSessions.
type SessionListEntry = (
    String,            // session id
    u32,               // uid
    String,            // user name
    String,            // seat
    OwnedObjectPath,   // session object path
);

/// Tuple shape returned by Manager.ListSeats.
type SeatListEntry = (String, OwnedObjectPath);

#[interface(name = "org.freedesktop.login1.Manager")]
impl Manager {
    // -----------------------------------------------------------------
    // Session lifecycle
    // -----------------------------------------------------------------

    #[allow(clippy::too_many_arguments)]
    async fn create_session(
        &self,
        uid: u32,
        pid: u32,
        service: String,
        session_type: String,
        class: String,
        desktop: String,
        seat_id: String,
        vtnr: u32,
        tty: String,
        display: String,
        remote: bool,
        remote_user: String,
        remote_host: String,
        _properties: Vec<(String, zbus::zvariant::OwnedValue)>,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
        #[zbus(object_server)] server: &zbus::object_server::ObjectServer,
    ) -> fdo::Result<CreateSessionReply> {
        let _ = (desktop, tty, remote_user, remote_host); // not surfaced in v1

        // Scoped lock so the MutexGuard is dropped before any .await.
        let (id, info) = {
            let mut st = self.state.lock().unwrap();
            let id = st.allocate_session_id();
            let info = SessionInfo {
                id: id.clone(),
                uid,
                user_name: lookup_username(uid),
                seat: if seat_id.is_empty() { "seat0".to_string() } else { seat_id },
                vtnr,
                display,
                remote,
                service: if service.is_empty() {
                    "writeonce-login".to_string()
                } else {
                    service
                },
                class: if class.is_empty() { "user".to_string() } else { class },
                session_type: if session_type.is_empty() {
                    "tty".to_string()
                } else {
                    session_type
                },
                state: "active".to_string(),
                idle_hint: false,
                idle_since: 0,
                leader_pid: pid,
                timestamp_usec: now_usec(),
            };
            st.sessions.insert(id.clone(), info.clone());
            (id, info)
        };

        info!("CreateSession id={id} uid={uid} pid={pid}");

        // Register a Session object at /org/freedesktop/login1/session/<id>.
        let session_iface = Session::new(self.state.clone(), id.clone());
        let path = session_object_path(&id);
        server
            .at(path.clone(), session_iface)
            .await
            .map_err(|e| fdo::Error::Failed(format!("at(session): {e}")))?;

        // Build the runtime dir path (we don't actually mkdir it here —
        // writeonce-login or systemd-tmpfiles equivalent does that).
        let runtime_path = format!("/run/user/{uid}");

        // Allocate the lifecycle FIFO. The caller is expected to hold
        // this FD open as long as the session is alive. (Currently we
        // do not yet watch it server-side for HUP — that's a Round 2f
        // item paralleling the inhibitor lifecycle work.)
        let (_read_std, write_std) = make_pipe_cloexec()
            .map_err(|e| fdo::Error::Failed(format!("pipe: {e}")))?;
        let write_zb: OwnedFd = write_std.into();

        // Signal: SessionNew(id, path)
        Self::session_new(&emitter, id.clone(), path.clone())
            .await
            .map_err(|e| fdo::Error::Failed(format!("emit SessionNew: {e}")))?;

        Ok((
            id,
            path,
            runtime_path,
            write_zb,
            uid,
            info.seat,
            vtnr,
            false,
        ))
    }

    async fn release_session(
        &self,
        session_id: String,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
        #[zbus(object_server)] server: &zbus::object_server::ObjectServer,
    ) -> fdo::Result<()> {
        let removed = {
            let mut st = self.state.lock().unwrap();
            st.sessions.remove(&session_id).is_some()
        };
        if !removed {
            return Err(fdo::Error::Failed(format!(
                "no such session: {session_id}"
            )));
        }

        let path = session_object_path(&session_id);
        let _ = server.remove::<Session, _>(path.clone()).await;

        Self::session_removed(&emitter, session_id, path)
            .await
            .map_err(|e| fdo::Error::Failed(format!("emit SessionRemoved: {e}")))?;

        Ok(())
    }

    // -----------------------------------------------------------------
    // Lookup
    // -----------------------------------------------------------------

    fn get_session(&self, session_id: String) -> fdo::Result<OwnedObjectPath> {
        let st = self.state.lock().unwrap();
        if st.sessions.contains_key(&session_id) {
            Ok(session_object_path(&session_id))
        } else {
            Err(fdo::Error::Failed(format!(
                "no such session: {session_id}"
            )))
        }
    }

    fn get_session_by_pid(&self, pid: u32) -> fdo::Result<OwnedObjectPath> {
        let st = self.state.lock().unwrap();
        if let Some(s) = st.find_session_by_pid(pid) {
            Ok(session_object_path(&s.id))
        } else {
            Err(fdo::Error::Failed(format!(
                "pid {pid} not in any session"
            )))
        }
    }

    /// Resolve "the session the D-Bus caller belongs to" by:
    ///   1. Asking the bus daemon for the sender's PID
    ///   2. Walking that PID up the process tree until we hit a known
    ///      session leader (state::find_session_by_pid does this)
    /// Matches systemd's behaviour.
    async fn get_current_session(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> fdo::Result<OwnedObjectPath> {
        let (_, pid) = resolve_sender(conn, &header)
            .await
            .map_err(|e| fdo::Error::Failed(format!("sender resolution: {e}")))?;
        let path = {
            let st = self.state.lock().unwrap();
            st.find_session_by_pid(pid)
                .map(|s| session_object_path(&s.id))
        };
        path.ok_or_else(|| {
            fdo::Error::Failed(format!("pid {pid} not associated with any session"))
        })
    }

    fn list_sessions(&self) -> fdo::Result<Vec<SessionListEntry>> {
        Ok(self.state.lock().unwrap().list_sessions_tuples())
    }

    fn list_seats(&self) -> fdo::Result<Vec<SeatListEntry>> {
        // We expose exactly one seat: seat0.
        Ok(vec![("seat0".to_string(), seat_object_path("seat0"))])
    }

    fn list_users(&self) -> fdo::Result<Vec<(u32, String, OwnedObjectPath)>> {
        let st = self.state.lock().unwrap();
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        for s in st.sessions.values() {
            if seen.insert(s.uid) {
                out.push((s.uid, s.user_name.clone(), user_object_path(s.uid)));
            }
        }
        Ok(out)
    }

    // -----------------------------------------------------------------
    // Lock / unlock signals
    // -----------------------------------------------------------------

    async fn lock_session(
        &self,
        session_id: String,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> fdo::Result<()> {
        let exists = self.state.lock().unwrap().sessions.contains_key(&session_id);
        if !exists {
            return Err(fdo::Error::Failed(format!(
                "no such session: {session_id}"
            )));
        }
        // Emit Lock signal on the per-session object so i3more-lock
        // (which subscribes to its own session's signals) wakes up.
        let path = session_object_path(&session_id);
        let session_emitter = SignalEmitter::new(emitter.connection(), path).unwrap();
        Session::lock_signal(&session_emitter)
            .await
            .map_err(|e| fdo::Error::Failed(format!("emit Lock: {e}")))?;
        Ok(())
    }

    async fn unlock_session(
        &self,
        session_id: String,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> fdo::Result<()> {
        let exists = self.state.lock().unwrap().sessions.contains_key(&session_id);
        if !exists {
            return Err(fdo::Error::Failed(format!(
                "no such session: {session_id}"
            )));
        }
        let path = session_object_path(&session_id);
        let session_emitter = SignalEmitter::new(emitter.connection(), path).unwrap();
        Session::unlock_signal(&session_emitter)
            .await
            .map_err(|e| fdo::Error::Failed(format!("emit Unlock: {e}")))?;
        Ok(())
    }

    // -----------------------------------------------------------------
    // Inhibitors
    // -----------------------------------------------------------------

    async fn inhibit(
        &self,
        what: String,
        who: String,
        why: String,
        mode: String,
        #[zbus(header)] header: zbus::message::Header<'_>,
        #[zbus(connection)] conn: &zbus::Connection,
    ) -> fdo::Result<OwnedFd> {
        // Validate mode.
        if mode != "block" && mode != "delay" {
            return Err(fdo::Error::InvalidArgs(format!(
                "mode must be 'block' or 'delay', got '{mode}'"
            )));
        }

        // Resolve caller uid + pid via the bus daemon's own API.
        let (uid, pid) = match resolve_sender(conn, &header).await {
            Ok(v) => v,
            Err(e) => {
                warn!("Inhibit: sender resolution failed: {e}");
                (0, 0)
            }
        };

        // Allocate inhibitor record + pipe before registering with
        // the watcher so the id↔fd map is consistent.
        let id = {
            let mut st = self.state.lock().unwrap();
            let id = st.allocate_inhibitor_id();
            st.inhibitors.push(Inhibitor {
                id,
                what: what.clone(),
                who: who.clone(),
                why: why.clone(),
                mode: mode.clone(),
                uid,
                pid,
            });
            id
        };

        // Daemon keeps the READ end (polled for EPOLLHUP). Caller
        // gets the WRITE end. When the caller closes their end, our
        // read-end gets HUP, the watcher thread removes the record.
        let (read_std, write_std) = make_pipe_cloexec()
            .map_err(|e| fdo::Error::Failed(format!("pipe: {e}")))?;
        self.watcher.register(id, read_std);

        info!("Inhibit registered: id={id} what={what} who={who} mode={mode} uid={uid} pid={pid}");
        Ok(write_std.into())
    }

    fn list_inhibitors(&self) -> fdo::Result<Vec<(String, String, String, String, u32, u32)>> {
        let st = self.state.lock().unwrap();
        Ok(st
            .inhibitors
            .iter()
            .map(|i| {
                (
                    i.what.clone(),
                    i.who.clone(),
                    i.why.clone(),
                    i.mode.clone(),
                    i.uid,
                    i.pid,
                )
            })
            .collect())
    }

    // -----------------------------------------------------------------
    // Properties
    // -----------------------------------------------------------------

    #[zbus(property)]
    fn n_current_sessions(&self) -> u32 {
        self.state.lock().unwrap().sessions.len() as u32
    }

    #[zbus(property)]
    fn preparing_for_shutdown(&self) -> bool {
        self.state.lock().unwrap().preparing_for_shutdown
    }

    #[zbus(property)]
    fn preparing_for_sleep(&self) -> bool {
        self.state.lock().unwrap().preparing_for_sleep
    }

    #[zbus(property)]
    fn block_inhibited(&self) -> String {
        let st = self.state.lock().unwrap();
        join_mode(&st.inhibitors, "block")
    }

    #[zbus(property)]
    fn delay_inhibited(&self) -> String {
        let st = self.state.lock().unwrap();
        join_mode(&st.inhibitors, "delay")
    }

    #[zbus(property)]
    fn idle_hint(&self) -> bool {
        let st = self.state.lock().unwrap();
        st.sessions.values().all(|s| s.idle_hint)
    }

    // -----------------------------------------------------------------
    // Power-management probes — answer honestly that we *can* do them
    // even though the actual reboot/shutdown is delegated to PID 1.
    // -----------------------------------------------------------------

    fn can_reboot(&self) -> fdo::Result<String> {
        Ok("yes".into())
    }

    fn can_power_off(&self) -> fdo::Result<String> {
        Ok("yes".into())
    }

    fn can_suspend(&self) -> fdo::Result<String> {
        // We don't yet implement suspend — kernel s2idle / deep sleep
        // hooks need wiring through writeonce-svc. Honest answer for v1.
        Ok("no".into())
    }

    fn can_hibernate(&self) -> fdo::Result<String> {
        Ok("no".into())
    }

    fn reboot(&self, _interactive: bool) -> fdo::Result<()> {
        warn!("Reboot() called — delegating to PID 1 via SIGTERM");
        // PID 1's signal loop interprets SIGTERM as orderly reboot
        // (writeonce-pid1's main.rs).
        unsafe { libc::kill(1, libc::SIGTERM) };
        Ok(())
    }

    fn power_off(&self, _interactive: bool) -> fdo::Result<()> {
        warn!("PowerOff() called — delegating to PID 1 via SIGUSR1");
        // We use SIGUSR1 as the "power-off" signal — PID 1 maps that
        // to LINUX_REBOOT_CMD_POWER_OFF. (See writeonce-pid1/src/main.rs.)
        unsafe { libc::kill(1, libc::SIGUSR1) };
        Ok(())
    }

    // -----------------------------------------------------------------
    // Signals
    // -----------------------------------------------------------------

    #[zbus(signal)]
    async fn session_new(emitter: &SignalEmitter<'_>, id: String, path: OwnedObjectPath) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn session_removed(emitter: &SignalEmitter<'_>, id: String, path: OwnedObjectPath) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn prepare_for_shutdown(emitter: &SignalEmitter<'_>, start: bool) -> zbus::Result<()>;

    #[zbus(signal)]
    async fn prepare_for_sleep(emitter: &SignalEmitter<'_>, start: bool) -> zbus::Result<()>;
}

// ---- helpers ---------------------------------------------------------------

fn join_mode(inhibitors: &[Inhibitor], mode: &str) -> String {
    let mut parts: Vec<&str> = inhibitors
        .iter()
        .filter(|i| i.mode == mode)
        .flat_map(|i| i.what.split(':'))
        .collect();
    parts.sort();
    parts.dedup();
    parts.join(":")
}

/// Resolve a D-Bus sender's (uid, pid) via the bus daemon's own
/// org.freedesktop.DBus API. Used by Inhibit + GetCurrentSession to
/// attribute calls to specific local processes.
async fn resolve_sender(
    conn: &zbus::Connection,
    header: &zbus::message::Header<'_>,
) -> zbus::Result<(u32, u32)> {
    let sender = header
        .sender()
        .ok_or_else(|| zbus::Error::Failure("no sender on message".into()))?
        .clone();
    let bus = zbus::fdo::DBusProxy::new(conn).await?;
    let pid = bus
        .get_connection_unix_process_id(sender.clone().into())
        .await?;
    let uid = bus.get_connection_unix_user(sender.into()).await?;
    Ok((uid, pid))
}
