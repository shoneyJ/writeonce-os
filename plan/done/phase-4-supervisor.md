# Phase 4 — Rust service supervisor + cgroup v2 + minimal logind shim

**Goal.** Bring up multiple services in a dependency-resolved order, place each in its own cgroup v2, expose a small logind-compatible D-Bus surface (needed by `i3more-lock` and PAM-aware services in Phase 9).

## Subtasks

1. **Design the service descriptor format** (`/etc/writeonce/services/*.toml`):
   ```toml
   name = "sshd"
   description = "OpenSSH daemon"
   after = ["network-online"]
   requires = ["network-online"]
   exec = "/usr/sbin/sshd -D"
   restart = "on-failure"
   cgroup = "system.slice/sshd.service"
   user = "root"
   ```

2. **Implement the dependency resolver** as a topological sort over the `after`/`requires` graph. Cycle detection at config-load time.

3. **Implement service lifecycle.**
   - `fork` + `execve` per service.
   - cgroup v2: create `/sys/fs/cgroup/system.slice/<name>.service`, write the child PID to `cgroup.procs`.
   - State machine: `inactive → activating → active → deactivating → inactive | failed`.
   - Restart policy: `always | on-failure | never`.

4. **Implement targets** (collections of services): `sysinit.target`, `basic.target`, `multi-user.target`, `graphical.target`. Boot ends when the configured `default.target` is `active`.

5. **Wire supervisor to PID 1.** PID 1 spawns the supervisor as its single child (per Phase 3 step 6). Supervisor is **PID 2**, not PID 1 — keeps the PID 1 binary minimal and auditable.

6. **Implement a minimal D-Bus surface** that satisfies i3More's needs:
   - `org.freedesktop.login1.Manager.Inhibit()` — needed by `i3more-lock` to block VT switching. Implement the method (it can be a no-op shim that returns a valid file-descriptor lock initially; revisit if VT switching matters).
   - Use the `zbus` crate (pure-Rust D-Bus, async).
   - System bus on `/var/run/dbus/system_bus_socket`. Run `dbus-daemon` as a service first (Phase 8), then add the writeonce-logind shim.

7. **Implement structured logging.** Each service's stdout/stderr → ring buffer in supervisor → tee to `/var/log/writeonce/<service>.log` + journal-style binary log at `/var/log/writeonce/journal`. CLI tool `wo-logs` to query.

8. **Implement a control CLI** — `wo-ctl start/stop/restart/status/list/journal <unit>`. Communicates with the supervisor over a Unix socket at `/run/writeonce/control.sock`.

9. **Implement orderly shutdown.** PID 1 receives SIGTERM → tells supervisor to stop all services in reverse dependency order with per-service grace period → unmounts → kexec or `reboot(2)`.

10. **Test in QEMU with a fake service graph** (dummy sleep services) before the T450.

## Deliverable

A working supervisor that starts and stops services in dependency order with cgroup v2 placement, controllable via `wo-ctl`.

## Acceptance criteria

- `wo-ctl list` shows services in `active` after boot to `multi-user.target`.
- `systemd-cgls` equivalent (a built-in `wo-ctl cgroups`) shows the expected hierarchy.
- `busctl --system tree org.freedesktop.login1` shows the `Inhibit` method.
- Killing a service externally (`kill -9 $PID`) triggers `on-failure` restart.

## References

- `../.agents/reference/linux/Documentation/admin-guide/cgroup-v2.rst` — cgroup v2 reference.
- s6 / dinit / finit source as reference architectures (browse externally; do not vendor).
- `zbus` crate docs for the D-Bus implementation.

### Design companion docs

These four learning docs flesh out the Phase 4 sketch into implementable detail:

- [`../docs/learning/systemd-feature-survey.md`](../../docs/learning/systemd-feature-survey.md) — what the WriteOnce supervisor mirrors from systemd (and what it doesn't).
- [`../docs/learning/phase-4-cgroup-isolation.md`](../../docs/learning/phase-4-cgroup-isolation.md) — `clone3(CLONE_INTO_CGROUP)` placement design.
- [`../docs/learning/phase-4-service-toml-schema.md`](../../docs/learning/phase-4-service-toml-schema.md) — unit-file schema + `WantedBy` reverse-dependency semantics.
- [`../docs/learning/phase-4-dependency-graph.md`](../../docs/learning/phase-4-dependency-graph.md) — transaction build, edge types, cycle handling.

## Risks

- D-Bus surface is the slipperiest dependency — i3More may use more of logind than `Inhibit`. Mitigation: re-run the i3More OS-dependency survey at the start of Phase 9 to catch additions; iterate the shim.
