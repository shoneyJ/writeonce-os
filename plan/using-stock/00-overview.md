# Alternative track — systemd as PID 1

> An alternative to the [`plan/phase-3-rust-pid1.md`](../../phase-3-rust-pid1.md) +
> [`plan/phase-4-supervisor.md`](../../phase-4-supervisor.md) pair. Adopts
> systemd as the canonical PID 1, supervisor, and logind provider instead of
> building those layers from scratch in Rust.

## Why this exists

Phases 3 and 4 of the primary plan are the highest-risk, highest-time portion
of the WriteOnce roadmap. A bug in PID 1 panics the kernel; a bug in the
supervisor breaks every service it owns. systemd is the contemporary
reference implementation of both contracts and has 15 years of accumulated
edge-case handling for SIGCHLD reaping races, cgroup placement under
process forking, D-Bus surface drift, user-session bookkeeping, etc. This
alternative track adopts systemd in exchange for skipping the from-scratch
PID 1 + supervisor work, getting WriteOnce to "T450 booting to i3More
desktop" faster.

The trade against the project's pedagogical premise is explicit:

| Goal of WriteOnce              | Default (Rust PID 1)               | This alternative (systemd)         |
| ------------------------------ | ---------------------------------- | ---------------------------------- |
| Understand PID 1 contract      | Author it; learn by failing        | Read systemd source ([[reference-systemd]]) |
| Own the supervisor design      | Author it; one user, one target    | Configure systemd                  |
| Own the bootloader             | Yes (Phase 6)                      | Yes (unchanged)                    |
| Own the kernel config + a Rust module | Yes (Phase 7)               | Yes (unchanged)                    |
| Own the userspace stack        | Yes; Xorg + curated                | Yes; Xorg + curated (unchanged)    |
| Time to first usable desktop   | Weeks                              | Days                               |

## When to choose this

This track makes sense if you want to:

1. **Verify the whole chain end-to-end** — kernel boots, hardware works,
   X11 + i3 + i3More renders, audio works — *before* investing weeks in a
   custom PID 1. Reduces "did I miss a hardware quirk in Phase 2?" risk.
2. **Develop a Rust PID 1 *inside* a working system** instead of *as* the
   entry point of a half-working system. After this track lands, you can
   swap the Rust PID 1 in via a kernel cmdline edit, with systemd as the
   known-good fallback.
3. **Reach a publishable artifact sooner** (an installable ISO that boots
   the T450 to i3More). The primary plan reaches this in ~10 phases; this
   alternative collapses it into ~8.

Do **not** choose this if the pedagogy of authoring PID 1 + supervisor is
itself the goal you came for — that's why the default plan exists.

## What this track changes

### Phases that go away

| Default plan                                | This alternative                            |
| ------------------------------------------- | ------------------------------------------- |
| Phase 3 — Rust PID 1                        | **dropped** (systemd is PID 1)              |
| Phase 4 — Rust supervisor + logind shim     | **dropped** (systemd-logind, systemd units) |

### Phases that change

