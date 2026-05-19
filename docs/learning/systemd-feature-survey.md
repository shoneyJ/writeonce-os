# systemd feature survey — what WriteOnce mirrors and what it deliberately doesn't

> Reference companion to [`plan/phase-3-rust-pid1.md`](../../plan/phase-3-rust-pid1.md)
> and [`plan/phase-4-supervisor.md`](../../plan/phase-4-supervisor.md).
> A description, not a prescription: this file records what systemd *does*
> in the areas WriteOnce cares about. Per-area design decisions for the
> WriteOnce equivalents live in the sibling Phase-4 design docs.
>
> Source mirror: `.agents/reference/systemd/` (shallow clone of
> github.com/systemd/systemd, ~136 MB). See [[reference-systemd]] memory.

## 1. PID 1 entry — `src/core/main.c`

systemd's PID 1 startup is a long, deliberately sequenced procedure. The
function call chain we care about — the parts we are mirroring in Rust —
is short:

| Step | Function (in `src/core/main.c` and friends)                    | What it does                                    | WriteOnce equivalent |
| ---- | -------------------------------------------------------------- | ----------------------------------------------- | -------------------- |
| 1    | `parse_proc_cmdline_item()`                                    | Parses `/proc/cmdline` for `systemd.*` options  | Read kernel cmdline for `wo.*` options |
| 2    | `parse_config_file()`                                          | Reads `/etc/systemd/system.conf` (LogLevel, watchdog, …) | Parse `/etc/writeonce/pid1.toml`     |
| 3    | `mount_setup()` (`src/core/mount-setup.c`)                     | Mounts `proc`, `sysfs`, `devtmpfs`, `devpts`, `tmpfs` on `/run`, `cgroup2` | `mount::mount_essentials()` |
| 4    | `manager_new()` (`src/core/manager.c`)                         | Constructs the central manager state machine    | Out of prototype scope (Phase 4)     |
| 5    | `manager_setup_signals()` (`src/basic/signal-util.h`)          | Blocks all signals, then creates `signalfd` for SIGCHLD/SIGTERM/SIGINT/SIGHUP | `signal::install_handler()` |
| 6    | `sd_event_run(m->event)` reaping loop                          | Blocks on the event loop; on SIGCHLD calls `manager_dispatch_sigchld()` which drains zombies | `main::event_loop()` with `epoll_wait` + `waitpid(WNOHANG)` |
| 7    | Shutdown — `emergency_action()` / `manager_start_target()`     | On SIGTERM: stop all units in reverse dependency order, sync, `reboot(2)` | `signal::handle_term()` (kill child, sync, reboot) |

What we are **not** mirroring from main.c: capability bounding sets,
`CrashReboot`, watchdog setup, kdbus probing, `RuntimeWatchdogSec`,
audit subsystem integration, container detection, and the giant
selinux/apparmor/smack policy load. Our PID 1 stays under 300 lines of
Rust; systemd's `main.c` is 173 KB of C.

## 2. Manager state machine — `src/core/manager.c`

systemd's manager is the central object that owns every unit and every
job. The state model has two layers:

### Unit active states (`UnitActiveState` in `src/core/unit.h`)

```
UNIT_INACTIVE       not running, not transitioning
UNIT_ACTIVATING     start in progress
UNIT_ACTIVE         running
UNIT_DEACTIVATING   stop in progress
UNIT_FAILED         start or stop failed
UNIT_RELOADING      reload in progress      (we omit)
UNIT_REFRESHING     transient refresh       (we omit)
```

WriteOnce's supervisor adopts the five-state subset; we don't implement
`reloading` or `refreshing` because they apply to features (mount
re-reading, transient unit refresh) we don't carry.

### Job states (`JobState` in `src/core/job.h`)

```
JOB_WAITING   queued, awaiting dependencies
JOB_RUNNING   actively executing
JOB_FINISHED  done; check JobResult for the outcome
```

Job results (`JobResult`):

```
JOB_DONE          success
JOB_FAILED        the work itself failed
JOB_DEPENDENCY    a required dependency failed
JOB_TIMEOUT       grace period elapsed
JOB_CANCELED      conflict resolution canceled this job
```

### Event loop integration

systemd uses `sd-event` (a libsystemd-internal epoll wrapper). The
Manager registers callbacks for:

- `manager_dispatch_sigchld()` — triggered by SIGCHLD; drains zombies
- `manager_dispatch_run_queue()` — processes queued job executions
- `manager_dispatch_jobs_in_progress()` — timer watchdog for stuck jobs
- `manager_dispatch_signal_fd()` — handles SIGTERM/SIGINT/SIGHUP

`sd_event_run(m->event)` is the blocking call that ties them together.

WriteOnce's equivalent uses raw `epoll` and `signalfd` from `libc`
directly — see `phase-4-cgroup-isolation.md` for the loop structure.

