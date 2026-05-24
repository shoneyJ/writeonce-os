//! `/proc/cmdline` parser.
//!
//! Recognises the kernel boot parameters that affect initramfs behaviour:
//!
//!   root=UUID=...   /  root=PARTUUID=...  /  root=LABEL=...  /  root=/dev/sdaN
//!   rootfstype=ext4 (override autodetection)
//!   rootflags=...   (mount(2) options)
//!   init=/path      (path of PID 1 to exec; defaults to /sbin/writeonce-pid1)
//!   wo.recovery     (drop to recovery shell immediately)
//!   wo.fake         (synonym; for development outside QEMU)

use std::fs;
use std::io;

#[derive(Debug, Default, Clone)]
pub struct CmdLine {
    pub root_spec:    Option<RootSpec>,
    pub rootfstype:   Option<String>,
    pub rootflags:    Option<String>,
    pub init_path:    String,
    pub recovery:     bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RootSpec {
    /// `root=UUID=ccdc4477-...`
    Uuid(String),
    /// `root=PARTUUID=...`
    PartUuid(String),
    /// `root=LABEL=...`
    Label(String),
    /// `root=/dev/sda3`
    Device(String),
}

impl CmdLine {
    pub fn load() -> io::Result<Self> {
        let body = fs::read_to_string("/proc/cmdline").unwrap_or_default();
        Ok(Self::parse(&body))
    }

    pub fn parse(s: &str) -> Self {
        let mut out = CmdLine::default();
        out.init_path = "/sbin/writeonce-pid1".into();
        for tok in s.split_whitespace() {
            if let Some(v) = tok.strip_prefix("root=") {
                out.root_spec = Some(parse_root(v));
            } else if let Some(v) = tok.strip_prefix("rootfstype=") {
                out.rootfstype = Some(v.to_string());
            } else if let Some(v) = tok.strip_prefix("rootflags=") {
                out.rootflags = Some(v.to_string());
            } else if let Some(v) = tok.strip_prefix("init=") {
                out.init_path = v.to_string();
            } else if tok == "wo.recovery" || tok == "wo.fake" {
                out.recovery = true;
            }
        }
        out
    }
}

fn parse_root(v: &str) -> RootSpec {
    if let Some(u) = v.strip_prefix("UUID=")     { return RootSpec::Uuid(u.to_string()); }
    if let Some(u) = v.strip_prefix("PARTUUID=") { return RootSpec::PartUuid(u.to_string()); }
    if let Some(u) = v.strip_prefix("LABEL=")    { return RootSpec::Label(u.to_string()); }
    RootSpec::Device(v.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_typical_cmdline() {
        let c = CmdLine::parse(
            "BOOT_IMAGE=/bzImage root=UUID=418b032e-1143-4f14 ro quiet console=tty0"
        );
        assert_eq!(c.root_spec, Some(RootSpec::Uuid("418b032e-1143-4f14".into())));
        assert_eq!(c.init_path, "/sbin/writeonce-pid1");
        assert!(!c.recovery);
    }

    #[test]
    fn parse_device_root() {
        let c = CmdLine::parse("root=/dev/sda3 rootfstype=ext4 init=/sbin/bash");
        assert_eq!(c.root_spec, Some(RootSpec::Device("/dev/sda3".into())));
        assert_eq!(c.rootfstype.as_deref(), Some("ext4"));
        assert_eq!(c.init_path, "/sbin/bash");
    }

    #[test]
    fn recovery_token_sets_flag() {
        let c = CmdLine::parse("wo.recovery");
        assert!(c.recovery);
    }

    #[test]
    fn defaults_when_no_root() {
        let c = CmdLine::parse("ro quiet");
        assert!(c.root_spec.is_none());
        assert_eq!(c.init_path, "/sbin/writeonce-pid1");
    }
}
