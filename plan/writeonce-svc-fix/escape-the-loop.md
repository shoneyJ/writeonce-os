# Escape the per-iteration-bug loop

> Strategy plan + this-round implementation list. Goal: stop the
> "one missing file / wrong path per T450 boot" thrash by
> centralising all pre-service plumbing into a single oneshot, and
> validating the staged sysroot on the **workstation** before the
> USB flash + boot cycle.

## Why

Four consecutive bring-up rounds in this session each surfaced
1-3 fresh issues that share the same shape: "service X expects
file/directory/permission Y that nobody created":

| Round | Bug class | Resolution |
|-------|-----------|------------|
| 1 | Read-only filesystem | initramfs cmdline `rw` parsing |
| 1 | Burst-cap math wrong | tightened defaults |
| 2 | Hardcoded host assumptions (dhcpcd interface, iwd in boot path) | bare-minimum-boot refit; enabled.d opt-in model |
| 3 | libpam not staged | merge `$LFS/lib*` → `usr/lib` |
| 3 | `/etc/machine-id` missing | installer writes per-install |
| 3 | `/run/dbus/` doesn't exist on tmpfs | pid1 creates it post-mount |

Pattern: each fix is correct but lives in a different place
(initramfs, supervisor defaults, unit TOMLs, staging script,
installer customize, PID 1 mount). The next "service X needs Y"
fix will live in yet *another* place. We're chasing files across
six modules.

## The escape

**Centralise pre-service plumbing in one place: `writeonce-bootstrap`,
a oneshot that runs before dbus.service.** Anything in the shape
"create dir / make file / fix permission before services start"
goes here. Future "service X needs Y" is one line added to a shell
script — not a new crate edit + Rust rebuild cycle.

**Plus: `just check-staging`, a workstation-side pre-flight script
that asserts the staged sysroot has what it needs before USB flash.**
Catches missing libs / unit files / skeleton entries **today**, on
the workstation, instead of **tomorrow**, after a 5-minute USB+boot
cycle on the T450.

The two together collapse the per-iteration debug loop:
- New "X is missing" surfaces fast (check-staging) AND
- Has a clear single fix-site (writeonce-bootstrap for runtime;
  build/skeleton for static state).

## Phase 1 — `writeonce-bootstrap.service`

A shell script + oneshot service that runs immediately after
`sysinit.target` and before `dbus.service`. The shell script
performs every "pre-service plumbing" task we currently scatter:

```
1. Generate /etc/machine-id if missing (32 hex + \n).
2. Symlink /var/lib/dbus/machine-id → /etc/machine-id.
3. Create /run/dbus     (uid/gid 99, mode 755)
   Create /run/lock     (mode 1777)
   Create /run/log      (mode 755)
   Create /run/user     (mode 755)
4. Ensure /tmp is mode 1777.
5. Symlink /etc/mtab → /proc/mounts if missing.
6. (Future) anything else.
```

**Boot chain** becomes:

```
sysinit.target
   ↓
writeonce-bootstrap.service   ← NEW: runs ONCE, sets up the world
   ↓
dbus.service
   ↓
logind.service
   ↓
writeonce-login.service
   ↓
console.target
```

**Why shell, not Rust:** every future "add /run/foo" fix is a
one-line script edit (`mkdir -p /run/foo && chown foo:foo /run/foo`).
A Rust binary needs a rebuild + restage. Shell wins for the "minimum
friction for the next fix" goal. The script is also legible to anyone
who needs to debug on the T450 directly.

**Single source of truth.** Moving machine-id + /run/dbus creation
into bootstrap means *removing* the code we just shipped in
`customize.rs` and `mount.rs`. Defense-in-depth (write in N places)
is tempting but creates the same "which place actually did it?"
debug pain we're escaping. **One owner. Bootstrap.**

## Phase 2 — `just check-staging` pre-flight

A `build/check-staging.sh` script that runs every check that *would*
have caught one of our recent bugs:

```
[*] Required files in staged sysroot:
    /etc/{passwd,group,shadow,fstab,hosts,hostname,nsswitch.conf}
    /etc/writeonce/pid1.toml
    /etc/writeonce/services/{console,default,multi-user,sysinit}.target.toml
    /etc/writeonce/services/{dbus,logind,writeonce-login,writeonce-bootstrap}.service.toml
    /etc/pam.d/{writeonce-login,sudo}
    /usr/sbin/{writeonce-pid1,writeonce-svc,writeonce-login,writeonce-logind,writeonce-bootstrap}
    /usr/bin/{wo-ctl,dbus-daemon,bash}
    /usr/lib/{libpam.so.0,libc.so.6,libgcc_s.so.1}
    /lib/modules/<kernel-ver>/modules.alias

[*] System users present:
    root, messagebus

[*] ldd resolution of writeonce-login points into staging /usr/lib:
    libpam.so.0  → staging path  ✓

[*] No stale binaries (writeonce-pid1 newer than its source file):
    [...]

[*] Skeleton /run is empty (will be tmpfs at boot; bootstrap creates content):
    OK
```

