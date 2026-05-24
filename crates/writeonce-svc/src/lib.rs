//! WriteOnce OS service supervisor — library surface.
//!
//! Designed for use from `src/main.rs` and integration tests under
//! `tests/`. The runtime modules are:
//!
//!   - [`config`]  service.toml parser + directory loader
//!   - [`graph`]   UnitRegistry + transaction-build algorithm
//!   - [`cgroup`]  `clone3(CLONE_INTO_CGROUP)` placement
//!   - [`spawn`]   fork/exec into a service's cgroup
//!   - [`state`]   SupervisorState: in-memory unit + pid tracking
//!   - [`signal`]  signalfd + epoll loop

pub mod cgroup;
pub mod config;
pub mod control;
pub mod graph;
pub mod signal;
pub mod spawn;
pub mod state;
