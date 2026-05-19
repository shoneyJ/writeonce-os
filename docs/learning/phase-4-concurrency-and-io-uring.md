# Concurrency model for PID 1 + supervisor — and where io_uring fits

> Companion to [`plan/phase-3-rust-pid1.md`](../../plan/phase-3-rust-pid1.md)
> and [`plan/phase-4-supervisor.md`](../../plan/phase-4-supervisor.md).
> Answers three recurring design questions: are these binaries
> single-threaded? When does threaded parallelism help? Where does
> io_uring earn its keep?

## TL;DR

| Component         | Today                       | Future-feasible?                         |
| ----------------- | --------------------------- | ---------------------------------------- |
| `writeonce-pid1`  | single-threaded             | should stay single-threaded              |
| `writeonce-svc`   | single-threaded (planned)   | yes — but **async-parallel, not thread-parallel** |
| io_uring          | not used                    | useful for the supervisor's log forwarding; not for PID 1; not for the spawn itself |

## 1. Why single-threaded by default

Both binaries are structured around **one thread driving an event loop** over a single epoll instance:

```
                ┌──── signalfd (SIGCHLD/TERM/INT/HUP) ───┐
                │                                       │
   epoll_wait ──┼──── inotify on cgroup.events ─────────┼── one thread,
                │                                       │   one main loop
                └──── unix socket (wo-ctl)  ────────────┘
                       D-Bus fd (logind shim)
```

Reasons to stay single-threaded:

- **Fork-after-threads is a minefield.** `fork()` and `clone3()` clone only the calling thread. Mutex/heap state held by other threads remains in the child in whatever state it was at the fork. Any allocator call between fork and execve can deadlock. Single-threaded supervisors avoid this entirely.
- **PID 1's contract is small** — reaping + signal dispatch + (eventually) a control-socket listener. Threading buys nothing.
- **systemd is structured the same way.** Its main loop is single-threaded over `sd-event` (epoll-backed). Worker threads exist only in narrow auxiliary paths (journald compressor, some D-Bus marshaling) — never for spawn.
- **Debugging is tractable.** A single-threaded event loop is trivially deterministic given the same external input. Service-supervisor bugs surface as ordering puzzles; threads turn those puzzles into Heisenbugs.

## 2. Parallel service activation — async, not threaded

Spawning many services in parallel is real and valuable: `dbus.service`, `pipewire.service`, and `xorg.service` may have no ordering edges between them, so an honest supervisor should activate them concurrently. systemd calls this unit-startup parallelization.

The right tool for it is **async I/O on the single thread**, not threads:

```rust
// Pseudocode, single-threaded:
for unit in ready_to_start_units {     // units with no unsatisfied After/Before edges
    let cgroup_fd = open_or_create_cgroup_dir(&unit)?;
    let pid = clone3_into_cgroup(cgroup_fd, &unit.exec)?;
    state.mark_activating(unit.id, pid);
}
// Now block on epoll_wait — when any service's cgroup.events fires
// (= it reached "active" or "failed"), the loop handles it and
// activates the next batch of units whose dependencies are now satisfied.
```

The whole graph runs to completion this way. CPU time spent inside the supervisor is microseconds per service; what we save is the wall-clock time during which services are doing their own initialization work in parallel. Threading the supervisor itself would not shorten that wait.

## 3. Where threads do make sense (later, optional)

| Workload                                     | Thread-worthy?                                                          |
| -------------------------------------------- | ----------------------------------------------------------------------- |
| Parsing ~100 `service.toml` files at boot    | Embarrassingly parallel. `rayon::par_iter().map(parse)`. Sub-ms either way; ergonomic, not necessary. |
| Per-service stdout/stderr → journal forwarding | Could thread per service. We won't — `epoll` over all pipes is enough. |
| Journal compression / rotation               | Real CPU work in the background. Future, optional.                       |
| Cgroup statistics scraping                   | Background poll — possible but trivial cost.                            |
| **`fork`/`clone3` itself**                   | **Never.** Always from the single main thread.                          |

The invariant we hold to: **whatever calls `fork`/`clone3` is single-threaded.** Auxiliary CPU-bound work (compression, parsing) can run on a worker pool that sends results back to the main thread via channels. The main thread retains exclusive ownership of "spawn services."

## 4. io_uring — where it earns its keep

io_uring is Linux's modern async-syscall mechanism: a userspace ring of submitted operations (SQEs) and a ring of completions (CQEs). Available since kernel 5.1; opcodes for `openat`, `read`, `write`, `close`, `connect`, `accept`, `recv`, `send` matured in 5.6–5.11. The T450's 6.12 has everything we would want.

### PID 1 — no real fit

PID 1's I/O surface is tiny:

