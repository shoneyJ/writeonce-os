# WriteOnce OS

A from-scratch Linux distribution, built as a learning vehicle for kernel internals and userspace primitives. Target: a ThinkPad T450 running the user's own desktop environment, [i3More](https://github.com/shoneyj/i3More), on top of an OS where every layer below it was understood and (where reasonable) re-implemented by the author.

## Status

**Phase 8 complete — 85 packages cross-built.** Phases 0–8 land:
cross-toolchain, kernel, initramfs, Rust PID 1, supervisor, bootloader,
Xorg + Mesa (iris), GTK4, alsa-lib + pipewire + wireplumber, ell + iwd
+ iproute2 + iputils + dhcpcd. Kernel rebuilt 2026-05-24 with the 30
missing CONFIG_* options from `kernel-config-additions.fragment`
(CONTAINERS, BPF, MICROCODE_INTEL, MISC_RTSX, etc.) — bzImage 14 MB,
initramfs.img 2.4 MB.

Next: Phase 9 (i3 + i3More integration via `17-stage-sysroot.sh` →
`18-make-artifacts.sh`).

Quick driver: `just phase-8a` … `just phase-8f` runs each round; on
failure use `just audit-last` to surface the next missing dep, fix,
re-run. See `justfile`.

## Phase 8 build fixes (chronological)

Captured here so the next person doesn't re-trial-and-error these.

### Toolchain
- **gcc-pass2 added.** gcc-pass1 was built `--disable-threads
  --without-headers --disable-libstdcxx` (pre-glibc bootstrap); the
  resulting libstdc++ left `_GLIBCXX_HAS_GTHREADS` undefined →
  `std::mutex` missing → Mesa's `texcompress_astc_luts.h` failed to
  compile. New `step_gcc-2` in `02-cross-toolchain.sh` rebuilds the
  cross gcc with `--enable-threads=posix --enable-shared
  --with-sysroot=$LFS`, then re-runs `step_libstdcxx` against it.
- **libstdc++ standalone needs gthr-default.h.** Symlinked
  `libgcc/gthr-posix.h` → `gthr-default.h` before configuring so the
  `gthreads library` probe finds it (otherwise `_GLIBCXX_HAS_GTHREADS`
  stays undefined even with threaded gcc).
- **meson cross-file: `needs_exe_wrapper = true`.** Same-arch cross
  builds (x86_64 host, x86_64 target) made meson think it could run
  target binaries on the host. Mesa et al. then linked their build-time
  helpers (mesa_clc/intel_clc) against host libs via the cross linker,
  which couldn't resolve transitive deps through `--sysroot=$LFS`. The
  flag forces native:true on those helpers.

### Mesa
- **24.3.4 → 24.0.9.** Mesa 24.1+ implicitly forces `intel_clc` when
  the iris driver is enabled. intel_clc's static-lib deps are
  host-machine in meson terms, but the executable wants build-machine
  → "mixed machine" error with no clean workaround. 24.0.9 only builds
  intel_clc on explicit `-Dintel-clc=enabled`. Broadwell HD 5500 hits
  the same OpenGL 4.6 / GLSL 4.60 caps on either release.
- **No LLVM at all.** iris doesn't need LLVM (uses Mesa's in-tree
  brw_compile C backend). With Mesa 24.0.9 + `-Dllvm=disabled`, the
  entire LLVM toolchain (libclc, llvm-19-dev, libllvmspirvlib,
  spirv-tools) dropped from the Containerfile.
- **Mesa option fixes:** `-Dglvnd=disabled` (feature) → `=false`
  (boolean); removed `-Dintel-rt`, `-Dinstall-intel-clc`,
  `-Dlmsensors` (not in 24.0).

