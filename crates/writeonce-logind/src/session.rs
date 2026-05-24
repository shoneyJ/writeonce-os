// session.rs — org.freedesktop.login1.Session interface.
//
// One instance registered per /org/freedesktop/login1/session/<id>.

use std::os::fd::AsRawFd;
use std::sync::{Arc, Mutex};

use log::info;
use zbus::object_server::SignalEmitter;
use zbus::{fdo, interface};

use crate::state::{now_usec, seat_object_path, user_object_path, AppState};

/// VT_ACTIVATE from <linux/vt.h>. Switches the active virtual terminal.
const VT_ACTIVATE: libc::c_ulong = 0x5606;

fn activate_vt(vtnr: u32) -> std::io::Result<()> {
    // Open /dev/tty0 (the active console). The kernel routes
    // VT_ACTIVATE ioctls on this fd to the VT subsystem. Requires
    // CAP_SYS_TTY_CONFIG — running as root gives us that.
    let tty0 = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open("/dev/tty0")?;
    let rc = unsafe { libc::ioctl(tty0.as_raw_fd(), VT_ACTIVATE, vtnr as libc::c_ulong) };
    if rc != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

pub struct Session {
    pub state: Arc<Mutex<AppState>>,
    pub session_id: String,
}

impl Session {
    pub fn new(state: Arc<Mutex<AppState>>, session_id: String) -> Self {
        Self { state, session_id }
    }
}

#[interface(name = "org.freedesktop.login1.Session")]
impl Session {
    // -----------------------------------------------------------------
    // Methods
    // -----------------------------------------------------------------

    async fn lock(
        &self,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> fdo::Result<()> {
        info!("Session({}).Lock", self.session_id);
        Self::lock_signal(&emitter)
            .await
            .map_err(|e| fdo::Error::Failed(format!("emit Lock: {e}")))
    }

    async fn unlock(
        &self,
        #[zbus(signal_emitter)] emitter: SignalEmitter<'_>,
    ) -> fdo::Result<()> {
        info!("Session({}).Unlock", self.session_id);
        Self::unlock_signal(&emitter)
            .await
            .map_err(|e| fdo::Error::Failed(format!("emit Unlock: {e}")))
    }

    async fn activate(&self) -> fdo::Result<()> {
        let vtnr = {
            let st = self.state.lock().unwrap();
            st.sessions
                .get(&self.session_id)
                .map(|s| s.vtnr)
                .ok_or_else(|| {
                    fdo::Error::Failed(format!("no such session: {}", self.session_id))
                })?
        };
        if vtnr == 0 {
            return Err(fdo::Error::Failed(format!(
                "session {} has no VT (probably remote)",
                self.session_id
            )));
        }
        activate_vt(vtnr).map_err(|e| {
            fdo::Error::Failed(format!("VT_ACTIVATE({vtnr}) failed: {e}"))
        })?;
        info!("Session({}).Activate switched to VT {vtnr}", self.session_id);
        Ok(())
    }

    fn terminate(&self) -> fdo::Result<()> {
        // Send SIGTERM to the session leader. Real logind also walks the
        // session's cgroup; we don't do that yet.
        let st = self.state.lock().unwrap();
        if let Some(s) = st.sessions.get(&self.session_id) {
            let pid = s.leader_pid;
            drop(st);
            unsafe { libc::kill(pid as i32, libc::SIGTERM) };
            info!("Session({}).Terminate sent SIGTERM to leader pid {pid}", self.session_id);
            Ok(())
        } else {
            Err(fdo::Error::Failed(format!(
                "no such session: {}",
                self.session_id
            )))
        }
    }

    fn set_idle_hint(&self, idle: bool) -> fdo::Result<()> {
        let mut st = self.state.lock().unwrap();
        if let Some(s) = st.sessions.get_mut(&self.session_id) {
            s.idle_hint = idle;
            s.idle_since = if idle { now_usec() } else { 0 };
            Ok(())
        } else {
            Err(fdo::Error::Failed(format!(
                "no such session: {}",
                self.session_id
            )))
        }
    }

    // -----------------------------------------------------------------
    // Properties
    // -----------------------------------------------------------------

    #[zbus(property)]
    fn id(&self) -> String {
        self.session_id.clone()
    }

    #[zbus(property)]
    fn user(&self) -> (u32, zbus::zvariant::OwnedObjectPath) {
        let st = self.state.lock().unwrap();
        let uid = st
            .sessions
            .get(&self.session_id)
            .map(|s| s.uid)
            .unwrap_or(0);
        (uid, user_object_path(uid))
    }

    #[zbus(property)]
    fn name(&self) -> String {
        self.state
            .lock()
            .unwrap()
            .sessions
            .get(&self.session_id)
            .map(|s| s.user_name.clone())
            .unwrap_or_default()
    }

    #[zbus(property)]
    fn timestamp(&self) -> u64 {
        self.state
            .lock()
            .unwrap()
            .sessions
            .get(&self.session_id)
            .map(|s| s.timestamp_usec / 1_000_000)
            .unwrap_or(0)
    }

    #[zbus(property)]
    fn timestamp_monotonic(&self) -> u64 {
        // Approximation — real logind tracks CLOCK_MONOTONIC separately.
        self.state
            .lock()
            .unwrap()
            .sessions
            .get(&self.session_id)
            .map(|s| s.timestamp_usec)
            .unwrap_or(0)
    }

    #[zbus(property, name = "VTNr")]
    fn vtnr(&self) -> u32 {
        self.state
            .lock()
            .unwrap()
            .sessions
            .get(&self.session_id)
            .map(|s| s.vtnr)
            .unwrap_or(0)
    }

    #[zbus(property)]
    fn seat(&self) -> (String, zbus::zvariant::OwnedObjectPath) {
        let st = self.state.lock().unwrap();
        let seat = st
            .sessions
            .get(&self.session_id)
            .map(|s| s.seat.clone())
            .unwrap_or_else(|| "seat0".into());
        let path = seat_object_path(&seat);
        (seat, path)
    }

    #[zbus(property)]
    fn display(&self) -> String {
        self.state
            .lock()
            .unwrap()
            .sessions
            .get(&self.session_id)
            .map(|s| s.display.clone())
            .unwrap_or_default()
    }

    #[zbus(property)]
    fn remote(&self) -> bool {
        self.state
            .lock()
            .unwrap()
            .sessions
            .get(&self.session_id)
            .map(|s| s.remote)
            .unwrap_or(false)
    }

    #[zbus(property)]
    fn service(&self) -> String {
        self.state
            .lock()
            .unwrap()
            .sessions
            .get(&self.session_id)
            .map(|s| s.service.clone())
            .unwrap_or_default()
    }

    #[zbus(property, name = "Type")]
    fn session_type(&self) -> String {
        self.state
            .lock()
            .unwrap()
            .sessions
            .get(&self.session_id)
            .map(|s| s.session_type.clone())
            .unwrap_or_default()
    }

    #[zbus(property)]
    fn class(&self) -> String {
        self.state
            .lock()
            .unwrap()
            .sessions
            .get(&self.session_id)
            .map(|s| s.class.clone())
            .unwrap_or_default()
    }

    #[zbus(property)]
    fn state(&self) -> String {
        self.state
            .lock()
            .unwrap()
            .sessions
            .get(&self.session_id)
            .map(|s| s.state.clone())
            .unwrap_or_default()
    }

    #[zbus(property)]
    fn active(&self) -> bool {
        self.state
            .lock()
            .unwrap()
            .sessions
            .get(&self.session_id)
            .map(|s| s.state == "active")
            .unwrap_or(false)
    }

    #[zbus(property)]
    fn idle_hint(&self) -> bool {
        self.state
            .lock()
            .unwrap()
            .sessions
            .get(&self.session_id)
            .map(|s| s.idle_hint)
            .unwrap_or(false)
    }

    #[zbus(property)]
    fn idle_since_hint(&self) -> u64 {
        self.state
            .lock()
            .unwrap()
            .sessions
            .get(&self.session_id)
            .map(|s| s.idle_since)
            .unwrap_or(0)
    }

    #[zbus(property, name = "Leader")]
    fn leader(&self) -> u32 {
        self.state
            .lock()
            .unwrap()
            .sessions
            .get(&self.session_id)
            .map(|s| s.leader_pid)
            .unwrap_or(0)
    }

    // -----------------------------------------------------------------
    // Signals — emitted by Manager.LockSession / UnlockSession too.
    // -----------------------------------------------------------------

    #[zbus(signal)]
    pub async fn lock_signal(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;

    #[zbus(signal)]
    pub async fn unlock_signal(emitter: &SignalEmitter<'_>) -> zbus::Result<()>;
}
