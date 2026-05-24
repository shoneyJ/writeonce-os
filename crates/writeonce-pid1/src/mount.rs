//! Mount the essential pseudo-filesystems for a viable userspace.
//!
//! Idempotent: a second mount of an already-mounted target returns EBUSY,
//! which we silently swallow so repeated invocations of `mount_essentials()`
//! (e.g. across PID 1 restarts) are safe.

use std::ffi::CString;
use std::io;

struct MountSpec {
    source: &'static str,
    target: &'static str,
    fstype: &'static str,
    flags:  libc::c_ulong,
    data:   Option<&'static str>,
}

const MOUNTS: &[MountSpec] = &[
    MountSpec {
        source: "proc", target: "/proc", fstype: "proc",
        flags: libc::MS_NOSUID | libc::MS_NOEXEC | libc::MS_NODEV,
        data: None,
    },
    MountSpec {
        source: "sysfs", target: "/sys", fstype: "sysfs",
        flags: libc::MS_NOSUID | libc::MS_NOEXEC | libc::MS_NODEV,
        data: None,
    },
    MountSpec {
        source: "devtmpfs", target: "/dev", fstype: "devtmpfs",
        flags: libc::MS_NOSUID,
        data: Some("mode=755"),
    },
    MountSpec {
        source: "devpts", target: "/dev/pts", fstype: "devpts",
        flags: libc::MS_NOSUID | libc::MS_NOEXEC,
        data: Some("gid=5,mode=620"),
    },
    MountSpec {
        source: "tmpfs", target: "/run", fstype: "tmpfs",
        flags: libc::MS_NOSUID | libc::MS_NODEV,
        data: Some("mode=755"),
    },
    MountSpec {
        source: "cgroup2", target: "/sys/fs/cgroup", fstype: "cgroup2",
        flags: libc::MS_NOSUID | libc::MS_NOEXEC | libc::MS_NODEV,
        data: None,
    },
];

pub fn mount_essentials() -> io::Result<()> {
    for spec in MOUNTS {
        let _ = std::fs::create_dir_all(spec.target);
        do_mount(spec)?;
        println!("writeonce-pid1: mount {} on {}", spec.fstype, spec.target);
    }
    Ok(())
}

fn do_mount(spec: &MountSpec) -> io::Result<()> {
    let source = CString::new(spec.source).unwrap();
    let target = CString::new(spec.target).unwrap();
    let fstype = CString::new(spec.fstype).unwrap();
    let data = spec.data.map(|d| CString::new(d).unwrap());

    let data_ptr: *const libc::c_void = data
        .as_ref()
        .map(|c| c.as_ptr() as *const libc::c_void)
        .unwrap_or(std::ptr::null());

    let rc = unsafe {
        libc::mount(
            source.as_ptr(),
            target.as_ptr(),
            fstype.as_ptr(),
            spec.flags,
            data_ptr,
        )
    };

    if rc < 0 {
        let err = io::Error::last_os_error();
        if err.raw_os_error() == Some(libc::EBUSY) {
            // Already mounted — fine.
            return Ok(());
        }
        return Err(err);
    }
    Ok(())
}
