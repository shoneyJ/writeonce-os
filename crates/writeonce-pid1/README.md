# writeonce-pid1

The WriteOnce OS **PID 1** — the first userspace process the kernel starts. It is
deliberately tiny and auditable: it mounts the essential pseudo-filesystems,
takes over signal handling, execs **one** child (the service supervisor), reaps
every zombie on the system, and drives orderly shutdown. All real service
management lives in its child, `writeonce-svc` (PID 2).

```
kernel → initramfs (switch_root) → writeonce-pid1 (PID 1) → writeonce-svc (PID 2) → services
```

Keeping PID 1 minimal is the point: PID 1 is uniquely privileged (the kernel
panics if it exits, and it is immune to `SIGKILL`), so the less it does, the
smaller the blast radius. Complexity is pushed down into the supervisor, which
can be replaced without touching this binary.

## The PID 1 contract (what it does)

1. **Refuse to run unless it is PID 1** (override with `WO_PID1_FAKE=1` for dev).
2. **Mount** the essential pseudo-filesystems (idempotent — `EBUSY` is swallowed).
3. **Block all handled signals** and re-deliver them via a `signalfd` + `epoll` loop.
4. **`fork` + `execve`** the configured child on a controlling tty.
5. **Reap zombies** with `waitpid(-1, WNOHANG)` on every `SIGCHLD` (orphans on the
   whole system reparent to PID 1, so this loop owns them all).
6. **Shut down** on `SIGTERM`/`SIGINT`: `SIGTERM` the child → wait the grace
   window → `SIGKILL` → `sync()` → `reboot(2)`.

## Build

Built as a **static musl** binary (no `libc.so` runtime dependency — appropriate
for PID 1):

```bash
cargo build-pid1     # alias for: build -p writeonce-pid1 --release --target x86_64-unknown-linux-musl
cargo test-pid1      # alias for: test -p writeonce-pid1   (runs natively)
```

Dependencies are only `libc`, `serde`, `toml`.

## Running it

In production the initramfs `switch_root`s into the real root and `execve`s this
binary as PID 1 (selected by `init=` on the kernel cmdline; default
`/sbin/writeonce-pid1`). It then execs the child named in
`/etc/writeonce/pid1.toml` — normally `writeonce-svc`.

For development on a normal workstation it cannot be PID 1, so:

```bash
WO_PID1_FAKE=1 ./target/.../writeonce-pid1
```

`WO_PID1_FAKE=1` skips the is-PID-1 check **and** the real `mount(2)` calls, so it
runs harmlessly as an ordinary process (it still forks the configured child, e.g.
`/bin/sh`). If it ever does hit a fatal error *while actually PID 1*, it pauses
forever instead of exiting — exiting PID 1 panics the kernel, and pausing lets
netconsole capture the diagnostic.

## Configuration — `/etc/writeonce/pid1.toml`

Tolerant of a missing file and missing keys; anything unset takes the default.

| Key | Default | Meaning |
| --- | --- | --- |
| `tty` | `/dev/tty1` | tty the child is attached to (becomes its controlling terminal). |
| `child` | `/bin/sh` | Binary PID 1 execs as its single child. |
| `child_args` | `["sh"]` | argv for `child` (first element is conventionally the basename). |
| `shutdown_grace_seconds` | `10` | Window between `SIGTERM` and `SIGKILL` of the child on shutdown. |

The default (`/bin/sh` on tty1) is the bare debugging fallback. The production
config points `child` at the supervisor, e.g.:

```toml
tty   = "/dev/tty1"
child = "/usr/sbin/writeonce-svc"
child_args = [
    "writeonce-svc",
    "--units", "/etc/writeonce/services",
    "--default-target", "default.target",
]
shutdown_grace_seconds = 15
```

## Mounts

`mount_essentials()` mounts, in order (creating the mountpoint first; a second
mount of an already-mounted target returns `EBUSY` and is ignored):

| Target | Type | Flags / data |
| --- | --- | --- |
| `/proc` | `proc` | `nosuid,noexec,nodev` |
| `/sys` | `sysfs` | `nosuid,noexec,nodev` |
| `/dev` | `devtmpfs` | `nosuid`, `mode=755` |
| `/dev/pts` | `devpts` | `nosuid,noexec`, `gid=5,mode=620` |
| `/run` | `tmpfs` | `nosuid,nodev`, `mode=755` |
| `/sys/fs/cgroup` | `cgroup2` | `nosuid,noexec,nodev` |

PID 1 intentionally creates **no** `/run` subdirectories (e.g. `/run/dbus`) — that
moved to `writeonce-bootstrap.service`, the boot-time oneshot. PID 1 mounts the
filesystems and nothing else.

## Child exec

The child is set up to own a fresh session and the configured tty
([`exec.rs`](src/exec.rs)):

- `setsid()` — become session/process-group leader.
- `open(tty)` and `dup2` it onto stdin/stdout/stderr; claim it via `TIOCSCTTY`.
  (If the tty can't be opened — e.g. dev mode — the child inherits PID 1's fds.)
- Reset the signal mask (the child must not inherit PID 1's blocked-signal set).
- `execve(child, child_args, env)` with a minimal env: `PATH`, `HOME=/root`,
  `TERM=linux`.

## Signals & shutdown

PID 1 blocks and handles `SIGCHLD`, `SIGTERM`, `SIGINT`, `SIGHUP` via `signalfd`:

| Signal | Action |
| --- | --- |
| `SIGCHLD` | Reap all exited children (`waitpid(-1, WNOHANG)` loop); clear the tracked child pid if it was the one. |
| `SIGTERM` / `SIGINT` | Begin shutdown: `SIGTERM` the child, start the grace timer; after `shutdown_grace_seconds`, `SIGKILL` it, `sync()`, then `reboot(LINUX_REBOOT_CMD_RESTART)`. |
| `SIGHUP` | Logged; config reload is **not implemented**. |

> **Note:** shutdown currently always performs a **restart**
> (`LINUX_REBOOT_CMD_RESTART`). Distinct power-off / halt actions (e.g. a
> `SIGUSR1`→power-off, `SIGUSR2`→halt mapping referenced by `writeonce-logind`
> and `writeonce.toml`) are **not yet wired up** in this crate. `reboot(2)`
> returning `EPERM` (i.e. not actually PID 1) is tolerated so dev runs exit
> cleanly.

## Source layout

| File | Responsibility |
| --- | --- |
| `main.rs` | Startup: PID-1 check, config load, mounts, spawn, enter the event loop. |
| `config.rs` | `/etc/writeonce/pid1.toml` schema + defaults (+ tests). |
| `mount.rs` | The pseudo-filesystem mount table and idempotent `mount(2)`. |
| `exec.rs` | `fork` + session/tty setup + `execve` of the configured child. |
| `signal.rs` | `signalfd` install + the `epoll` event loop, reaping, and shutdown. |

## See also

- `crates/writeonce-svc` — the PID 2 supervisor PID 1 hands off to.
- `crates/writeonce-initramfs` — `switch_root`s into the real root and execs this binary.
- `build/skeleton/etc/writeonce/pid1.toml` — the production config staged onto the image.
