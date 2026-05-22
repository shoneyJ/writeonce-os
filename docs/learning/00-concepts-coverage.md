# Learning curriculum — OS concepts covered, pending, mapped to phases

> Index of every OS concept the WriteOnce project will eventually touch,
> with the current coverage state and the phase in which each becomes
> load-bearing. Update this file when a new learning doc lands so the
> "what's left to learn" answer stays accurate.

## Coverage status legend

- ✓ Covered — there's a doc under `docs/learning/` (or in a `plan/phase-N-*.md`) and the concept has been used in code.
- ◐ Partially covered — referenced or surveyed but not yet a deep doc.
- ✗ Pending — listed as relevant, no doc yet, will be written when the corresponding phase begins.

## Covered today

| Concept                         | Doc                                                    |
| ------------------------------- | ------------------------------------------------------ |
| Cross-compilation / host-target | [`phase-0-cross-toolchain.md`](phase-0-cross-toolchain.md) |
| `$LFS/tools` layout             | [`phase-0-lfs-tools-layout.md`](phase-0-lfs-tools-layout.md) |
| LFS Ch. 6 transitional userspace | [`phase-0-temp-tools-result.md`](phase-0-temp-tools-result.md) |
| `chroot(2)` semantics           | [`phase-2-chroot.md`](phase-2-chroot.md) |
| BusyBox / multi-call binaries   | [`phase-2-busybox.md`](phase-2-busybox.md) |
| UEFI-from-USB boot chain        | [`phase-2-bootable-usb.md`](phase-2-bootable-usb.md) |
| systemd feature surface         | [`systemd-feature-survey.md`](systemd-feature-survey.md) |
| cgroup-v2 + `clone3(CLONE_INTO_CGROUP)` | [`phase-4-cgroup-isolation.md`](phase-4-cgroup-isolation.md) |
| Unit-file format + `WantedBy`   | [`phase-4-service-toml-schema.md`](phase-4-service-toml-schema.md) |
| Dependency-graph / topological sort | [`phase-4-dependency-graph.md`](phase-4-dependency-graph.md) |
| Concurrency model + io_uring    | [`phase-4-concurrency-and-io-uring.md`](phase-4-concurrency-and-io-uring.md) |

## Pending — per upcoming phase

### Phase 5 — Rust initramfs

| Concept | File to write |
| --- | --- |
| `pivot_root(2)` / `switch_root` | `phase-5-initramfs-handoff.md` |
| Kernel module loading (`finit_module`, depmod) | `phase-5-module-loading.md` |
| Root device discovery (`/proc/cmdline`, `/sys/class/block/`) | (in `phase-5-initramfs-handoff.md`) |
| Initramfs format (cpio newc, zstd vs gzip) | `phase-5-initramfs-format.md` |

### Phase 6 — Rust UEFI bootloader (biggest single concept gap)

| Concept | File to write |
| --- | --- |
| UEFI Boot Services vs Runtime Services, `ExitBootServices()` | `phase-6-uefi-boot-protocol.md` |
| `boot_params` / "zero page" layout | (same file) |
| EFI handover protocol — 64-bit entry point | (same file) |
| EFI memory map → e820 translation | (same file) |
| GPT format, protective MBR, partition entries | `phase-6-gpt-and-mbr.md` |
| Secure Boot / shim mechanics | `phase-6-secureboot-as-future.md` |

### Phase 7 — Kernel customization + Rust module

| Concept | File to write |
| --- | --- |
| Kbuild / Kconfig / Makefile recursion | `phase-7-kernel-build-system.md` |
| Module subsystem (symbol exports, `MODULE_LICENSE`) | `phase-7-kernel-modules.md` |
| Rust-for-Linux internals (`kernel::` crate, `pin-init`) | `phase-7-rust-for-linux.md` |
| CPU microcode (early vs late, `intel-ucode/`) | `phase-7-microcode.md` |
| Sysctl + `/proc/sys/` | `phase-7-sysctl.md` |
| Kernel command line | (in `phase-7-kernel-build-system.md`) |

### Phase 8 — X11 + GTK4 substrate

| Concept | File to write |
| --- | --- |
| DRM / KMS modesetting, GBM, i915 driver model | `phase-8-drm-kms.md` |
| X11 protocol (requests, replies, events, `Xauthority`) | `phase-8-x11-protocol.md` |
| D-Bus IPC architecture (system/session, objects, signals, introspection) | `phase-8-dbus-architecture.md` |
| PAM stack (modules, conversation, four module types) | `phase-8-pam.md` |
| ALSA → PipeWire audio model | `phase-8-audio-stack.md` |
| Fontconfig + freetype rendering pipeline | `phase-8-fonts.md` |