### Phase 8 missing packages
| Pkg | Why | Phase | Source |
|---|---|---|---|
| pixman | xorg-server's Render/Composite ext | 8c | cairographics.org |
| libxkbfile | xorg-server keyboard subsystem | 8c | x.org |
| font-util | xorg-server's `fontutil.pc` | 8c | x.org |
| libxcvt | xorg-server mode-line calc | 8c | x.org |
| libmd | xorg-server SHA1 backend (smallest of md/libsha1/nettle/gcrypt/openssl) | 8c | hadrons.org |
| libXdmcp | xorg-server `xdmcp.pc` | 8c | x.org |
| pcre2 | glib's GRegex (no system → wrap downloads → fails offline) | 8d | github.com/PCRE2Project |
| fribidi | pango bidi text (wrap fallback otherwise) | 8d | github.com/fribidi |
| libtiff | gtk4 4.16 hardcodes it as required (no flag to disable) | 8d | download.osgeo.org |
| readline | iwd's iwctl CLI line editor | 8f | gnu.org |
| libcap | iputils' ping links libcap for CAP_NET_RAW | 8f | mirrors.edge.kernel.org |

### xorg-server option fixes
- Removed `-Dxwayland=false` — not a valid option in 21.1.x (xwayland
  is a separate package).
- `-Ddevel_docs` → `-Ddevel-docs` (dash, not underscore).
- `-Dsecure-rpc=false` — glibc 2.40 dropped Sun-RPC; libtirpc not
  built; modern desktops don't use DES auth.
- `-Dxdm-auth-1=false` — DES-based XDM auth, equally dead.

### Phase 8e (audio) / 8f (network) script bugs
- **`local A=foo B=$A` doesn't expand `$A` under `set -u`.** Bash
  evaluates RHS before the local-scope binding exists; result is
  "name: unbound variable" on the helper-script-locals pattern used
  for lua / iproute2 / dhcpcd / readline. Split into two `local` lines.
- **wireplumber option:** `-Ddocumentation=disabled` → `-Ddoc=disabled`
  (actual upstream option name).
- **ncurses.pc hand-rolled** before readline. LFS Ch.6 built ncurses
  without `--enable-pc-files`, so readline's auto-generated .pc lists
  `Requires.private: ncurses` and downstream `pkg-config --exists
  readline` fails (which is what iwd does, not direct linking). Six
  lines of static .pc text + ncursesw.pc / tinfo.pc symlinks resolve it.
- **readline 8.2 SHLIB_LIBS override.** readline's configure correctly
  detects `tgetent` in `-lncurses`, but the resulting shared-lib
  Makefile leaves `SHLIB_LIBS` empty — so `libreadline.so` has no
  DT_NEEDED on libncursesw and clients (iwctl) fail to link with
  undefined `tputs`/`tgetent`/etc. Pass `SHLIB_LIBS=-lncursesw` to
  both `make` and `make install`; also export `bash_cv_termcap_lib`
  during configure. Same workaround the LFS book uses.
- **`just audit-last` only sees meson failures.** Autoconf packages
  (iwd, ell, readline, iproute2, dhcpcd) leave no `meson-log.txt`, so
  the tool surfaces the *previous* meson failure instead. For autoconf
  failures, read `build/logs/blfs-<pkg>-configure.log` directly — the
  "stopping at <pkg>" line in the build output names the right package.

### GTK4 stack
- **gobject-introspection dropped from STEPS.** Genuinely hostile to
  cross-compile (probes target-arch binaries that must run on the
  build machine; needs target `python-3.12.pc`). i3More uses static
  `gtk4-rs` bindings — runtime introspection is never invoked.
- **hicolor-icon-theme 0.18:** migrated from autoconf to meson —
  changed `build_pkg` → `build_meson`.
- **shared-mime-info tarball:** GitLab's archive endpoint returns a
  16 KB HTML auth page for `.tar.xz` but serves `.tar.gz` correctly.
  Switched URL + extension.
- **gtk4 4.16 option renames:** removed `-Dgtk_doc=false` (renamed
  to `documentation`, already set) and `-Dmedia-ffmpeg=disabled`
  (option dropped; only gstreamer backend remains).

