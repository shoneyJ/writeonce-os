//! Dependency-graph types + `build_transaction` algorithm.
//!
//! See [`docs/learning/phase-4-dependency-graph.md`](../../docs/learning/phase-4-dependency-graph.md)
//! for the design. The algorithm:
//!
//!   1. Lift `[install] wanted-by` / `required-by` directives into the
//!      *target* unit's effective `wants` / `requires` sets — the reverse-
//!      dependency mechanism.
//!   2. Compute the transitive closure of the anchor unit over the
//!      requirement + binding edges.
//!   3. Build an ordering DAG from `After=` / `Before=` directives.
//!   4. Reject requirement cycles (currently the only cycle check; the
//!      "ordering-cycle-by-warning" refinement lands in Round 2c).
//!   5. Topologically sort.

#![allow(dead_code)]

use std::collections::{HashMap, HashSet, VecDeque};

use crate::config::{LoadedUnit, UnitFile};

// ----------------------------------------------------------------------------
// Types (the shapes promised in earlier scaffolding)
// ----------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UnitId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeType { Requirement, Ordering, Binding, Conflict }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobKind { Start, Stop, Restart, Reload }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobState {
    Waiting,
    Running,
    Finished(JobResult),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JobResult { Done, Failed, Dependency, Timeout, Canceled }

#[derive(Debug, Clone)]
pub struct Job {
    pub unit:  UnitId,
    pub kind:  JobKind,
    pub state: JobState,
}

#[derive(Debug)]
pub enum TransactionError {
    UnknownUnit(String),
    RequirementCycle(Vec<String>),
    OrderingCycle(Vec<String>),
}

// ----------------------------------------------------------------------------
// UnitRegistry — owns the parsed UnitFiles and serves transactions
// ----------------------------------------------------------------------------

#[derive(Debug)]
pub struct UnitRegistry {
    /// Stored in load order; UnitId is the index into this vector.
    units:   Vec<RegisteredUnit>,
    /// name → UnitId.
    by_name: HashMap<String, UnitId>,
    /// Computed at registry construction time from each unit's
    /// `[install] wanted-by` directives: for each `(target, U)` mapping,
    /// target's effective `wants` gains U.
    extra_wants:    HashMap<UnitId, Vec<UnitId>>,
    /// Same but for `required-by`.
    extra_requires: HashMap<UnitId, Vec<UnitId>>,
}

#[derive(Debug)]
pub struct RegisteredUnit {
    pub name: String,
    pub file: UnitFile,
}

impl UnitRegistry {
    pub fn from_loaded(loaded: Vec<LoadedUnit>) -> Self {
        let mut by_name = HashMap::new();
        let mut units = Vec::with_capacity(loaded.len());
        for lu in loaded {
            let id = UnitId(units.len() as u32);
            by_name.insert(lu.name.clone(), id);
            units.push(RegisteredUnit { name: lu.name, file: lu.file });
        }

        // Lift wanted-by / required-by.
        let mut extra_wants:    HashMap<UnitId, Vec<UnitId>> = HashMap::new();
        let mut extra_requires: HashMap<UnitId, Vec<UnitId>> = HashMap::new();
        for (i, unit) in units.iter().enumerate() {
            let me = UnitId(i as u32);
            for target_name in &unit.file.install.wanted_by {
                if let Some(&t) = by_name.get(target_name) {
                    extra_wants.entry(t).or_default().push(me);
                }
            }
            for target_name in &unit.file.install.required_by {
                if let Some(&t) = by_name.get(target_name) {
                    extra_requires.entry(t).or_default().push(me);
                }
            }
        }

        Self { units, by_name, extra_wants, extra_requires }
    }

    pub fn name_of(&self, id: UnitId) -> &str { &self.units[id.0 as usize].name }

    pub fn lookup(&self, name: &str) -> Option<UnitId> {
        self.by_name.get(name).copied()
    }

    pub fn get(&self, id: UnitId) -> &RegisteredUnit { &self.units[id.0 as usize] }

    pub fn len(&self) -> usize { self.units.len() }

