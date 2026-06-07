//! `writeonce-greeter` — library surface for the TUI login greeter.
//!
//! The binary (`src/main.rs`) is the loop + tty plumbing; the reusable,
//! unit-testable pieces (config schema, `.desktop` session discovery) live
//! here. The PAM FFI, terminal helpers, and privilege-drop/exec are reused
//! from the `writeonce-login` crate (`writeonce_login::{pam, term, session}`).

pub mod config;
pub mod sessions;
