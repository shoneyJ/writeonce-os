# CLAUDE.md

## 0. Take Pride in providing outstanding results

**The result speaks for itself**

- You go the extra mile if the result is worth it
- You dont sugarcoat subpar solutions, you despise them
- You think outside the box

## 1. Think Before Coding

**Don't assume. Don't hide confusion. Surface tradeoffs.**

Before implementing:

- State your assumptions explicitly. If uncertain, ask.
- If multiple interpretations exist, present them - don't pick silently.
- If a simpler approach exists, say so. Push back when warranted.
- If something is unclear, stop. Name what's confusing. Ask.

## 2. Simplicity First

**Minimum code that solves the problem. Nothing speculative.**

- No features beyond what was asked.
- No abstractions for single-use code.
- No "flexibility" or "configurability" that wasn't requested.
- No error handling for impossible scenarios.
- If you write 200 lines and it could be 50, rewrite it.

Ask yourself: "Would a senior engineer say this is overcomplicated?" If yes, simplify.

## 3. Surgical Changes

**Touch only what you must. Clean up only your own mess.**

When editing existing code:

- Don't "improve" adjacent code, comments, or formatting.
- Don't refactor things that aren't broken.
- Match existing style, even if you'd do it differently.
- If you notice unrelated dead code, mention it - don't delete it.

When your changes create orphans:

- Remove imports/variables/functions that YOUR changes made unused.
- Don't remove pre-existing dead code unless asked.

The test: Every changed line should trace directly to the user's request.

## 4. Goal-Driven Execution

**Define success criteria. Loop until verified.**

Transform tasks into verifiable goals:

- "Add validation" → "Write tests for invalid inputs, then make them pass"
- "Fix the bug" → "Write a test that reproduces it, then make it pass"
- "Refactor X" → "Ensure tests pass before and after"

For multi-step tasks, state a brief plan:

```
1. [Step] → verify: [check]
2. [Step] → verify: [check]
3. [Step] → verify: [check]
```

Strong success criteria let you loop independently. Weak criteria ("make it work") require constant clarification.

## 4. Fix Errors as you encounter them

**An error means a broken baseline. Fix any error you encounter. No bandaids.**

---

**These guidelines are working if:** fewer unnecessary changes in diffs, fewer rewrites due to overcomplication, and clarifying questions come before implementation rather than after mistakes.

---

# Repository context

WriteOnce OS: a from-scratch Linux distro targeting a ThinkPad T450 (Broadwell). Cross-compiled from this workstation, deployed to the T450.

**Status: Phase 9 (i3More desktop bring-up).** Phases 0–8 are complete — cross-toolchain → kernel → initramfs → Rust PID 1 / supervisor / UEFI bootloader → the 85-package X11/Mesa-iris/GTK4/audio/network substrate (their plans are archived under `plan/done/`). The T450 now boots `writeonce-pid1 → writeonce-svc → dbus → logind → writeonce-login` on tty1. Recent boot-chain fixes have landed and await an end-to-end verification boot: libpam staging (the `$LFS/lib64` → `usr/lib` merge in `17-stage-sysroot.sh`), the `writeonce-bootstrap` oneshot (machine-id + `/run/*` dirs), and the `xinit`/`startx` session launcher. `writeonce-svc` Phase-4 hardening (User=/Group= drop, per-unit logs, oneshot start-timeout, cgroup teardown, `wo-ctl journal/cgroups`) is done. Next: verify the boot reaches the i3More X11 desktop, then Phase 10 (packaging).

## Authoritative reading order
1. `writeonce-session-notes.md` — the *why*
2. `plan/00-roadmap.md` — phase map (Phase 0 → Phase 10) + cross-cutting tracks
3. `plan/phase-9-i3more.md`, `plan/phase-10-packaging.md` — active phases (completed phases archived in `plan/done/`); `plan/writeonce-svc-fix/` holds the in-flight boot-bring-up rounds
4. `docs/learning/` — long-form rationale captured per phase (per the `feedback_persist_explanations` convention)

## Phase-to-component map
- **Phase 0 (Bash, `build/02-cross-toolchain.sh`)** — LFS Ch. 5 cross-toolchain → `$LFS/tools/`
- **Phase 2 (Bash, `build/03-` to `build/06-`)** — kernel, initramfs, QEMU smoke
- **Phases 3-6 (Rust, `crates/`)** — PID 1, supervisor, initramfs, UEFI bootloader, login/logind/installer
- **Phase 8 (Bash, `build/08-` to `build/13-`)** — Xorg/Mesa/GTK4/audio/network substrate built cross via `blfs-pkg.sh`
- **Phase 9** — **NOT built here**: i3 + i3More artifacts come from `.agents/reference/i3More/`, copied in by `build/17-stage-sysroot.sh`

## Build commands

All shell builds run in the `wo-builder` Docker image (auto-rebuilt when `build/Containerfile` changes):

```bash
./build/in-container.sh ./build/<NN-step>.sh           # one phase step
./build/in-container.sh --no-network ./build/<NN>.sh   # compile-only, supply-chain isolated
./build/in-container.sh                                # interactive shell in image
```

Each `build/NN-*.sh` script is sentinel-driven (`build/logs/.done-<step>`). Delete the sentinel to redo. Per-step logs live in `build/logs/<step>-{configure,make,install}.log`.

`build/blfs-pkg.sh` (sourced library) provides `build_pkg` (autoconf) and `build_meson` (meson) used by all Phase 8 scripts. It writes a meson cross-file with `needs_exe_wrapper = true` so packages like Mesa correctly identify the build helpers as native.

Phase 0 environment (`build/setup-env.sh`):
- `$LFS = build/sysroot/` — target rootfs being built
- `$LFS_TOOLS = build/cross-tools/` — host-resident cross-gcc/binutils (exposed inside sysroot at `$LFS/tools`)
- `$LFS_TGT = x86_64-lfs-linux-gnu`
- `CFLAGS/CXXFLAGS/LDFLAGS` baked with `--sysroot=$LFS` because the cross-gcc was *not* built with `--with-sysroot=$LFS` (gcc-pass1 used `--without-headers`)

## Rust workspace

`cargo build --workspace --target x86_64-unknown-linux-musl --release` cross-builds all `crates/*` to static musl binaries. Toolchain pinned via `rust-toolchain.toml`; musl + uefi targets pre-added in the container.

## Conventions
- **Numbered build scripts** (`00-`, `01-`, … `18-`): see `feedback_numbered_scripts` memory. `setup-env.sh` and `blfs-pkg.sh` stay unnumbered — they're sourced libraries.
- **Long-form rationale → `docs/learning/`**, not inline comments. See `feedback_persist_explanations` memory.
- **No dev libs on host** — install in `build/Containerfile`, then build inside the container. See `feedback_dev_libs_in_container` memory.
- **Host-vs-target rule** — host limits never drive target compromises; fix the host (Containerfile), not the target. See `feedback_host_vs_target_decisions` memory.
- **i3 / i3More are external** — built in the i3More repo, copied in by `17-stage-sysroot.sh`. Do not build them here. See `project_i3_external` memory.

## Upstream references (read-only symlinks under `.agents/reference/`)
- `lfs/` — Linux From Scratch book sources (build recipe authority)
- `linux/` — kernel tree (pinned to 6.12 LTS)
- `systemd/` — read-only reference for PID 1 / logind shim design (not vendored)
- `i3More/` — the end-goal desktop environment (defines the userspace substrate Phase 8 must provide)
