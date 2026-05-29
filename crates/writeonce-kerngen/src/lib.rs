//! `writeonce-kerngen` — derive a target-specific kernel `.config` from
//! a hardware probe of the running (or target) system.
//!
//! Phase 7a (this crate): the `probe` subcommand. Walks `/sys/bus/*`,
//! `/proc/cpuinfo`, `/sys/class/dmi/id` and `/sys/firmware/efi` and
//! emits a JSON dump. See [`crate::types::Probe`] for the schema.
//!
//! Future (Phase 7b): a `resolve` subcommand that consumes a probe
//! JSON + a kernel source tree and emits a Kconfig fragment.

pub mod probe;
pub mod types;
