# WriteOnce OS

A from-scratch Linux distribution, built as a learning vehicle for kernel internals and userspace primitives. Target: a ThinkPad T450 running the user's own desktop environment, [i3More](https://github.com/shoneyj/i3More), on top of an OS where every layer below it was understood and (where reasonable) re-implemented by the author.

## Status

**Phases 0–8 complete; Phase 9 (i3More desktop) bring-up.** Phases 0–8 land:
cross-toolchain, kernel, initramfs, Rust PID 1 + supervisor + UEFI bootloader,
and the 85-package userspace substrate (Xorg + Mesa/iris, GTK4, alsa-lib +
pipewire + wireplumber, ell + iwd + iproute2 + iputils + dhcpcd). Kernel rebuilt
2026-05-24 with the 30 `kernel-config-additions.fragment` options
(CONTAINERS, BPF, MICROCODE_INTEL, MISC_RTSX, …) — bzImage 14 MB, initramfs.img
2.4 MB. Completed phase plans are archived under [`plan/done/`](plan/done/).

**Now:** the T450 boots `writeonce-pid1 → writeonce-svc → dbus → logind →
writeonce-login` on tty1. The fixes that clear the last boot failures (seen in
`.agents/PXL_20260527_213400915.jpg`) have landed and await a verification boot:
libpam staging (the `$LFS/lib64` → `usr/lib` merge in `17-stage-sysroot.sh`),
the `writeonce-bootstrap` oneshot (machine-id + `/run/*` dirs), and the
`xinit`/`startx` session launcher. Then: login → `startx` → Xorg → i3 + i3More.
See [`plan/writeonce-svc-fix/`](plan/writeonce-svc-fix/) and
[`docs/learning/t450-boot-debugging.md`](docs/learning/t450-boot-debugging.md).

Quick driver: `just phase-8a` … `just phase-8f` runs each Phase-8 round; on a
break use `just audit-last` to surface the next missing dep. Before flashing,
`just check-staging` validates the staged sysroot (libs, units, `startx`,
dbus/logind ldd). See `justfile`.

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
plan/                        Build roadmap (00-roadmap) + active phase plans (phase-9, phase-10);
                             completed phases archived in plan/done/; writeonce-svc-fix/ = boot bring-up
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
3. Active phase plans — [`plan/phase-9-i3more.md`](plan/phase-9-i3more.md), [`plan/phase-10-packaging.md`](plan/phase-10-packaging.md) — for the *what next*; completed phases are in [`plan/done/`](plan/done/)
4. [`paper/writeonce.pdf`](paper/writeonce.pdf) — the long-form technical paper (10 pp.), maintained from [`paper/writeonce.tex`](paper/writeonce.tex); cites the LFS book and contemporary references. Rebuild with `cd paper && make`.