| Operation                  | Frequency           | io_uring win?                          |
| -------------------------- | ------------------- | -------------------------------------- |
| `read` from signalfd       | once per signal     | No — signalfd reads are 128 B, dwarfed by syscall fixed cost. |
| `waitpid(-1, WNOHANG)`     | once per child death | No opcode exists.                     |
| `mount(2)` (boot only)     | 6 times, once       | No opcode; would be a syscall anyway.  |
| `reboot(2)`                | once, at shutdown   | N/A.                                   |

A small statically-linked binary is more valuable than a few microseconds saved per signal. **Keep PID 1 on plain epoll + signalfd.**

### Supervisor — modest fit, deferred to later

Where io_uring actually helps:

1. **Service log forwarding.** Each service writes to a pipe; the supervisor `read`s the pipe and `write`s the bytes into a per-service log file (plus a binary journal). With 10–20 services that is 20–40 pipes to multiplex. Today's plan does this with `epoll` + blocking `read`/`write`. With io_uring you batch many `read`s and many `write`s into a single submission, halving syscall count under load. Real but secondary win.
2. **Boot-time unit-file loading.** At startup the supervisor scans `/etc/writeonce/services/*.toml` — maybe 100 small reads. `IOSQE_IO_LINK`-chained `openat → read → close` triples per file are faster than the serial syscalls, at the cost of more code. Marginal.
3. **Cgroup writes for limit-application.** `pids.max`, `memory.max`, `cpu.max` writes per service — could batch via io_uring. Trivial benefit; these writes happen once per service start.

### Where io_uring does not help

- **`clone3(CLONE_INTO_CGROUP)`.** There is no io_uring opcode for `clone3`. The actual process creation remains a synchronous syscall. Since this is the single most "expensive" operation in the supervisor's hot path (microseconds, not milliseconds), io_uring on the rest of the I/O is icing.
- **D-Bus.** zbus does its own I/O scheduling; replacing it with io_uring under the hood is upstream-tracker territory, not ours.
- **inotify reads.** Same story as signalfd — too small to benefit.

### Crate options if/when we adopt it

| Crate            | Style                                           | When it fits                                                 |
| ---------------- | ----------------------------------------------- | ------------------------------------------------------------ |
| `io-uring`       | Bare ring access, manual SQE/CQE bookkeeping    | If we only need io_uring for one subsystem (log forwarding). ~5 KB extra binary. |
| `tokio-uring`    | tokio runtime backed by io_uring                | Easier to write, but pulls in the full tokio dep tree (~30 crates). Overkill for a single-threaded supervisor. |
| `glommio`        | Single-thread, thread-per-core async runtime over io_uring | Closest match to our design model. Worth evaluating once we have real telemetry showing the journal is a hot spot. |
| `monoio`         | Similar to glommio, thread-per-core             | Same notes as glommio.                                       |

## 5. Concrete recommendation per phase

- **Phase 3 (PID 1).** Stays as written: single thread, `epoll_wait` + `signalfd` + `waitpid(WNOHANG)`. No threads, no io_uring.
- **Phase 4 first cut (supervisor).** Single thread, `epoll_wait` over signalfd + inotify(cgroup.events) + Unix-socket fds + D-Bus fd. Parallel service activation via async I/O (multiple `clone3` calls from the main thread, then wait on cgroup.events for each).
- **Phase 4-b / Phase 10 hardening.** Consider `io-uring` (the bare crate) for the log-forwarding subsystem behind a Cargo feature flag. Adopt only after telemetry shows the synchronous write path is a real bottleneck (unlikely on a single-user laptop, but worth measuring).
- **Auxiliary CPU work.** When configuration parsing or journal compression becomes measurable, move it to a `rayon`-style worker pool whose results funnel back to the main thread via `mpsc::channel`. Never let those workers call `fork`/`clone3`.

## 6. What changes if the supervisor *itself* is multi-threaded

Hypothetical question: what would have to change if we *did* make `writeonce-svc` multi-threaded?

- **One designated "spawn thread" with exclusive ownership of `fork`/`clone3`.** All other threads send "please spawn service X" requests over a channel; the spawn thread serves them. The spawn thread never holds any allocator lock when it forks (typically it does nothing else).
- **Allocator discipline.** Either pre-allocate everything needed for spawn before issuing it, or use a fork-safe allocator like `mimalloc` with the `secure` feature.
- **Signal-handling refactor.** Only one thread reads from signalfd. Others are blocked from all signals via `pthread_sigmask(SIG_BLOCK)`.
- **Lock ordering documented.** Every shared state mutex acquires in a documented order; deadlocks become a static analysis problem.

All of this is a substantial increase in implementation complexity for a single-user-laptop supervisor. We don't take it on. If WriteOnce is later ported to a workload where it matters (a many-core machine running thousands of services), revisit then with telemetry first.
