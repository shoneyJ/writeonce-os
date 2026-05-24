//! signalfd + epoll plumbing + the supervisor's main event loop.

use std::io;
use std::mem::MaybeUninit;
use std::os::fd::RawFd;
use std::time::{Duration, Instant};

use crate::control::{self, ControlListener};
use crate::state::SupervisorState;

const SIGNALS: &[libc::c_int] = &[
    libc::SIGCHLD,
    libc::SIGTERM,
    libc::SIGINT,
    libc::SIGHUP,
];

const KEY_SIGNAL_FD:    u64 = 1;
const KEY_LISTENER_FD:  u64 = 2;

/// Block our signals and create a non-blocking signalfd that delivers them.
pub fn install() -> io::Result<RawFd> {
    let mut mask: libc::sigset_t = unsafe { MaybeUninit::zeroed().assume_init() };
    unsafe { libc::sigemptyset(&mut mask) };
    for &sig in SIGNALS {
        unsafe { libc::sigaddset(&mut mask, sig) };
    }
    let rc = unsafe { libc::sigprocmask(libc::SIG_BLOCK, &mask, std::ptr::null_mut()) };
    if rc < 0 { return Err(io::Error::last_os_error()); }

    let fd = unsafe { libc::signalfd(-1, &mask, libc::SFD_CLOEXEC | libc::SFD_NONBLOCK) };
    if fd < 0 { return Err(io::Error::last_os_error()); }
    Ok(fd)
}

/// Main event loop. Returns when shutdown is requested and all child
/// processes have been reaped.
pub fn event_loop(
    signal_fd: RawFd,
    listener:  Option<&ControlListener>,
    state:     &mut SupervisorState,
) -> io::Result<()> {
    let epfd = unsafe { libc::epoll_create1(libc::EPOLL_CLOEXEC) };
    if epfd < 0 { return Err(io::Error::last_os_error()); }

    register(epfd, signal_fd, KEY_SIGNAL_FD)?;
    if let Some(l) = listener {
        register(epfd, l.as_raw_fd(), KEY_LISTENER_FD)?;
    }

    let mut events: [libc::epoll_event; 8] =
        [libc::epoll_event { events: 0, u64: 0 }; 8];

    loop {
        let timeout_ms = compute_timeout(state);
        let n = unsafe {
            libc::epoll_wait(epfd, events.as_mut_ptr(), events.len() as i32, timeout_ms)
        };
        if n < 0 {
            let e = io::Error::last_os_error();
            if e.raw_os_error() == Some(libc::EINTR) { continue; }
            return Err(e);
        }

        for ev in &events[..(n as usize)] {
            match ev.u64 {
                KEY_SIGNAL_FD   => drain_signalfd(signal_fd, state)?,
                KEY_LISTENER_FD => {
                    if let Some(l) = listener {
                        if control::handle_ready(l, state) {
                            state.initiate_shutdown();
                        }
                    }
                }
                _ => {}
            }
        }

        // Whether or not epoll fired, the timer may be due.
        state.fire_due_restarts();

        if state.shutting_down && state.all_units_quiet() {
            return Ok(());
        }
    }
}

fn register(epfd: RawFd, fd: RawFd, key: u64) -> io::Result<()> {
    let mut ev = libc::epoll_event {
        events: libc::EPOLLIN as u32,
        u64: key,
    };
    let rc = unsafe { libc::epoll_ctl(epfd, libc::EPOLL_CTL_ADD, fd, &mut ev) };
    if rc < 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
}

/// epoll_wait timeout in milliseconds. -1 = infinite (no pending restart).
fn compute_timeout(state: &SupervisorState) -> libc::c_int {
    let Some(when) = state.next_restart_deadline() else {
        // No pending restarts; during shutdown, poll quickly so we don't
        // miss the "all quiet" check.
        return if state.shutting_down { 100 } else { -1 };
    };
    let now = Instant::now();
    if when <= now {
        return 0;
    }
    let dt = when - now;
    // Cap to i32::MAX-ish to avoid overflow; epoll_wait accepts up to
    // i32::MAX ms ≈ 24 days.
    dt.min(Duration::from_millis(60_000)).as_millis() as libc::c_int
}

fn drain_signalfd(signal_fd: RawFd, state: &mut SupervisorState) -> io::Result<()> {
    loop {
        let mut info: libc::signalfd_siginfo =
            unsafe { MaybeUninit::zeroed().assume_init() };
        let size = std::mem::size_of::<libc::signalfd_siginfo>();
        let n = unsafe {
            libc::read(signal_fd, &mut info as *mut _ as *mut libc::c_void, size)
        };
        if n < 0 {
            let e = io::Error::last_os_error();
            if e.raw_os_error() == Some(libc::EAGAIN) { return Ok(()); }
            return Err(e);
        }
        if n == 0 { return Ok(()); }

        match info.ssi_signo as libc::c_int {
            libc::SIGCHLD => reap(state),
            libc::SIGHUP  => println!("writeonce-svc: SIGHUP (reload not implemented)"),
            libc::SIGTERM | libc::SIGINT => {
                if !state.shutting_down {
                    state.initiate_shutdown();
                }
            }
            other => println!("writeonce-svc: unhandled signal {other}"),
        }
    }
}

fn reap(state: &mut SupervisorState) {
    loop {
        let mut status: libc::c_int = 0;
        let pid = unsafe { libc::waitpid(-1, &mut status, libc::WNOHANG) };
        if pid <= 0 { return; }
        state.on_child_exit(pid, status);
    }
}
