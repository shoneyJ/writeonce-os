//! `pivot_root` the initramfs out and `execve` PID 1.
//!
//! Sequence (per `man switch_root` and the kernel's
//! `Documentation/admin-guide/initrd.rst`):
//!
//!   1. Mount the real root on `/sysroot`.
//!   2. Move-mount `/proc`, `/sys`, `/dev` (and `/run` if present) into
//!      `/sysroot/*` so the real system finds them mounted.
//!   3. `chdir(/sysroot)`.
//!   4. `pivot_root(., .)` — old root becomes the current directory
//!      (now `/sysroot`); new root is what was at `/sysroot` (now `/`).
//!   5. `chroot(.)` — defensive (some kernels need it).
//!   6. `umount2("/sysroot", MNT_DETACH)` — release the old initramfs
//!      memory (the kernel returns it to the page allocator).
//!   7. `execve(init_path, [init_path], envp)` — typically
//!      `/sbin/writeonce-pid1`.

use std::ffi::CString;
use std::io;

use crate::mount;

pub fn switch_and_exec(
    root_dev: &str,
    fstype: &str,
    mount_flags: u64,
    init_path: &str,
) -> io::Result<std::convert::Infallible> {
    // 1. Mount the real root on /sysroot.
    let _ = std::fs::create_dir_all("/sysroot");
    mount::mount(root_dev, "/sysroot", fstype, mount_flags as libc::c_ulong, None)?;

    // 2. Move /proc, /sys, /dev into the new root.
    for d in ["/proc", "/sys", "/dev"] {
        let inside = format!("/sysroot{d}");
        let _ = std::fs::create_dir_all(&inside);
        if std::path::Path::new(d).exists() {
            if let Err(e) = mount::move_mount(d, &inside) {
                eprintln!("writeonce-initramfs: move-mount {d} -> {inside}: {e}");
            }
        }
    }

    // 3. cd /sysroot
    let sysroot = CString::new("/sysroot").unwrap();
    if unsafe { libc::chdir(sysroot.as_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }

    // 4. pivot_root(., .)
    let dot = CString::new(".").unwrap();
    let rc = unsafe { libc::syscall(libc::SYS_pivot_root, dot.as_ptr(), dot.as_ptr()) };
    if rc < 0 {
        return Err(io::Error::last_os_error());
    }

    // 5. chroot(.) — defensive
    if unsafe { libc::chroot(dot.as_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    if unsafe { libc::chdir(CString::new("/").unwrap().as_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }

    // 6. Detach the old initramfs.
    let old_root = CString::new("/").unwrap();
    let _ = unsafe { libc::umount2(old_root.as_ptr(), libc::MNT_DETACH) };

    // 7. execve the PID 1 binary.
    let prog = CString::new(init_path)
        .map_err(|_| io::Error::other("init path has NUL"))?;
    let argv: [*const libc::c_char; 2] = [prog.as_ptr(), std::ptr::null()];

    let env_pairs = [
        c"PATH=/usr/bin:/usr/sbin:/bin:/sbin".as_ptr(),
        c"TERM=linux".as_ptr(),
    ];
    let envp: [*const libc::c_char; 3] = [
        env_pairs[0],
        env_pairs[1],
        std::ptr::null(),
    ];

    unsafe { libc::execve(prog.as_ptr(), argv.as_ptr(), envp.as_ptr()) };
    Err(io::Error::last_os_error())
}
