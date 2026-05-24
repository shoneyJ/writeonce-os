//! `writeonce-login` — library surface for the PAM-based console login.
//!
//! The binary at `src/main.rs` is a thin entry point. Most of the
//! reusable code (PAM FFI bindings, terminal helpers, config) lives in
//! these modules so they can be unit-tested.

pub mod config;
pub mod pam;
pub mod term;
