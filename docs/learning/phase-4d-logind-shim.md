# Rounds 2d–2e — the writeonce-logind D-Bus shim

> Companion to [`../../crates/writeonce-logind/`](../../crates/writeonce-logind/).
> Explains why i3More needs a logind, what surface the shim implements,
> what's deliberately stubbed, and how `writeonce-login` will integrate
> with it in Phase 9.
>
> Round 2d landed the initial skeleton (interfaces + service registration);
> Round 2e closed the three biggest stubs (inhibitor lifecycle tracking,
> VT switching on Session.Activate, sender-PID resolution for
> GetCurrentSession). See "Round 2e changes" below.

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
| `GetCurrentSession()` | ✓ real (Round 2e) | Asks the bus daemon for the sender's PID via `org.freedesktop.DBus.GetConnectionUnixProcessID`, then walks PPid chain in /proc/<pid>/status. |
| `ListSessions()` | ✓ real | Returns `[(id, uid, name, seat, path), …]`. |
| `ListSeats()` | ✓ real | Always `[("seat0", /org/freedesktop/login1/seat/seat0)]`. |
| `ListUsers()` | ✓ real | Unique-by-uid of all session owners. |
| `ListInhibitors()` | ✓ real | Returns current inhibitor records. |
| `Inhibit(what, who, why, mode)` | ✓ real (Round 2e) | Returns a pipe write-end fd. Daemon keeps the read-end and watches it via epoll on a dedicated thread; EPOLLHUP fires when caller closes their fd → inhibitor auto-removed. Caller uid+pid resolved via D-Bus sender lookup. |
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

**Methods**: `Lock`, `Unlock`, `Activate` (✓ Round 2e — VT_ACTIVATE
ioctl on /dev/tty0), `Terminate` (sends SIGTERM to the leader pid),
`SetIdleHint`.

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
| Session create / destroy + signals | ✓ Round 2d | | |
| Lock / Unlock signals | ✓ Round 2d | | |
| Inhibitor FD allocation | ✓ Round 2d | | |
| **Inhibitor lifecycle (close-detect)** | **✓ Round 2e** | | Watcher thread + epoll on pipe read-ends |
| Reboot / PowerOff via PID 1 | ✓ Round 2d | | |
| Suspend / Hibernate | | ✗ returns "no" | Round 2f — wire kernel s2idle |
| **GetCurrentSession multi-session** | **✓ Round 2e** | | DBus sender→PID via GetConnectionUnixProcessID |
| **VT-switch on Session.Activate** | **✓ Round 2e** | | VT_ACTIVATE ioctl on /dev/tty0 |
| Linger sessions (post-logout) | | ✗ not supported | Maybe never — niche |
| User objects under `/login1/user/_<uid>` | | ✗ no methods | Round 2f if any client probes |
| Session lifecycle FIFO HUP-detect | | ✗ daemon discards write-end | Round 2f — same shape as inhibitor watcher |

## Round 2e changes in detail

### Inhibitor lifecycle tracking (`crates/writeonce-logind/src/inhibitor.rs`)

The Round 2d implementation of `Manager.Inhibit` was an honour-system
stub: we created a pipe, returned the read-end, discarded the
write-end, and never reaped the inhibitor record. Round 2e flips the
pipe direction and adds a dedicated watcher thread.

**The new shape:**

```
┌──────────────────────────────┐         ┌──────────────────────────┐
│ writeonce-logind             │         │ Caller (e.g. updater)    │
│                              │         │                          │
│   Manager.Inhibit() ─────────┼─────────┤  receives WRITE-end fd   │
│        │                     │         │  via D-Bus FD passing    │
│        ▼                     │         │                          │
│   pipe2(O_CLOEXEC) ───┐       │         │  holds it open while     │
│                       │       │         │  inhibition needed       │
│   keep READ-end ◄──── ┘       │         │                          │
│   register(id, fd)            │         │  process exits → kernel  │
│        │                      │         │  closes write-end        │
│        ▼                      │         │                          │
│ ┌──────────────────┐          │         └──────────────────────────┘
│ │ InhibitorWatcher │                                  │
│ │ (thread)         │                                  │
│ │                  │                                  │
│ │ epoll_wait(...)  │◄─────── EPOLLHUP on read-end ────┘
│ │   on EPOLLHUP:   │
│ │     state.lock() │
│ │     remove(id)   │
│ │     close fd     │
│ └──────────────────┘                                              │
└──────────────────────────────────────────────────────────────────┘
```

**Key invariants:**

- The daemon owns the **read** end (the one that gets EPOLLHUP when the
  peer closes).
- The caller owns the **write** end. When their process exits or
  crashes, the kernel closes all their FDs; the kernel emits HUP on
  our read-end.
- The watcher thread reaps the inhibitor record from `AppState` in
  response, holding the mutex only long enough to remove the entry.
- A "wake pipe" (`InhibitorWatcher.wake_{read,write}`) is registered
  with the same epoll so `register()` can interrupt a blocked
  `epoll_wait` when new inhibitors are added between iterations.

**Why a separate thread.** zbus runs an async executor on its own
threads. The watcher could in principle be wired into that executor,
but `epoll_wait` is fundamentally blocking and we don't want to occupy
an executor task with it. A dedicated thread with one syscall in
flight is the cleanest model — zero coordination with zbus's runtime.

### VT switching (`crates/writeonce-logind/src/session.rs`)

`Session.Activate` now calls `VT_ACTIVATE` directly:

```rust
const VT_ACTIVATE: libc::c_ulong = 0x5606;

fn activate_vt(vtnr: u32) -> std::io::Result<()> {
    let tty0 = std::fs::OpenOptions::new()
        .read(true).write(true)
        .open("/dev/tty0")?;
    let rc = unsafe { libc::ioctl(tty0.as_raw_fd(), VT_ACTIVATE, vtnr as libc::c_ulong) };
    if rc != 0 { return Err(std::io::Error::last_os_error()); }
    Ok(())
}
```

