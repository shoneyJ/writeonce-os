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
//!   2. `setsid()` — detaches from the supervisor's controlling terminal
//!      (PID 1 hands us `/dev/tty1`) so services don't fight the login
//!      prompt for the tty or for terminal-directed signals.
//!   3. Redirects stdio: stdin ← `/dev/null`, stdout/stderr → the unit's
//!      per-service log file (`<log_dir>/<unit>.log`, opened by the parent;
//!      falls back to `/dev/null` if it can't be opened).
//!   4. Drops privileges to `User=`/`Group=` (`setgroups`/`setresgid`/
//!      `setresuid`) when they are not root.
//!   5. Splits exec-start on whitespace (good enough for v1; shell-style
//!      escaping comes later), builds argv + envp, and `execve()`s. On
//!      failure, prints the error and exits 127.
//!
//! Deferred:
//!   - `Type=forking` / `Type=notify` lifecycle hooks (all current services
//!     are `Type=simple`/`oneshot`; see docs/learning + phase-4 non-goals).

use std::ffi::CString;
use std::io;
use std::os::fd::RawFd;

use crate::cgroup;
use crate::config::ServiceSection;

pub struct SpawnRequest<'a> {
    /// Unit name (e.g. `"dbus.service"`); becomes the cgroup leaf dir.
    pub name:    &'a str,
    pub service: &'a ServiceSection,
    /// If `true`, skip cgroup placement and use plain `fork()`.
    pub fake:    bool,
    /// Directory for per-service log files (`<log_dir>/<name>.log`).
    pub log_dir: &'a str,
}

/// Spawn the service. Returns the child PID in the parent.
pub fn spawn(req: &SpawnRequest<'_>) -> io::Result<libc::pid_t> {
    // Resolve User=/Group= → numeric ids in the PARENT (getpwnam/getgrnam
    // touch /etc/passwd & lock — keep them out of the post-fork child, which
    // must stay async-signal-safe). `None` ⇒ root, no drop needed.
    let creds = resolve_creds(req.service)?;

    // Open the per-service log in the PARENT so the child only does the
    // async-signal-safe dup2. Best-effort: a log we can't open must not
    // block boot — fall back to /dev/null (logfd = -1) in the child.
    let logfd = match open_log(req.log_dir, req.name) {
        Ok(fd) => fd,
        Err(e) => {
            eprintln!("writeonce-svc: {}: cannot open log in {}: {e} (output → /dev/null)",
                      req.name, req.log_dir);
            -1
        }
    };

    let res = if req.fake {
        spawn_plain_fork(req, creds, logfd)
    } else {
        spawn_with_cgroup(req, creds, logfd)
    };

    // Parent: drop our copy of the log fd (the child kept its own).
    if logfd >= 0 {
        unsafe { libc::close(logfd) };
    }
    res
}

/// Resolve `User=`/`Group=` to `(uid, gid)`, or `None` when both are root
/// (the common case — no privilege syscalls needed).
fn resolve_creds(service: &ServiceSection) -> io::Result<Option<(u32, u32)>> {
    if service.user == "root" && service.group == "root" {
        return Ok(None);
    }
    let uid = uid_for_user(&service.user)?;
    let gid = gid_for_group(&service.group)?;
    Ok(Some((uid, gid)))
}

fn uid_for_user(name: &str) -> io::Result<u32> {
    let cname = CString::new(name).map_err(|_| io::Error::other("user contained NUL"))?;
    // Single-threaded parent: the static-storage return is safe to read now.
    let pw = unsafe { libc::getpwnam(cname.as_ptr()) };
    if pw.is_null() {
        return Err(io::Error::other(format!("unknown user: {name}")));
    }
    Ok(unsafe { (*pw).pw_uid })
}

fn gid_for_group(name: &str) -> io::Result<u32> {
    let cname = CString::new(name).map_err(|_| io::Error::other("group contained NUL"))?;
    let gr = unsafe { libc::getgrnam(cname.as_ptr()) };
    if gr.is_null() {
        return Err(io::Error::other(format!("unknown group: {name}")));
    }
    Ok(unsafe { (*gr).gr_gid })
}

