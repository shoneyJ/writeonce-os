# Round 2d — the writeonce-logind D-Bus shim

> Companion to [`../../crates/writeonce-logind/`](../../crates/writeonce-logind/).
> Explains why i3More needs a logind, what surface the shim implements,
> what's deliberately stubbed, and how `writeonce-login` will integrate
> with it in Phase 9.

## Why this exists

Mainstream Linux desktops assume `systemd-logind` is running on the
system bus at `org.freedesktop.login1`. Apps query it to:

- **Discover which session they're in.** XDG_SESSION_ID alone isn't
  enough; many apps call `Manager.GetSessionByPID(getpid())` to walk
  their PID up the process tree and find the owning session.
- **Listen for lock / unlock signals.** i3more-lock subscribes to
  `Session.Lock` and `Session.Unlock` on its own session's object
  path. Screensavers and other apps subscribe to `PrepareForSleep` so
  they can flush state before suspend.
- **Inhibit shutdown / sleep / idle.** "I'm in the middle of something
  important, don't power off." A daemon returns a file descriptor whose
  close ends the inhibition; refcounted lifecycle without a daemon poll.
- **Request reboot / power-off.** GUI shutdown buttons call
  `Manager.Reboot()`; we delegate to PID 1.

We don't ship systemd. **writeonce-logind** speaks the minimum subset
of the logind D-Bus protocol that i3More and the standard desktop
applets need — nothing more.

## The interface surface

### `org.freedesktop.login1.Manager` (one instance, at `/org/freedesktop/login1`)

**Methods**:

| Method | Implementation | Notes |
| --- | --- | --- |
| `CreateSession(...)` | ✓ real | Called by writeonce-login post-PAM. Registers a Session object at `/org/freedesktop/login1/session/<id>`. Returns id + path + runtime dir + lifecycle fd + uid + seat + vtnr + existing-flag. |
| `ReleaseSession(id)` | ✓ real | Removes the session object + state entry. Emits `SessionRemoved`. |
| `GetSession(id)` | ✓ real | Returns the object path. |
| `GetSessionByPID(pid)` | ✓ real | Walks `/proc/<pid>/status:PPid` up to 32 hops looking for a known leader_pid. |
| `GetCurrentSession()` | partial | Returns the single session if exactly one exists. Multi-session case needs DBus sender→PID lookup which we punt on for v1. |
| `ListSessions()` | ✓ real | Returns `[(id, uid, name, seat, path), …]`. |
| `ListSeats()` | ✓ real | Always `[("seat0", /org/freedesktop/login1/seat/seat0)]`. |
| `ListUsers()` | ✓ real | Unique-by-uid of all session owners. |
| `ListInhibitors()` | ✓ real | Returns current inhibitor records. |
| `Inhibit(what, who, why, mode)` | partial | Returns a pipe read-end fd. Caller holding fd open = inhibitor active. We **do not yet** track inhibitor lifecycle (writer-end close detection) — honour-system only. |
| `LockSession(id)` | ✓ real | Emits `Session.Lock` signal on the session's object path. |
| `UnlockSession(id)` | ✓ real | Emits `Session.Unlock`. |
| `CanReboot()` / `CanPowerOff()` | ✓ stub | Returns "yes". |
| `CanSuspend()` / `CanHibernate()` | ✓ stub | Returns "no" — kernel s2idle / deep-sleep hooks aren't wired through writeonce-svc yet. |
| `Reboot(interactive)` | ✓ real | `kill(1, SIGTERM)` — PID 1 maps to `LINUX_REBOOT_CMD_RESTART`. |
| `PowerOff(interactive)` | ✓ real | `kill(1, SIGUSR1)` — PID 1 maps to `LINUX_REBOOT_CMD_POWER_OFF`. |

**Properties**:
`NCurrentSessions`, `PreparingForShutdown`, `PreparingForSleep`,
`BlockInhibited`, `DelayInhibited`, `IdleHint` — all readable, real.

**Signals**:
`SessionNew`, `SessionRemoved`, `PrepareForShutdown`, `PrepareForSleep`
— emitted on real state transitions.

### `org.freedesktop.login1.Session` (one instance per session)

**Methods**: `Lock`, `Unlock`, `Activate` (stub — no VT switch yet),
`Terminate` (sends SIGTERM to the leader pid), `SetIdleHint`.

**Properties**: `Id`, `User`, `Name`, `Timestamp`, `TimestampMonotonic`,
`VTNr`, `Seat`, `Display`, `Remote`, `Service`, `Type`, `Class`,
`State`, `Active`, `IdleHint`, `IdleSinceHint`, `Leader`.

**Signals**: `Lock`, `Unlock`.

### `org.freedesktop.login1.Seat` (one instance, "seat0")

**Properties**: `Id`, `ActiveSession`, `Sessions`, `CanGraphical`,
`CanTTY`, `CanMultiSession`, `IdleHint`.

No methods. Most callers just read `ActiveSession` after a VT switch.

## What's stubbed vs. what's real

| Capability | Real impl | Stub | Future round |
| --- | --- | --- | --- |
| Session create / destroy + signals | ✓ | | |
| Lock / Unlock signals | ✓ | | |
| Inhibitor FD allocation | ✓ | | |
| Inhibitor lifecycle (close-detect) | | ✗ honour-system | Round 2e — epoll on the write-end |
| Reboot / PowerOff via PID 1 | ✓ | | |
| Suspend / Hibernate | | ✗ returns "no" | Round 2f — wire kernel s2idle |
| GetCurrentSession multi-session | | ✗ punts | Round 2e — DBus sender→PID lookup |
| VT-switch on Session.Activate | | ✗ stub | Round 2e — VT_ACTIVATE ioctl |
| Linger sessions (post-logout) | | ✗ not supported | Maybe never — niche |
| User objects under `/login1/user/_<uid>` | | ✗ no methods | Round 2e if any client probes |

