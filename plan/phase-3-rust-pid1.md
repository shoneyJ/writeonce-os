# Phase 3 — Rust PID 1

**Goal.** Replace the BusyBox `/init` (and any sysv stand-in) with a Rust binary that **owns the init contract**: mount essential filesystems, reap zombies, handle signals correctly per PID-1 rules.

## Subtasks

1. **Scaffold the crate.** `src/init/` in the repo. Edition 2021, `#![deny(unsafe_op_in_unsafe_fn)]`, single static-musl binary (`x86_64-unknown-linux-musl`).

2. **Choose linkage strategy.** musl-static gives a self-contained binary with no glibc surprises in early boot. Add musl to the cross-toolchain in Phase 0 (revisit).

3. **Implement the reaping loop.** `nix::sys::wait::waitpid(Pid::from_raw(-1), Some(WaitPidFlag::WNOHANG))` in a `loop`. Reference: `../writeonce-session-notes.md` Topic 2.

4. **Implement signal handling per PID 1 rules** (signalfd + epoll, or a single-thread async loop):
   - SIGTERM/SIGINT: initiate orderly shutdown (Phase 4 will define what that means).
   - SIGHUP: re-read config.
   - SIGCHLD: drives the reaping loop.

5. **Mount essential filesystems** (just enough for a usable userspace):
   - `proc` on `/proc`, `sysfs` on `/sys`, `devtmpfs` on `/dev`, `tmpfs` on `/run`, `devpts` on `/dev/pts`, `cgroup2` on `/sys/fs/cgroup`.

6. **Spawn a placeholder "service"** — exec `/bin/sh` on tty1. (Phase 4 replaces this with the supervisor.)

7. **Define a config file format** for the supervisor handoff. TOML, parsed by `serde`. Schema (draft):
   ```toml
   [pid1]
   tty = "/dev/tty1"
   supervisor = "/usr/bin/writeonce-svc"
   shutdown_grace_seconds = 10
   ```

8. **Test the binary as init in QEMU.** `-append "init=/sbin/writeonce-pid1"`. Trigger a panic (`echo c > /proc/sysrq-trigger`) and confirm netconsole catches it.

9. **Write unit tests** for the parts that can be tested outside PID 1 (config parser, signal-mask construction, mount-list builder). Use `cargo test` on the workstation.

10. **Deploy to the T450.** Install at `/sbin/writeonce-pid1`. Update GRUB cmdline to `init=/sbin/writeonce-pid1`. Keep BusyBox path as fallback (`init=/bin/sh` is a valid panic-button via GRUB edit).

## Deliverable

A Rust binary that **is PID 1** on the T450, reaps children correctly, and execs a shell. Replaces BusyBox `/init` entirely.

## Acceptance criteria

- `ls -la /proc/1/exe` on the T450 → `/sbin/writeonce-pid1`.
- `cat /proc/1/cgroup` shows the root cgroup.
- Spawn 1000 short-lived children from the shell; `ps aux | grep defunct` → empty (no zombies).
- `kill -KILL 1` from root is silently ignored (kernel-enforced).

## References

- `../.agents/reference/linux/init/main.c::kernel_init` — what the kernel does just before PID 1 execs.
- `../.agents/reference/linux/kernel/exit.c::do_wait` — semantics of `waitpid`.
- Rust crates: `nix` for syscalls, `libc` for raw constants, `signal-hook` for signalfd.

## Risks

- A bug in PID 1 = kernel panic on `init` exit. Mitigation: always keep a known-good GRUB entry with `init=/bin/sh`.
- musl + cross-compile gotchas (libpthread merge, locale data). Mitigation: stick to crates that work cleanly on `x86_64-unknown-linux-musl`.
