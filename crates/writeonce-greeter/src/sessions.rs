//! Session discovery — the "gear icon" list.
//!
//! Reads freedesktop session desktop files (the same `*.desktop` files GDM /
//! SDDM / greetd read) from the standard locations PLUS the Nix profile dirs,
//! since on WriteOnce the compositor (and thus its `share/wayland-sessions/
//! <name>.desktop`) is installed via `nix profile add`. A `.desktop` only
//! exists once its package is installed, so "discovered" == "installed" — no
//! extra availability probe is needed.
//!
//! A synthetic **Shell (bash)** entry is always appended so local login works
//! even before any compositor is installed (and as the lockout backstop).

use std::fs;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionKind {
    /// A Wayland compositor session (`XDG_SESSION_TYPE=wayland`).
    Wayland,
    /// The plain login shell (`XDG_SESSION_TYPE=tty`).
    Shell,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    /// Display name (`Name=`), shown in the menu.
    pub name: String,
    /// Command to run (`Exec=`, field codes stripped). For `Shell` this is
    /// the user's login shell.
    pub exec: String,
    /// `DesktopNames=` (semicolon-separated), e.g. `"sway"` / `"Hyprland"`.
    pub desktop_names: String,
    pub kind: SessionKind,
}

impl Session {
    /// First `DesktopNames` entry (for `XDG_SESSION_DESKTOP`); empty if none.
    pub fn primary_desktop(&self) -> &str {
        self.desktop_names.split(';').next().unwrap_or("").trim()
    }
}

/// The directories scanned for `*.desktop` session files, in priority order.
/// Nix profile dirs mirror the PATH set by `/etc/profile.d/nix.sh`.
pub fn session_dirs(home: &str) -> Vec<String> {
    vec![
        "/usr/share/wayland-sessions".to_string(),
        format!("{home}/.nix-profile/share/wayland-sessions"),
        "/nix/var/nix/profiles/default/share/wayland-sessions".to_string(),
        format!("{home}/.local/share/wayland-sessions"),
    ]
}

/// Discover installed Wayland sessions for the given user's `$HOME`.
/// Deduplicated by display Name; sorted case-insensitively. An empty result
/// is normal (nothing installed yet) — callers still get a Shell entry.
pub fn discover(home: &str) -> Vec<Session> {
    let mut out: Vec<Session> = Vec::new();
    for dir in session_dirs(home) {
        let entries = match fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("desktop") {
                continue;
            }
            if let Ok(content) = fs::read_to_string(&path) {
                if let Some(s) = parse_desktop(&content) {
                    if !out.iter().any(|x| x.name == s.name) {
                        out.push(s);
                    }
                }
            }
        }
    }
    out.sort_by_key(|s| s.name.to_lowercase());
    out
}

/// The always-present login-shell session.
pub fn shell_session(shell: &str) -> Session {
    Session {
        name: "Shell (bash)".to_string(),
        exec: shell.to_string(),
        desktop_names: String::new(),
        kind: SessionKind::Shell,
    }
}

/// Full menu for a user: discovered compositors + the Shell entry.
pub fn menu_for(home: &str, shell: &str) -> Vec<Session> {
    let mut m = discover(home);
    m.push(shell_session(shell));
    m
}

/// Parse a `.desktop` file's `[Desktop Entry]` group into a `Session`.
/// Returns `None` if there's no usable `Exec`/`TryExec`.
pub fn parse_desktop(content: &str) -> Option<Session> {
    let mut name: Option<String> = None;
    let mut exec: Option<String> = None;
    let mut try_exec: Option<String> = None;
    let mut desktop_names = String::new();
    let mut in_entry = false;

    for raw in content.lines() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            in_entry = line.eq_ignore_ascii_case("[Desktop Entry]");
            continue;
        }
        if !in_entry {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            let (k, v) = (k.trim(), v.trim());
            match k {
                // Locale-qualified keys ("Name[de]") have a different key and
                // are ignored; we take the first plain "Name".
                "Name" if name.is_none() => name = Some(v.to_string()),
                "Exec" if exec.is_none() => exec = Some(strip_field_codes(v)),
                "TryExec" if try_exec.is_none() => try_exec = Some(v.to_string()),
                "DesktopNames" => desktop_names = v.trim_end_matches(';').to_string(),
                _ => {}
            }
        }
    }

    let exec = exec.or(try_exec)?;
    if exec.is_empty() {
        return None;
    }
    let name = name.unwrap_or_else(|| exec.clone());
    Some(Session {
        name,
        exec,
        desktop_names,
        kind: SessionKind::Wayland,
    })
}

/// Strip freedesktop `Exec` field codes (%f %F %u %U %i %c %k %d %D %n %N %v %m).
/// They never apply to a session launch and must not be passed to the compositor.
fn strip_field_codes(exec: &str) -> String {
    exec.split_whitespace()
        .filter(|tok| !(tok.len() == 2 && tok.starts_with('%')))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_typical_sway_desktop() {
        let d = "\
[Desktop Entry]
Name=Sway
Comment=An i3-compatible Wayland compositor
Exec=sway
Type=Application
DesktopNames=sway
";
        let s = parse_desktop(d).expect("should parse");
        assert_eq!(s.name, "Sway");
        assert_eq!(s.exec, "sway");
        assert_eq!(s.desktop_names, "sway");
        assert_eq!(s.kind, SessionKind::Wayland);
        assert_eq!(s.primary_desktop(), "sway");
    }

    #[test]
    fn strips_exec_field_codes() {
        assert_eq!(strip_field_codes("Hyprland %U"), "Hyprland");
        assert_eq!(strip_field_codes("startplasma-wayland %f"), "startplasma-wayland");
        assert_eq!(strip_field_codes("foo"), "foo");
    }

    #[test]
    fn ignores_keys_outside_desktop_entry_group() {
        let d = "\
[Desktop Entry]
Name=Hyprland
Exec=Hyprland
[Desktop Action new]
Exec=should-be-ignored
";
        let s = parse_desktop(d).unwrap();
        assert_eq!(s.exec, "Hyprland");
    }

    #[test]
    fn falls_back_to_tryexec_and_then_exec_name() {
        let d = "[Desktop Entry]\nTryExec=wayfire\n";
        let s = parse_desktop(d).unwrap();
        assert_eq!(s.exec, "wayfire");
        assert_eq!(s.name, "wayfire"); // no Name= → use exec
    }

    #[test]
    fn no_exec_means_no_session() {
        assert!(parse_desktop("[Desktop Entry]\nName=Broken\n").is_none());
    }

    #[test]
    fn shell_session_is_tty_kind() {
        let s = shell_session("/bin/bash");
        assert_eq!(s.kind, SessionKind::Shell);
        assert_eq!(s.exec, "/bin/bash");
    }
}
