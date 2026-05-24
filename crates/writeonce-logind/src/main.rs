// writeonce-logind — minimal logind D-Bus shim.
//
// Single-process daemon. Opens the system D-Bus, claims the
// org.freedesktop.login1 well-known name, and registers three object
// trees:
//
//   /org/freedesktop/login1                       Manager interface
//   /org/freedesktop/login1/seat/seat0            Seat interface
//   /org/freedesktop/login1/session/<id>          Session interface (per-session)
//
// Spawned by writeonce-svc as a system service. Requires dbus.service
// to be up.

use std::sync::{Arc, Mutex};

use log::{error, info};
use signal_hook::consts::{SIGINT, SIGTERM};
use signal_hook::iterator::Signals;
use zbus::blocking::connection;

mod inhibitor;
mod manager;
mod seat;
mod session;
mod state;

use inhibitor::InhibitorWatcher;
use manager::Manager;
use seat::Seat;
use state::AppState;

const SERVICE_NAME: &str = "org.freedesktop.login1";
const MANAGER_PATH: &str = "/org/freedesktop/login1";
const SEAT_PATH: &str = "/org/freedesktop/login1/seat/seat0";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // RUST_LOG controls verbosity. Default to info-level.
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("info"),
    )
    .format_timestamp(None)
    .init();

    info!("writeonce-logind starting");

    let app_state = Arc::new(Mutex::new(AppState::new()));

    // Spawn the inhibitor watcher first so it's ready before any
    // CreateSession / Inhibit calls arrive over the bus.
    let inhibitor_watcher = InhibitorWatcher::new()?;
    inhibitor_watcher.spawn(app_state.clone());

    // Build the connection. system bus requires the daemon to be
    // running with appropriate dbus-1 policy permitting it to own the
    // name (see examples/dbus-policy.conf).
    let manager_iface = Manager::new(app_state.clone(), inhibitor_watcher);
    let seat_iface = Seat::new(app_state.clone(), "seat0".to_string());

    let conn = connection::Builder::system()?
        .name(SERVICE_NAME)?
        .serve_at(MANAGER_PATH, manager_iface)?
        .serve_at(SEAT_PATH, seat_iface)?
        .build()?;

    info!("D-Bus name claimed: {SERVICE_NAME}");
    info!("Listening at {MANAGER_PATH} and {SEAT_PATH}");

    // Hold the connection alive and wait for SIGTERM / SIGINT.
    let mut signals = Signals::new([SIGINT, SIGTERM])?;
    for sig in signals.forever() {
        info!("received signal {sig}, shutting down");
        break;
    }

    // Drop the connection to release the well-known name cleanly.
    drop(conn);
    info!("writeonce-logind exited cleanly");
    Ok(())
}

/// Hook to make this code observable in cargo test runs without
/// needing a real D-Bus.
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_session_lifecycle() {
        let mut st = AppState::new();
        let id = st.allocate_session_id();
        st.sessions.insert(
            id.clone(),
            crate::state::SessionInfo {
                id: id.clone(),
                uid: 1000,
                user_name: "test".into(),
                seat: "seat0".into(),
                vtnr: 1,
                display: ":0".into(),
                remote: false,
                service: "writeonce-login".into(),
                class: "user".into(),
                session_type: "tty".into(),
                state: "active".into(),
                idle_hint: false,
                idle_since: 0,
                leader_pid: 0,
                timestamp_usec: 0,
            },
        );
        assert_eq!(st.sessions.len(), 1);
        let listed = st.list_sessions_tuples();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].0, id);
        assert_eq!(listed[0].1, 1000);
    }

    #[test]
    fn inhibitor_id_monotonic() {
        let mut st = AppState::new();
        assert_eq!(st.allocate_inhibitor_id(), 1);
        assert_eq!(st.allocate_inhibitor_id(), 2);
        assert_eq!(st.allocate_inhibitor_id(), 3);
    }

    #[test]
    fn session_id_format() {
        let mut st = AppState::new();
        assert_eq!(st.allocate_session_id(), "c1");
        assert_eq!(st.allocate_session_id(), "c2");
    }
}

/// Best-effort error logging hook used inside D-Bus method
/// implementations that want to record failure paths.
#[allow(dead_code)]
fn log_dbus_error(context: &str, e: &dyn std::error::Error) {
    error!("{context}: {e}");
}
