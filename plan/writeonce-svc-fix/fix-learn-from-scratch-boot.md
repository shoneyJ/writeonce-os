# Bare-minimum boot to login (learn-from-scratch profile)

> Refit of the original "fix four concurrent boot spins" plan. The
> root cause behind every failure in
> `.agents/PXL_20260527_200326737.jpg` is the same: **too many
> services running at boot**. Restructure the boot chain so only
> the absolute essentials run unattended; everything else is
> user-opt-in via `wo-ctl enable`. Two correctness fixes (rw mount,
> burst-cap defaults) still ship as supporting details. **Status:
> draft, awaiting user review/refinement.**

## Context

Boot photo shows kernel, initramfs, switch_root, writeonce-pid1,
and writeonce-svc all running on the T450. What spins is the
service chain pulled in by `multi-user.target`: dbus, dhcpcd, iwd,
logind, writeonce-modules-load. Each one fails for an independent
reason (read-only fs, missing user, wrong interface, missing dep)
and the supervisor restarts them forever.

The narrow "fix each failure" approach (the previous draft of this
plan) is correct but treats symptoms. The architectural fix is:
**don't auto-start services that aren't required to reach a login
prompt.** Only `sysinit → dbus → logind → writeonce-login` is
strictly necessary. Network (iwd + dhcpcd + modules-load) is a
*desktop-feature* dependency, not a *boot* dependency — the user
gets a login prompt without it.

After login, the user opts into services they want with
`wo-ctl enable <unit>`, which persists across reboot via drop-in
files in `/etc/writeonce/enabled.d/`. Services that fail in this
opt-in mode get caught by the (now correctly-tuned) burst-cap and
marked `Failed` — same as systemd's behaviour after `systemctl
enable && systemctl start`.

### Intended outcome

- **Boot path is unconditionally short and quiet.** sysinit → dbus
  → logind → writeonce-login. Four services, all stable. Login
  prompt visible within seconds.
- **Failures in optional services can't block login.** dhcpcd
  failing is a `wo-ctl status` row, not a boot blocker.
- **Headless / server profiles still work** — the installer
  pre-enables iwd + dhcpcd + sshd at install time when the spec
  picks a headless profile.
- **The `learn-from-scratch` profile** (per
  [`../00-roadmap.md`](../00-roadmap.md) and the customizable
  track) finally boots to a working login prompt on the T450.

## The minimal boot chain

```
                                    wanted-by
                                       │
                                   default.target
                                       │
                                  console.target
                                  │   (NEW: minimal)
                                  ├── sysinit.target
                                  ├── dbus.service
                                  ├── logind.service
                                  └── writeonce-login.service
