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
}

impl SupervisorState {
    pub fn new(registry: UnitRegistry, fake: bool) -> Self {
        let units: Vec<UnitMeta> = (0..registry.len())
            .map(|i| UnitMeta {
                id:    UnitId(i as u32),
                state: UnitActiveState::Inactive,
                pid:   None,
            })
            .collect();
        Self {
            registry,
            units,
            by_pid:           HashMap::new(),
            pending_restarts: Vec::new(),
            shutting_down:    false,
            fake,
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
        // Targets are pure synchronisation points; no process.
        let unit = self.registry.get(id);
        match &unit.file.service {
            None => {
                self.units[id.0 as usize].state = UnitActiveState::Active;
                println!("writeonce-svc: target  {} active", &unit.name);
                Ok(())
            }
            Some(service) => {
                let req = SpawnRequest { name: &unit.name, service, fake: self.fake };
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
                        let mut status: libc::c_int = 0;
                        unsafe { libc::waitpid(pid, &mut status, 0) };
                        let final_state = if service.remain_after_exit {
                            UnitActiveState::Active
                        } else {
                            UnitActiveState::Inactive
                        };
                        self.units[id.0 as usize].state = final_state;
                        self.units[id.0 as usize].pid = None;
                        self.by_pid.remove(&pid);
                        println!("writeonce-svc: oneshot {} → {:?}", &unit.name, final_state);
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
        let Some(&unit_id) = self.by_pid.get(&pid) else { return };
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
        let fire_at = Instant::now() + delay;
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

        let mut expected: HashSet<libc::pid_t> = HashSet::new();
        for id in plan {
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

        // Drain SIGCHLDs synchronously with a 10-second grace.
        let grace = Duration::from_secs(10);
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