## 3. Unit-file directives — `src/core/load-fragment-gperf.gperf.in`

systemd has ~200 unit-file directives. The subset WriteOnce supports
(captured in `phase-4-service-toml-schema.md`) is roughly 20:

### Required for any minimal supervisor

| Section     | Directive               | Purpose                                                         |
| ----------- | ----------------------- | --------------------------------------------------------------- |
| `[Unit]`    | `Description=`          | human-readable name                                             |
| `[Unit]`    | `After=`                | *ordering* — start me after these                               |
| `[Unit]`    | `Before=`               | *ordering* — start me before these                              |
| `[Unit]`    | `Wants=`                | *weak* requirement — pull them in; failures non-fatal           |
| `[Unit]`    | `Requires=`             | *hard* requirement — pull them in; failures fatal               |
| `[Unit]`    | `Conflicts=`            | mutually exclusive                                              |
| `[Unit]`    | `DefaultDependencies=`  | if `yes`, add implicit deps on `basic.target` + shutdown        |
| `[Service]` | `Type=`                 | `simple` \| `forking` \| `oneshot` \| `notify`                  |
| `[Service]` | `ExecStart=`            | command to start                                                |
| `[Service]` | `ExecStop=`             | command to stop (empty → SIGTERM with timeout escalation)       |
| `[Service]` | `Restart=`              | `no` \| `always` \| `on-failure`                                |
| `[Service]` | `RestartSec=`           | delay before restart                                            |
| `[Service]` | `TimeoutStartSec=`      | give up if start hasn't completed                               |
| `[Service]` | `TimeoutStopSec=`       | escalate to SIGKILL after this                                  |
| `[Service]` | `User=` / `Group=`      | drop privileges                                                 |
| `[Service]` | `Slice=`                | cgroup slice for placement                                       |
| `[Service]` | `RemainAfterExit=`      | for `Type=oneshot`: stay "active" after exit                    |
| `[Install]` | `WantedBy=`             | **reverse-dep** — be pulled in by these targets when enabled    |
| `[Install]` | `RequiredBy=`           | **reverse-dep, hard** — same but `Requires=`                    |

### Included for completeness

| Directive       | Reason                                                       |
| --------------- | ------------------------------------------------------------ |
| `[Unit]` `BindsTo=` | "If you stop, I stop." Bidirectional binding.            |
| `[Unit]` `PartOf=`  | "If you stop, I stop." Unidirectional (child of parent).|

### Deliberately omitted

| Directive class                                | Why                                                          |
| ---------------------------------------------- | ------------------------------------------------------------ |
| `Sandboxing` (`ProtectSystem=`, `PrivateTmp=`, `NoNewPrivileges=`, `CapabilityBoundingSet=`) | Achievable via cgroup limits + namespaces later; not required for first boot |
| `Triggering` (`AssertPath*=`, `ConditionPath*=`) | Adds a small dependency-graph layer not needed for the i3More service set |
| `OnFailure=` / `OnSuccess=`                    | Re-introduce later with a transitional fault-event API       |
| `Socket activation` (`.socket` unit type)      | Useful but out of scope; xinetd-style fallback if needed     |
| `Path activation` (`.path` unit type)          | Same                                                         |
| `Timer units` (`.timer` unit type)             | Replaced by cron-like external tooling                       |
| `Mount` / `automount` units                    | Out of scope; rely on `/etc/fstab` + `mount.service`         |
| `Scope units` (transient)                      | We don't need transient supervision                          |

## 4. Dependency resolution — `src/core/transaction.c`

The transaction is the *job graph* built when systemd is asked to bring a
unit (or target) up. systemd distinguishes four edge types:

| Edge type    | Source                           | Semantic                                                                 |
| ------------ | -------------------------------- | ------------------------------------------------------------------------ |
| Requirement  | `Requires=`, `Wants=`            | "if I activate, I want/need these too"                                   |
| Ordering     | `After=`, `Before=`              | "order me with respect to these in the transaction" (no pulling)         |
| Binding      | `BindsTo=`, `PartOf=`            | "if my partner goes down, I go down"                                     |
| Conflict     | `Conflicts=`                     | "we cannot be active simultaneously"                                     |

### Transaction building

```text
build_transaction(anchor):
  unit_set = transitive closure of {anchor} via Wants ∪ Requires ∪ BindsTo
  for each unit in unit_set:
      create Job{unit, kind=Start, state=Waiting}
  for each pair (a, b) in unit_set:
      if a.Before contains b.id:  edge(a → b)   (a must finish before b starts)
      if a.After  contains b.id:  edge(b → a)
  detect cycles in ordering edges → break weakest, log warning
  detect cycles in requirement edges → error (refuse the transaction)
  detect conflicts → cancel the lower-priority job
  topological sort by ordering edges
  return ordered job list
```