The ioctl number `0x5606` is `_IO('V', 6)` from `<linux/vt.h>`. /dev/tty0
is the controlling tty for the active VT subsystem; the kernel routes
the ioctl through to the VT layer. Caller-side: we need
`CAP_SYS_TTY_CONFIG`, which root has (the daemon runs as root).

Sessions with `vtnr == 0` (e.g. remote ssh sessions) refuse the call
with a clear error — there's no VT to activate.

### Sender PID lookup (`Manager.GetCurrentSession`)

Round 2d's stub returned the single session if exactly one existed and
errored on multi-session. Round 2e does it properly:

```rust
async fn resolve_sender(
    conn: &zbus::Connection,
    header: &zbus::message::Header<'_>,
) -> zbus::Result<(u32, u32)> {
    let sender = header.sender().ok_or(...)?.clone();
    let bus = zbus::fdo::DBusProxy::new(conn).await?;
    let pid = bus.get_connection_unix_process_id(sender.clone().into()).await?;
    let uid = bus.get_connection_unix_user(sender.into()).await?;
    Ok((uid, pid))
}
```

The bus daemon (`dbus-daemon`) keeps a UID + PID per connection (it
gets these via `SO_PEERCRED` when the client connects to its Unix
socket). The `GetConnectionUnixProcessID` / `GetConnectionUnixUser`
methods on `org.freedesktop.DBus` expose them.

Once we have the sender PID, `AppState::find_session_by_pid` walks
`/proc/<pid>/status:PPid` up to 32 hops looking for a `leader_pid`
match. This is the same algorithm systemd-logind uses (search for
`manager_get_session_by_pid` in systemd's source).

The same `resolve_sender` helper now populates the `uid` + `pid`
fields on Inhibitor records, so `ListInhibitors` returns accurate
attribution instead of zeros.

## How writeonce-login integrates (Round 2g, done)

The boot path:

```
writeonce-pid1 → writeonce-svc → dbus.service     (Phase 9)
                                ↓
                       writeonce-logind.service   (claims org.freedesktop.login1)
                                ↓
              writeonce-login.service on tty1     (PAM prompt loop)
                                ↓
                       PAM authenticate + acct_mgmt + open_session ✓
                                ↓
                       fork()                                       (parent waits)
                                ↓                                   ↓
                       child (still root):                  parent loops on next user
                       ↓
                       execve /usr/sbin/writeonce-session-create
                              --user <name> --uid <u> --gid <g>
                              --home <h> --shell <s>
                              --tty /dev/tty1 --vtnr 1
                              --session-script /usr/bin/startx
                                ↓
                       writeonce-session-create (still root):
                       - Connection::system() opens system bus
                       - conn.call_method("CreateSession", uid, pid=getpid(), …)
                       - returns (session_id, path, runtime_path, fifo_fd, …)
                       - fcntl F_SETFD: clear FD_CLOEXEC on fifo_fd
                       - mkdir + chown + chmod 0700 /run/user/<uid>
                       - initgroups + setresgid + setresuid (uid, gid)
                       - chdir $HOME
                       - build env: USER, HOME, SHELL, PATH,
                                    XDG_SESSION_ID=<id>, XDG_RUNTIME_DIR=…,
                                    XDG_SESSION_CLASS=user, XDG_SESSION_TYPE=tty
                       - execve /usr/bin/startx  ← fifo_fd inherited (no CLOEXEC)
                                ↓
                       startx → Xorg + ~/.xinitrc → i3 → user
                                ↓
                       i3More applets call GetCurrentSession() → matches our PID
                       i3more-lock subscribes to Session.Lock signals → works.
```

**Why writeonce-session-create is a separate binary.** zbus pulls in
~50 transitive deps and ~3 MB of binary weight. writeonce-login is a
small (~900 KB) libc + PAM tool whose dep profile we want to keep
minimal. By moving the D-Bus client into a dedicated helper that's
exec'd as the final step before the user shell, writeonce-login stays
lean while still completing the session-registration handshake.

**Why session-create runs as root.** `CreateSession` is restricted to
root by D-Bus policy (only root can claim arbitrary uids for new
sessions). session-create starts as root (writeonce-login's child
hasn't dropped privileges yet), calls CreateSession, then drops to the
user just before execve.

**Why the FD survives.** Both `into_raw_fd()` (releases Rust's
ownership of the FD) and `fcntl(F_SETFD, !FD_CLOEXEC)` (clears the
close-on-exec flag) are required. The kernel preserves open FDs
across execve only if both conditions hold. The FD then propagates
through startx → Xorg → i3 because none of them mark inherited FDs as
CLOEXEC.

When the user logs out: i3 exits → startx tears Xorg down → all
inherited FDs close (including our lifecycle FD) → writeonce-logind's
read-end gets EPOLLHUP (Round 2e watcher thread) → session
auto-released, the per-session D-Bus object is removed.

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
target/release/writeonce-logind   5.5 MB unstripped, dynamic-glibc
                                  (Round 2d shipped 5.2 MB; the
                                   inhibitor watcher + DBusProxy +
                                   ioctl wiring added ~300 KB)
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
- `plan/done/phase-4-supervisor.md` — context for why logind is a separate
  service unit, not part of writeonce-svc.
- `crates/writeonce-svc/examples/services/logind.service.toml` — the
  service unit that writeonce-svc consumes to spawn this daemon.
- `crates/writeonce-logind/src/inhibitor.rs` — the Round 2e watcher
  thread + epoll machinery.
- Phase 9 (TBD) — writeonce-login + i3more-lock integration.
