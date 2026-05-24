//! Tiny wrappers around `mount(2)` and `umount2(2)` for inside the
//! initramfs. We re-implement the small surface rather than depend on
//! `nix` to keep the binary minimal.

use std::ffi::CString;
use std::io;

/// Mount the in-kernel pseudo-filesystems the initramfs userland needs.
/// Idempotent: `EBUSY` is swallowed so re-runs don't fail.
pub fn essentials() -> io::Result<()> {
    mount("proc",     "/proc",     "proc",     0, None)?;
    mount("sysfs",    "/sys",      "sysfs",    0, None)?;
    mount("devtmpfs", "/dev",      "devtmpfs", 0, Some("mode=755"))?;
    Ok(())
}

pub fn mount(
    source: &str,
    target: &str,
    fstype: &str,
    flags:  libc::c_ulong,
    data:   Option<&str>,
) -> io::Result<()> {
    let _ = std::fs::create_dir_all(target);
    let c_source = CString::new(source).unwrap();
    let c_target = CString::new(target).unwrap();
    let c_fstype = CString::new(fstype).unwrap();
    let c_data = data.map(|d| CString::new(d).unwrap());
    let data_ptr = c_data.as_ref()
        .map(|c| c.as_ptr() as *const libc::c_void)
        .unwrap_or(std::ptr::null());
    let rc = unsafe {
        libc::mount(c_source.as_ptr(), c_target.as_ptr(), c_fstype.as_ptr(), flags, data_ptr)
    };
    if rc < 0 {
        let e = io::Error::last_os_error();
        if e.raw_os_error() == Some(libc::EBUSY) { return Ok(()); }
        return Err(e);
    }
    Ok(())
}

pub fn move_mount(source: &str, target: &str) -> io::Result<()> {
    let c_source = CString::new(source).unwrap();
    let c_target = CString::new(target).unwrap();
    let rc = unsafe {
        libc::mount(c_source.as_ptr(), c_target.as_ptr(),
                    std::ptr::null(), libc::MS_MOVE, std::ptr::null())
    };
    if rc < 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
}
