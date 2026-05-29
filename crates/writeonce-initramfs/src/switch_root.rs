//! Replace the kernel-loaded initramfs with the real root, then
//! `execve` PID 1.
//!
//! We deliberately do **not** use `pivot_root(2)`. The kernel's
//! `Documentation/admin-guide/initrd.rst` is explicit: "It is
//! impossible to call pivot_root() from the initramfs because rootfs
//! cannot be unmounted." Calls return `EINVAL`. The busybox
//! `switch_root` and systemd `initrd-switch-root.service` both use
//! the `mount --move` + `chroot` idiom instead:
//!
//!   1. Mount the real root on `/sysroot`.
//!   2. Move-mount `/proc`, `/sys`, `/dev` (and `/run` if present)
//!      into `/sysroot/*` so the real system finds them mounted.
//!   3. `chdir(/sysroot)`.
//!   4. `mount --move /sysroot /` — the new root takes the place of
//!      the kernel rootfs. (This is what pivot_root would have done,
//!      but without the rootfs-can't-unmount restriction.)
//!   5. `chroot(.)` — make `/` resolve into the moved-in fs.
//!   6. `chdir(/)`.
//!   7. `execve(init_path, [init_path], envp)` — typically
//!      `/usr/sbin/writeonce-pid1`.

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

    // 4. mount --move /sysroot to /  — replaces the kernel rootfs.
    //    Equivalent to what pivot_root(2) would have done, but works
    //    from inside an initramfs where pivot_root returns EINVAL.
    mount::move_mount(".", "/")?;

    // 5. chroot(.) — now `/` resolves to the moved-in real root.
    let dot = CString::new(".").unwrap();
    if unsafe { libc::chroot(dot.as_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }
    if unsafe { libc::chdir(CString::new("/").unwrap().as_ptr()) } != 0 {
        return Err(io::Error::last_os_error());
    }

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