/// Open `<log_dir>/<name>.log` append-only for the child's stdout/stderr.
/// `O_CLOEXEC` so the original fd vanishes at `execve` — the dup'd 1/2 stay.
fn open_log(log_dir: &str, name: &str) -> io::Result<RawFd> {
    let path = format!("{log_dir}/{name}.log");
    let cpath = CString::new(path).map_err(|_| io::Error::other("log path contained NUL"))?;
    let fd = unsafe {
        libc::open(
            cpath.as_ptr(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_APPEND | libc::O_CLOEXEC,
            0o644,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(fd)
}

fn spawn_plain_fork(req: &SpawnRequest<'_>, creds: Option<(u32, u32)>, logfd: RawFd)
    -> io::Result<libc::pid_t>
{
    let pid = unsafe { libc::fork() };
    if pid < 0 {
        return Err(io::Error::last_os_error());
    }
    if pid == 0 {
        // Child branch — never returns.
        child_exec(req, creds, logfd);
    }
    Ok(pid)
}

fn spawn_with_cgroup(req: &SpawnRequest<'_>, creds: Option<(u32, u32)>, logfd: RawFd)
    -> io::Result<libc::pid_t>
{
    let rel = format!("wo.slice/{}", req.name);
    let limits = cgroup::ResourceLimits::default();
    let cgroup_fd = cgroup::prepare_cgroup(&rel, &limits)?;

    // Safety: cgroup_fd is the O_PATH descriptor we just opened. We are
    // single-threaded (supervisor invariant — see
    // docs/learning/phase-4-concurrency-and-io-uring.md).
    let rc = unsafe { cgroup::clone3_into_cgroup(cgroup_fd) }?;
    if rc == 0 {
        // Child branch.
        child_exec(req, creds, logfd);
    }
    // Parent: close the cgroup fd (the kernel cloned it).
    unsafe { libc::close(cgroup_fd) };
    Ok(rc as libc::pid_t)
}

fn child_exec(req: &SpawnRequest<'_>, creds: Option<(u32, u32)>, logfd: RawFd) -> ! {
    // 1. Reset the signal mask the supervisor blocked.
    let mut mask: libc::sigset_t =
        unsafe { std::mem::MaybeUninit::zeroed().assume_init() };
    unsafe {
        libc::sigemptyset(&mut mask);
        libc::sigprocmask(libc::SIG_SETMASK, &mask, std::ptr::null_mut());
    }

    // 1a. New session: drop the controlling terminal (tty1) we inherited
    //     from the supervisor so we don't contend with the login prompt.
    unsafe { libc::setsid() };

    // 1b. Redirect stdio. stdin ← /dev/null; stdout/stderr → logfd (or
    //     /dev/null if the log couldn't be opened). Done while still root so
    //     the log fd's permissions don't matter; the open fd survives the
    //     privilege drop below.
    let devnull = CString::new("/dev/null").unwrap();
    let nullfd = unsafe { libc::open(devnull.as_ptr(), libc::O_RDWR) };
    let out_fd = if logfd >= 0 { logfd } else { nullfd };
    unsafe {
        if nullfd >= 0 {
            libc::dup2(nullfd, libc::STDIN_FILENO);
        }
        if out_fd >= 0 {
            libc::dup2(out_fd, libc::STDOUT_FILENO);
            libc::dup2(out_fd, libc::STDERR_FILENO);
        }
        if nullfd > 2 {
            libc::close(nullfd);
        }
        if logfd > 2 {
            libc::close(logfd);
        }
    }

    // 1c. Drop privileges if User=/Group= is not root. Order matters:
    //     shed root's supplementary groups, then gid, then uid (after uid
    //     we can no longer setgid). setgroups also removes inherited groups
    //     a service shouldn't keep.
    if let Some((uid, gid)) = creds {
        let g: [libc::gid_t; 1] = [gid];
        unsafe {
            if libc::setgroups(1, g.as_ptr()) != 0 {
                eprintln!("writeonce-svc(child): setgroups({gid}) failed: {}",
                          io::Error::last_os_error());
                libc::_exit(127);
            }
            if libc::setresgid(gid, gid, gid) != 0 {
                eprintln!("writeonce-svc(child): setresgid({gid}) failed: {}",
                          io::Error::last_os_error());
                libc::_exit(127);
            }
            if libc::setresuid(uid, uid, uid) != 0 {
                eprintln!("writeonce-svc(child): setresuid({uid}) failed: {}",
                          io::Error::last_os_error());
                libc::_exit(127);
            }
        }
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