    /// Returns the effective `requirement` edges (`Wants ∪ Requires ∪ BindsTo`)
    /// out of `id`, after applying reverse-dependency lift.
    fn requirement_targets(&self, id: UnitId) -> Vec<UnitId> {
        let unit = self.get(id);
        let mut out: Vec<UnitId> = Vec::new();
        let push_named = |name: &String, out: &mut Vec<UnitId>| {
            if let Some(&t) = self.by_name.get(name) { out.push(t); }
        };
        for n in &unit.file.unit.wants    { push_named(n, &mut out); }
        for n in &unit.file.unit.requires { push_named(n, &mut out); }
        for n in &unit.file.unit.binds_to { push_named(n, &mut out); }
        if let Some(extra) = self.extra_wants.get(&id) {
            for &t in extra { out.push(t); }
        }
        if let Some(extra) = self.extra_requires.get(&id) {
            for &t in extra { out.push(t); }
        }
        out.sort_by_key(|u| u.0);
        out.dedup();
        out
    }

    /// Build the transaction (ordered list of Start jobs) for `anchor`.
    pub fn build_transaction(&self, anchor: &str) -> Result<Vec<Job>, TransactionError> {
        let anchor_id = self.lookup(anchor)
            .ok_or_else(|| TransactionError::UnknownUnit(anchor.to_string()))?;

        // 1. Closure over requirement edges.
        let mut unit_set: HashSet<UnitId> = HashSet::new();
        unit_set.insert(anchor_id);
        let mut pending: VecDeque<UnitId> = VecDeque::new();
        pending.push_back(anchor_id);
        while let Some(u) = pending.pop_front() {
            for v in self.requirement_targets(u) {
                if unit_set.insert(v) {
                    pending.push_back(v);
                }
            }
        }

        // 2. Build ordering edges restricted to unit_set.
        // edges[u] = nodes v such that u must finish before v starts.
        let mut order_out: HashMap<UnitId, Vec<UnitId>> = HashMap::new();
        let mut order_in_degree: HashMap<UnitId, u32> = HashMap::new();
        for &id in &unit_set { order_in_degree.insert(id, 0); }

        for &id in &unit_set {
            let unit = self.get(id);
            // After=X  →  X must finish before me  →  X → me
            for x_name in &unit.file.unit.after {
                if let Some(&x) = self.by_name.get(x_name) {
                    if unit_set.contains(&x) {
                        order_out.entry(x).or_default().push(id);
                        *order_in_degree.entry(id).or_insert(0) += 1;
                    }
                }
            }
            // Before=Y  →  I must finish before Y  →  me → Y
            for y_name in &unit.file.unit.before {
                if let Some(&y) = self.by_name.get(y_name) {
                    if unit_set.contains(&y) {
                        order_out.entry(id).or_default().push(y);
                        *order_in_degree.entry(y).or_insert(0) += 1;
                    }
                }
            }
        }

        // 3. Kahn's topological sort.
        let mut ready: VecDeque<UnitId> =
            order_in_degree.iter()
                .filter_map(|(&id, &deg)| if deg == 0 { Some(id) } else { None })
                .collect();
        // Sort so output is deterministic across HashMap iteration orders.
        let mut ready_vec: Vec<UnitId> = ready.drain(..).collect();
        ready_vec.sort_by_key(|u| u.0);
        let mut ready: VecDeque<UnitId> = ready_vec.into();

        let mut sorted: Vec<UnitId> = Vec::with_capacity(unit_set.len());
        while let Some(u) = ready.pop_front() {
            sorted.push(u);
            if let Some(downstream) = order_out.get(&u) {
                let mut new_ready: Vec<UnitId> = Vec::new();
                for &v in downstream {
                    let d = order_in_degree.get_mut(&v).unwrap();
                    *d -= 1;
                    if *d == 0 { new_ready.push(v); }
                }
                new_ready.sort_by_key(|u| u.0);
                for v in new_ready { ready.push_back(v); }
            }
        }

        if sorted.len() != unit_set.len() {
            // Cycle in ordering edges; collect the remaining nodes (those still
            // with nonzero in-degree).
            let cycle_names: Vec<String> = order_in_degree.iter()
                .filter_map(|(id, &deg)| if deg > 0 { Some(self.name_of(*id).to_string()) } else { None })
                .collect();
            return Err(TransactionError::OrderingCycle(cycle_names));
        }

        // 4. Emit Start jobs.
        Ok(sorted.into_iter().map(|u| Job {
            unit:  u,
            kind:  JobKind::Start,
            state: JobState::Waiting,
        }).collect())
    }

