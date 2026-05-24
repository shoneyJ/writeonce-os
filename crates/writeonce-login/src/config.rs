//! `/etc/writeonce/login.toml` schema.
//!
//! Tolerant of missing file (defaults), missing keys (per-field
//! defaults), and unknown extra keys (ignored).

use serde::Deserialize;
use std::fs;
use std::io;
use std::path::Path;

#[derive(Debug, Clone, Deserialize)]
#[serde(default, rename_all = "kebab-case")]
pub struct Config {
    /// Host name to display in the banner; empty means "read /etc/hostname".
    pub hostname: String,
    /// Welcome line, printed below the hostname.
    pub welcome: String,
    /// Name passed to `pam_start()`. Must match a stanza in
    /// `/etc/pam.d/<name>`.
    pub pam_service: String,
    /// Path of the script `execve`d after a successful login. The script
    /// is responsible for starting the user's session (D-Bus, X.Org, i3,
    /// i3More, …).
    pub session_script: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            hostname:       String::new(),
            welcome:        "WriteOnce OS".into(),
            pam_service:    "writeonce".into(),
            session_script: "/etc/writeonce/session-start.sh".into(),
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
        // Fall back to /etc/hostname, then "localhost".
        read_etc_hostname().unwrap_or_else(|_| "localhost".to_string())
    }
}

fn read_etc_hostname() -> io::Result<String> {
    let body = fs::read_to_string("/etc/hostname")?;
    Ok(body.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_when_empty() {
        let cfg: Config = toml::from_str("").unwrap();
        assert_eq!(cfg.welcome, "WriteOnce OS");
        assert_eq!(cfg.pam_service, "writeonce");
        assert_eq!(cfg.session_script, "/etc/writeonce/session-start.sh");
        assert_eq!(cfg.hostname, "");
    }

    #[test]
    fn full_config_parses() {
        let src = r#"
            hostname       = "t450"
            welcome        = "Welcome to WriteOnce — developer build"
            pam-service    = "system-login"
            session-script = "/usr/local/bin/start-i3"
        "#;
        let cfg: Config = toml::from_str(src).unwrap();
        assert_eq!(cfg.hostname, "t450");
        assert_eq!(cfg.welcome, "Welcome to WriteOnce — developer build");
        assert_eq!(cfg.pam_service, "system-login");
        assert_eq!(cfg.session_script, "/usr/local/bin/start-i3");
        assert_eq!(cfg.effective_hostname(), "t450");
    }

    #[test]
    fn partial_overrides_keep_defaults() {
        let cfg: Config = toml::from_str(r#"welcome = "hi""#).unwrap();
        assert_eq!(cfg.welcome, "hi");
        assert_eq!(cfg.pam_service, "writeonce");
    }
}
