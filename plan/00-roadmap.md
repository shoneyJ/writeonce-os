# WriteOnce OS — Implementation Roadmap

> Master index for the build plan. Per-phase detail in `phase-0-…` through `phase-10-…`.

---

## Context

**Why this plan exists.** Build WriteOnce OS — a from-scratch Linux distribution — on a spare ThinkPad T450 as a learning vehicle for kernel internals and userspace primitives. Design notes already exist (`../writeonce-session-notes.md`: 8 LFS phases, 15-phase boot sequence, per-phase language matrix). This roadmap adds the **buildable ordering**: phases, subtasks, acceptance criteria, and concrete decisions tied to the actual target hardware (`../.agents/target-machine.md`).

**Key facts that shape the plan:**

- **Target hardware (T450)** — Intel i5-5300U Broadwell (2c/4t, x86_64; VT-x, AES-NI, AVX2, RDRAND); 16 GB DDR3-1600; Samsung SSD 850 500 GB SATA AHCI; UEFI 64-bit, **Secure Boot disabled**; Intel HD 5500 GPU (i915); Intel I218-LM ethernet (e1000e); Intel Wireless 7265 (iwlwifi, needs firmware blob); xHCI USB3; snd_hda_intel audio. Currently runs Ubuntu 24.04 / kernel 6.8.
- **i3More is X11 / i3 / GTK4** — *not* Wayland/smithay as the session notes originally assumed. The OS must provide Xorg + i3 + D-Bus + PAM + a logind-compatible interface, plus PipeWire/PulseAudio. Phase ⑮ language stack changes accordingly.
- **User decisions (session 2026-05-17):**
  1. Plans live in `./plan/` at repo root.
  2. **Cross-compile from this workstation** (LFS host/target separation).
  3. **Wipe Ubuntu** — WriteOnce becomes the only OS on the T450. Rescue USB required.
  4. Plan **all phases at equal depth** up front.

**Intended outcome.** A T450 booting WriteOnce OS — kernel + initramfs + Rust PID 1 + Rust service supervisor + X11/i3 + i3More — from a user-built UEFI bootloader, reproducibly rebuildable from the workstation.

---

## Phase map

| #  | Phase                                                    | Primary language(s)      | Boot-sequence map (notes) | File |
| -- | -------------------------------------------------------- | ------------------------ | ------------------------- | ---- |
| 0  | Workstation cross-compile environment                    | Bash / Make              | (host-side prep)          | [phase-0-toolchain.md](phase-0-toolchain.md) |
| 1  | T450 prep — rescue USB, disk plan, console               | Bash                     | (target-side prep)        | [phase-1-target-prep.md](phase-1-target-prep.md) |
| 2  | LFS-style minimal Linux (mainline kernel + BusyBox init) | C / ASM via stock kernel | ⑧⑨⑩⑫⑬ (transitional)      | [phase-2-minimal-linux.md](phase-2-minimal-linux.md) |
| 3  | Rust PID 1                                               | Rust (std-light)         | ⑬                         | [phase-3-rust-pid1.md](phase-3-rust-pid1.md) |
| 4  | Rust service supervisor + cgroup v2 + minimal logind     | Rust (tokio)             | ⑭                         | [phase-4-supervisor.md](phase-4-supervisor.md) |
| 5  | Rust initramfs                                           | Rust (std/musl-static)   | ⑫                         | [phase-5-initramfs.md](phase-5-initramfs.md) |
| 6  | Rust UEFI bootloader (uefi-rs)                           | Rust (no_std)            | ⑥⑦                        | [phase-6-bootloader.md](phase-6-bootloader.md) |
| 7  | Kernel customization + Rust kernel module                | C / ASM / Rust           | ⑩⑪                        | [phase-7-kernel.md](phase-7-kernel.md) |
| 8  | X11/i3 userspace + GTK4 stack                            | C (Xorg) + curated       | ⑮ (X11 server side)       | [phase-8-x11-gtk4.md](phase-8-x11-gtk4.md) |
| 9  | i3More integration + login/DM flow                       | Rust                     | ⑮ (DE side)               | [phase-9-i3more.md](phase-9-i3more.md) |
| 10 | Packaging, reproducibility, install ISO                  | Rust + custom            | (meta)                    | [phase-10-packaging.md](phase-10-packaging.md) |

Phases are **sequential by dependency**, but several can overlap once the prior phase has a working artifact (e.g. Phase 4 supervisor design can begin while Phase 3 PID 1 stabilises).

---

## Cross-cutting tracks (parallel to the phases)

- **Track L — Learning log.** `docs/learning/<phase>-<topic>.md` per major concept (PID 1, cgroup v2, EFI handover, kernel Rust, etc.). The user's primary goal is *learning*; this is the evidence.
- **Track R — Reference repo curation.** Keep `.agents/reference/` lean and current. Update per-repo memories when upstreams change.
- **Track T — Testing.** QEMU-based smoke tests for every phase artifact. Never deploy untested to the T450.
- **Track S — Snapshot points.** At end of each phase, `tar` the `build/sysroot/` + `build/artifacts/` to `snapshots/phase-N/`. Cheap rollback if the next phase breaks badly.

---

## Critical files (this repo + references)

- `../writeonce-session-notes.md` — architectural source of truth.
- `../.agents/target-machine.md` — hardware ground truth; consult before any `menuconfig` decision.
- `../.agents/reference/linux/` — kernel source; primary references for boot, init, cgroup, EFI, Rust kernel.
- `../.agents/reference/i3More/` — DE source; **definitive list of OS-side requirements** (see Phase 8/9 references).
- `../.agents/reference/writeonce-all/` — sibling project; consult for conventions/infra patterns when relevant.
- `../scripts/survey-target-machine.sh` — re-run if T450 hardware changes (RAM upgrade, SSD swap, etc.).

---

## Verification — how to know each phase landed

| Phase | Verification command / observation |
| ----- | ----------------------------------- |
| 0     | `./build/cross-toolchain.sh` runs clean on a fresh workstation checkout |
| 1     | Rescue USB boots T450 in UEFI mode; netconsole captures Ubuntu boot log |
| 2     | T450 boots to BusyBox shell on custom kernel; `ping 1.1.1.1` works |
| 3     | `readlink /proc/1/exe` → `/sbin/writeonce-pid1`; zombie soak test passes |
| 4     | `wo-ctl list` shows expected services; cgroup hierarchy matches design |
| 5     | Cold boot under 10 s; recovery shell appears when root is unavailable |
| 6     | `efibootmgr -v` shows WriteOnce-only; no GRUB residue |
| 7     | `lsmod \| grep writeonce_thermal`; kernel-config-rationale.md complete |
| 8     | `Xorg + i3 + xterm + gtk4-demo` all run; D-Bus + PAM + PipeWire smoke-test pass |
| 9     | Power-on → login → i3More desktop in < 30 s |
| 10    | Two clean builds → identical ISO SHA-256; ISO installs on a fresh disk |

---

## Open questions to revisit at each phase boundary

1. **musl vs glibc** for the sysroot — current plan uses glibc (LFS default); revisit if musl saves enough size/complexity to be worth a redo.
2. **Logind shim depth** — start minimal (`Inhibit` only); expand as i3More + future apps demand. Re-run the OS-dep survey at Phase 9 start.
3. **DM strategy** — (a) console `wo-login` vs (b) graphical DM. Start with (a).
4. **Bootloader sophistication** — Phase 6 ships a minimal uefi-rs app. Decide later if a config-rich menu (chainloading, kernel param editing) is worth Phase 6b.
5. **Upstream contribution** — Phase 7 stretch goal; track separately, don't gate the project on it.