### Kernel rebuild (Phase 5)
- **`bc` + `libssl-dev` missing in container.** Kernel 6.12 needs
  `bc` for `timeconst.h` generation and `openssl/bio.h` (libssl-dev)
  for `certs/extract-cert`. Added both to `build/Containerfile`.
- **`step_kernel-build` phantom sentinel.** Same un-chained
  `make … | tee … ; cp …` pattern as libjpeg-turbo: make failed but
  the cp silently no-op'd on the missing bzImage, leaving a stub
  bzImage from initial setup in artifacts/. Chained with `&&` so
  cp + sentinel only run on successful make.

### Phantom sentinel bug (Phase 8a)
- `step_libjpeg-turbo` in `08-base-substrate.sh` ran `cmake -S/-B/install`
  un-chained → configure silently failed (missing
  `CMAKE_SYSTEM_PROCESSOR=x86_64` → SIMD detection blew up → no
  Makefile generated) → `touch sentinel` ran anyway → downstream
  pkg-config couldn't find `libjpeg.pc` (we'd "built" nothing).
  Chained with `&&`, added the missing var. `gdk-pixbuf`'s
  `just audit-last` is what surfaced this — a foundation-level
  bug detected only by an auditor four phases later.

### Tooling that came out of this
- `build/audit-deps.sh` + `just audit <pkg>` / `just audit-last` —
  parses meson-log.txt for required-fatal + optional-NO probes.
  See *Workflow for the next break* below.
- `.agents/reference/xserver` — shallow clone of
  freedesktop.org/xorg/xserver for fast grep when probing options.

### Workflow for the next break
```bash
just phase-8d        # fails on $PKG
just audit-last      # shows: Fatal / Required / Optional / Headers / Programs
# fix per the audit; re-run.
```
`— Fatal:` empty + sentinel set ⇒ the build actually succeeded
(audit ran on a stale log). Use `just progress` to confirm.

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
paper/                       LaTeX technical paper (writeonce.tex + writeonce.bib + PDF)
scripts/
  survey-target-machine.sh   Hardware survey script run on the T450
.agents/
  target-machine.md          Captured hardware survey output
  reference/                 Symlinks to external repos used as read-only reference material
    linux/                   → ~/projects/linux/ (kernel source)
    i3More/                  → ~/projects/github/shoneyj/i3More/ (target DE)
    lfs/                     → ~/projects/github/lfs-book/lfs/ (LFS book sources)
    systemd/                 → ~/projects/github/systemd/systemd/ (PID 1 reference)
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
4. [`paper/writeonce.pdf`](paper/writeonce.pdf) — the long-form technical paper (10 pp.), maintained from [`paper/writeonce.tex`](paper/writeonce.tex); cites the LFS book and contemporary references. Rebuild with `cd paper && make`.

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

### systemd

- **Upstream:** [github.com/systemd/systemd](https://github.com/systemd/systemd)
- **Local mirror:** [`.agents/reference/systemd/`](.agents/reference/systemd/) (shallow clone)
- **Used by:** [`plan/phase-3-rust-pid1.md`](plan/phase-3-rust-pid1.md) (PID 1 contract), [`plan/phase-4-supervisor.md`](plan/phase-4-supervisor.md) (service supervisor + cgroup v2 + logind D-Bus surface).
- **Why:** the most battle-tested PID 1 and service supervisor in the Linux world. WriteOnce is deliberately *not* using systemd, but reads it as the reference implementation for edge cases (reaping, signal handling, cgroup placement, dependency resolution, logind D-Bus). Read for understanding; do not vendor or port. Key files: `src/core/main.c` (PID 1 entry), `src/core/manager.c` (state machine), `src/core/cgroup.c`, `src/login/logind-dbus.c`.

## License

Not yet decided; will be set before the first code lands.
