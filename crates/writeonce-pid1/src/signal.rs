//! signalfd + epoll plumbing and the PID 1 event loop.

use std::io;
use std::mem::MaybeUninit;
use std::os::fd::RawFd;
use std::time::Instant;

use crate::config::Config;

const SIGNALS: &[libc::c_int] = &[
    libc::SIGCHLD,
    libc::SIGTERM,
    libc::SIGINT,
    libc::SIGHUP,
];

/// Block the signals we care about and return a non-blocking signalfd that
/// delivers them as readable bytes.
pub fn install() -> io::Result<RawFd> {
    let mut mask: libc::sigset_t = unsafe { MaybeUninit::zeroed().assume_init() };
    unsafe { libc::sigemptyset(&mut mask) };
    for &sig in SIGNALS {
        unsafe { libc::sigaddset(&mut mask, sig) };
    }

    let rc = unsafe { libc::sigprocmask(libc::SIG_BLOCK, &mask, std::ptr::null_mut()) };
    if rc < 0 {
        return Err(io::Error::last_os_error());
    }

    let fd = unsafe {
        libc::signalfd(-1, &mask, libc::SFD_CLOEXEC | libc::SFD_NONBLOCK)
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(fd)
}

/// Block on epoll and process incoming signals until reboot.
pub fn event_loop(
    signal_fd: RawFd,
    initial_child: libc::pid_t,
    cfg: &Config,
) -> Result<(), Box<dyn std::error::Error>> {
    let epfd = unsafe { libc::epoll_create1(libc::EPOLL_CLOEXEC) };
    if epfd < 0 {
        return Err(io::Error::last_os_error().into());
    }

    let mut ev = libc::epoll_event {
        events: libc::EPOLLIN as u32,
        u64: 0,
    };
    let rc = unsafe { libc::epoll_ctl(epfd, libc::EPOLL_CTL_ADD, signal_fd, &mut ev) };
    if rc < 0 {
        return Err(io::Error::last_os_error().into());
    }

    let mut shutting_down: Option<Instant> = None;
    let mut child_pid = initial_child;

    let mut events: [libc::epoll_event; 4] =
        [libc::epoll_event { events: 0, u64: 0 }; 4];

    loop {
        let timeout_ms = match shutting_down {
            Some(start) => {
                let elapsed = start.elapsed();
                let grace = cfg.shutdown_grace();
                if elapsed >= grace {
                    return finish_shutdown(child_pid);
                }
                ((grace - elapsed).as_millis() as libc::c_int).max(1)
            }
            None => -1,
        };

        let n = unsafe {
            libc::epoll_wait(
                epfd,
                events.as_mut_ptr(),
                events.len() as i32,
                timeout_ms,
            )
        };
        if n < 0 {
            let e = io::Error::last_os_error();
            if e.raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            return Err(e.into());
        }

        for _ in 0..n {
            drain_signalfd(signal_fd, &mut child_pid, &mut shutting_down)?;
        }
    }
}

fn finish_shutdown(child_pid: libc::pid_t) -> Result<(), Box<dyn std::error::Error>> {
    if child_pid > 0 {
        unsafe { libc::kill(child_pid, libc::SIGKILL) };
    }
    unsafe { libc::sync() };
    println!("writeonce-pid1: rebooting");
    // Only fires if we actually are PID 1; otherwise reboot() returns EPERM
    // and we exit normally for development.
    let rc = unsafe { libc::reboot(libc::LINUX_REBOOT_CMD_RESTART) };
    if rc < 0 {
        let e = io::Error::last_os_error();
        eprintln!("writeonce-pid1: reboot() failed: {e}");
    }
    Ok(())
}

fn drain_signalfd(
    signal_fd: RawFd,
    child_pid: &mut libc::pid_t,
    shutting_down: &mut Option<Instant>,
) -> io::Result<()> {
    loop {
        let mut info: libc::signalfd_siginfo =
            unsafe { MaybeUninit::zeroed().assume_init() };
        let size = std::mem::size_of::<libc::signalfd_siginfo>();
        let n = unsafe {
            libc::read(
                signal_fd,
                &mut info as *mut _ as *mut libc::c_void,
                size,
            )
        };
        if n < 0 {
            let e = io::Error::last_os_error();
            // EAGAIN == EWOULDBLOCK on Linux; one arm suffices.
            if e.raw_os_error() == Some(libc::EAGAIN) {
                return Ok(());
            }
            return Err(e);
        }
        if n == 0 {
            return Ok(());
        }

        match info.ssi_signo as libc::c_int {
            libc::SIGCHLD => reap_children(child_pid),
            libc::SIGHUP => {
                println!("writeonce-pid1: SIGHUP (config reload not implemented)");
            }
            libc::SIGTERM | libc::SIGINT => {
                if shutting_down.is_none() {
                    println!(
                        "writeonce-pid1: shutdown requested; sending SIGTERM to child"
                    );
                    if *child_pid > 0 {
                        unsafe { libc::kill(*child_pid, libc::SIGTERM) };
                    }
                    *shutting_down = Some(Instant::now());
                }
            }
            other => println!("writeonce-pid1: unhandled signal {other}"),
        }
    }
}

fn reap_children(child_pid: &mut libc::pid_t) {
    loop {
        let mut status: libc::c_int = 0;
        let pid = unsafe { libc::waitpid(-1, &mut status, libc::WNOHANG) };
        if pid <= 0 {
            return;
        }
        println!("writeonce-pid1: reaped pid={pid} status={status}");
        if pid == *child_pid {
            *child_pid = 0;
        }
    }
}
