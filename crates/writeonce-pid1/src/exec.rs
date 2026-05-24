//! fork + execve a placeholder service (defaults to `/bin/sh` on tty1).
//!
//! After Phase 4 lands, this module is replaced by a supervisor-spawn path.

use std::ffi::CString;
use std::io;
use std::os::fd::RawFd;

use crate::config::Config;

pub fn spawn_placeholder(cfg: &Config) -> io::Result<libc::pid_t> {
    let pid = unsafe { libc::fork() };
    if pid < 0 {
        return Err(io::Error::last_os_error());
    }

    if pid == 0 {
        // child branch — never returns on success
        if let Err(e) = child_setup(cfg) {
            eprintln!("writeonce-pid1 (child): exec setup failed: {e}");
        }
        unsafe { libc::_exit(127) };
    }

    Ok(pid)
}

fn child_setup(cfg: &Config) -> io::Result<()> {
    // Become a new session and process-group leader so the tty becomes the
    // controlling terminal.
    unsafe { libc::setsid() };

    // Open the configured tty and dup it to stdin/stdout/stderr.
    let tty_c = CString::new(cfg.tty.as_bytes()).map_err(io::Error::other)?;
    let fd = unsafe { libc::open(tty_c.as_ptr(), libc::O_RDWR) };
    if fd >= 0 {
        for target in 0..3 {
            unsafe { libc::dup2(fd, target as RawFd) };
        }
        if fd >= 3 {
            unsafe { libc::close(fd) };
        }
        // Claim controlling terminal.
        unsafe { libc::ioctl(0, libc::TIOCSCTTY as _, 0i32) };
    }
    // If tty open failed (development on workstation), child inherits parent's fds.

    // Unblock all signals in the child — the inherited block mask from PID 1
    // would otherwise suppress every signal for the child too.
    let mut mask: libc::sigset_t =
        unsafe { std::mem::MaybeUninit::zeroed().assume_init() };
    unsafe {
        libc::sigemptyset(&mut mask);
        libc::sigprocmask(libc::SIG_SETMASK, &mask, std::ptr::null_mut());
    }

    // Build argv as null-terminated C strings.
    let prog = CString::new(cfg.child.as_bytes()).map_err(io::Error::other)?;
    let argv: Vec<CString> = cfg
        .child_args
        .iter()
        .map(|a| CString::new(a.as_bytes()).map_err(io::Error::other))
        .collect::<io::Result<_>>()?;
    let mut argv_ptrs: Vec<*const libc::c_char> =
        argv.iter().map(|c| c.as_ptr()).collect();
    argv_ptrs.push(std::ptr::null());

    // Minimal environment for the placeholder shell.
    let env_strs: [&[u8]; 3] = [
        b"PATH=/usr/bin:/usr/sbin:/bin:/sbin\0",
        b"HOME=/root\0",
        b"TERM=linux\0",
    ];
    let mut envp: Vec<*const libc::c_char> = env_strs
        .iter()
        .map(|s| s.as_ptr() as *const libc::c_char)
        .collect();
    envp.push(std::ptr::null());

    unsafe { libc::execve(prog.as_ptr(), argv_ptrs.as_ptr(), envp.as_ptr()) };
    Err(io::Error::last_os_error())
}
