//! Supervisor state machine — owns unit metadata, the PID→unit map,
//! restart bookkeeping, and the planned-shutdown flag.
//!
//! Round-2c scope additions over Round 2b:
//!   - Restart policies (`Always` / `OnFailure` / `OnAbnormal`) with
//!     `RestartSec` delay, scheduled via `pending_restarts` and driven by
//!     the event loop's dynamic timeout.
//!   - `shutting_down` flag suppresses restart scheduling during planned
//!     shutdown — clean exits during shutdown stop being marked `Failed`.
//!   - `stop_unit(name)` — synchronous reverse-topo stop transaction.

#![allow(dead_code)]

use std::collections::{HashMap, HashSet};
use std::io;
use std::time::{Duration, Instant};

use crate::config::{LoadedUnit, RestartPolicy, ServiceType, parse_duration};
use crate::graph::{Job, JobState, UnitId, UnitRegistry};
use crate::spawn::{spawn, SpawnRequest};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnitActiveState {
    Inactive,
    Activating,
    Active,
    Deactivating,
    Failed,
}

#[derive(Debug)]
pub struct UnitMeta {
    pub id:    UnitId,
    pub state: UnitActiveState,
    pub pid:   Option<libc::pid_t>,
    /// Timestamps of restart attempts inside the current
    /// `start_limit_interval_sec` window. Pruned on every consideration
    /// in `maybe_schedule_restart`; once `len() >= start_limit_burst`
    /// scheduling is suppressed and the unit stays Failed.
    pub restart_history: Vec<Instant>,
}

#[derive(Debug)]
struct PendingRestart {
    unit:    UnitId,
    fire_at: Instant,
}

#[derive(Debug)]
pub struct SupervisorState {
    pub registry: UnitRegistry,
    units:        Vec<UnitMeta>,
    by_pid:       HashMap<libc::pid_t, UnitId>,
    pending_restarts: Vec<PendingRestart>,
    /// When set, do not schedule new restarts; treat all clean exits as
    /// `Inactive` instead of `Failed`.
    pub shutting_down: bool,
    /// If true: plain `fork(2)`, no cgroup placement. Set at construction.
    pub fake: bool,
    /// Path to `/etc/writeonce/enabled.d/` for the lifetime of this
    /// supervisor — used by `wo-ctl enable / disable` over the control
    /// socket. Defaults to [`crate::enabled::DEFAULT_DIR`].
    pub enabled_d: String,
    /// Directory holding per-service log files (`<log_dir>/<unit>.log`),
    /// where each service's stdout/stderr is captured. Defaults to
    /// [`DEFAULT_LOG_DIR`].
    pub log_dir: String,
}

/// Default per-service log directory.
pub const DEFAULT_LOG_DIR: &str = "/var/log/writeonce";

/// Result of waiting for a `Type=oneshot` child.
enum OneshotOutcome {
    /// Exited 0.
    Clean,
    /// Exited non-zero or was signaled.
    Failed,
    /// Did not exit within `timeout_start_sec`; SIGKILLed and reaped.
    TimedOut,
}

