# WriteOnce OS — Session Notes

> Consolidated from design session: boot sequence, PID 1, language selection

---

## Topic 1 — Linux From Scratch Build Pipeline

**Goal:** Build a minimal x86_64 Linux system from official kernel.org sources as the foundation for WriteOnce OS.

### The 8 LFS phases

1. **Host setup & partition** — verify host toolchain (GCC ≥ 12, make, gawk, bison, flex), create `$LFS` partition (ext4, ≥ 20 GB), set `LFS=/mnt/lfs`.

2. **Download sources** — use LFS wget-list; kernel tarball from `kernel.org/pub/linux/kernel/v6.x/`. Verify GPG signatures.

3. **Cross-toolchain** (as `lfs` user) — build order: binutils pass 1 → GCC pass 1 → Linux API headers (`make headers`) → glibc → libstdc++. Target triple: `x86_64-lfs-linux-gnu`. Tools land in `$LFS/tools`, isolating host from target.

4. **Temporary tools** — ~20 packages targeting `$LFS/usr` for chroot preparation. Includes binutils pass 2 and GCC pass 2.

5. **Chroot + VFS** — bind-mount `/dev`, `/proc`, `/sys`, `/dev/pts` into `$LFS`. `chroot` into the new root. Create `/etc/passwd`, `/etc/group`, device nodes.

6. **Basic system** — ~80 packages inside chroot. All POSIX utilities, GCC, glibc, util-linux, e2fsprogs, shadow.

7. **Kernel build** — `make mrproper && make menuconfig && make -j$(nproc) && make modules_install`. Copy `bzImage` to `/boot`. Critical menuconfig choices: ext4 built-in, storage driver (NVMe/SATA/virtio), framebuffer.

8. **System config + bootloader** — `/etc/fstab`, hostname, network, GRUB install and `grub-mkconfig`.

### Key commands

```bash
# Fetch and verify kernel
wget https://www.kernel.org/pub/linux/kernel/v6.x/linux-6.12.tar.xz
xz -d linux-6.12.tar.xz && gpg --verify linux-6.12.tar.sign

# Kernel build inside chroot
make mrproper
make menuconfig
make -j$(nproc)
make modules_install
cp arch/x86/boot/bzImage /boot/vmlinuz-6.x.y-lfs
```

---

## Topic 2 — PID 1 and the Init Process

**Context:** Understanding what makes PID 1 special before designing WriteOnce's init system.

### What makes PID 1 unique

- **Cannot be killed** — `SIGKILL` from any userspace process (including root) is silently dropped by the kernel. Only kernel panic/halt terminates it.
- **Universal parent reaper** — orphaned processes (parent exited) are reparented to PID 1. Init must call `waitpid(-1, ...)` to collect them and prevent zombie accumulation. This loop is the core contract of any init.
- **Bootstraps all of userspace** — mounts filesystems, starts services, establishes session hierarchy.

### Minimal viable init (the contract)

```c
int main(void) {
    mount("proc",    "/proc",    "proc",    0, NULL);
    mount("sysfs",   "/sys",     "sysfs",   0, NULL);
    mount("devtmpfs","/dev",     "devtmpfs",0, NULL);

    pid_t child = fork();
    if (child == 0) execv("/bin/sh", (char*[]){"/bin/sh", NULL});

    // The reaping loop — mandatory
    for (;;) {
        int status;
        waitpid(-1, &status, 0);
    }
}
```

### Signal behaviour of PID 1

| Signal  | Normal process | PID 1 (systemd)        |
| ------- | -------------- | ---------------------- |
| SIGTERM | Terminate      | Ignored unless handled |
| SIGKILL | Terminate      | **Kernel ignores it**  |
| SIGHUP  | Hangup         | `daemon-reload`        |
| SIGINT  | Interrupt      | Initiate reboot        |

### systemd cgroup hierarchy (reference)

```
systemd (PID 1)
├─ system.slice
│   ├─ sshd.service
│   ├─ NetworkManager.service
│   └─ display-manager.service   ← WriteOnce GTK4 DM here
├─ user.slice
│   └─ user-1000.slice
│       └─ session-1.scope       ← Sway / i3 here
└─ init.scope
```

### Debugging PID 1

```bash
ls -la /proc/1/exe          # what binary is PID 1
cat /proc/1/environ | tr '\0' '\n'  # startup environment
cat /proc/1/cgroup          # cgroup membership
strace -p 1 -e trace=process  # live syscall trace (root)
# Recovery: kernel cmdline init=/bin/bash gives root shell as PID 1
```

---

## Topic 3 — Full Boot Sequence (Power Button → Userspace)

**15 phases from switch-on to a running Sway session.**

### Hardware layer (phases ①–③)

| #   | Event        | Detail                                                                 |
| --- | ------------ | ---------------------------------------------------------------------- |
| ①   | Power button | Shorts PS_ON# → PSU starts 12 V/5 V/3.3 V. RESET# held low.            |
| ②   | Power Good   | PWR_OK asserted once rails stable (~100–500 ms). RESET# released.      |
| ③   | Reset vector | BSP jumps to 0xFFFFFFF0 in 16-bit real mode. Firmware ROM mapped here. |

### Firmware layer (phases ④–⑤)

| #   | Event       | Detail                                                                |
| --- | ----------- | --------------------------------------------------------------------- |
| ④   | POST / UEFI | SEC → PEI (RAM init) → DXE (device drivers) → BDS (boot selection).   |
| ⑤   | Boot device | UEFI reads NVRAM BootOrder, finds ESP (FAT32), loads EFI application. |

### Bootloader layer (phases ⑥–⑦)

