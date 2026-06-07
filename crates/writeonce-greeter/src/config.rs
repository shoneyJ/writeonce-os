//! `/etc/writeonce/greeter.toml` schema.
//!
//! Tolerant of a missing file (defaults), missing keys (per-field defaults),
//! and unknown extra keys (ignored) — same discipline as writeonce-login.

use serde::Deserialize;
use std::fs;
use std::path::Path;

#[derive(Debug, Clone, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Config {
    /// Host name shown in the banner; empty => read /etc/hostname.
    pub hostname: String,
    /// Welcome line.
    pub welcome: String,
    /// Name passed to `pam_start()`. MUST match a file in `/etc/pam.d/`.
    /// A mismatch makes PAM fall back to `other` (deny-by-default) — a total
    /// local lockout — so the default points at the file we ship.
    pub pam_service: String,
    /// Seed default session (by display Name) when nothing is remembered yet.
    pub default_session: String,
    /// Root-owned file storing the last-chosen session Name (never written
    /// into the user's $HOME from the root greeter).
    pub state_path: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            hostname:        String::new(),
            welcome:         "WriteOnce OS".into(),
            pam_service:     "writeonce-greeter".into(),
            default_session: String::new(),
            state_path:      "/var/lib/writeonce-greeter/last".into(),
        }
    }
}

impl Config {
    pub fn load_or_default<P: AsRef<Path>>(path: P) -> Self {
        Self::load_from(path).unwrap_or_default()
    }

    pub fn load_from<P: AsRef<Path>>(path: P) -> Option<Self> {
        let body = fs::read_to_string(path).ok()?;
        toml::from_str(&body).ok()
    }

    pub fn effective_hostname(&self) -> String {
        if !self.hostname.is_empty() {
            return self.hostname.clone();
        }
        fs::read_to_string("/etc/hostname")
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|_| "localhost".to_string())
    }

    /// Last session Name the user picked (None if never / unreadable).
    pub fn read_last_session(&self) -> Option<String> {
        fs::read_to_string(&self.state_path)
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    }

    /// Persist the chosen session Name (best-effort; failure is non-fatal).
    pub fn write_last_session(&self, name: &str) {
        if let Some(parent) = Path::new(&self.state_path).parent() {
            let _ = fs::create_dir_all(parent);
        }
        let _ = fs::write(&self.state_path, format!("{name}\n"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_when_empty() {
        let cfg: Config = toml::from_str("").unwrap();
        assert_eq!(cfg.welcome, "WriteOnce OS");
        // The PAM service default MUST match the shipped /etc/pam.d file —
        // guard against an accidental change that would lock out local login.
        assert_eq!(cfg.pam_service, "writeonce-greeter");
        assert_eq!(cfg.state_path, "/var/lib/writeonce-greeter/last");
    }

    #[test]
    fn partial_overrides_keep_defaults() {
        let cfg: Config = toml::from_str(r#"welcome = "hi""#).unwrap();
        assert_eq!(cfg.welcome, "hi");
        assert_eq!(cfg.pam_service, "writeonce-greeter");
    }

    #[test]
    fn full_config_parses() {
        let src = r#"
            hostname        = "t450"
            welcome         = "WriteOnce — dev"
            pam-service     = "writeonce-greeter"
            default-session = "Sway"
            state-path      = "/var/lib/writeonce-greeter/last"
        "#;
        let cfg: Config = toml::from_str(src).unwrap();
        assert_eq!(cfg.hostname, "t450");
        assert_eq!(cfg.default_session, "Sway");
        assert_eq!(cfg.effective_hostname(), "t450");
    }
}
