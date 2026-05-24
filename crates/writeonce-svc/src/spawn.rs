//! Spawn a service process. Two modes:
//!
//!   - `fake = true`:  plain `fork(2)` — for development on the workstation
//!                     where `/sys/fs/cgroup/wo.slice/...` is not writable.
//!   - `fake = false`: `clone3(CLONE_INTO_CGROUP)` placing the child in
//!                     `/sys/fs/cgroup/wo.slice/<unit-name>/`.
//!
//! Either way the child:
//!   1. Unblocks signals (PID 1 / supervisor's blocked mask is inherited
//!      via clone/fork; the child does its own signal work).
//!   2. Splits exec-start on whitespace (good enough for v1; shell-style
//!      escaping comes later).
//!   3. Builds argv + envp as null-terminated arrays.
//!   4. `execve()`. On failure, prints the error and exits 127.
//!
//! Deferred to Round 2c:
//!   - setresuid/setresgid based on User=/Group= (needs getpwnam_r).
//!   - Stdout/stderr pipe → supervisor journal.
//!   - `Type=forking` / `Type=notify` lifecycle hooks.

use std::ffi::CString;
use std::io;

use crate::cgroup;
use crate::config::ServiceSection;

pub struct SpawnRequest<'a> {
    /// Unit name (e.g. `"dbus.service"`); becomes the cgroup leaf dir.
    pub name:    &'a str,
    pub service: &'a ServiceSection,
    /// If `true`, skip cgroup placement and use plain `fork()`.
    pub fake:    bool,
}

/// Spawn the service. Returns the child PID in the parent.
pub fn spawn(req: &SpawnRequest<'_>) -> io::Result<libc::pid_t> {
    if req.fake {
        spawn_plain_fork(req)
    } else {
        spawn_with_cgroup(req)
    }
}

fn spawn_plain_fork(req: &SpawnRequest<'_>) -> io::Result<libc::pid_t> {
    let pid = unsafe { libc::fork() };
    if pid < 0 {
        return Err(io::Error::last_os_error());
    }
    if pid == 0 {
        // Child branch — never returns.
        child_exec(req);
    }
    Ok(pid)
}

fn spawn_with_cgroup(req: &SpawnRequest<'_>) -> io::Result<libc::pid_t> {
    let rel = format!("wo.slice/{}", req.name);
    let limits = cgroup::ResourceLimits::default();
    let cgroup_fd = cgroup::prepare_cgroup(&rel, &limits)?;

    // Safety: cgroup_fd is the O_PATH descriptor we just opened. We are
    // single-threaded (supervisor invariant — see
    // docs/learning/phase-4-concurrency-and-io-uring.md).
    let rc = unsafe { cgroup::clone3_into_cgroup(cgroup_fd) }?;
    if rc == 0 {
        // Child branch.
        child_exec(req);
    }
    // Parent: close the cgroup fd (the kernel cloned it).
    unsafe { libc::close(cgroup_fd) };
    Ok(rc as libc::pid_t)
}

fn child_exec(req: &SpawnRequest<'_>) -> ! {
    // 1. Reset the signal mask the supervisor blocked.
    let mut mask: libc::sigset_t =
        unsafe { std::mem::MaybeUninit::zeroed().assume_init() };
    unsafe {
        libc::sigemptyset(&mut mask);
        libc::sigprocmask(libc::SIG_SETMASK, &mask, std::ptr::null_mut());
    }

    // 2. Build argv: split exec-start on whitespace. Trivial for v1.
    let parts: Vec<&str> = req.service.exec_start.split_whitespace().collect();
    if parts.is_empty() {
        eprintln!("writeonce-svc(child): exec-start is empty");
        unsafe { libc::_exit(127) };
    }
    let prog = match CString::new(parts[0]) {
        Ok(c) => c,
        Err(_) => {
            eprintln!("writeonce-svc(child): exec-start contained NUL");
            unsafe { libc::_exit(127) };
        }
    };
    let argv: Vec<CString> = match parts.iter()
        .map(|s| CString::new(*s))
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(v) => v,
        Err(_) => {
            eprintln!("writeonce-svc(child): argv contained NUL");
            unsafe { libc::_exit(127) };
        }
    };
    let mut argv_ptrs: Vec<*const libc::c_char> =
        argv.iter().map(|c| c.as_ptr()).collect();
    argv_ptrs.push(std::ptr::null());

    // 3. Build envp from service.environment, with a minimal fallback.
    let env: Vec<CString> = if req.service.environment.is_empty() {
        vec![
            CString::new("PATH=/usr/bin:/usr/sbin:/bin:/sbin").unwrap(),
            CString::new("TERM=linux").unwrap(),
        ]
    } else {
        req.service.environment.iter()
            .filter_map(|s| CString::new(s.as_str()).ok())
            .collect()
    };
    let mut envp: Vec<*const libc::c_char> = env.iter().map(|c| c.as_ptr()).collect();
    envp.push(std::ptr::null());

    // 4. execve.
    unsafe {
        libc::execve(prog.as_ptr(), argv_ptrs.as_ptr(), envp.as_ptr());
    }
    let err = io::Error::last_os_error();
    eprintln!("writeonce-svc(child): execve {} failed: {}", parts[0], err);
    unsafe { libc::_exit(127) };
}