| #   | Event                  | Detail                                                                            |
| --- | ---------------------- | --------------------------------------------------------------------------------- |
| ⑥   | GRUB loaded            | grubx64.efi runs as EFI app. Reads grub.cfg. Shows menu.                          |
| ⑦   | Kernel + initrd loaded | vmlinuz + initrd.img read into RAM. boot_params built. ExitBootServices() called. |

### Kernel layer (phases ⑧–⑫)

| #   | Event            | Detail                                                                              |
| --- | ---------------- | ----------------------------------------------------------------------------------- |
| ⑧   | Decompression    | startup_64 decompresses vmlinux. KASLR base randomised.                             |
| ⑨   | Early arch setup | GDT, IDT, 4-level paging (PML4), long mode, SSE/AVX. Calls start_kernel().          |
| ⑩   | start_kernel()   | mm_init, sched_init, trap_init, irq_init, time_init, vfs_caches_init, console_init. |
| ⑪   | rest_init()      | PID 0 (idle), PID 1 (kernel_init), PID 2 (kthreadd) created. Scheduler active.      |
| ⑫   | initramfs        | tmpfs root mounted. /init loads storage modules. switch_root to real root.          |

### Userspace layer (phases ⑬–⑮)

| #   | Event           | Detail                                                                               |
| --- | --------------- | ------------------------------------------------------------------------------------ |
| ⑬   | PID 1 exec      | try_to_run_init_process() → execve(/sbin/init). First userspace process.             |
| ⑭   | Init system     | Service graph resolved. Targets activated: sysinit → basic → multi-user → graphical. |
| ⑮   | DM → compositor | display-manager.service starts. PAM auth. Sway/i3 launched in session cgroup.        |

### Where the OS starts

**Phase ⑨ — the call to `start_kernel()`** is the precise boundary. Phases ⑧ and earlier are either hardware, firmware, bootloader, or a pre-kernel stub with no OS data structures. From `start_kernel()` onward, the kernel has a memory allocator, scheduler, and VFS — the minimum set that constitutes a running OS.

---

## Topic 4 — Language Selection Per Phase

**Principle:** use the language whose constraints best match the phase's runtime environment. No GC in the kernel or PID 1. No Rust where the runtime hasn't been established yet.

| Phase                       | Language             | Rationale                                                          |
| --------------------------- | -------------------- | ------------------------------------------------------------------ |
| ①②③ Hardware                | —                    | Not software                                                       |
| ④⑤ Firmware                 | C                    | UEFI is C. No stack initially.                                     |
| ⑥⑦ Bootloader               | **Rust** (`uefi-rs`) | EFI app context, file I/O available, Rust safe here                |
| ⑧ Decompression stub        | ASM + C              | Position-independent, no runtime, must precede page tables         |
| ⑨ Early arch (head_64.S)    | ASM + C              | `lgdt`, `lidt`, `mov cr3` — no Rust abstraction exists             |
| ⑩ start_kernel() subsystems | C → **Rust**         | Rust in kernel since 6.1; use for drivers and subsystems           |
| ⑪ Kernel threads            | **Rust**             | Pure logic, no hardware quirks, ownership maps to thread lifecycle |
| ⑫ initramfs /init           | **Rust**             | No shell dependency; `no_std` binary                               |
| ⑬ PID 1 binary              | **Rust**             | `pid1` crate; no GC pause acceptable                               |
| ⑭ Init system               | **Rust**             | Async (tokio) + no GC; cgroup v2 via nix crate                     |
| ⑮ DM + WM / DE              | **Rust** (i3More) + C (Xorg, i3) | i3More is X11/i3/GTK4 — see correction note below                  |

### The WriteOnce language stack

```
Phases ④⑤   C (vendor firmware — not owned)
Phases ⑥⑦   Rust  ← uefi-rs custom bootloader
Phases ⑧⑨   ASM + C  ← unavoidable minimum
Phases ⑩⑪   Rust  ← no_std kernel code
Phases ⑫⑬   Rust  ← init binary
Phase  ⑭    Rust  ← service supervisor
Phase  ⑮    Rust + C  ← i3More (Rust/GTK4) on Xorg + i3 (C)
```

### Correction: Phase ⑮ is X11, not Wayland

Initial draft assumed a smithay/Wayland compositor at ⑮. Surveying the actual DE (`.agents/reference/i3More/`) shows i3More is built **on X11 / i3 / GTK4**, not Wayland. The OS therefore needs Xorg + i3 + D-Bus + PAM + a logind-compatible D-Bus surface + PipeWire — see `plan/phase-8-x11-gtk4.md` and `plan/phase-9-i3more.md` for the corrected userspace stack. The Rust phase-⑮ ownership stays (i3More itself is Rust); the *substrate* below it is X11/C, not Wayland/Rust.

### Hard boundary

Phases ⑧–⑨ cannot be pure Rust because several x86 instructions have no safe Rust abstraction:

- `lgdt` / `lidt` — load descriptor tables
- `mov cr0 / cr3 / cr4` — control register writes
- `wrmsr` — write model-specific registers

These require inline ASM (`core::arch::asm!`) or a C shim. The Rust boundary is solid from `start_kernel()` (⑩) and total from phase ⑫ onward.

---

## Active Threads & Next Steps

- **Bootloader**: evaluate `uefi-rs` + limine vs. custom EFI app
- **Kernel config**: baseline from `defconfig` then audit, or `allnoconfig` + build up
- **Init system**: design the supervision tree (s6-style reference) in Rust
- **initramfs**: Rust binary (no busybox dependency) vs. minimal shell
- **Package strategy**: curated source builds, Nix-style, or custom format
