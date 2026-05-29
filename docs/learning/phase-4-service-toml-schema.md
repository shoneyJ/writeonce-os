# Phase 4 — `service.toml` schema and `WantedBy` semantics

> Design companion to [`plan/done/phase-4-supervisor.md`](../../plan/done/phase-4-supervisor.md).
> Specifies the unit-file format the WriteOnce supervisor will consume
> and how the reverse-dependency mechanism (`WantedBy=` /
> `RequiredBy=`) maps onto an in-memory dependency graph.
>
> systemd analogue: `man systemd.unit`, `man systemd.service`,
> `man systemd.install`.

## File layout on disk

```
/etc/writeonce/
├── pid1.toml                          ← PID 1 config (kept separate)
└── services/
    ├── dbus.service.toml
    ├── pipewire.service.toml
    ├── wireplumber.service.toml
    ├── xorg.service.toml
    ├── getty@tty1.service.toml         ← templated; @-suffix → instance name
    ├── multi-user.target.toml          ← targets are .target.toml (no [service])
    └── graphical.target.toml
```

Filename grammar: `<name>[.<kind>].toml`. The `kind` is one of
`service`, `target`, `socket` (future), `mount` (future), `timer`
(future). A unit's name (used in `Wants=`, `After=`, etc.) is the
filename minus `.toml`. So `dbus.service.toml` is referenced as
`dbus.service`.

## Schema

Three TOML tables, mirroring systemd's `[Unit]`, `[Service]`,
`[Install]`:

```toml
[unit]
description       = "OpenSSH daemon"
after             = ["network-online.target"]
before            = []
requires          = ["network-online.target"]
wants             = []
binds-to          = []
part-of           = []
conflicts         = []
default-dependencies = true   # if true, supervisor implicitly adds
                              #   after = [basic.target] and
                              #   conflicts = [shutdown.target]

[service]
type              = "simple"  # simple | forking | oneshot | notify
exec-start        = "/usr/sbin/sshd -D"
exec-stop         = ""        # empty → SIGTERM with escalation
exec-reload       = ""
restart           = "on-failure"   # no | always | on-failure | on-abnormal
restart-sec       = "5s"
timeout-start-sec = "30s"
timeout-stop-sec  = "10s"
user              = "root"
group             = "root"
slice             = "system.slice"
remain-after-exit = false
environment       = ["LC_ALL=POSIX", "PATH=/usr/bin:/usr/sbin"]

[install]
wanted-by         = ["multi-user.target"]
required-by       = []
```

For a **target** unit (e.g. `multi-user.target.toml`), only `[unit]`
appears — no `[service]` section.

## The `[install]` section: reverse dependencies

This is the most subtle part of the schema, and the easiest to misread
from systemd's wording.

**Surface fact:** the file `sshd.service.toml` contains
`wanted-by = ["multi-user.target"]`.

**Effect:** when this unit is *enabled* (via `wo-ctl enable
sshd.service`), the supervisor adds an implicit
`wants = ["sshd.service"]` to `multi-user.target`'s in-memory unit
record. Equivalently: a symlink would be created at
`/etc/writeonce/services/multi-user.target.wants/sshd.service` — but
WriteOnce keeps it in-memory rather than relying on filesystem
symlinks.

**Why this matters:** `multi-user.target.toml` does not need to
enumerate every service that should activate with it. Each service
declares "I want to be part of this target" in its own file. Targets
remain stable as services come and go.

```text
                        ┌──────────────────────────┐
sshd.service.toml:      │ [install]                │
                        │ wanted-by =              │
                        │   ["multi-user.target"]  │
                        └─────────────┬────────────┘
                                      │
                                      │ at supervisor load time
                                      │ this becomes an implicit
                                      │ Wants= edge:
                                      ▼
multi-user.target  (in-memory)  Wants= sshd.service
```

The transitive closure logic in
[`phase-4-dependency-graph.md`](phase-4-dependency-graph.md) then
treats this exactly like an explicit `wants = ["sshd.service"]` in
`multi-user.target.toml`.

`required-by` works identically but produces an implicit
`requires = […]` edge (hard requirement — if `sshd.service` fails to
start, `multi-user.target` fails too).

## Rust types

