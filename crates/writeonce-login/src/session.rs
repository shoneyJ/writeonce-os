//! Privilege drop + session `execve`, shared by the greeter (and reusable by
//! other launchers). Dependency-free (libc only) so it links cleanly into the
//! glibc-dynamic login/greeter binaries.
//!
//! NOTE: on the systemd flavor the logind session is created by `pam_systemd`
//! at `pam_open_session` — so this module deliberately does NOT call
//! `CreateSession` or create `XDG_RUNTIME_DIR` (logind does that, and exposes
//! the values via `pam_getenvlist`). It only drops privileges, `chdir`s, and
//! `execve`s. Contrast `writeonce-session-create`, which drives the
//! writeonce-logind D-Bus path on the rust-init flavor.
//!
//! All functions here are intended to run in the post-`fork` child.

use std::ffi::CString;
use std::io;

/// `initgroups` + `setresgid` + `setresuid` to (uid, gid). gid is set before
/// uid so we don't lose the privilege to setgid. Call AFTER `fork()`, in the
/// child, while still root.
pub fn drop_privileges(uid: u32, gid: u32, user: &str) -> io::Result<()> {
    let user_c = CString::new(user)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "username has NUL"))?;

    if unsafe { libc::initgroups(user_c.as_ptr(), gid) } != 0 {
        return Err(io::Error::last_os_error());
    }
    if unsafe { libc::setresgid(gid, gid, gid) } != 0 {
        return Err(io::Error::last_os_error());
    }
    if unsafe { libc::setresuid(uid, uid, uid) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// `chdir(dir)`.
pub fn chdir(dir: &str) -> io::Result<()> {
    let c = CString::new(dir)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "dir has NUL"))?;
    if unsafe { libc::chdir(c.as_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Replace the process image with `prog` (full `argv`, full `envp`). Returns
/// the OS error only on failure — on success it never returns.
pub fn exec(prog: &str, argv: &[String], env: &[String]) -> io::Error {
    let prog_c = match CString::new(prog) {
        Ok(c) => c,
        Err(_) => return io::Error::new(io::ErrorKind::InvalidInput, "prog has NUL"),
    };
    let argv_c: Vec<CString> = argv.iter().filter_map(|s| CString::new(s.as_str()).ok()).collect();
    let env_c: Vec<CString> = env.iter().filter_map(|s| CString::new(s.as_str()).ok()).collect();

    let argv_p: Vec<*const libc::c_char> = argv_c
        .iter()
        .map(|c| c.as_ptr())
        .chain(std::iter::once(std::ptr::null()))
        .collect();
    let env_p: Vec<*const libc::c_char> = env_c
        .iter()
        .map(|c| c.as_ptr())
        .chain(std::iter::once(std::ptr::null()))
        .collect();

    unsafe { libc::execve(prog_c.as_ptr(), argv_p.as_ptr(), env_p.as_ptr()) };
    io::Error::last_os_error()
}
