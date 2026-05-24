//! `/etc/writeonce/pid1.toml` schema. Tolerant of missing file / missing keys.

use serde::Deserialize;
use std::fs;
use std::path::Path;
use std::time::Duration;

const DEFAULT_PATH: &str = "/etc/writeonce/pid1.toml";

#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Path to the tty to attach the placeholder service to.
    pub tty: String,
    /// Binary the prototype execs on first boot.
    pub child: String,
    /// argv passed to `child`. First element is conventionally the basename.
    pub child_args: Vec<String>,
    /// SIGTERM-to-SIGKILL grace window for the placeholder.
    pub shutdown_grace_seconds: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            tty: "/dev/tty1".into(),
            child: "/bin/sh".into(),
            child_args: vec!["sh".into()],
            shutdown_grace_seconds: 10,
        }
    }
}

impl Config {
    pub fn load_or_default() -> Self {
        Self::load_from(DEFAULT_PATH).unwrap_or_default()
    }

    pub fn load_from<P: AsRef<Path>>(path: P) -> Option<Self> {
        let body = fs::read_to_string(path).ok()?;
        toml::from_str(&body).ok()
    }

    pub fn shutdown_grace(&self) -> Duration {
        Duration::from_secs(self.shutdown_grace_seconds)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_when_empty_toml() {
        let cfg: Config = toml::from_str("").unwrap();
        assert_eq!(cfg.tty, "/dev/tty1");
        assert_eq!(cfg.child, "/bin/sh");
        assert_eq!(cfg.child_args, vec!["sh".to_string()]);
        assert_eq!(cfg.shutdown_grace_seconds, 10);
    }

    #[test]
    fn full_config_parses() {
        let src = r#"
            tty = "/dev/ttyS0"
            child = "/sbin/writeonce-svc"
            child_args = ["writeonce-svc", "--config", "/etc/writeonce/svc.toml"]
            shutdown_grace_seconds = 30
        "#;
        let cfg: Config = toml::from_str(src).unwrap();
        assert_eq!(cfg.tty, "/dev/ttyS0");
        assert_eq!(cfg.child, "/sbin/writeonce-svc");
        assert_eq!(cfg.child_args.len(), 3);
        assert_eq!(cfg.shutdown_grace_seconds, 30);
        assert_eq!(cfg.shutdown_grace(), Duration::from_secs(30));
    }

    #[test]
    fn partial_override_keeps_defaults() {
        let cfg: Config = toml::from_str(r#"tty = "/dev/ttyS1""#).unwrap();
        assert_eq!(cfg.tty, "/dev/ttyS1");
        // unmentioned keys still take defaults
        assert_eq!(cfg.child, "/bin/sh");
        assert_eq!(cfg.shutdown_grace_seconds, 10);
    }

    #[test]
    fn load_from_missing_file_returns_none() {
        let r = Config::load_from("/nonexistent/path/pid1.toml");
        assert!(r.is_none());
    }
}
