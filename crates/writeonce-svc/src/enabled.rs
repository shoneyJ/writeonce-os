//! `/etc/writeonce/enabled.d/` — opt-in service registry.
//!
//! Each `.toml` file in this directory is a *stub* that names one
//! unit which should be pulled in by `multi-user.target` at boot.
//! The stub format is intentionally minimal:
//!
//! ```toml
//! # /etc/writeonce/enabled.d/iwd.service.toml
//! unit = "iwd.service"
//! ```
//!
//! Stubs are written by `wo-ctl enable <unit>` (or by the installer
//! for headless profiles, before the artifact is sealed). The
//! supervisor calls [`load`] at startup and injects each entry into
//! the dependency graph as if the unit had declared
//! `[install] wanted-by = ["multi-user.target"]`.
//!
//! The model deliberately mirrors systemd's `multi-user.target.wants/`
//! drop-in directory: file presence = enabled; remove file = disabled.
//! Per-stub mtime serves as the "when was this enabled" record.

use serde::Deserialize;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// Default location, baked into the supervisor; can be overridden
/// at the CLI with `--enabled-d <path>`.
pub const DEFAULT_DIR: &str = "/etc/writeonce/enabled.d";

#[derive(Debug, Deserialize)]
struct Stub {
    unit: String,
}

/// Read every `*.toml` under `dir` and return the list of unit names
/// they reference. A missing directory is treated as "nothing enabled"
/// — first-boot before the installer has populated anything is a
/// valid state.
///
/// Malformed stubs are logged to stderr and skipped, never fatal —
/// one broken file shouldn't refuse the entire boot.
pub fn load<P: AsRef<Path>>(dir: P) -> io::Result<Vec<String>> {
    let dir = dir.as_ref();
    let mut out = Vec::new();
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(out),
        Err(e) => return Err(e),
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let Some(ext) = path.extension() else { continue };
        if ext != "toml" {
            continue;
        }
        match fs::read_to_string(&path)
            .map_err(|e| e.to_string())
            .and_then(|body| toml::from_str::<Stub>(&body).map_err(|e| e.to_string()))
        {
            Ok(stub) => out.push(stub.unit),
            Err(e) => eprintln!("writeonce-svc: enabled.d/{}: {}",
                                path.file_name().unwrap().to_string_lossy(), e),
        }
    }
    out.sort();
    out.dedup();
    Ok(out)
}

/// Write a stub file enabling `unit_name`. Returns the stub path.
/// Idempotent — re-enabling a unit that's already enabled is a no-op.
pub fn enable<P: AsRef<Path>>(dir: P, unit_name: &str) -> io::Result<PathBuf> {
    let dir = dir.as_ref();
    fs::create_dir_all(dir)?;
    let path = dir.join(format!("{unit_name}.toml"));
    let body = format!("# Stub written by wo-ctl. Removing this file disables {unit_name}.\nunit = \"{unit_name}\"\n");
    fs::write(&path, body)?;
    Ok(path)
}

/// Remove the stub for `unit_name`. Returns Ok(true) if a stub was
/// removed, Ok(false) if no stub existed (idempotent disable).
pub fn disable<P: AsRef<Path>>(dir: P, unit_name: &str) -> io::Result<bool> {
    let dir = dir.as_ref();
    let path = dir.join(format!("{unit_name}.toml"));
    match fs::remove_file(&path) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn missing_dir_returns_empty() {
        let p = std::env::temp_dir().join("wo-test-missing-dir-xyzzy");
        let _ = std::fs::remove_dir_all(&p);
        let units = load(&p).expect("missing dir should be empty, not error");
        assert!(units.is_empty());
    }

    #[test]
    fn enable_then_load_roundtrip() {
        let dir = std::env::temp_dir().join(format!(
            "wo-enabled-rt-{}", std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        // Clean slate.
        for e in std::fs::read_dir(&dir).unwrap() { let _ = std::fs::remove_file(e.unwrap().path()); }

        enable(&dir, "iwd.service").unwrap();
        enable(&dir, "dhcpcd.service").unwrap();
        let units = load(&dir).unwrap();
        assert_eq!(units, vec!["dhcpcd.service", "iwd.service"]);  // sorted

        let removed = disable(&dir, "iwd.service").unwrap();
        assert!(removed);
        let units = load(&dir).unwrap();
        assert_eq!(units, vec!["dhcpcd.service"]);

        // Re-disable should report false.
        let removed = disable(&dir, "iwd.service").unwrap();
        assert!(!removed);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn malformed_stub_is_skipped_not_fatal() {
        let dir = std::env::temp_dir().join(format!(
            "wo-enabled-bad-{}", std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        for e in std::fs::read_dir(&dir).unwrap() { let _ = std::fs::remove_file(e.unwrap().path()); }

        // Good stub.
        enable(&dir, "iwd.service").unwrap();
        // Bad stub.
        let mut f = std::fs::File::create(dir.join("bad.toml")).unwrap();
        writeln!(f, "this = is not = valid toml [").unwrap();

        // load() should warn-and-continue, returning just the valid one.
        let units = load(&dir).unwrap();
        assert_eq!(units, vec!["iwd.service"]);

        std::fs::remove_dir_all(&dir).ok();
    }
}
