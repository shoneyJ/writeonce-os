# WriteOnce OS

A from-scratch Linux distribution, built as a learning vehicle for kernel internals and userspace primitives. Target: a ThinkPad T450 running the user's own desktop environment, [i3More](https://github.com/shoneyj/i3More), on top of an OS where every layer below it was understood and (where reasonable) re-implemented by the author.

## Status

Planning. No code yet — the repo currently holds design notes, a target-machine hardware survey, and a phase-by-phase implementation roadmap.

## Target machine

- ThinkPad T450 — Intel i5-5300U Broadwell (2c/4t), 16 GB DDR3, Samsung SSD 850 500 GB SATA AHCI
- UEFI 64-bit, **Secure Boot disabled** (custom EFI app is straightforward)
- Intel HD 5500 (i915), Intel I218-LM ethernet (e1000e), Intel Wireless 7265 (iwlwifi)
- Full survey: [`.agents/target-machine.md`](.agents/target-machine.md)

## What's in this repo

```
writeonce-session-notes.md   Architectural source of truth: 8 LFS phases, 15-phase boot,
                             language-per-phase decisions
plan/                        Phase-by-phase implementation roadmap (00-roadmap + 11 phase files)
scripts/
  survey-target-machine.sh   Hardware survey script run on the T450
.agents/
  target-machine.md          Captured hardware survey output
  reference/                 Symlinks to external repos used as read-only reference material
    linux/                   → ~/projects/linux/ (kernel source)
    i3More/                  → ~/projects/github/shoneyj/i3More/ (target DE)
    lfs/                     → ~/projects/github/lfs-book/lfs/ (LFS book sources)
    writeonce-all/           → ~/projects/github/shoneyj/writeonce/writeonce-all/
```

## Build approach

Cross-compiled from the workstation; deployed to the T450. The T450 will be wiped of its current Ubuntu install once the first WriteOnce kernel boots cleanly in QEMU. See [`plan/00-roadmap.md`](plan/00-roadmap.md).

## Language stack (per boot phase)

| Boot phase                  | Language          |
| --------------------------- | ----------------- |
| Firmware (vendor)           | C                 |
| Bootloader (Phase 6)        | Rust (`uefi-rs`)  |
| Early arch (head_64.S)      | ASM + C           |
| Kernel + Rust modules       | C + Rust          |
| initramfs (Phase 5)         | Rust (musl)       |
| PID 1 (Phase 3)             | Rust (musl)       |
| Service supervisor (Phase 4)| Rust (tokio)      |
| Xorg substrate (Phase 8)    | C (curated)       |
| Desktop (i3More, Phase 9)   | Rust + GTK4       |

See `writeonce-session-notes.md` Topic 4 for the rationale.

## Reading order

1. [`writeonce-session-notes.md`](writeonce-session-notes.md) — start here for the *why*
2. [`plan/00-roadmap.md`](plan/00-roadmap.md) — start here for the *how*
3. Individual `plan/phase-N-*.md` files — start here for the *what next*

## Upstream references

External projects this build draws from. All are mirrored locally as read-only symlinks under `.agents/reference/` for fast grepping.

### Linux From Scratch (LFS) book

- **Upstream:** [github.com/lfs-book/lfs](https://github.com/lfs-book/lfs) — official sources of the LFS book (DocBook XML)
- **Project site:** [linuxfromscratch.org](https://www.linuxfromscratch.org/)
- **Local mirror:** [`.agents/reference/lfs/`](.agents/reference/lfs/)
- **Used by:** [`plan/phase-0-toolchain.md`](plan/phase-0-toolchain.md) (LFS chapters 5–6 = cross-toolchain + temporary tools), [`plan/phase-2-minimal-linux.md`](plan/phase-2-minimal-linux.md) (chapter 7+ chroot flow)
- **Why:** authoritative recipe for the LFS-style build sequence — package versions, build orders, configure flags. WriteOnce may deviate, but deviations are documented per-phase.

### Linux kernel

- **Upstream:** [kernel.org](https://www.kernel.org/) / [git.kernel.org](https://git.kernel.org/pub/scm/linux/kernel/git/torvalds/linux.git)
- **Local mirror:** [`.agents/reference/linux/`](.agents/reference/linux/)
- **Pinned version:** Linux 6.12 LTS (see [`plan/phase-2-minimal-linux.md`](plan/phase-2-minimal-linux.md))
- **Used by:** every kernel-touching phase; primary entry points are `init/main.c`, `kernel/exit.c`, `fs/init.c`, `Documentation/rust/`, `arch/x86/`.

### i3More

- **Upstream:** [github.com/shoneyj/i3More](https://github.com/shoneyj/i3More) (user's own project)
- **Local mirror:** [`.agents/reference/i3More/`](.agents/reference/i3More/)
- **Used by:** [`plan/phase-8-x11-gtk4.md`](plan/phase-8-x11-gtk4.md) (substrate requirements), [`plan/phase-9-i3more.md`](plan/phase-9-i3more.md) (integration).
- **Why:** end-goal desktop environment — its dependency surface (X11, GTK4, D-Bus, PAM, PipeWire) defines the userspace stack WriteOnce must provide.

## License

Not yet decided; will be set before the first code lands.
