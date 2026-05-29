# writeonce-initramfs

The Rust `/init` binary for the WriteOnce OS initramfs (Phase 5). Replaces
the BusyBox shell stub from Phase 2.

The kernel unpacks the initramfs and runs this binary as PID 1. Its job is to
prepare just enough userspace to find the real root filesystem, pivot into it,
and hand off to `/sbin/writeonce-pid1`.

## Boot flow

1. **Sanity check** — confirm `pid == 1` (or `WO_INITRAMFS_FAKE=1` for
   development outside QEMU).
2. **Mount essentials** — `/proc`, `/sys`, and `/dev` (devtmpfs).
3. **Parse `/proc/cmdline`** — `root=`, `rootfstype=`, `rootflags=`, `init=`,
   `ro`/`rw`, `writeonce.rootwait=N`, `wo.recovery`.
4. **Load kernel modules** — names from `/etc/modules-load.conf` and
   `/etc/modules-load.d/`, loaded via `finit_module(2)`. Best-effort; failures
   are non-fatal.
5. **Discover the root device** — resolve the `root=` spec, polling up to
   `rootwait` seconds for the device to appear.
6. **switch_root + execve** — mount the root on `/sysroot`, move the pseudo-fs
   mounts in, `mount --move /sysroot /`, `chroot`, then `execve` PID 1.

On any error it drops to a built-in **recovery shell** for inspecting
`/proc`, `/sys`, and `/dev` without needing binaries on the initramfs.

## Modules

| Module           | Responsibility                                                        |
| ---------------- | --------------------------------------------------------------------- |
| `main.rs`        | Orchestrates the boot flow; hosts the recovery shell.                 |
| `cmdline.rs`     | Parses `/proc/cmdline` into a `CmdLine` struct.                       |
| `mount.rs`       | Thin `mount(2)` / `umount2(2)` wrappers (no `nix` dependency).        |
| `modules.rs`     | Loads kernel modules listed in config via `finit_module(2)`.          |
| `discover.rs`    | Resolves `root=` (UUID / PARTUUID / LABEL / `/dev/...`) to a device.  |
| `switch_root.rs` | `mount --move` + `chroot` handoff to the real root's PID 1.           |

## Kernel cmdline parameters

| Parameter               | Default                  | Notes                                                       |
| ----------------------- | ------------------------ | ----------------------------------------------------------- |
| `root=`                 | _(required)_             | `UUID=`, `PARTUUID=`, `LABEL=`, or `/dev/...`.              |
| `rootfstype=`           | `ext4`                   | Filesystem of the real root.                                |
| `rootflags=`            | —                        | Extra `mount(2)` options.                                   |
| `init=`                 | `/sbin/writeonce-pid1`   | Path of the PID 1 to exec after pivot.                      |
| `ro` / `rw`             | `rw` (writable, noatime) | Last token wins.                                            |
| `writeonce.rootwait=N`  | `30`                     | Seconds to poll for the root device. Namespaced to avoid the kernel's own `rootwait`. |
| `wo.recovery`           | off                      | Drop straight to the recovery shell.                        |

## Why `mount --move` instead of `pivot_root(2)`

`pivot_root(2)` returns `EINVAL` from an initramfs — rootfs cannot be
unmounted (`Documentation/admin-guide/initrd.rst`). Like BusyBox `switch_root`
and systemd's `initrd-switch-root.service`, this crate uses the
`mount --move` + `chroot` idiom instead. See `switch_root.rs` for the full
sequence.

## Notes

- Built static against musl: `cargo build -p writeonce-initramfs --target x86_64-unknown-linux-musl --release`.
- Root discovery reads the ext2/3/4 superblock directly (magic `0xEF53`) to
  match UUID/LABEL; only ext filesystems are probed because WriteOnce's root
  is always ext4.
- Run tests with `cargo test -p writeonce-initramfs` (cmdline parsing and
  module-config parsing have unit coverage).