```rust
use serde::Deserialize;
use std::time::Duration;

#[derive(Debug, Deserialize)]
pub struct UnitFile {
    #[serde(default)]
    pub unit:    UnitSection,
    #[serde(default)]
    pub service: Option<ServiceSection>,
    #[serde(default)]
    pub install: InstallSection,
}

#[derive(Debug, Default, Deserialize)]
pub struct UnitSection {
    #[serde(default)]
    pub description:          String,
    #[serde(default)]
    pub after:                Vec<String>,
    #[serde(default)]
    pub before:               Vec<String>,
    #[serde(default)]
    pub requires:             Vec<String>,
    #[serde(default)]
    pub wants:                Vec<String>,
    #[serde(default, rename = "binds-to")]
    pub binds_to:             Vec<String>,
    #[serde(default, rename = "part-of")]
    pub part_of:              Vec<String>,
    #[serde(default)]
    pub conflicts:            Vec<String>,
    #[serde(default = "default_true", rename = "default-dependencies")]
    pub default_dependencies: bool,
}
fn default_true() -> bool { true }

#[derive(Debug, Deserialize)]
pub struct ServiceSection {
    #[serde(rename = "type", default = "default_service_type")]
    pub type_:                ServiceType,
    #[serde(rename = "exec-start")]
    pub exec_start:           String,
    #[serde(default, rename = "exec-stop")]
    pub exec_stop:            String,
    #[serde(default, rename = "exec-reload")]
    pub exec_reload:          String,
    #[serde(default = "default_restart")]
    pub restart:              RestartPolicy,
    #[serde(default = "five_seconds", rename = "restart-sec",
            deserialize_with = "deserialize_duration")]
    pub restart_sec:          Duration,
    #[serde(default = "thirty_seconds", rename = "timeout-start-sec",
            deserialize_with = "deserialize_duration")]
    pub timeout_start_sec:    Duration,
    #[serde(default = "ten_seconds", rename = "timeout-stop-sec",
            deserialize_with = "deserialize_duration")]
    pub timeout_stop_sec:     Duration,
    #[serde(default = "root")]
    pub user:                 String,
    #[serde(default = "root")]
    pub group:                String,
    #[serde(default = "system_slice")]
    pub slice:                String,
    #[serde(default, rename = "remain-after-exit")]
    pub remain_after_exit:    bool,
    #[serde(default)]
    pub environment:          Vec<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct InstallSection {
    #[serde(default, rename = "wanted-by")]
    pub wanted_by:   Vec<String>,
    #[serde(default, rename = "required-by")]
    pub required_by: Vec<String>,
}

#[derive(Debug, Deserialize, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum ServiceType { Simple, Forking, Oneshot, Notify }

#[derive(Debug, Deserialize, Clone, Copy)]
#[serde(rename_all = "kebab-case")]
pub enum RestartPolicy { No, Always, OnFailure, OnAbnormal }
```

Note the kebab-case `rename` attributes: TOML keys are `restart-sec`,
not `restart_sec`. This keeps the file format human-friendly while the
Rust types stay snake-case.

## Worked example: `multi-user.target` + `getty@tty1.service`

### `/etc/writeonce/services/multi-user.target.toml`

```toml
[unit]
description = "Multi-user runtime"
requires    = ["basic.target"]
after       = ["basic.target"]
conflicts   = ["rescue.target"]
```

No `[service]` section (targets are pure synchronization points).

### `/etc/writeonce/services/getty@tty1.service.toml`

```toml
[unit]
description = "Login on tty1"
after       = ["systemd-user-sessions.service"]
conflicts   = ["rescue.service"]

[service]
type        = "simple"
exec-start  = "/usr/sbin/agetty --noclear tty1 linux"
restart     = "always"
restart-sec = "1s"
user        = "root"

[install]
wanted-by   = ["multi-user.target"]
```

### What the supervisor sees at load time

1. Parse both files into `UnitFile` structs.
2. Walk `getty@tty1.service.install.wanted_by` →
   add an implicit `wants` entry to the in-memory
   `multi-user.target.unit.wants` vector pointing at `getty@tty1.service`.
3. When `wo-ctl start multi-user.target` runs, the dependency-graph
   build (see [`phase-4-dependency-graph.md`](phase-4-dependency-graph.md))
   walks the closure: `multi-user.target` → `basic.target` (via
   `requires`) and `getty@tty1.service` (via the implicit `wants`
   from step 2). Both get queued.
4. The ordering edges (`after = ["basic.target"]` on
   `multi-user.target`; `after = ["systemd-user-sessions.service"]` on
   `getty@tty1.service`) order the topological sort.

## What's not in the schema

Listed for transparency:

- `OnFailure=` / `OnSuccess=` callback chains — deferred to a fault-event API.
- `ConditionPath*=` / `AssertPath*=` — adds a conditional layer not yet needed.
- `ExecStartPre=` / `ExecStartPost=` — modeled as separate `oneshot`
  units with `before = [main]`.
- `Sandboxing` (PrivateTmp/ProtectSystem/CapabilityBoundingSet/…) —
  reaches via cgroup limits + namespaces later, not via unit-file
  directives in v1.
- Socket / timer / path / mount unit types — out of scope.
- Drop-in directories (`<unit>.d/`) — useful but adds a parse layer
  we skip until needed.
