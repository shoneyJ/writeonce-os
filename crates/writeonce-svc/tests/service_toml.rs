//! Verify the service.toml parser against the worked examples from
//! `docs/learning/phase-4-service-toml-schema.md`.

use writeonce_svc::config::*;

/// The worked-example `multi-user.target.toml` — no `[service]` section.
const MULTI_USER_TARGET: &str = r#"
[unit]
description = "Multi-user runtime"
requires    = ["basic.target"]
after       = ["basic.target"]
conflicts   = ["rescue.target"]
"#;

/// The worked-example `getty@tty1.service.toml`.
const GETTY_AT_TTY1: &str = r#"
[unit]
description = "Login on tty1"
after       = ["systemd-user-sessions.service"]
conflicts   = ["rescue.service"]

[service]
type        = "simple"
exec-start  = "/usr/sbin/agetty --noclear tty1 linux"
restart     = "always"
restart-sec = "1s"
user        = "root"

[install]
wanted-by   = ["multi-user.target"]
"#;

/// A target file that exercises just defaults.
const EMPTY_TARGET: &str = "";

#[test]
fn parse_multi_user_target() {
    let uf: UnitFile = toml::from_str(MULTI_USER_TARGET).unwrap();
    assert_eq!(uf.unit.description, "Multi-user runtime");
    assert_eq!(uf.unit.requires, vec!["basic.target"]);
    assert_eq!(uf.unit.after,    vec!["basic.target"]);
    assert_eq!(uf.unit.conflicts, vec!["rescue.target"]);
    assert!(uf.service.is_none(), "target unit must have no [service] section");
    assert!(uf.install.wanted_by.is_empty());
}

#[test]
fn parse_getty_service() {
    let uf: UnitFile = toml::from_str(GETTY_AT_TTY1).unwrap();
    assert_eq!(uf.unit.description, "Login on tty1");

    let svc = uf.service.expect("[service] section must be present");
    assert_eq!(svc.kind, ServiceType::Simple);
    assert_eq!(svc.exec_start, "/usr/sbin/agetty --noclear tty1 linux");
    assert_eq!(svc.restart, RestartPolicy::Always);
    assert_eq!(svc.restart_sec, "1s");
    assert_eq!(svc.user, "root");
    // Defaults flow through for fields not present in the TOML:
    assert_eq!(svc.timeout_start_sec, "30s");
    assert_eq!(svc.slice, "system.slice");

    assert_eq!(uf.install.wanted_by, vec!["multi-user.target"]);
    assert!(uf.install.required_by.is_empty());
}

#[test]
fn parse_empty_target_uses_defaults() {
    let uf: UnitFile = toml::from_str(EMPTY_TARGET).unwrap();
    assert_eq!(uf.unit.description, "");
    assert!(uf.unit.after.is_empty());
    assert!(uf.service.is_none());
    assert!(uf.install.wanted_by.is_empty());
}

#[test]
fn restart_policies_round_trip() {
    let cases = [
        ("no",          RestartPolicy::No),
        ("always",      RestartPolicy::Always),
        ("on-failure",  RestartPolicy::OnFailure),
        ("on-abnormal", RestartPolicy::OnAbnormal),
    ];
    for (text, expected) in cases {
        let toml_src = format!(
            r#"
                [service]
                exec-start = "/bin/true"
                restart    = "{text}"
            "#
        );
        let uf: UnitFile = toml::from_str(&toml_src).unwrap();
        assert_eq!(uf.service.unwrap().restart, expected, "for {text}");
    }
}

#[test]
fn service_types_round_trip() {
    for (text, expected) in [
        ("simple",  ServiceType::Simple),
        ("forking", ServiceType::Forking),
        ("oneshot", ServiceType::Oneshot),
        ("notify",  ServiceType::Notify),
    ] {
        let toml_src = format!(
            r#"
                [service]
                exec-start = "/bin/true"
                type       = "{text}"
            "#
        );
        let uf: UnitFile = toml::from_str(&toml_src).unwrap();
        assert_eq!(uf.service.unwrap().kind, expected, "for {text}");
    }
}