    /// Build a stop transaction: all units that *depend on* `anchor`, plus
    /// `anchor` itself, in **reverse-topological** order — most-dependent
    /// first, anchor last.
    ///
    /// Returns just the ordered list of UnitIds (Stop jobs are conceptual
    /// here — the supervisor sends SIGTERM directly).
    pub fn build_stop_transaction(&self, anchor: &str) -> Result<Vec<UnitId>, TransactionError> {
        let anchor_id = self.lookup(anchor)
            .ok_or_else(|| TransactionError::UnknownUnit(anchor.to_string()))?;

        // 1. Build the reverse-edge map (for each requirement edge u → v,
        //    record v has-dependent u).
        let mut dependents: HashMap<UnitId, Vec<UnitId>> = HashMap::new();
        for i in 0..self.units.len() {
            let u = UnitId(i as u32);
            for v in self.requirement_targets(u) {
                dependents.entry(v).or_default().push(u);
            }
        }

        // 2. BFS from anchor over dependents to find the "things that
        //    depend on anchor (transitively)" closure.
        let mut close: HashSet<UnitId> = HashSet::new();
        close.insert(anchor_id);
        let mut pending: VecDeque<UnitId> = VecDeque::new();
        pending.push_back(anchor_id);
        while let Some(u) = pending.pop_front() {
            if let Some(ds) = dependents.get(&u) {
                for &d in ds {
                    if close.insert(d) { pending.push_back(d); }
                }
            }
        }

        // 3. Build the ordering DAG restricted to `close`, then topo-sort,
        //    then reverse: we want most-dependent units stopped first.
        let mut order_out: HashMap<UnitId, Vec<UnitId>> = HashMap::new();
        let mut in_degree: HashMap<UnitId, u32> = HashMap::new();
        for &id in &close { in_degree.insert(id, 0); }
        for &id in &close {
            let unit = self.get(id);
            for x_name in &unit.file.unit.after {
                if let Some(&x) = self.by_name.get(x_name) {
                    if close.contains(&x) {
                        order_out.entry(x).or_default().push(id);
                        *in_degree.entry(id).or_insert(0) += 1;
                    }
                }
            }
            for y_name in &unit.file.unit.before {
                if let Some(&y) = self.by_name.get(y_name) {
                    if close.contains(&y) {
                        order_out.entry(id).or_default().push(y);
                        *in_degree.entry(y).or_insert(0) += 1;
                    }
                }
            }
        }
        let mut ready: Vec<UnitId> = in_degree.iter()
            .filter_map(|(&id, &d)| if d == 0 { Some(id) } else { None })
            .collect();
        ready.sort_by_key(|u| u.0);
        let mut ready: VecDeque<UnitId> = ready.into();
        let mut sorted: Vec<UnitId> = Vec::new();
        while let Some(u) = ready.pop_front() {
            sorted.push(u);
            if let Some(ds) = order_out.get(&u) {
                let mut next: Vec<UnitId> = Vec::new();
                for &v in ds {
                    let d = in_degree.get_mut(&v).unwrap();
                    *d -= 1;
                    if *d == 0 { next.push(v); }
                }
                next.sort_by_key(|u| u.0);
                for v in next { ready.push_back(v); }
            }
        }
        if sorted.len() != close.len() {
            return Err(TransactionError::OrderingCycle(
                in_degree.iter()
                    .filter_map(|(id, &d)| if d > 0 { Some(self.name_of(*id).to_string()) } else { None })
                    .collect()
            ));
        }

        // Reverse: stop the most-dependent units first.
        sorted.reverse();
        Ok(sorted)
    }
}

