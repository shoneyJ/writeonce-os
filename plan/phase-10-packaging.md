# Phase 10 — Packaging, reproducibility, install ISO

**Goal.** Anyone (including future-user) can rebuild WriteOnce from scratch and produce an installable artifact. Locks in the learning.

## Subtasks

1. **Reproducible builds audit.**
   - Pin every source tarball by SHA-256.
   - Pin Rust toolchain via `rust-toolchain.toml`.
   - Strip build timestamps (`SOURCE_DATE_EPOCH`), normalize paths.
   - Verify with two consecutive clean builds: `sha256sum build/artifacts/*` must match.

2. **Package format decision.** Three real options:
   - **(a)** Source-build only (no packages) — simplest, true to LFS spirit.
   - **(b)** Custom tarball format with manifest (name, version, files, deps, scripts).
   - **(c)** Nix-style content-addressed store at `/wo/store/<hash>-<name>/`.
   - **Recommendation:** start with (a) for Phase 2–9; add (b) in this phase as a learning artifact.

3. **Install ISO.**
   - Build a Hybrid UEFI ISO via `xorriso`. Contents: Rust bootloader (Phase 6) → live kernel + initramfs → Rust installer that runs the Phase 2/8/9 install sequence on the target's disk.
   - The installer is a Rust binary: prompts for disk, hostname, user; runs `parted`, `mkfs`, copies the sysroot, runs `efibootmgr` to register the bootloader.

4. **Documentation.**
   - `docs/install.md` — user-facing install instructions.
   - `docs/kernel-config-rationale.md` — finalized from Phase 7.
   - `docs/architecture.md` — boot sequence with WriteOnce-specific component names slotted in.
   - `docs/recovery.md` — how to fix it when it breaks.

5. **CI** — at minimum, a workstation script `make ci` that does: fetch → toolchain → kernel → initramfs → all Rust crates `cargo test` → build ISO → boot ISO in QEMU → smoke-test it boots to login prompt. Run before any tagged release.

6. **Tag v0.1.0** when the ISO installs cleanly on a wiped USB-stick test target (separate from the production T450 install).

## Deliverable

`build/artifacts/writeonce-v0.1.0-x86_64.iso` that installs the OS on a wiped disk.

## Acceptance criteria

- Two consecutive builds from a clean checkout produce identical ISO SHA-256s.
- The ISO boots in QEMU UEFI mode (OVMF) and reaches the installer.
- A third laptop / VM, wiped and installed via the ISO, reaches the i3More desktop.