- **Phase 5 (initramfs).** Still useful as a Rust learning exercise, but
  now optional. If kept, its `execve` target becomes `/sbin/init` (which is
  systemd's PID 1 entry, typically a symlink to `/lib/systemd/systemd`)
  rather than a custom `/sbin/writeonce-pid1`. If dropped, use a stock
  initramfs generator (dracut or mkinitcpio) integrated into the build.

- **Phase 9 (i3More integration).** Simpler. The bespoke logind D-Bus shim
  isn't needed because systemd-logind ships the real `org.freedesktop.login1.Manager.Inhibit`
  method natively. `i3more-lock` works out of the box.

### Phases that stay unchanged

- Phase 0 — cross-toolchain.
- Phase 1 — T450 prep (rescue USB, netconsole, firmware archive).
- Phase 2 — LFS minimal Linux + transitional BusyBox initramfs.
- Phase 6 — Rust UEFI bootloader (the boot-path ownership argument stands).
- Phase 7 — Kernel customization + Rust module experiment.
- Phase 8 — Xorg + i3 + GTK4 + D-Bus + PAM + PipeWire substrate.
- Phase 10 — Packaging, reproducible builds, install ISO.

## The new phase: 3-S — Build and configure systemd

This replaces Phases 3 and 4 in the timeline.

### Goal

A WriteOnce sysroot in which `/sbin/init` is systemd, configured to bring
up exactly the services i3More needs (D-Bus, PipeWire, Xorg, login). Every
other systemd component (resolved, networkd, timesyncd, homed, oomd,
portabled, sysupdate, sysext, userdb) is **disabled at build time**.

### Subtasks

1. **Add systemd + build deps to `build/versions.env`.**

   ```
   SYSTEMD_VERSION=256.x
   MESON_VERSION=1.5.x
   NINJA_VERSION=1.12.x
   LIBCAP_VERSION=2.70
   GPERF_VERSION=3.1
   ```

   meson + ninja + libcap + gperf are the principal new dependencies. The
   util-linux temp tools from LFS Chapter 6 already cover libmount/libblkid.

2. **Extend `01-fetch.sh` and `checksums.txt`** with the new tarballs and
   their GPG keys (`import-keys.sh` should pick the systemd key
   automatically from the .sig files).

3. **Build meson + ninja into the sysroot** (BLFS chapter, not LFS Ch. 6 —
   add a step `03b-meson-ninja` or similar).

4. **Write `build/07-systemd.sh`.** Same model as the existing numbered
   scripts: extract → configure → build → install. Key meson options for a
   *minimal* WriteOnce systemd build:

   ```
   meson setup build \
     --prefix=/usr \
     -Dsplit-usr=false \
     -Dsplit-bin=false \
     -Drootlibdir=/usr/lib \
     -Dsysvinit-path=/etc/init.d \
     -Dsysvrcnd-path=/etc/rc.d \
     -Dlocalstatedir=/var \
     \
     -Dresolved=false           # we use systemd-resolved? no.
     -Dnetworkd=false           # network managed by user-space tools later
     -Dtimesyncd=false          # ntpsec / chrony if needed
     -Dhomed=false              # we run as a single user
     -Doomd=false               # not on a 16 GB ThinkPad
     -Dportabled=false
     -Dsysupdate=false
     -Dsysext=false
     -Duserdb=false
     -Drepart=false
     \
     -Dlogind=true              # i3more-lock needs Inhibit()
     -Djournald=true            # central log
     -Dudev=true                # device hotplug, KMS, input
     -Dseccomp=true             # sandbox individual services
     -Dpam=true                 # PAM session integration
     -Dapparmor=false
     -Dselinux=false
   ```

   Build with: `meson compile -C build && DESTDIR=$LFS meson install -C build`

5. **Author WriteOnce-specific systemd unit files.** Place under
   `$LFS/etc/systemd/system/`:

   - `multi-user.target.wants/`: sshd.service (if enabled), getty@tty1.service
   - `graphical.target.wants/`: writeonce-graphical.service (launches Xorg + i3 + i3More)
   - `dbus.socket`, `dbus.service` (or rely on the upstream-shipped ones)

6. **Configure `/etc/systemd/system/default.target` → `graphical.target`.**

7. **Test in QEMU** (`build/06-qemu-smoke.sh` extended to expect the systemd
   banner instead of BusyBox's). Acceptance: cold boot lands at the
   graphical target, `systemctl status` shows expected services, `loginctl
   list-sessions` reports a session.

8. **Document divergence from upstream LFS-systemd.** WriteOnce's systemd
   build disables the majority of optional components — record this in
   `docs/learning/alt-systemd-minimal.md` so a future contributor knows
   which features are deliberately absent.

### Deliverable

A `$LFS/sbin/init` symlink to `/lib/systemd/systemd`. Cold boot through
the kernel + initramfs reaches systemd, which then activates
`graphical.target` and produces a login screen ready for i3More.

### Acceptance criteria

- `readlink /sbin/init` → `/lib/systemd/systemd` (or `/usr/lib/systemd/systemd` on merged-usr layouts).
- `systemctl --version` runs.
- `systemctl list-unit-files | wc -l` is in the hundreds (sanity check that
  the install is intact).
- `journalctl --boot` shows a clean boot to `graphical.target` with no
  units in `failed` state.
- `busctl --system tree org.freedesktop.login1` shows the real
  Inhibit/SuspendThenHibernate/etc. surface — not a shim.
- Time from power-on to `i3More` desktop on the T450: under 30 seconds.

### References

- `.agents/reference/systemd/` (the systemd source mirror; see [[reference-systemd]]).
  Particularly `src/core/main.c` (PID 1 entry), `src/core/manager.c`
  (lifecycle), `src/login/logind-dbus.c` (the logind surface i3More uses).
- BLFS book chapter "Systemd Utilities Group" for the upstream build flags
  the LFS world recommends.
- [systemd unit-file specification](https://www.freedesktop.org/software/systemd/man/systemd.unit.html).

## Migration paths

This track is **not a fork**. The default Rust track and this systemd track
can co-exist in the repo because Phases 0, 1, 2, 6, 7, 8, 9 (substantially),
10 are shared. The fork point is between Phase 2 and Phase 3/3-S.

Three realistic timelines:

1. **systemd-only.** Run Phase 0 → 2 → 3-S → 6 → 7 → 8 → 9 → 10. WriteOnce
   v0.1 ships with systemd; revisit Rust PID 1 in a v0.2.
2. **systemd-first, Rust-second.** Land 3-S, prove the chain, then in a
   later milestone re-implement Phase 3 (Rust PID 1) and Phase 4 (Rust
   supervisor) and add a GRUB / Rust-bootloader menu entry to choose between
   them at boot time. The systemd path remains the safety net.
3. **Rust-only (default plan).** Skip this track entirely.

The plan files for the Rust track (`phase-3-rust-pid1.md`,
`phase-4-supervisor.md`) remain in place regardless of which timeline is
followed — they describe the eventual end state of full ownership, even
if reached via the indirect route.

## Risks specific to this track

| Risk                                                                      | Mitigation                                                              |
| ------------------------------------------------------------------------- | ----------------------------------------------------------------------- |
| systemd build fails on the cross-toolchain (it's meson + complex deps)   | Build native inside a chroot; cross-compiling systemd is its own project |
| Disabled systemd components turn out to be required by i3More transitively| Re-survey i3More's D-Bus calls; toggle the relevant meson option back on |
| systemd init introduces a binary blob mindset the project meant to avoid  | Track this in `docs/learning/` honestly; treat it as the explicit trade-off it is |
| Reproducibility hurts (systemd build is large, with conditional deps)     | Pin systemd version + every dep version + record meson cache hash in artifacts |

## What stays Rust regardless

Even on this track, several layers stay Rust-authored:

- **Phase 6 bootloader** — `uefi-rs` EFI app. systemd does not write
  bootloaders; this remains WriteOnce's own.
- **Phase 7 Rust kernel module** — the thermal `/dev` exercise stands.
- **`wo-login` (Phase 9)** — *if* the console-login path (option 9a) is
  preferred over a graphical DM, the small Rust binary that wraps PAM
  remains. With systemd, this is alternatively replaceable by
  `systemd-logind` + `agetty --autologin` plus a `.bash_profile` that
  launches X.
- **Future Rust crates** — kernel modules, observability tools, an
  alternative supervisor that runs *under* systemd as a slice manager.

So this isn't "abandon Rust" — it's "pick which contracts to author and
which to delegate." PID 1 contract gets delegated; everything around it
stays in scope.

## Concrete next step on this track

If you want to pilot this:

1. Add the systemd + meson + ninja + libcap + gperf entries to
   `build/versions.env`.
2. Run `./01-fetch.sh` once — it'll record the new SHA-256s as next-locks
   and download a few hundred MB of sources.
3. Run `./import-keys.sh` to grab the systemd signing key (Lennart's, or
   the rotating release manager's).
4. Commit those updates as a separate "systemd track scaffolding" commit so
   the diff is reviewable.
5. Then start drafting `build/07-systemd.sh`.

Say the word and I'll scaffold the fetch updates + a `07-systemd.sh`
skeleton matching the same step-function pattern as `04-kernel.sh`.
