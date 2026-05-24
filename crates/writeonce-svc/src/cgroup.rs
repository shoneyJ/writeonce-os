//! cgroup-v2 service-placement via `clone3(CLONE_INTO_CGROUP)`.
//!
//! See [`docs/learning/phase-4-cgroup-isolation.md`](../../docs/learning/phase-4-cgroup-isolation.md).
//! Round-2b implements:
//!   - `prepare_cgroup()` — create the unit's cgroup directory, write any
//!     resource limits, return a file descriptor to it (O_PATH).
//!   - `clone3_into_cgroup()` — the actual syscall wrapper.
//!
//! Not yet implemented (deferred to Round 2c):
//!   - inotify on `cgroup.events` for unpopulated-cgroup detection.
//!   - Cgroup teardown on service stop.

#![allow(dead_code)]

use std::ffi::CString;
use std::fs;
use std::io;
use std::os::fd::RawFd;

/// `CLONE_INTO_CGROUP` flag value from `<linux/sched.h>`.
pub const CLONE_INTO_CGROUP: u64 = 0x200000000;

/// Rust mirror of `struct clone_args` from `<linux/sched.h>`. Field order
/// is layout-critical — must match the kernel header byte-for-byte. New
/// fields are appended at the end by the kernel; the `size` argument to
/// `clone3` lets older callers pass a smaller struct.
#[repr(C)]
#[derive(Debug, Default)]
pub struct CloneArgs {
    pub flags:        u64,
    pub pidfd:        u64,
    pub child_tid:    u64,
    pub parent_tid:   u64,
    pub exit_signal:  u64,
    pub stack:        u64,
    pub stack_size:   u64,
    pub tls:          u64,
    pub set_tid:      u64,
    pub set_tid_size: u64,
    pub cgroup:       u64,
}

/// Spawn a child process directly into the target cgroup. Returns the
/// child's pid in the parent, 0 in the child.
///
/// # Safety
/// - `cgroup_fd` must be a valid O_PATH file descriptor to a cgroup-v2
///   directory the calling process has write access to.
/// - The child must follow async-signal-safe rules until `execve()`.
/// - The caller must be single-threaded (no other threads may hold
///   mutexes, allocators in inconsistent states, etc.).
pub unsafe fn clone3_into_cgroup(cgroup_fd: RawFd) -> io::Result<i64> {
    let args = CloneArgs {
        flags:       CLONE_INTO_CGROUP,
        exit_signal: libc::SIGCHLD as u64,
        cgroup:      cgroup_fd as u64,
        ..Default::default()
    };
    let rc = libc::syscall(
        libc::SYS_clone3,
        &args as *const _,
        std::mem::size_of::<CloneArgs>(),
    );
    if rc < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(rc)
    }
}

/// Resource limits to apply to a service cgroup before its first
/// process is spawned. `None` means "leave the kernel default in
/// place" (no `*.max` file written).
#[derive(Debug, Default, Clone)]
pub struct ResourceLimits {
    /// `memory.max` value in bytes (e.g. `Some(2 * 1024 * 1024 * 1024)`
    /// for 2 GiB). Above this, the cgroup is OOM-killed.
    pub memory_max: Option<u64>,
    /// `pids.max` value (count). Hard cap on processes in the cgroup.
    pub pids_max:   Option<u64>,
    /// `cpu.max` raw string (e.g. `"200000 100000"` for 200% of one CPU
    /// over a 100ms window). Passed through verbatim to the file.
    pub cpu_max:    Option<String>,
}

/// Create the unit's cgroup directory, apply limits, and return an
/// `O_PATH` fd suitable for `clone3(CLONE_INTO_CGROUP)`.
///
/// The path is created relative to `/sys/fs/cgroup/`. Caller passes the
/// suffix (e.g. `"wo.slice/sshd.service"`).
pub fn prepare_cgroup(rel_path: &str, limits: &ResourceLimits) -> io::Result<RawFd> {
    let abs = format!("/sys/fs/cgroup/{rel_path}");

    // mkdir -p
    fs::create_dir_all(&abs)?;

    // Apply each limit by writing to its *.max file. Errors here usually
    // mean the corresponding controller isn't enabled in the parent's
    // cgroup.subtree_control — we surface them so the supervisor can log.
    if let Some(b) = limits.memory_max {
        fs::write(format!("{abs}/memory.max"), b.to_string())?;
    }
    if let Some(p) = limits.pids_max {
        fs::write(format!("{abs}/pids.max"), p.to_string())?;
    }
    if let Some(c) = &limits.cpu_max {
        fs::write(format!("{abs}/cpu.max"), c)?;
    }

    // Open as O_PATH | O_DIRECTORY for clone3.
    let cpath = CString::new(abs.clone())
        .map_err(|_| io::Error::other("cgroup path contained NUL"))?;
    let fd = unsafe { libc::open(cpath.as_ptr(), libc::O_PATH | libc::O_DIRECTORY) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(fd)
}

/// Remove the cgroup directory. Called by the supervisor after a
/// service's processes have all exited.
///
/// Safe to call on a missing directory (returns `Ok`). Errors when the
/// cgroup is still populated.
pub fn remove_cgroup(rel_path: &str) -> io::Result<()> {
    let abs = format!("/sys/fs/cgroup/{rel_path}");
    match fs::remove_dir(&abs) {
        Ok(_) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}
