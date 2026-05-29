# Phase 4 — cgroup-v2 service isolation via `clone3(CLONE_INTO_CGROUP)`

> Design companion to [`plan/done/phase-4-supervisor.md`](../../plan/done/phase-4-supervisor.md).
> Describes the placement model the WriteOnce supervisor will use to
> launch services into their own cgroups atomically.

## Goal

Every service the WriteOnce supervisor spawns lives in its own cgroup
under `/sys/fs/cgroup/system.slice/<name>.service/`. The placement must
be **race-free**: the child process should never observably be a member
of the supervisor's cgroup, not even for a microsecond. This matters
for two reasons:

1. **Accounting correctness** — cgroup-based accounting (memory, CPU,
   pids) must attribute the child's lifetime entirely to its own
   cgroup.
2. **Limit enforcement** — `pids.max`, `memory.max`, `cpu.max` set on
   the service cgroup must apply from the child's first instruction.

The kernel feature that makes this possible is
`clone3(CLONE_INTO_CGROUP, cgroup_fd)`, available since Linux 5.7.
The T450 runs 6.12, so we are well clear of the version floor and
WriteOnce uses this path exclusively — no legacy `fork()` +
`write(cgroup.procs)` fallback.

## The flow

```
1. Make sure /sys/fs/cgroup is mounted as cgroup2.
   (PID 1's mount_essentials() does this once at boot.)

2. Create the unit's cgroup directory:
       mkdir("/sys/fs/cgroup/system.slice/sshd.service", 0755)

3. Apply resource limits BEFORE the child starts:
       echo "2G"           > .../memory.max
       echo "512"          > .../pids.max
       echo "200000 100000" > .../cpu.max    (200% of one CPU)

4. Open the directory as a file descriptor:
       cgroup_fd = open(".../sshd.service", O_PATH | O_DIRECTORY)

5. Build clone_args:
       struct clone_args ca = {
           .flags    = CLONE_INTO_CGROUP,
           .cgroup   = cgroup_fd,
           .exit_signal = SIGCHLD,
       };

6. Invoke clone3:
       long pid = syscall(SYS_clone3, &ca, sizeof(ca));
       if (pid == 0) {
           /* child — already inside sshd.service cgroup */
           setresuid(uid, uid, uid);
           setresgid(gid, gid, gid);
           execve("/usr/sbin/sshd", argv, envp);
           _exit(127);                   /* execve failed */
       }

7. Register cgroup.events with inotify so the supervisor learns when
   the cgroup becomes unpopulated (last process exited):
       inotify_add_watch(ifd, ".../cgroup.events", IN_MODIFY);
```

## The `libc` crate wrapper

The Rust `libc` crate (as of 0.2.x) does not expose a typed
`clone3()` wrapper. We use the generic `syscall` shim:

```rust
use libc::{c_long, syscall, SYS_clone3};
use std::os::fd::RawFd;

#[repr(C)]
struct CloneArgs {
    flags: u64,
    pidfd: u64,
    child_tid: u64,
    parent_tid: u64,
    exit_signal: u64,
    stack: u64,
    stack_size: u64,
    tls: u64,
    set_tid: u64,
    set_tid_size: u64,
    cgroup: u64,
}

const CLONE_INTO_CGROUP: u64 = 0x200000000;

/// Returns 0 in the child, the child's pid in the parent, or -1 on error.
unsafe fn clone3_into_cgroup(cgroup_fd: RawFd) -> c_long {
    let args = CloneArgs {
        flags:        CLONE_INTO_CGROUP,
        exit_signal:  libc::SIGCHLD as u64,
        cgroup:       cgroup_fd as u64,
        pidfd: 0, child_tid: 0, parent_tid: 0,
        stack: 0, stack_size: 0, tls: 0,
        set_tid: 0, set_tid_size: 0,
    };
    syscall(SYS_clone3, &args as *const _, size_of::<CloneArgs>())
}
```

This is `unsafe` for good reasons: the kernel-side state machine of
`clone3` is delicate, and getting the struct layout wrong (e.g. missing
fields added in a later kernel revision) produces `-EINVAL` at best,
silent misbehavior at worst. The `CloneArgs` struct must match the
kernel's `struct clone_args` from `<linux/sched.h>` exactly. We accept
this and isolate the unsafe block in a single, well-tested module.

## The race the legacy path has

For posterity, here's what `clone3(CLONE_INTO_CGROUP)` saves us from.
The classic two-step:

```rust
let pid = libc::fork();
if pid == 0 {
    /* child runs here, still in PARENT's cgroup */
    execv(...);
} else {
    write_to(cgroup_procs_file, pid);
    /* now the child is in its own cgroup, sometime after */
}
```

Between `fork` returning in the parent and the parent's `write` landing,
the child may have already executed dozens of instructions — possibly
including `execve()` itself — while still attributed to the parent's
cgroup. Under heavy load that window stretches to milliseconds.

Concretely, this means:

- A `pids.max=512` limit set on the *child's* cgroup does not apply to
  the brief moment after fork but before placement.
- Any allocations the child performs in that window are accounted to
  the parent's `memory.max`.
- A `memory.max=2G` limit on the child's cgroup could be exceeded
  briefly before placement, in violation of the policy.

For a supervisor that fork-execs many short-lived helpers, the
cumulative effect of the race is observable. `CLONE_INTO_CGROUP`
removes it entirely: the child is born inside its cgroup with the
limits already in force.

## What the supervisor monitors

After clone, the supervisor watches `cgroup.events` via inotify:

```
cgroup.events contents:
    populated 1     ← one or more processes in the cgroup
    frozen 0
```

When the last process exits, the kernel updates the file to
`populated 0` and fires the inotify event. The supervisor learns the
service is "done" without polling `waitpid`. (We still `waitpid` to
reap the zombie, but the cgroup-events stream is the authoritative
liveness signal — it handles double-forks and detached daemons that
PID-based tracking misses.)

## State transitions covered by this design

| Service event              | Signal source            | Supervisor action                              |
| -------------------------- | ------------------------ | ---------------------------------------------- |
| Service starts             | `clone3` returns         | Record PID → cgroup mapping; state `activating` |
| Main process exits cleanly | inotify on `cgroup.events` → populated 0 | reap via `waitpid`; state `inactive`        |
| Main process crashes       | same                     | reap; state `failed`; consult `Restart=`        |
| Cgroup limit hit (OOM)     | inotify on `memory.events` | log, state `failed`, treat as crash       |
| Manual stop                | supervisor itself        | send SIGTERM; on timeout escalate to SIGKILL; state `deactivating` → `inactive` |

## What's deferred

This design covers placement and the basic event surface. Future work:

- Per-service namespace isolation (`unshare()` with PID/mount/network
  namespaces) — adds container-grade isolation but requires more
  unit-file directives.
- `seccomp` filters for service-specific syscall allowlists.
- BPF cgroup attaches for finer-grained policy (mostly accounting).

These belong to a Phase 4-b or Phase 10-style hardening pass.