Exit 0 on all-pass, exit 1 with red findings on any miss. Wire into
`just usb-install` as a precondition (the install recipe runs
`just check-staging` and refuses to flash if it fails). Catches new
bugs the same hour you write them, not the next morning at the T450.

## What's removed (single source of truth)

| File | What changes |
|------|--------------|
| `crates/writeonce-installer/src/customize.rs` | Remove `write_machine_id` function + 3 tests. Bootstrap owns it. |
| `crates/writeonce-pid1/src/mount.rs` | Remove `ensure_runtime_dirs()` + `ensure_run_subdir()` + test. Bootstrap owns runtime dirs. |
| `crates/writeonce-installer/src/customize.rs::apply()` | Drop the `write_machine_id` call. |
| `crates/writeonce-pid1/src/mount.rs::mount_essentials()` | Drop the `ensure_runtime_dirs()` call. |

The libpam staging fix (`build/17-stage-sysroot.sh`) **stays** — it's
infrastructure, not pre-service plumbing.

## Critical files

| File | Change |
|------|--------|
| `build/skeleton/usr/sbin/writeonce-bootstrap` | **NEW** — shell script, ~40 lines |
| `crates/writeonce-svc/examples/services/writeonce-bootstrap.service.toml` | **NEW** — oneshot unit, after sysinit, wanted-by console |
| `crates/writeonce-svc/examples/services/console.target.toml` | `requires` gains `writeonce-bootstrap.service` |
| `crates/writeonce-svc/examples/services/dbus.service.toml` | `after` gains `writeonce-bootstrap.service` (ordering) |
| `crates/writeonce-installer/src/customize.rs` | Remove `write_machine_id` block + 3 tests + call site |
| `crates/writeonce-pid1/src/mount.rs` | Remove runtime-dir helpers + call site + test |
| `build/check-staging.sh` | **NEW** — pre-flight checklist |
| `justfile` | New `check-staging` recipe; `usb-install` runs check first |

## Verification

1. `cargo test -p writeonce-installer` — confirms machine-id-removal didn't break customize.
2. `cargo test -p writeonce-pid1` — confirms mount.rs still tests cleanly.
3. `./build/skeleton/usr/sbin/writeonce-bootstrap` (point it at a tmp tree) — assert it creates the expected files/dirs idempotently.
4. `just check-staging` — should pass on the current staged sysroot.
5. `just stage && just check-staging && just artifacts && just usb-install /dev/sda && just usb-cmdline-debug /dev/sda` — boot T450. Expected: login prompt, no `Failed` lines for any service in the bare-minimum chain.
6. **Next bug surfaces:** if anything is still missing post-boot, the diff fixing it should be one of: a `mkdir` line in `writeonce-bootstrap`, an entry in the `check-staging` checklist, or a static file in `build/skeleton/`. **No new Rust edits required.** If a fix needs Rust code, that's a signal the bootstrap pattern needs extending.

## What's NOT in this round

- **Phase 3** (customizable architecture C0-C3) — a multi-week piece;
  starts AFTER this round lands and the T450 boots cleanly.
- **Removing the libpam staging fix** — it's infrastructure, stays.
- **Refactoring writeonce-pid1 to invoke bootstrap directly** — pid1
  just spawns writeonce-svc; the supervisor handles unit ordering
  per its dependency graph, exactly as today.
- **systemd-tmpfiles compatibility format** (`/etc/tmpfiles.d/*.conf`).
  Bootstrap is shell-driven for now; if the list grows beyond ~20
  entries, port to a `tmpfiles.d`-style format and a small Rust
  reader. Not yet.

## Long-term: how this composes with the customizable plan

[`../customizable/00-overview.md`](../customizable/00-overview.md)
defines axis-based profiles. `writeonce-bootstrap` is part of every
profile that uses writeonce-svc as `init` — it's at the same layer
as console.target. The `stock-desktop` profile (systemd-based) uses
`systemd-tmpfiles-setup.service` + `systemd-machine-id-setup.service`
in its place. Same role; different implementations per axis. The
bootstrap pattern actually makes the customizable architecture
*easier* to build because the pre-service plumbing is no longer
spread across writeonce-pid1, writeonce-installer, the supervisor —
it's localised to one swappable unit.

## Cross-references

- [`fix-libpam-and-dbus.md`](fix-libpam-and-dbus.md) — the previous
  round; bootstrap absorbs its machine-id + /run/dbus fixes.
- [`fix-learn-from-scratch-boot.md`](fix-learn-from-scratch-boot.md)
  — the round before that; bootstrap fits cleanly into its
  console.target chain.
- [`../../docs/learning/t450-boot-debugging.md`](../../docs/learning/t450-boot-debugging.md)
  — running log; this round becomes the next row.
