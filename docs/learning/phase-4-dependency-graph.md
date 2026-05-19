# Phase 4 — Dependency graph and job-transaction algorithm

> Design companion to [`plan/phase-4-supervisor.md`](../../plan/phase-4-supervisor.md).
> Specifies how the WriteOnce supervisor turns a collection of
> [`service.toml`](phase-4-service-toml-schema.md) files into an ordered
> startup plan.
>
> systemd analogue: `src/core/transaction.c`, summarised in
> [`systemd-feature-survey.md`](systemd-feature-survey.md#4-dependency-resolution--srccoretransactionc).

## The problem

Given:

- A registry of loaded units (parsed `UnitFile`s, keyed by name).
- A request — `start multi-user.target` — naming an *anchor* unit.

Produce:

- An ordered list of `Job`s (each is a `(unit, kind)` pair) that, when
  executed in order, satisfies the anchor's requirements without
  violating any ordering, requirement, binding, or conflict rule.
- Or, an error: cycle in requirements / conflict that cannot be
  resolved.

## Edge types

| Edge       | Source directives                | Semantic                                                                              |
| ---------- | -------------------------------- | ------------------------------------------------------------------------------------- |
| Requirement | `Requires=` / `Wants=`            | If A has edge to B: B is pulled into the transaction. `Requires` is hard; `Wants` is soft (B's failure does not fail A). |
| Ordering   | `After=` / `Before=`             | If A `After` B: in the transaction, B must reach `active` before A starts.            |
| Binding    | `BindsTo=` / `PartOf=`           | `BindsTo` is bidirectional ("if either of us stops, both stop"); `PartOf` is unidirectional. |
| Conflict   | `Conflicts=`                     | A and B cannot be `active` simultaneously; activating one cancels the other.          |

Reverse-dependency directives (`WantedBy=`, `RequiredBy=`) are
*resolved at unit-load time* into the same in-memory edges — see
[`phase-4-service-toml-schema.md`](phase-4-service-toml-schema.md#the-install-section-reverse-dependencies).

## Rust types

```rust
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UnitId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeType { Requires, Wants, BindsTo, PartOf, After, Before, Conflicts }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobKind  { Start, Stop, Restart, Reload }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobState { Waiting, Running, Finished(JobResult) }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobResult { Done, Failed, Dependency, Timeout, Canceled }

pub struct Job {
    pub unit:  UnitId,
    pub kind:  JobKind,
    pub state: JobState,
}

pub struct DependencyGraph {
    pub units: HashMap<UnitId, Unit>,
    pub edges: Vec<(UnitId, UnitId, EdgeType)>,
}

pub enum TransactionError {
    RequirementCycle(Vec<UnitId>),
    Conflict { winner: UnitId, loser: UnitId },
    UnknownUnit(String),
}
```

## The algorithm

```text
build_transaction(graph, anchor: UnitId) -> Result<Vec<Job>, TransactionError>:

    # ---- 1. Compute the unit set ----
    unit_set := {anchor}
    pending  := [anchor]
    while pending nonempty:
        u := pop pending
        for each edge (u, v, kind) in graph.edges:
            match kind:
                Requires | Wants | BindsTo | PartOf:
                    if v not in unit_set:
                        unit_set.add(v)
                        pending.push(v)
                Conflicts:
                    # conflict goes into a separate set, handled in step 3
                Ordering edges (After/Before):
                    # not pulling — handled in step 4
                _: continue

    # ---- 2. Create JOB_START for each unit ----
    jobs := {u -> Job{u, kind=Start, state=Waiting} for u in unit_set}

    # ---- 3. Conflict resolution ----
    for each (a, b, Conflicts) where a in unit_set and b in unit_set:
        # the unit with lower job-priority gets canceled
        # (priority: anchor > Requires-chain > Wants-chain)
        if priority(a) > priority(b):
            jobs.remove(b)
            unit_set.remove(b)
        elif priority(b) > priority(a):
            jobs.remove(a)
            unit_set.remove(a)
        else:
            return Err(Conflict { winner: a, loser: b })

    # ---- 4. Build ordering DAG over the (remaining) unit set ----
    order_edges := []
    for each (a, b, After) in graph.edges where a, b in unit_set:
        order_edges.push(b -> a)    # b must finish before a starts
    for each (a, b, Before) in graph.edges where a, b in unit_set:
        order_edges.push(a -> b)

    # ---- 5. Cycle detection ----
    if find_cycle(unit_set, graph.requirement_edges) returns C:
        return Err(RequirementCycle(C))

    if find_cycle(unit_set, order_edges) returns C:
        # break the weakest edge in the cycle (lowest priority Wants edge)
        # and emit a log warning; do not abort.
        break_weakest_edge(C)

    # ---- 6. Topological sort ----
    ordered := topo_sort(unit_set, order_edges)

    # ---- 7. Map back to Vec<Job> in topo order ----
    return Ok(ordered.iter().map(|u| jobs[u]).collect())
```

## Why two cycle classes are treated differently

systemd makes a deliberate distinction:

- **Requirement cycles** (`Requires=`/`Wants=`) — *unresolvable*. If A requires B and B requires A, neither can be the "first" to come up. Reject the transaction with an error.
- **Ordering cycles** (`After=`/`Before=`) — *resolvable by breaking*. If A says `After=B` and B says `After=A`, both orderings cannot be satisfied; but starting them in *any* order is at least correct in a "we tried" sense. Log a warning, break the weakest edge (lowest-priority `Wants`-originated ordering), proceed.

WriteOnce adopts the same split. Worked-example tests in
`crates/writeonce-svc/tests/` will cover each.

## Worked example: bringing up `graphical.target`

### The relevant units

| Unit                       | `[Unit]` directives                                             |
| -------------------------- | --------------------------------------------------------------- |
| `graphical.target`         | `requires = ["multi-user.target"]` `after = ["multi-user.target"]` |
| `multi-user.target`        | `requires = ["basic.target"]` `after = ["basic.target"]`        |
| `basic.target`             | (empty)                                                         |
| `dbus.service`             | `[install] wanted-by = ["multi-user.target"]`                   |
| `xorg.service`             | `requires = ["dbus.service"]` `after = ["dbus.service"]` `[install] wanted-by = ["graphical.target"]` |

### Step 1 — load-time install resolution

- `dbus.service.install.wanted-by = ["multi-user.target"]` →
  implicit edge `(multi-user.target, dbus.service, Wants)`.
- `xorg.service.install.wanted-by = ["graphical.target"]` →
  implicit edge `(graphical.target, xorg.service, Wants)`.

### Step 2 — transaction build for anchor `graphical.target`

Closure:

```
{graphical.target}
  + multi-user.target           (via Requires)
  + dbus.service                (via Wants from install resolution)
  + xorg.service                (via Wants from install resolution)
  + basic.target                (via Requires from multi-user.target)
```

`unit_set = {graphical.target, multi-user.target, basic.target, dbus.service, xorg.service}`.

### Step 3 — conflict resolution

None.

### Step 4 — ordering DAG (`a → b` means a must finish before b starts)

From `After=`:
- `multi-user.target → graphical.target`
- `basic.target → multi-user.target`
- `dbus.service → xorg.service`

### Step 5 — cycles

None.

### Step 6 — topological sort

One valid linearisation:

```
basic.target
multi-user.target
dbus.service
xorg.service
graphical.target
```

### Step 7 — emitted job list

```
Job{basic.target,      Start, Waiting}
Job{multi-user.target, Start, Waiting}
Job{dbus.service,      Start, Waiting}
Job{xorg.service,      Start, Waiting}
Job{graphical.target,  Start, Waiting}
```

The runtime then executes them in order, transitioning each `Waiting →
Running → Finished(Done)` as the underlying service activates.
`Job.state` is what gets reported by `wo-ctl status`.

## Stop transactions

Symmetric: `stop graphical.target` walks the *reverse* closure (units
that have edges *to* the anchor) and emits `Stop` jobs in *reverse*
topological order. The same algorithm with the edge direction flipped
and the kind set to `Stop`.

## Complexity

- Closure construction: O(V + E) (BFS).
- Cycle detection: O(V + E) (Tarjan / DFS).
- Topological sort: O(V + E) (Kahn).

For WriteOnce's expected scale (a few dozen units), the whole transaction
build is sub-millisecond. We don't index for incremental updates; the
graph is small enough to rebuild from scratch on each `wo-ctl` request.

## What this design omits

- **Job merging** — systemd merges duplicate jobs targeting the same
  unit (`transaction_merge_and_delete_job()`). WriteOnce's
  job-per-unit invariant comes for free from the `HashMap<UnitId, Job>`
  in step 2 (a unit can only have one job; later requests update its
  kind), so the merging logic is implicit rather than explicit.
- **Job inversion / replacement** — systemd's interactive
  `--ignore-dependencies` and "replace" semantics for the same unit
  with a different kind. Out of scope for v1.
- **Watchdog timers on jobs** — `JobTimeoutSec=` will be added but the
  algorithm above does not yet enforce it.
- **D-Bus job introspection** — `wo-ctl` reads the in-memory job list
  directly via the Unix-socket control plane; no D-Bus equivalent of
  `org.freedesktop.systemd1.Job` for now.
