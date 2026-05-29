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
//!   writeonce.rootwait=N
//!                   (seconds to poll for the root device before
//!                   giving up; defaults to 30. Namespaced to avoid
//!                   collision with the kernel's own `rootwait` flag,
//!                   which is consumed by `init/do_mounts.c` and
//!                   doesn't reach our Rust PID 1.)

use std::fs;
use std::io;

#[derive(Debug, Default, Clone)]
pub struct CmdLine {
    pub root_spec:    Option<RootSpec>,
    pub rootfstype:   Option<String>,
    pub rootflags:    Option<String>,
    pub init_path:    String,
    pub recovery:     bool,
    /// Seconds to wait for the root device to appear in /sys/class/block
    /// before giving up. Default 30. Set via `writeonce.rootwait=N` on
    /// the kernel cmdline. Critical for boot-from-USB on hardware where
    /// USB enumeration is asynchronous and slower than the initramfs
    /// reaches `discover::locate_root`.
    pub rootwait_secs: u64,
    /// `mount(2)` flags applied when mounting the real root onto
    /// `/sysroot`. Default: `MS_NOATIME` (writable, no atime
    /// updates — matches the systemd / dracut default). Cleared
    /// of `MS_RDONLY` when the cmdline contains `rw`; set with
    /// `MS_RDONLY` when the cmdline contains `ro`. Without this
    /// field the initramfs used to hardcode `MS_RDONLY` and the
    /// kernel-cmdline `rw` token was silently discarded — every
    /// service that tried to write to / failed with `EROFS`.
    pub mount_flags: u64,
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
        out.rootwait_secs = 30;
        out.mount_flags = libc::MS_NOATIME as u64;
        for tok in s.split_whitespace() {
            if let Some(v) = tok.strip_prefix("root=") {
                out.root_spec = Some(parse_root(v));
            } else if let Some(v) = tok.strip_prefix("rootfstype=") {
                out.rootfstype = Some(v.to_string());
            } else if let Some(v) = tok.strip_prefix("rootflags=") {
                out.rootflags = Some(v.to_string());
            } else if let Some(v) = tok.strip_prefix("init=") {
                out.init_path = v.to_string();
            } else if let Some(v) = tok.strip_prefix("writeonce.rootwait=") {
                // Garbage parses to 0 → no wait (fail-fast). Operator
                // can set `writeonce.rootwait=0` deliberately for that.
                out.rootwait_secs = v.parse().unwrap_or(0);
            } else if tok == "rw" {
                out.mount_flags &= !(libc::MS_RDONLY as u64);
            } else if tok == "ro" {
                out.mount_flags |= libc::MS_RDONLY as u64;
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
        assert_eq!(c.rootwait_secs, 30);
    }

    #[test]
    fn rootwait_override() {
        let c = CmdLine::parse("root=LABEL=writeonce-root writeonce.rootwait=45");
        assert_eq!(c.rootwait_secs, 45);
    }

    #[test]
    fn rootwait_zero_means_fail_fast() {
        let c = CmdLine::parse("writeonce.rootwait=0");
        assert_eq!(c.rootwait_secs, 0);
    }

    #[test]
    fn rootwait_garbage_falls_back_to_zero() {
        // Reasonable: invalid input → don't wait. Operator notices the
        // immediate fail and fixes the cmdline.
        let c = CmdLine::parse("writeonce.rootwait=ten");
        assert_eq!(c.rootwait_secs, 0);
    }

    #[test]
    fn rw_token_clears_rdonly() {
        let c = CmdLine::parse("root=UUID=abc rw");
        assert_eq!(c.mount_flags & libc::MS_RDONLY as u64, 0,
            "rw should clear MS_RDONLY");
        assert_ne!(c.mount_flags & libc::MS_NOATIME as u64, 0,
            "MS_NOATIME should remain set");
    }

    #[test]
    fn ro_token_sets_rdonly() {
        let c = CmdLine::parse("root=UUID=abc ro");
        assert_ne!(c.mount_flags & libc::MS_RDONLY as u64, 0,
            "ro should set MS_RDONLY");
    }

    #[test]
    fn default_mount_flags_is_writable() {
        let c = CmdLine::parse("root=UUID=abc");
        assert_eq!(c.mount_flags & libc::MS_RDONLY as u64, 0,
            "no token → writable by default");
        assert_ne!(c.mount_flags & libc::MS_NOATIME as u64, 0);
    }

    #[test]
    fn ro_then_rw_keeps_writable() {
        // Last token wins; useful when overriding at the GRUB prompt.
        let c = CmdLine::parse("root=UUID=abc ro rw");
        assert_eq!(c.mount_flags & libc::MS_RDONLY as u64, 0);
    }
}