## How writeonce-login will integrate (Phase 9)

The boot path becomes:

```
writeonce-pid1 → writeonce-svc → dbus.service     ✓ existing
                                ↓
                       writeonce-logind.service   ← this round registers
                                ↓
                       (logind owns the bus name)
                                ↓
              writeonce-login spawns on tty1      ← Phase 9 integration
                       ↓
                       PAM auth ✓
                       ↓
                       D-Bus call to Manager.CreateSession(...)
                       ↓
                       Get back: session_id, runtime_dir, lifecycle_fd
                       ↓
                       export XDG_SESSION_ID=$session_id
                       export XDG_RUNTIME_DIR=$runtime_dir
                       hold lifecycle_fd open
                       ↓
                       execve user's shell / .xinitrc
                       ↓
                       i3 starts, i3more applets connect to logind via
                       Manager.GetCurrentSession(), subscribe to Lock/Unlock
```

The lifecycle FD is the crucial bit: the user shell inherits it,
and when the user logs out (shell exits, kernel reaps everything,
all FDs close), the write-end of our pipe loses its last writer.
Round 2e will add an epoll watch on the read-end so the daemon
notices and auto-calls ReleaseSession.

## What zbus 5 forced us to do

- **Async-only signal emission.** `#[zbus(signal)]` fns must be
  `async fn`, even with the blocking-api feature. Any method that emits
  a signal becomes `async fn` too. We use `Arc<Mutex<AppState>>` for
  state but must scope-drop the `MutexGuard` before any `.await` — the
  compiler tracks `Send`-ness across awaits and `std::sync::MutexGuard`
  is not `Send`.
- **Pure-Rust D-Bus.** zbus implements DBus over Unix sockets directly
  in Rust; no libdbus.so runtime dependency. Just dbus-daemon (the
  reference broker) needs to be running, which Phase 8a already
  installs.
- **An async executor under the blocking façade.** zbus 5 default
  features pull in `async-io` + `async-executor` even when you only
  call the blocking API. The blocking wrapper runs an executor on a
  background thread and blocks the caller while futures resolve.
  Result: writeonce-logind has ~50 transitive dependencies. Acceptable
  for a userspace D-Bus daemon; the binary is 5.2 MB stripped.

## D-Bus policy

`crates/writeonce-logind/examples/dbus-policy.conf` installs to
`/etc/dbus-1/system.d/org.freedesktop.login1.conf`. Grants:

- **root**: `own` the well-known name, `send` + `receive` everything.
- **everyone**: `send` to the listed lookup / inhibitor / lock methods
  + property reads; `receive` signals; restricted access to Reboot /
  PowerOff (we currently let anyone call them and refuse in-daemon if
  uid != 0 — placeholder until a PolicyKit-equivalent ships).

Mirrors `systemd/src/login/org.freedesktop.login1.conf` so any
third-party `/etc/dbus-1/system.d/*.conf` drop-ins (rare but real)
continue to work.

## Service unit (writeonce-svc / Phase 9)

```toml
# /etc/writeonce/services/logind.service.toml
[unit]
description = "WriteOnce logind D-Bus shim"
after       = ["dbus.service"]
requires    = ["dbus.service"]

[service]
type        = "simple"
exec-start  = "/usr/sbin/writeonce-logind"
restart     = "on-failure"
restart-sec = "5s"
user        = "root"
group       = "root"
slice       = "system.slice"

[install]
wanted-by   = ["multi-user.target"]
```

Runs as root because (a) it claims a system-bus name only root can own
by D-Bus policy, and (b) inhibitor enforcement + VT switching need
CAP_SYS_ADMIN.

## Binary footprint

```
target/release/writeonce-logind   5.2 MB unstripped, dynamic-glibc
```

Larger than the static-musl boot path binaries because zbus pulls in
async-io + tracing + tokio-ish ecosystem. Stripped it should be ~3.5 MB.

Comparison: systemd-logind is ~600 KB on disk but transitively depends
on libsystemd (~1.5 MB), libcrypt (~150 KB), libpcre2 (~600 KB),
libcap (~80 KB), libacl (~50 KB) → about 3 MB of dependencies. We're
in the same order of magnitude.

## Testing strategy

Currently three in-binary unit tests cover state-allocation
invariants (session-id format, inhibitor-id monotonicity, list
shaping). The real D-Bus surface needs an integration test that:

1. Starts a dedicated dbus-daemon on an abstract socket
2. Forks writeonce-logind with `DBUS_SYSTEM_BUS_ADDRESS=<that socket>`
3. Connects from the test process and exercises CreateSession +
   ListSessions + Lock/Unlock signals + ReleaseSession

That's a Round 2e or Phase 9 task — fitting CI infrastructure isn't
the point of this round.

For now, manual smoke test after Phase 9 boots:

```bash
# On the running system:
gdbus introspect --system --dest org.freedesktop.login1 \
    --object-path /org/freedesktop/login1

# Should print the Manager interface XML.
# Then:
busctl --system call org.freedesktop.login1 \
    /org/freedesktop/login1 \
    org.freedesktop.login1.Manager ListSessions
```

If both commands return real data instead of "name not found", the
shim is alive on the bus.

## Cross-references

- `docs/learning/systemd-feature-survey.md` § Logind minimum surface —
  the original feature audit this implementation realises.
- `plan/phase-4-supervisor.md` — context for why logind is a separate
  service unit, not part of writeonce-svc.
- `crates/writeonce-svc/examples/services/logind.service.toml` — the
  service unit that writeonce-svc consumes to spawn this daemon.
- Phase 9 (TBD) — writeonce-login + i3more-lock integration.