See [Documentation map](#documentation-map) below for the full set of in-repo docs.

## Documentation map

Every in-repo Markdown document, grouped and linked. (External reference mirrors
under `.agents/reference/` are covered in *Upstream references* below.)

**Top level**
- [`CLAUDE.md`](CLAUDE.md) — contributor/agent guide + repository context
- [`writeonce-session-notes.md`](writeonce-session-notes.md) — architectural source of truth (LFS phases, 15-phase boot, language-per-phase)
- [`.agents/target-machine.md`](.agents/target-machine.md) — T450 hardware survey output

**Plan — active** ([`plan/`](plan/))
- [`00-roadmap.md`](plan/00-roadmap.md) — master phase index (Phase 0 → 10)
- [`phase-9-i3more.md`](plan/phase-9-i3more.md) · [`phase-10-packaging.md`](plan/phase-10-packaging.md)
- [`developer-workstation-implementation.md`](plan/developer-workstation-implementation.md) · [`Kernel7/kernel-7.0-bump.md`](plan/Kernel7/kernel-7.0-bump.md)
- Boot bring-up rounds: [`writeonce-svc-fix/escape-the-loop.md`](plan/writeonce-svc-fix/escape-the-loop.md) · [`fix-learn-from-scratch-boot.md`](plan/writeonce-svc-fix/fix-learn-from-scratch-boot.md) · [`fix-libpam-and-dbus.md`](plan/writeonce-svc-fix/fix-libpam-and-dbus.md)

**Plan — done** ([`plan/done/`](plan/done/)) — completed phases
- [`phase-0-toolchain.md`](plan/done/phase-0-toolchain.md) · [`phase-1-target-prep.md`](plan/done/phase-1-target-prep.md) · [`phase-2-minimal-linux.md`](plan/done/phase-2-minimal-linux.md) · [`phase-3-rust-pid1.md`](plan/done/phase-3-rust-pid1.md) · [`phase-4-supervisor.md`](plan/done/phase-4-supervisor.md)
- [`phase-5-initramfs.md`](plan/done/phase-5-initramfs.md) · [`phase-6-bootloader.md`](plan/done/phase-6-bootloader.md) · [`phase-7-kernel.md`](plan/done/phase-7-kernel.md) · [`phase-7-kerngen.md`](plan/done/phase-7-kerngen.md) · [`phase-8-x11-gtk4.md`](plan/done/phase-8-x11-gtk4.md)

**Crate READMEs** ([`crates/`](crates/)) — Rust boot-path components
- [`writeonce-bootloader`](crates/writeonce-bootloader/README.md) · [`writeonce-initramfs`](crates/writeonce-initramfs/README.md) · [`writeonce-pid1`](crates/writeonce-pid1/README.md) · [`writeonce-installer`](crates/writeonce-installer/README.md)
- No README yet: `writeonce-svc`, `writeonce-login`, `writeonce-logind`, `writeonce-session-create`, `writeonce-kerngen`

**docs/ — long-form rationale**
- [`kernel-build-history.md`](docs/kernel-build-history.md)
- Cross-cutting: [`00-concepts-coverage.md`](docs/learning/00-concepts-coverage.md) · [`supply-chain-defense.md`](docs/learning/supply-chain-defense.md) · [`containers-kernel-requirements.md`](docs/learning/containers-kernel-requirements.md) · [`multi-gpu-portability.md`](docs/learning/multi-gpu-portability.md) · [`future-installer-remote-build.md`](docs/learning/future-installer-remote-build.md) · [`t450-boot-debugging.md`](docs/learning/t450-boot-debugging.md)
- Phase 0: [`phase-0-cross-toolchain.md`](docs/learning/phase-0-cross-toolchain.md) · [`phase-0-lfs-tools-layout.md`](docs/learning/phase-0-lfs-tools-layout.md) · [`phase-0-temp-tools-result.md`](docs/learning/phase-0-temp-tools-result.md)
- Phase 2: [`phase-2-busybox.md`](docs/learning/phase-2-busybox.md) · [`phase-2-chroot.md`](docs/learning/phase-2-chroot.md)
- Phase 4: [`phase-4-cgroup-isolation.md`](docs/learning/phase-4-cgroup-isolation.md) · [`phase-4-concurrency-and-io-uring.md`](docs/learning/phase-4-concurrency-and-io-uring.md) · [`phase-4-dependency-graph.md`](docs/learning/phase-4-dependency-graph.md) · [`phase-4-service-toml-schema.md`](docs/learning/phase-4-service-toml-schema.md) · [`phase-4d-logind-shim.md`](docs/learning/phase-4d-logind-shim.md) · [`systemd-feature-survey.md`](docs/learning/systemd-feature-survey.md)
- Phase 6: [`phase-6-bootloader-efi-stub-delegation.md`](docs/learning/phase-6-bootloader-efi-stub-delegation.md)
- Phase 8: [`phase-8-userspace-build-strategy.md`](docs/learning/phase-8-userspace-build-strategy.md) · [`phase-8b-x11-protocol-stack.md`](docs/learning/phase-8b-x11-protocol-stack.md) · [`phase-8c-xorg-server-and-drm.md`](docs/learning/phase-8c-xorg-server-and-drm.md) · [`phase-8d-gtk-stack.md`](docs/learning/phase-8d-gtk-stack.md) · [`phase-8e-audio-stack.md`](docs/learning/phase-8e-audio-stack.md) · [`phase-8f-network-stack.md`](docs/learning/phase-8f-network-stack.md)
- Phase 9–10: [`phase-9-desktop-bringup.md`](docs/learning/phase-9-desktop-bringup.md) · [`phase-10-installer.md`](docs/learning/phase-10-installer.md)

**Build**
- [`build/README.md`](build/README.md) · [`build/keys/README.md`](build/keys/README.md)

## Upstream references

External projects this build draws from. All are mirrored locally as read-only symlinks under `.agents/reference/` for fast grepping.

### Linux From Scratch (LFS) book

- **Upstream:** [github.com/lfs-book/lfs](https://github.com/lfs-book/lfs) — official sources of the LFS book (DocBook XML)
- **Project site:** [linuxfromscratch.org](https://www.linuxfromscratch.org/)
- **Local mirror:** [`.agents/reference/lfs/`](.agents/reference/lfs/)
- **Used by:** [`plan/done/phase-0-toolchain.md`](plan/done/phase-0-toolchain.md) (LFS chapters 5–6 = cross-toolchain + temporary tools), [`plan/done/phase-2-minimal-linux.md`](plan/done/phase-2-minimal-linux.md) (chapter 7+ chroot flow)
- **Why:** authoritative recipe for the LFS-style build sequence — package versions, build orders, configure flags. WriteOnce may deviate, but deviations are documented per-phase.

### Linux kernel

- **Upstream:** [kernel.org](https://www.kernel.org/) / [git.kernel.org](https://git.kernel.org/pub/scm/linux/kernel/git/torvalds/linux.git)
- **Local mirror:** [`.agents/reference/linux/`](.agents/reference/linux/)
- **Pinned version:** Linux 6.12 LTS (see [`plan/done/phase-2-minimal-linux.md`](plan/done/phase-2-minimal-linux.md))
- **Used by:** every kernel-touching phase; primary entry points are `init/main.c`, `kernel/exit.c`, `fs/init.c`, `Documentation/rust/`, `arch/x86/`.

### i3More

- **Upstream:** [github.com/shoneyj/i3More](https://github.com/shoneyj/i3More) (user's own project)
- **Local mirror:** [`.agents/reference/i3More/`](.agents/reference/i3More/)
- **Used by:** [`plan/done/phase-8-x11-gtk4.md`](plan/done/phase-8-x11-gtk4.md) (substrate requirements), [`plan/phase-9-i3more.md`](plan/phase-9-i3more.md) (integration).
- **Why:** end-goal desktop environment — its dependency surface (X11, GTK4, D-Bus, PAM, PipeWire) defines the userspace stack WriteOnce must provide.

### systemd

- **Upstream:** [github.com/systemd/systemd](https://github.com/systemd/systemd)
- **Local mirror:** [`.agents/reference/systemd/`](.agents/reference/systemd/) (shallow clone)
- **Used by:** [`plan/done/phase-3-rust-pid1.md`](plan/done/phase-3-rust-pid1.md) (PID 1 contract), [`plan/done/phase-4-supervisor.md`](plan/done/phase-4-supervisor.md) (service supervisor + cgroup v2 + logind D-Bus surface).
- **Why:** the most battle-tested PID 1 and service supervisor in the Linux world. WriteOnce is deliberately *not* using systemd, but reads it as the reference implementation for edge cases (reaping, signal handling, cgroup placement, dependency resolution, logind D-Bus). Read for understanding; do not vendor or port. Key files: `src/core/main.c` (PID 1 entry), `src/core/manager.c` (state machine), `src/core/cgroup.c`, `src/login/logind-dbus.c`.

## License

Not yet decided; will be set before the first code lands.
