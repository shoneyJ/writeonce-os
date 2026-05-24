//! `service.toml` schema, derived from
//! [`docs/learning/phase-4-service-toml-schema.md`](../../docs/learning/phase-4-service-toml-schema.md).
//!
//! Round-1 scaffold scope: the types deserialise from TOML; `*-sec`
//! duration fields stay as `String` for now. Round-4 will convert them to
//! `std::time::Duration` via a custom deserialiser.

use serde::Deserialize;
use std::fs;
use std::io;
use std::path::Path;
use std::time::Duration;

/// Parse a humantime-ish duration string ("5s", "100ms", "1m", "1h", or
/// bare seconds). Returns `None` for malformed input — callers should
/// fall back to a sensible default.
pub fn parse_duration(s: &str) -> Option<Duration> {
    let s = s.trim();
    if let Some(n) = s.strip_suffix("ms") {
        n.parse::<u64>().ok().map(Duration::from_millis)
    } else if let Some(n) = s.strip_suffix("ms") {
        n.parse::<u64>().ok().map(Duration::from_millis)
    } else if let Some(n) = s.strip_suffix('h') {
        n.parse::<u64>().ok().map(|v| Duration::from_secs(v * 3600))
    } else if let Some(n) = s.strip_suffix('m') {
        n.parse::<u64>().ok().map(|v| Duration::from_secs(v * 60))
    } else if let Some(n) = s.strip_suffix('s') {
        n.parse::<u64>().ok().map(Duration::from_secs)
    } else {
        s.parse::<u64>().ok().map(Duration::from_secs)
    }
}

#[cfg(test)]
mod duration_tests {
    use super::*;

    #[test]
    fn duration_seconds() {
        assert_eq!(parse_duration("5s"),   Some(Duration::from_secs(5)));
        assert_eq!(parse_duration("0s"),   Some(Duration::ZERO));
        assert_eq!(parse_duration("100"),  Some(Duration::from_secs(100)));
    }

    #[test]
    fn duration_millis_minutes_hours() {
        assert_eq!(parse_duration("100ms"), Some(Duration::from_millis(100)));
        assert_eq!(parse_duration("2m"),    Some(Duration::from_secs(120)));
        assert_eq!(parse_duration("1h"),    Some(Duration::from_secs(3600)));
    }

    #[test]
    fn duration_malformed_returns_none() {
        assert_eq!(parse_duration("abc"),    None);
        assert_eq!(parse_duration("5min"),   None);
        assert_eq!(parse_duration(""),       None);
    }
}

/// A parsed `*.service.toml` or `*.target.toml` file.
///
/// `service` is optional because target units have no `[service]` section.
#[derive(Debug, Default, Deserialize)]
#[serde(default)]
pub struct UnitFile {
    pub unit:    UnitSection,
    pub service: Option<ServiceSection>,
    pub install: InstallSection,
}

/// `[unit]` table — applies to every unit kind.
#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct UnitSection {
    pub description:          String,
    pub after:                Vec<String>,
    pub before:               Vec<String>,
    pub requires:             Vec<String>,
    pub wants:                Vec<String>,
    pub binds_to:             Vec<String>,
    pub part_of:              Vec<String>,
    pub conflicts:            Vec<String>,
    pub default_dependencies: bool,
}

/// `[service]` table — service-type units only.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct ServiceSection {
    #[serde(rename = "type", default = "default_service_type")]
    pub kind:              ServiceType,
    pub exec_start:        String,
    #[serde(default)]
    pub exec_stop:         String,
    #[serde(default)]
    pub exec_reload:       String,
    #[serde(default = "default_restart")]
    pub restart:           RestartPolicy,
    /// Duration strings (e.g. `"5s"`, `"30s"`). Parsed in Round 4.
    #[serde(default = "default_restart_sec")]
    pub restart_sec:       String,
    #[serde(default = "default_timeout_start_sec")]
    pub timeout_start_sec: String,
    #[serde(default = "default_timeout_stop_sec")]
    pub timeout_stop_sec:  String,
    #[serde(default = "default_user")]
    pub user:              String,
    #[serde(default = "default_group")]
    pub group:             String,
    #[serde(default = "default_slice")]
    pub slice:             String,
    #[serde(default)]
    pub remain_after_exit: bool,
    #[serde(default)]
    pub environment:       Vec<String>,
}

/// `[install]` table — reverse-dependency targets (see `WantedBy` semantics
/// in `docs/learning/phase-4-service-toml-schema.md`).
#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct InstallSection {
    pub wanted_by:   Vec<String>,
    pub required_by: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ServiceType { Simple, Forking, Oneshot, Notify }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RestartPolicy { No, Always, OnFailure, OnAbnormal }

// --- defaults (kebab-case forms in the actual TOML map to these) ---
fn default_service_type()     -> ServiceType   { ServiceType::Simple }
fn default_restart()          -> RestartPolicy { RestartPolicy::No }
fn default_restart_sec()      -> String { "5s".into() }
fn default_timeout_start_sec() -> String { "30s".into() }
fn default_timeout_stop_sec() -> String { "10s".into() }
fn default_user()             -> String { "root".into() }
fn default_group()            -> String { "root".into() }
fn default_slice()            -> String { "system.slice".into() }

// ----------------------------------------------------------------------------
// Directory loader
// ----------------------------------------------------------------------------

/// A parsed unit file paired with the name derived from its filename.
#[derive(Debug)]
pub struct LoadedUnit {
    /// e.g. `"multi-user.target"`, `"hello.service"`. Derived by stripping
    /// `.toml` from the filename.
    pub name: String,
    pub file: UnitFile,
}

/// Read every `*.service.toml` and `*.target.toml` file in `dir`. Returns
/// units sorted by name (for deterministic ordering across runs).
///
/// Filenames that don't end in `.service.toml` or `.target.toml` are
/// ignored silently — drop-in directories or stray files are tolerated.
pub fn load_directory<P: AsRef<Path>>(dir: P) -> io::Result<Vec<LoadedUnit>> {
    let dir = dir.as_ref();
    let mut out = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        if !path.is_file() { continue; }
        let Some(file_name) = path.file_name().and_then(|n| n.to_str()) else { continue };
        let Some(name) = file_name.strip_suffix(".toml") else { continue };
        if !(name.ends_with(".service") || name.ends_with(".target")) { continue; }

        let body = fs::read_to_string(&path)?;
        let file: UnitFile = toml::from_str(&body)
            .map_err(|e| io::Error::other(format!("{file_name}: {e}")))?;
        out.push(LoadedUnit { name: name.to_string(), file });
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}