```

After login, `wo-ctl enable <unit>` adds units to
`/etc/writeonce/enabled.d/<unit>.toml`. Next boot, writeonce-svc
reads that directory after `console.target` reaches Active and
folds those units into the activation plan (effectively making
`multi-user.target = console.target + enabled.d/*`).

### Comparison to systemd

| systemd                                           | writeonce-svc                                                                          |
| ------------------------------------------------- | -------------------------------------------------------------------------------------- |
| `default.target → graphical → multi-user → basic` | `default.target → console.target`                                                      |
| `multi-user.target.wants/*.service`               | `/etc/writeonce/enabled.d/*.toml` (drop-in pull-in for `multi-user.target` equivalent) |
| `systemctl enable foo.service`                    | `wo-ctl enable foo.service`                                                            |
| `systemctl edit foo.service`                      | drop a `.toml` in `/etc/writeonce/services/` and `wo-ctl enable` it                    |
| `systemctl --user`                                | deferred (per-user services not in this round)                                         |

## Recommended approach

### Part A — Restructure targets (the new boot chain)

**1. New target unit** —
`crates/writeonce-svc/examples/services/console.target.toml`:

```toml
# /etc/writeonce/services/console.target.toml
# The bare-minimum boot. Pulls in only what's needed for a login
# prompt: sysinit, dbus, logind, writeonce-login. Network, audio,
# wifi, etc. are opt-in via `wo-ctl enable`.

[unit]
description = "Console login reachable"
requires    = ["sysinit.target", "dbus.service", "logind.service", "writeonce-login.service"]
after       = ["sysinit.target", "dbus.service", "logind.service", "writeonce-login.service"]

[service]
type              = "oneshot"
exec-start        = "/bin/true"
remain-after-exit = true
slice             = "system.slice"
```

**2. Point `default.target` at `console.target`** —
`crates/writeonce-svc/examples/services/default.target.toml`:

```toml
# Boot target. Pulls in console.target (minimal login chain) by
# default; the installer may swap this to multi-user.target for
# profiles that want services up at boot (e.g. headless / SSH).
[unit]
description = "Default supervisor target"
requires    = ["console.target"]
after       = ["console.target"]
```

**3. Demote `multi-user.target`** to "console + everything
enabled". It loses its existing `[install] wanted-by` pulls of
dbus/iwd/dhcpcd/logind/login (those move under `console.target`
and `enabled.d/`).

**4. Remove `[install] wanted-by = ["multi-user.target"]`** from
`iwd.service.toml`, `dhcpcd.service.toml`, and
`writeonce-modules-load.service.toml`. After this, those units
exist in `/etc/writeonce/services/` but no target wants them until
the user explicitly enables them.

`dbus.service` / `logind.service` / `writeonce-login.service`
KEEP their pull (now via `console.target`).

### Part B — Enable-state machinery

**5. Drop-in directory** — `/etc/writeonce/enabled.d/`. Each enabled
unit gets a file:

```
/etc/writeonce/enabled.d/iwd.service.toml
/etc/writeonce/enabled.d/dhcpcd.service.toml
```

Each file is **a literal symlink (or a one-line stub)** referring
to a unit in `/etc/writeonce/services/`. Stub format (chosen for
simplicity, no symlinks-on-vfat surprises):

```toml
# enabled.d/<unit>.toml
unit = "iwd.service"
```

The stub-vs-symlink decision is small; recommend stub-files
because they survive a vfat ESP copy.

**6. writeonce-svc loads enabled.d at startup.** New code in
`crates/writeonce-svc/src/config.rs` (or a new module): after
`load_directory(args.units_dir)` succeeds, also scan
`/etc/writeonce/enabled.d/`, read each stub's `unit = ...` value,
and *as if the user had added `wanted-by = ["multi-user.target"]`*
inject that fact into the dependency graph. The activation plan
for `multi-user.target` then includes the enabled units.

For this round, since `default.target = console.target`,
the enabled.d pull only fires for **explicit** activations:
`wo-ctl start multi-user.target` (or `--default-target
multi-user.target` at the CLI).

### Part C — `wo-ctl` subcommands

**7. Extend the control protocol** — `wo-ctl` already talks to the
supervisor via the Unix socket at
`crates/writeonce-svc/src/control.rs:DEFAULT_SOCKET`. Add (or
verify present) four verbs:

```
wo-ctl start    <unit>     # start now, do not persist
wo-ctl stop     <unit>     # stop now, do not unpersist
wo-ctl enable   <unit>     # write enabled.d/<unit>.toml AND start now
wo-ctl disable  <unit>     # remove enabled.d/<unit>.toml AND stop now
wo-ctl status   [<unit>]   # show unit(s) state + recent log
wo-ctl list     [enabled|loaded|running]   # show available units
```

`wo-ctl enable` is sequenced: write the stub file first
(persistence guaranteed even if the start fails), then ask the
supervisor to start the unit. `wo-ctl disable` is the reverse:
stop, then remove the stub. This matches systemd's behaviour and
its failure semantics.

### Part D — Installer pre-enables network for headless profile

**8. `crates/writeonce-installer/src/customize.rs`** gains a step
that consults the resolved spec. When `profile = "headless"` (or
any spec where `network.enabled_at_boot = true`), the installer
writes the corresponding enabled.d stubs into the staging sysroot
before the artifact is sealed:

```rust
// pseudo: in customize.rs, after user/keyboard/locale rewrites
if spec.network.enabled_at_boot {
    write_enabled_stub(&staging, "iwd.service")?;
    write_enabled_stub(&staging, "dhcpcd.service")?;
    write_enabled_stub(&staging, "writeonce-modules-load.service")?;
}
```

Default for desktop profiles (`learn-from-scratch`,
`minimal-tiling`): `network.enabled_at_boot = false`. The desktop
user runs `wo-ctl enable iwd dhcpcd` once on first login;
subsequent boots have wifi automatically.

### Part E — Correctness fix: rw mount (carried over)

**9. Initramfs reads cmdline `rw`/`ro` tokens.** Independent of
the target restructure: without this, ANY service that writes to
disk fails (including `enabled.d/` stubs the installer creates,
and `dbus`'s machine-id rewrite, and `logind`'s session bookkeeping).

Three edits in `crates/writeonce-initramfs/`:

- `src/cmdline.rs`: add `mount_flags: u64` to `CmdLine`, default
  `libc::MS_NOATIME`, parse `rw` / `ro` tokens.
- `src/main.rs:68`: pass `cmd.mount_flags` instead of hardcoded
  `libc::MS_RDONLY`.
- `src/switch_root.rs`: unchanged (already propagates the flag).

Unit tests for the cmdline parsing.

### Part F — Correctness fix: burst-cap defaults (carried over)

**10. Tighten supervisor defaults** so the safety net actually
catches a user-enabled service that loops:

- `default_start_limit_burst`: `5` → `3`
- `default_start_limit_interval_sec`: `"10s"` → `"30s"`

Math: with `restart_sec=5s`, restart_history reaches 3 entries
within 15s (well inside the 30s window) — cap fires, unit marked
`Failed`, no more restarts. Edit:
`crates/writeonce-svc/src/config.rs:151-152`.

## Critical files

| File                                                                       | Change                                                                                                                                                       |
| -------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `crates/writeonce-svc/examples/services/console.target.toml`               | **NEW** — minimal boot target.                                                                                                                               |
| `crates/writeonce-svc/examples/services/default.target.toml`               | Now `requires = ["console.target"]`.                                                                                                                         |
| `crates/writeonce-svc/examples/services/multi-user.target.toml`            | Drops its current wanted-by pull-ins (which move to console.target via `requires`).                                                                          |
| `crates/writeonce-svc/examples/services/iwd.service.toml`                  | Remove `[install] wanted-by = ["multi-user.target"]`.                                                                                                        |
| `crates/writeonce-svc/examples/services/dhcpcd.service.toml`               | Remove `[install] wanted-by`. (Interface arg also dropped — same one-line edit as the old Fix 3.)                                                            |
| `crates/writeonce-svc/examples/services/writeonce-modules-load.service.toml` | Remove `[install] wanted-by` from sysinit.target (the file currently has `wanted-by = ["sysinit.target"]`; move to enabled.d when user opts in).             |
| `crates/writeonce-svc/src/config.rs`                                       | Add enabled.d loader; tighten burst-cap defaults (3 / 30s).                                                                                                  |
| `crates/writeonce-svc/src/state.rs` or `graph.rs`                          | Wire enabled.d entries into the dependency graph as virtual `wanted-by = ["multi-user.target"]`.                                                             |
| `crates/wo-ctl/src/main.rs` (or wherever wo-ctl's CLI lives)               | Add `enable` / `disable` / `list` subcommands; confirm `start` / `stop` / `status` are already there.                                                        |
| `crates/writeonce-svc/src/control.rs`                                      | Extend the control protocol with `Enable` / `Disable` / `List` messages.                                                                                     |
| `crates/writeonce-initramfs/src/cmdline.rs`                                | Add `mount_flags: u64` field; parse `rw`/`ro`; default `MS_NOATIME`.                                                                                         |
| `crates/writeonce-initramfs/src/main.rs`                                   | Pass `cmd.mount_flags` instead of hardcoded `MS_RDONLY`.                                                                                                     |
| `crates/writeonce-installer/src/customize.rs`                              | Write `enabled.d/<unit>.toml` stubs when the spec calls for boot-time network.                                                                               |
| `crates/writeonce-installer/src/spec.rs`                                   | Add `network.enabled_at_boot: bool` field (or similar — the customizable spec extension this needs).                                                         |
| `build/skeleton/etc/writeonce/enabled.d/.gitkeep`                          | **NEW** — empty dir so it exists on the live USB.                                                                                                            |

Items previously planned as separate fixes (the old Fix 2 missing
`dhcpcd` user, Fix 3b wifi.conf hardcoding) are **subsumed**: those
services no longer auto-start, so their hardcoded-config issues
don't appear during boot. When the user opts in, those issues
surface in their own time and are tractable one at a time.

## Existing utilities reused

- `crates/writeonce-svc/src/control.rs::DEFAULT_SOCKET` and the
  existing client/server skeleton — extend with new message types.
- `crates/writeonce-svc/src/graph.rs::build_transaction` — already
  computes the activation plan from a wanted-by closure; the
  enabled.d entries just inject more edges into the graph.
- `crates/writeonce-initramfs/src/cmdline.rs::parse` — already
  iterates tokens; just adding two branches.
- `build/17-stage-sysroot.sh:4` overlay logic — picks up new
  skeleton paths (`/etc/writeonce/enabled.d/`) automatically.

## Verification

1. **Unit tests pass.**
   ```
   cargo test -p writeonce-initramfs --lib cmdline
   cargo test -p writeonce-svc --lib
   ```

2. **Host smoke test of bare boot.** Stand up a tmpdir with the
   four bare-minimum units + a stub default.target. Run
   `writeonce-svc --units <tmpdir> --default-target default.target
   --fake`. Expect: 4 jobs in the plan, all reach Active, supervisor
   sits idle. No iwd / dhcpcd / modules-load referenced anywhere.

3. **Host smoke test of enable.** With the supervisor running,
   `wo-ctl enable iwd.service`. Expect: stub file appears under
   `enabled.d/`; supervisor activates iwd (fails immediately on the
   host, since `/usr/libexec/iwd` doesn't exist on the workstation
   — that's fine; we're verifying the wire-up).

4. **Rebuild initramfs only (no kernel rebuild).** Same as before:
   ```
   ./build/in-container.sh ./build/05-initramfs.sh
   just stage
   just artifacts
   just usb-install /dev/sda
   ```

5. **Boot T450**. Expected screen:
   - `sysinit.target active`
   - `dbus.service active`
   - `logind.service active`
   - `writeonce-login` banner
   - `login:` prompt on tty1
   - **No** iwd / dhcpcd / modules-load lines. No spin.

6. **Post-login**: user runs `wo-ctl enable iwd.service && wo-ctl
   enable dhcpcd.service`. iwd attempts to bring up wifi; dhcpcd
   scans interfaces. Either:
   - Works → user has network.
   - Fails → `wo-ctl status iwd` shows the error; burst cap stops
     the spin within 30 s.

7. **Reboot**: enabled services persist via `enabled.d/`; iwd +
   dhcpcd come up at boot (after `console.target` reaches Active).

## What's NOT in this round

- **Per-user services** (`~/.config/writeonce/services/`). Deferred
  to a future round once the system-wide enable model is proven.
- **Service templating** (systemd-style `%i` / `%n` parameters).
  Out of scope.
- **First-boot wizard.** If a user wants network on first login,
  they get a one-liner `wo-ctl enable iwd dhcpcd`. A TUI wizard
  to walk through opt-ins is nice-to-have, not blocking.
- **`graphical.target`** — currently a no-op stub. Will eventually
  bundle the display-shell axis's services (display server,
  compositor, etc.). Untouched in this round.
- **Reverse engineering systemd-network's `[Network]` semantics**
  (DHCP / static / route metric). User-enabled dhcpcd takes the
  simplest possible behaviour — DHCP on all UP interfaces. Bigger
  network configurability is future work.

## Cross-references

- [`.agents/PXL_20260527_200326737.jpg`](../../.agents/PXL_20260527_200326737.jpg)
  — boot screenshot that motivated this rework.
- [`../00-roadmap.md`](../00-roadmap.md) — `learn-from-scratch`
  profile in the customizable track; this fix unblocks it.
- [`../../docs/learning/t450-boot-debugging.md`](../../docs/learning/t450-boot-debugging.md)
  — running log of bring-up issues. The bare-minimum-boot
  restructure becomes its own row.
- systemd's
  [`bootup(7)`](https://www.freedesktop.org/software/systemd/man/bootup.html)
  — reference for the target-chain mental model.
