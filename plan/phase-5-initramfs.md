# Phase 5 — Rust initramfs

**Goal.** Replace the BusyBox-based initramfs from Phase 2 with a Rust `init` binary that loads storage modules, finds the real root, switches to it. No shell in initramfs.

## Subtasks

1. **Scaffold `src/initramfs-init/`** crate. `no_std` is overkill (we have an allocator from the kernel); use `std` with musl-static for sanity.

2. **Implement module loading.** Parse `/etc/modules-load.conf` (cpio-baked), call `finit_module(2)` via nix. Need: `ahci`, `ext4` (if not built-in), maybe `dm_mod` later, `iwlwifi` if we want early Wi-Fi (probably not in initramfs).

3. **Implement root device discovery.** Honor `root=UUID=...` from `/proc/cmdline`. Probe `/sys/class/block/`, read `/sys/class/block/*/uevent` for partition UUID, mount it on `/sysroot`.

4. **Implement `switch_root`.** `pivot_root(2)` or the kernel's `move_mount` family; `chroot("/")`; `execve("/sbin/writeonce-pid1")`. Reference: `../.agents/reference/linux/fs/init.c::init_mount_tree`.

5. **Build the initramfs image.**
   - cpio + zstd (smaller than gzip, kernel supports it: `CONFIG_RD_ZSTD`).
   - Contents: `/init` (Rust binary), `/lib/modules/.../*.ko`, `/lib/firmware/iwlwifi-*` (so wifi works on first boot if needed), `/etc/modules-load.conf`.

6. **Test in QEMU** with `-initrd build/artifacts/initramfs-rust.img`. Capture boot timing; aim < 1 second from kernel handoff to PID 1 exec.

7. **Add a recovery mode.** If root mount fails, instead of panicking, drop to a minimal Rust shell over the console (just `read line` / `execve`). Single binary, no BusyBox.

8. **Deploy to the T450.** Replace `/boot/initramfs.img`. Reboot. Confirm via netconsole that the Rust init runs before PID 1.

## Deliverable

A < 5 MB Rust initramfs that brings root online and hands off to Phase-3 PID 1.

## Acceptance criteria

- Cold boot from press-power to login prompt under 10 seconds on the T450 (Samsung SSD + small initramfs).
- `dmesg | grep "Run /sbin/writeonce-pid1"` confirms the chain.
- Yanking the SSD mid-boot drops to the recovery shell, not a kernel panic.

## References

- `../.agents/reference/linux/init/do_mounts.c`
- `../.agents/reference/linux/fs/init.c`