/// Wait for a `Type=oneshot` child to exit, bounded by `timeout`. Polls with
/// `WNOHANG` (SIGCHLD is blocked via signalfd, so we drive the wait directly)
/// so a hung oneshot can't freeze activation before the event loop starts. On
/// timeout the child is SIGKILLed and reaped to avoid a zombie.
fn wait_oneshot(pid: libc::pid_t, timeout: Duration) -> OneshotOutcome {
    let deadline = Instant::now() + timeout;
    loop {
        let mut status: libc::c_int = 0;
        let rc = unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) };
        if rc == pid {
            let clean = libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0;
            return if clean { OneshotOutcome::Clean } else { OneshotOutcome::Failed };
        }
        if rc < 0 {
            // ECHILD or similar — nothing to wait for; treat as failed.
            return OneshotOutcome::Failed;
        }
        // rc == 0: still running.
        if Instant::now() >= deadline {
            unsafe { libc::kill(pid, libc::SIGKILL) };
            let mut status: libc::c_int = 0;
            unsafe { libc::waitpid(pid, &mut status, 0) };
            return OneshotOutcome::TimedOut;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

impl SupervisorState {
    pub fn new(registry: UnitRegistry, fake: bool) -> Self {
        let units: Vec<UnitMeta> = (0..registry.len())
            .map(|i| UnitMeta {
                id:    UnitId(i as u32),
                state: UnitActiveState::Inactive,
                pid:   None,
                restart_history: Vec::new(),
            })
            .collect();
        Self {
            registry,
            units,
            by_pid:           HashMap::new(),
            pending_restarts: Vec::new(),
            shutting_down:    false,
            fake,
            enabled_d:        crate::enabled::DEFAULT_DIR.to_string(),
            log_dir:          DEFAULT_LOG_DIR.to_string(),
        }
    }

    pub fn from_loaded(loaded: Vec<LoadedUnit>, fake: bool) -> Self {
        Self::new(UnitRegistry::from_loaded(loaded), fake)
    }

    pub fn unit(&self, id: UnitId) -> &UnitMeta { &self.units[id.0 as usize] }
    pub fn unit_mut(&mut self, id: UnitId) -> &mut UnitMeta { &mut self.units[id.0 as usize] }

    pub fn iter_loaded(&self) -> impl Iterator<Item = (UnitId, &str)> + '_ {
        (0..self.registry.len()).map(move |i| {
            let id = UnitId(i as u32);
            (id, self.registry.name_of(id))
        })
    }

    /// Run the activation plan in topological order.
    pub fn activate_plan(&mut self, plan: &[Job]) -> io::Result<()> {
        for job in plan {
            if job.state != JobState::Waiting { continue; }
            self.start_unit_internal(job.unit)?;
        }
        Ok(())
    }

    fn start_unit_internal(&mut self, id: UnitId) -> io::Result<()> {
        // Don't start a unit whose HARD dependency (Requires=/BindsTo=) has
        // already Failed — mark it Failed and skip, so one root failure is
        // legible instead of cascading into a storm of dependents that crash
        // immediately. Soft Wants= failures are tolerated (not checked here).
        let failed_dep = self.registry.hard_requirement_targets(id)
            .into_iter()
            .find(|&dep| self.units[dep.0 as usize].state == UnitActiveState::Failed);
        if let Some(dep) = failed_dep {
            self.units[id.0 as usize].state = UnitActiveState::Failed;
            eprintln!("writeonce-svc: {} not started — required {} is Failed",
                      self.registry.name_of(id), self.registry.name_of(dep));
            return Ok(());
        }

        // Targets are pure synchronisation points; no process.
        let unit = self.registry.get(id);
        match &unit.file.service {
            None => {
                self.units[id.0 as usize].state = UnitActiveState::Active;
                println!("writeonce-svc: target  {} active", &unit.name);
                Ok(())
            }
            Some(service) => {
                let req = SpawnRequest { name: &unit.name, service, fake: self.fake, log_dir: &self.log_dir };
                self.units[id.0 as usize].state = UnitActiveState::Activating;
                let pid = match spawn(&req) {
                    Ok(p) => p,
                    Err(e) => {
                        eprintln!("writeonce-svc: FAILED to spawn {}: {}", &unit.name, e);
                        self.units[id.0 as usize].state = UnitActiveState::Failed;
                        return Ok(()); // non-fatal to other units
                    }
                };
                self.units[id.0 as usize].pid = Some(pid);
                self.by_pid.insert(pid, id);

                match service.kind {
                    ServiceType::Oneshot => {
                        // Bounded wait: this runs during activate_plan, BEFORE
                        // the event loop starts, so a hung oneshot (e.g.
                        // writeonce-bootstrap) must not block forever. Poll
                        // with WNOHANG up to timeout_start_sec, then SIGKILL.
                        let timeout = parse_duration(&service.timeout_start_sec)
                            .unwrap_or_else(|| Duration::from_secs(30));
                        let outcome = wait_oneshot(pid, timeout);
                        let final_state = match outcome {
                            OneshotOutcome::Clean if service.remain_after_exit =>
                                UnitActiveState::Active,
                            OneshotOutcome::Clean => UnitActiveState::Inactive,
                            OneshotOutcome::Failed | OneshotOutcome::TimedOut =>
                                UnitActiveState::Failed,
                        };
                        self.units[id.0 as usize].state = final_state;
                        self.units[id.0 as usize].pid = None;
                        self.by_pid.remove(&pid);
                        match outcome {
                            OneshotOutcome::TimedOut => eprintln!(
                                "writeonce-svc: oneshot {} exceeded timeout-start-sec ({:?}); \
                                 killed → Failed", &unit.name, timeout),
                            OneshotOutcome::Failed => eprintln!(
                                "writeonce-svc: oneshot {} exited non-zero → Failed", &unit.name),
                            OneshotOutcome::Clean => println!(
                                "writeonce-svc: oneshot {} → {:?}", &unit.name, final_state),
                        }
                    }
                    _ => {
                        self.units[id.0 as usize].state = UnitActiveState::Active;
                        println!("writeonce-svc: service {} active (pid={})", &unit.name, pid);
                    }
                }
                Ok(())
            }
        }
    }

    /// Called by the event loop when SIGCHLD reaping reports `pid` exited.
    /// Schedules a restart according to the unit's `RestartPolicy`, unless
    /// we're in planned shutdown.
    pub fn on_child_exit(&mut self, pid: libc::pid_t, status: libc::c_int) {
        let Some(&unit_id) = self.by_pid.get(&pid) else {
            // An orphaned grandchild (a service that forked before exec, or a
            // double-forker reparented to us). Reaping it is correct; surface
            // it so unexpected exits are auditable during boot debugging.
            eprintln!("writeonce-svc: reaped untracked child pid={pid} status={status}");
            return;
        };
        self.by_pid.remove(&pid);
        self.units[unit_id.0 as usize].pid = None;

        let exited_clean = libc::WIFEXITED(status) && libc::WEXITSTATUS(status) == 0;
        let signaled     = libc::WIFSIGNALED(status);

        // During planned shutdown, every exit is "as expected" — treat as Inactive.
        let new_state = if self.shutting_down {
            UnitActiveState::Inactive
        } else if exited_clean {
            UnitActiveState::Inactive
        } else {
            UnitActiveState::Failed
        };
        self.units[unit_id.0 as usize].state = new_state;
        let name = self.registry.name_of(unit_id).to_string();
        println!("writeonce-svc: {} {:?} (pid={} status={})",
                 &name, new_state, pid, status);

        // Schedule a restart if policy says so (and we're not shutting down).
        if !self.shutting_down {
            self.maybe_schedule_restart(unit_id, exited_clean, signaled);
        }

        // If the unit is terminal (won't restart) tear down its cgroup so we
        // don't leak /sys/fs/cgroup/wo.slice/<unit> dirs. A unit awaiting
        // restart keeps its cgroup (re-created on respawn). Final-shutdown
        // cleanup is unnecessary — the machine is going down.
        let restarting = self.pending_restarts.iter().any(|r| r.unit == unit_id);
        if !self.shutting_down && !restarting {
            self.reap_cgroup(unit_id);
        }
    }

    /// Remove a stopped unit's cgroup directory (non-fake mode only).
    /// Best-effort: `NotFound` is success; `EBUSY` (still populated by an
    /// escaped grandchild) is logged and retried on the next exit.
    fn reap_cgroup(&self, id: UnitId) {
        if self.fake { return; }
        let rel = format!("wo.slice/{}", self.registry.name_of(id));
        if let Err(e) = crate::cgroup::remove_cgroup(&rel) {
            eprintln!("writeonce-svc: cgroup cleanup {rel}: {e} (retried on next exit)");
        }
    }

    fn maybe_schedule_restart(&mut self, id: UnitId, exited_clean: bool, signaled: bool) {
        let unit = self.registry.get(id);
        let Some(svc) = &unit.file.service else { return };

        let want_restart = match svc.restart {
            RestartPolicy::No         => false,
            RestartPolicy::Always     => true,
            RestartPolicy::OnFailure  => !exited_clean,
            RestartPolicy::OnAbnormal => signaled,
        };
        if !want_restart { return; }

        let delay = parse_duration(&svc.restart_sec).unwrap_or_else(|| Duration::from_secs(5));
        let burst = svc.start_limit_burst;
        let interval = parse_duration(&svc.start_limit_interval_sec)
            .unwrap_or_else(|| Duration::from_secs(10));

        let now = Instant::now();
        let meta = &mut self.units[id.0 as usize];
        meta.restart_history.retain(|t| now.duration_since(*t) <= interval);

        // systemd-compatible: burst=0 disables rate-limiting entirely.
        if burst > 0 && meta.restart_history.len() >= burst as usize {
            meta.state = UnitActiveState::Failed;
            eprintln!(
                "writeonce-svc: {} hit start-limit-burst ({} failures in {:?}); \
                 marking Failed and suppressing further restarts",
                self.registry.name_of(id), burst, interval,
            );
            return;
        }
        meta.restart_history.push(now);

        let fire_at = now + delay;
        self.pending_restarts.push(PendingRestart { unit: id, fire_at });
        println!("writeonce-svc: scheduling restart of {} in {:?}",
                 self.registry.name_of(id), delay);
    }

    /// Smallest future restart timestamp, if any.
    pub fn next_restart_deadline(&self) -> Option<Instant> {
        self.pending_restarts.iter().map(|r| r.fire_at).min()
    }

    /// Fire any restarts whose `fire_at` is in the past, in time order.
    pub fn fire_due_restarts(&mut self) {
        let now = Instant::now();
        let mut due: Vec<UnitId> = Vec::new();
        self.pending_restarts.retain(|r| {
            if r.fire_at <= now {
                due.push(r.unit);
                false
            } else {
                true
            }
        });
        for id in due {
            println!("writeonce-svc: restarting {}", self.registry.name_of(id));
            let _ = self.start_unit_internal(id);
        }
    }

    /// Synchronous stop of `anchor` and all units that depend on it.
    /// Sends SIGTERM, drains SIGCHLD up to a grace, escalates to SIGKILL.
    pub fn stop_unit(&mut self, anchor: &str) -> Result<(), String> {
        let plan = self.registry.build_stop_transaction(anchor)
            .map_err(|e| format!("{e:?}"))?;
        // Suppress restart scheduling during this operation.
        let was_shutting_down = self.shutting_down;
        self.shutting_down = true;

        // Grace = the longest configured timeout-stop-sec among the units we
        // SIGTERM, so no unit is SIGKILLed before its own grace elapses.
        let grace = plan.iter()
            .filter_map(|&id| self.registry.get(id).file.service.as_ref())
            .filter_map(|svc| parse_duration(&svc.timeout_stop_sec))
            .max()
            .unwrap_or_else(|| Duration::from_secs(10));

        let mut expected: HashSet<libc::pid_t> = HashSet::new();
        for &id in &plan {
            // Drop any pending restart for this unit.
            self.pending_restarts.retain(|r| r.unit != id);

            match self.units[id.0 as usize].pid {
                Some(pid) => {
                    self.units[id.0 as usize].state = UnitActiveState::Deactivating;
                    unsafe { libc::kill(pid, libc::SIGTERM) };
                    expected.insert(pid);
                }
                None => {
                    self.units[id.0 as usize].state = UnitActiveState::Inactive;
                }
            }
        }

        // Drain SIGCHLDs synchronously with the computed grace.
        let start = Instant::now();
        let mut escalated = false;
        while !expected.is_empty() {
            if !escalated && start.elapsed() > grace {
                for &pid in &expected {
                    unsafe { libc::kill(pid, libc::SIGKILL) };
                }
                escalated = true;
            }
            loop {
                let mut status: libc::c_int = 0;
                let pid = unsafe { libc::waitpid(-1, &mut status, libc::WNOHANG) };
                if pid <= 0 { break; }
                expected.remove(&pid);
                self.on_child_exit(pid, status);
            }
            if !expected.is_empty() {
                std::thread::sleep(Duration::from_millis(50));
            }
        }

        // All stopped units are reaped now — tear down their cgroups.
        for &id in &plan {
            self.reap_cgroup(id);
        }

        self.shutting_down = was_shutting_down;
        Ok(())
    }

    /// Synchronous start of a previously-inactive unit (and its transitive
    /// requirement closure, in topological order).
    pub fn start_named(&mut self, name: &str) -> Result<(), String> {
        let plan = self.registry.build_transaction(name)
            .map_err(|e| format!("{e:?}"))?;
        for job in plan {
            // Don't re-start units that are already Active.
            if self.unit(job.unit).state == UnitActiveState::Active { continue; }
            self.start_unit_internal(job.unit).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    /// Begin orderly supervisor shutdown — kill every running service.
    pub fn initiate_shutdown(&mut self) {
        if self.shutting_down { return; }
        println!("writeonce-svc: shutdown initiated");
        self.shutting_down = true;
        self.pending_restarts.clear();
        for u in &self.units {
            if let Some(pid) = u.pid {
                unsafe { libc::kill(pid, libc::SIGTERM) };
            }
        }
    }

    pub fn all_units_quiet(&self) -> bool {
        self.units.iter().all(|u| u.pid.is_none())
    }

    pub fn print_summary(&self) {
        println!();
        println!("writeonce-svc: state summary");
        for u in &self.units {
            let name = self.registry.name_of(u.id);
            let pid = u.pid.map(|p| p.to_string()).unwrap_or_else(|| "—".to_string());
            println!("  {:32}  {:>12?}  pid={pid}", name, u.state);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{InstallSection, RestartPolicy, ServiceSection, UnitFile, UnitSection};

    fn svc(name: &str, exec: &str, kind: ServiceType, requires: Vec<String>) -> LoadedUnit {
        LoadedUnit {
            name: name.into(),
            file: UnitFile {
                unit: UnitSection {
                    requires: requires.clone(),
                    after: requires, // order the dep before us
                    ..Default::default()
                },
                service: Some(ServiceSection {
                    kind,
                    exec_start: exec.into(),
                    exec_stop: String::new(),
                    exec_reload: String::new(),
                    restart: RestartPolicy::No,
                    restart_sec: "5s".into(),
                    start_limit_burst: 3,
                    start_limit_interval_sec: "30s".into(),
                    timeout_start_sec: "5s".into(),
                    timeout_stop_sec: "5s".into(),
                    user: "root".into(),
                    group: "root".into(),
                    slice: "system.slice".into(),
                    remain_after_exit: false,
                    environment: vec![],
                }),
                install: InstallSection::default(),
            },
        }
    }

    /// A failed hard dependency (a oneshot that exits non-zero) must mark its
    /// dependent Failed and skip spawning it, rather than cascade.
    #[test]
    fn requires_failed_dep_skips_dependent() {
        let loaded = vec![
            svc("base.service", "/bin/false", ServiceType::Oneshot, vec![]),
            svc("dep.service", "/bin/true", ServiceType::Simple, vec!["base.service".into()]),
        ];
        let mut state = SupervisorState::from_loaded(loaded, /*fake=*/ true);
        let plan = state.registry.build_transaction("dep.service").unwrap();
        state.activate_plan(&plan).unwrap();

        let base = state.registry.lookup("base.service").unwrap();
        let dep = state.registry.lookup("dep.service").unwrap();
        assert_eq!(state.unit(base).state, UnitActiveState::Failed,
                   "a oneshot exiting non-zero must be Failed");
        assert_eq!(state.unit(dep).state, UnitActiveState::Failed,
                   "a unit requiring a Failed dep must be skipped (Failed), not started");
    }
}