### Phase 9 — DE integration

| Concept | File to write |
| --- | --- |
| logind deep dive (sessions, seats, VT switching, scope units) | `phase-9-logind-internals.md` |
| Session management (`XDG_RUNTIME_DIR`, `XDG_SESSION_*`) | `phase-9-xdg-session.md` |
| User services vs system services | `phase-9-user-services.md` |
| Wayland (the road not taken) | `phase-9-wayland-alternative.md` |

## Cross-cutting concepts (relevant to multiple phases)

These are general OS concepts that surface throughout the project. Write them when the first phase that needs them lands — but note all the later phases each will inform.

| Concept                                           | First load-bearing phase | Also relevant in     | File to write                    |
| ------------------------------------------------- | ------------------------ | -------------------- | -------------------------------- |
| **Linux namespaces** (PID, mount, net, user, IPC, UTS, cgroup, time) | Phase 4 (sandboxing) | Phase 9 (user sessions)        | `linux-namespaces.md` |
| **Virtual memory + page tables** (4-level paging, TLB, ASLR) | Phase 7 (kernel)         | Phase 6 (early arch)           | `virtual-memory.md` |
| **CPU scheduling** (CFS, real-time, deadline)     | Phase 4 (priorities)     | Phase 7 (kernel config)        | `cpu-scheduling.md` |
| **VFS layer** (inode/dentry/superblock)           | Phase 5 (root mount)     | Phase 7                        | `vfs-layer.md` |
| **ext4 internals** (extents, journal, htree)      | Phase 2 (partitions)     | Phase 5                        | `ext4-internals.md` |
| **Block layer + I/O scheduler** (bio, BFQ, AHCI)  | Phase 7                  | —                              | `block-layer.md` |
| **Networking stack** (sockets, netlink, routing, DHCP, DNS) | Phase 2 (boot net)       | Phase 8 (NM/connmand decision) | `networking-stack.md` |
| **Interrupt handling** (IRQ, MSI/MSI-X, softirq, threaded IRQs) | Phase 7                  | —                              | `interrupt-handling.md` |
| **DMA + IOMMU** (Intel VT-d on T450)              | Phase 7                  | —                              | `dma-and-iommu.md` |
| **Power management** (ACPI states, S3 suspend, runtime PM) | Phase 9 (`i3more-lock`)  | Phase 7                        | `power-management.md` |
| **Security primitives** (capabilities, seccomp, LSMs, ASLR/NX/PIE/SSP) | Phase 4 (sandboxing)     | Phase 6 (kernel)               | `security-primitives.md` |
| **IPC mechanisms** (pipes, Unix sockets, shm, futexes, eventfd, timerfd) | Phase 4 (control plane)  | Phase 8 (X11 SHM)              | `ipc-mechanisms.md` |
| **Time + clocks** (CLOCK_REALTIME/MONOTONIC/BOOT, RTC, NTP) | Phase 2 (boot logs)      | Phase 4                        | `time-and-clocks.md` |
| **Logging architecture** (`printk`, kmsg, journald, syslog) | Phase 4 (`wo-logs`)      | —                              | `logging-architecture.md` |
| **Hotplug + udev** (netlink uevents, rule processing, predictable names) | Phase 8 (input, GPU)     | Phase 9                        | `udev-and-hotplug.md` |
| **Locale / NSS** (`LC_*`, `nsswitch.conf`, NSS modules) | Phase 8 (locale data)    | —                              | `locale-and-nss.md` |
| **Dynamic linker** (`ld-linux-x86-64.so.2`, RPATH/RUNPATH, lazy binding) | Phase 7                  | Phase 8                        | `dynamic-linker.md` |

## Top-3 to write next

When picking the next learning doc to author, the three that pay off
most for the project's immediate trajectory:

1. **`linux-namespaces.md`** — Phase 4 service sandboxing needs this; Phase 9 user-session story needs it. The namespace concept is also among the most-cited / least-understood Linux primitives.
2. **`phase-5-initramfs-handoff.md`** — `pivot_root` / `switch_root` / module loading. Required before we replace the BusyBox transitional `/init` with the Rust one.
3. **`phase-6-uefi-boot-protocol.md`** — Boot Services, the zero-page, the EFI handover entry. Required before we replace GRUB with our own `uefi-rs` app. This is the largest single concept gap remaining in the roadmap.

Everything else can wait for its phase. The project's learning value comes from writing each doc when the corresponding code is being built — the alternative is a stack of speculative documents that don't quite match what gets implemented.