systemd merges duplicate jobs in
`transaction_merge_and_delete_job()` — if two transactions both want
unit X started, they collapse into one `JOB_START` for X.

WriteOnce simplifies further: no scope units, no transient oneshot
activation, no D-Bus-activated services. The algorithm boils down to a
graph traversal + topological sort + cycle detection. See
[`phase-4-dependency-graph.md`](phase-4-dependency-graph.md) for the
WriteOnce algorithm in pseudocode and Rust types.

## 5. Cgroup v2 integration — `src/core/cgroup.c`

systemd manages a unified cgroup hierarchy at `/sys/fs/cgroup`:

```
/sys/fs/cgroup/
├── system.slice/
│   ├── sshd.service/
│   │   ├── cgroup.procs        ← PIDs in this cgroup, one per line
│   │   ├── cgroup.events       ← inotify here for "populated" transitions
│   │   ├── cpu.max
│   │   └── memory.max
│   └── dbus.service/...
├── user.slice/
│   └── user-1000.slice/
│       └── session-1.scope/    ← X session, Sway/i3 etc.
└── init.scope                  ← systemd itself
```

### Placement mechanism

systemd uses `clone3(CLONE_INTO_CGROUP)` when the kernel supports it
(Linux 5.7+). The child is created already inside the target cgroup —
**no race window** between `fork()` and `write(cgroup.procs)` during
which the child could be in the parent's cgroup.

The legacy path (still in tree for older kernels):

```c
pid_t pid = fork();
if (pid == 0) { /* child */ execv(...); }
else { dprintf(cgroup_procs_fd, "%d\n", pid); }
```

…leaves the child briefly in the supervisor's cgroup until the parent's
write lands. Usually invisible, but observable under heavy load and
forbidden by some security profiles.

T450 runs kernel 6.12. **WriteOnce uses `clone3(CLONE_INTO_CGROUP)`
exclusively**; no legacy fallback. See
[`phase-4-cgroup-isolation.md`](phase-4-cgroup-isolation.md).

### Resource limits

After the cgroup directory exists and *before* the service starts (or
shortly after), systemd writes:

| File              | Example       | Meaning                                              |
| ----------------- | ------------- | ---------------------------------------------------- |
| `cpu.max`         | `200000 100000` | 200% of one CPU = 2 cores worth                    |
| `memory.max`      | `2G`          | OOM-killed above this                                |
| `pids.max`        | `512`         | hard cap on processes in the cgroup                  |
| `io.max`          | `8:0 wbps=1M` | per-device I/O limit                                 |

WriteOnce's first supervisor pass writes `pids.max` and `memory.max`
defaults; per-service overrides come from the `[Service]` section of
`service.toml`.

## 6. Logind minimum surface — `src/login/logind-dbus.c`

`org.freedesktop.login1.Manager` exposes ~40 D-Bus methods. The set
WriteOnce must implement to satisfy `i3more-lock` + a generic X session:

| Method / Property                    | Required by                                   | Notes                                                          |
| ------------------------------------ | --------------------------------------------- | -------------------------------------------------------------- |
| `Inhibit(what, who, why, mode)` → fd | **`i3more-lock`** (blocks VT switching)       | Must return a file descriptor that, when closed, releases the inhibitor |
| `ListSessions()` → array             | session managers                              | Walk in-memory session table                                   |
| `GetSession(id)` → object path       | session managers                              | Hash lookup                                                    |
| `ActivateSession(id)`                | login managers                                | Mark as foreground; VT switch                                  |
| Property `BlockInhibited`            | Power management UIs                          | String list of active inhibit types                            |
| Property `NCurrentInhibitors`        | Power management UIs                          | uint64                                                         |
| Property `PreparingForShutdown`      | Save-state on shutdown clients                | bool, signal on change                                         |
| Property `PreparingForSleep`         | Suspend-aware clients                         | bool, signal on change                                         |

systemd's full vtable has 40+ methods (`PowerOff`, `Reboot`, `Suspend`,
`Hibernate`, `LockSessions`, `CreateSession`, `ReleaseSession`,
`AttachDevice`, `FlushDevices`, …). WriteOnce implements the 4 methods +
4 properties above and stubs everything else with `org.freedesktop.DBus.Error.NotImplemented`.

The shim runs as a normal service supervised by the WriteOnce
supervisor; it sits on the system bus at
`/org/freedesktop/login1`.

---

## What this survey omits on purpose

This file is not "everything systemd does" — that document is the
systemd source itself, mirrored at `.agents/reference/systemd/`. This
survey captures only the surface WriteOnce intends to recreate, plus
explicit notes on what's omitted and why. When implementation work later
discovers that an i3More feature transitively depends on a systemd
directive listed under "deliberately omitted", the right move is to
update both this file (move it to the supported list) and the
corresponding Phase-4 schema doc, then implement.