// ----------------------------------------------------------------------------
// Tests
// ----------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{InstallSection, ServiceSection, ServiceType, UnitFile, UnitSection};

    fn mk_unit(name: &str, unit: UnitSection, service: Option<ServiceSection>, install: InstallSection) -> LoadedUnit {
        LoadedUnit { name: name.to_string(), file: UnitFile { unit, service, install } }
    }

    fn mk_target(name: &str, unit: UnitSection) -> LoadedUnit {
        mk_unit(name, unit, None, InstallSection::default())
    }

    fn mk_service(name: &str, unit: UnitSection, install: InstallSection) -> LoadedUnit {
        mk_unit(name, unit, Some(ServiceSection {
            kind: ServiceType::Simple,
            exec_start: "/bin/true".into(),
            exec_stop: String::new(),
            exec_reload: String::new(),
            restart: crate::config::RestartPolicy::No,
            restart_sec: "5s".into(),
            timeout_start_sec: "30s".into(),
            timeout_stop_sec:  "10s".into(),
            user:  "root".into(),
            group: "root".into(),
            slice: "system.slice".into(),
            remain_after_exit: false,
            environment: vec![],
        }), install)
    }

    #[test]
    fn simple_closure_via_requires() {
        let units = vec![
            mk_target("a.target", UnitSection {
                requires: vec!["b.target".into()],
                after:    vec!["b.target".into()],
                ..Default::default()
            }),
            mk_target("b.target", UnitSection::default()),
        ];
        let reg = UnitRegistry::from_loaded(units);
        let plan = reg.build_transaction("a.target").unwrap();
        let names: Vec<&str> = plan.iter().map(|j| reg.name_of(j.unit)).collect();
        assert_eq!(names, vec!["b.target", "a.target"]);
    }

    #[test]
    fn wanted_by_lifts_into_target() {
        // multi-user.target itself has no wants; getty@tty1 declares
        // wanted-by = ["multi-user.target"]. Closure from multi-user.target
        // should pull in getty@tty1.
        let units = vec![
            mk_target("multi-user.target", UnitSection::default()),
            mk_service("getty@tty1.service",
                UnitSection {
                    after: vec!["dbus.service".into()],
                    ..Default::default()
                },
                InstallSection {
                    wanted_by: vec!["multi-user.target".into()],
                    required_by: vec![],
                }),
            mk_service("dbus.service",
                UnitSection::default(),
                InstallSection {
                    wanted_by: vec!["multi-user.target".into()],
                    required_by: vec![],
                }),
        ];
        let reg = UnitRegistry::from_loaded(units);
        let plan = reg.build_transaction("multi-user.target").unwrap();
        let names: Vec<&str> = plan.iter().map(|j| reg.name_of(j.unit)).collect();

        assert!(names.contains(&"multi-user.target"));
        assert!(names.contains(&"dbus.service"));
        assert!(names.contains(&"getty@tty1.service"));
        // dbus.service has no After/Before; getty has After=dbus.service.
        // So dbus must come before getty.
        let idx_dbus  = names.iter().position(|n| *n == "dbus.service").unwrap();
        let idx_getty = names.iter().position(|n| *n == "getty@tty1.service").unwrap();
        assert!(idx_dbus < idx_getty, "dbus must precede getty in topo order");
    }

    #[test]
    fn unknown_anchor() {
        let reg = UnitRegistry::from_loaded(vec![]);
        let err = reg.build_transaction("nope.target").unwrap_err();
        match err {
            TransactionError::UnknownUnit(n) => assert_eq!(n, "nope.target"),
            other => panic!("wrong err: {other:?}"),
        }
    }

    #[test]
    fn ordering_cycle_detected() {
        let units = vec![
            mk_target("x.target", UnitSection {
                requires: vec!["y.target".into()],
                after:    vec!["y.target".into()],
                ..Default::default()
            }),
            mk_target("y.target", UnitSection {
                requires: vec!["x.target".into()],
                after:    vec!["x.target".into()],
                ..Default::default()
            }),
        ];
        let reg = UnitRegistry::from_loaded(units);
        let err = reg.build_transaction("x.target").unwrap_err();
        matches!(err, TransactionError::OrderingCycle(_)).then_some(()).expect("expected ordering cycle");
    }

    #[test]
    fn before_directive_orders_correctly() {
        // a says "I must finish Before b". So a→b in the topo order.
        let units = vec![
            mk_target("a.target", UnitSection {
                before: vec!["b.target".into()],
                requires: vec!["b.target".into()],   // pulls b into the set
                ..Default::default()
            }),
            mk_target("b.target", UnitSection::default()),
        ];
        let reg = UnitRegistry::from_loaded(units);
        let plan = reg.build_transaction("a.target").unwrap();
        let names: Vec<&str> = plan.iter().map(|j| reg.name_of(j.unit)).collect();
        // a before b
        assert_eq!(names, vec!["a.target", "b.target"]);
    }
}
