// customize.rs — post-extract sysroot mutations.
//
// Runs after the sysroot tarball is extracted and BEFORE the ESP is
// populated. Mutates the staged /etc/* + /home/* files in place to
// reflect the operator's choices.

use anyhow::{Context, Result};
use std::path::Path;

use crate::spec::{InstallationPlan, ResolvedKeyboard, ResolvedNetwork, ResolvedUser};

pub fn apply(plan: &InstallationPlan, mount_root: &Path) -> Result<()> {
    log::info!("Customising staged sysroot at {}", mount_root.display());

    rewrite_passwd(mount_root, &plan.user)?;
    rewrite_shadow(mount_root, &plan.user)?;
    rewrite_group(mount_root, &plan.user)?;
    rename_home(mount_root, &plan.user)?;
    patch_xinitrc(mount_root, &plan.user, &plan.keyboard)?;
    write_vconsole_conf(mount_root, &plan.keyboard)?;
    apply_network(mount_root, &plan.network)?;

    // NOTE: machine-id generation moved to writeonce-bootstrap (a
    // boot-time oneshot). The installer used to write /etc/machine-id
    // here, but that meant "same image, different machines" got the
    // same ID. Bootstrap generates fresh on first boot of each
    // machine. See plan/writeonce-svc-fix/escape-the-loop.md.

    log::info!("Customisation done");
    Ok(())
}

// ---- /etc/passwd ----------------------------------------------------------

fn rewrite_passwd(root: &Path, user: &ResolvedUser) -> Result<()> {
    let path = root.join("etc/passwd");
    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("read {}", path.display()))?;
    let mut out = String::new();
    let mut user_line_written = false;
    for line in raw.lines() {
        if line.starts_with("writeonce:") {
            // Replace the skeleton's placeholder writeonce user with
            // the chosen user.
            out.push_str(&format!(
                "{}:x:{}:{}:{}:/home/{}:{}\n",
                user.name, user.uid, user.gid, user.real_name, user.name, user.shell
            ));
            user_line_written = true;
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    if !user_line_written {
        out.push_str(&format!(
            "{}:x:{}:{}:{}:/home/{}:{}\n",
            user.name, user.uid, user.gid, user.real_name, user.name, user.shell
        ));
    }
    std::fs::write(&path, out).context("write /etc/passwd")?;
    log::info!("/etc/passwd: user '{}' uid={} gid={}", user.name, user.uid, user.gid);
    Ok(())
}

// ---- /etc/shadow ----------------------------------------------------------

fn rewrite_shadow(root: &Path, user: &ResolvedUser) -> Result<()> {
    let path = root.join("etc/shadow");
    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("read {}", path.display()))?;
    let mut out = String::new();
    let mut user_line_written = false;
    for line in raw.lines() {
        if line.starts_with("writeonce:") {
            // days-since-epoch for the "last password change" field
            let lastchange = days_since_epoch();
            out.push_str(&format!(
                "{}:{}:{lastchange}:0:99999:7:::\n",
                user.name, user.password_hash
            ));
            user_line_written = true;
        } else if line.starts_with("root:") {
            // Keep root locked. Operator can `passwd root` after first login.
            out.push_str(line);
            out.push('\n');
        } else {
            out.push_str(line);
            out.push('\n');
        }
    }
    if !user_line_written {
        let lastchange = days_since_epoch();
        out.push_str(&format!(
            "{}:{}:{lastchange}:0:99999:7:::\n",
            user.name, user.password_hash
        ));
    }
    std::fs::write(&path, out).context("write /etc/shadow")?;
    std::os::unix::fs::PermissionsExt::set_mode(
        &mut std::fs::metadata(&path)?.permissions(),
        0o640,
    );
    std::fs::set_permissions(&path, std::os::unix::fs::PermissionsExt::from_mode(0o640))?;
    log::info!("/etc/shadow: hash set for '{}'", user.name);
    Ok(())
}

// ---- /etc/group -----------------------------------------------------------

fn rewrite_group(root: &Path, user: &ResolvedUser) -> Result<()> {
    let path = root.join("etc/group");
    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("read {}", path.display()))?;
    let mut out = String::new();
    let group_set: std::collections::HashSet<&str> =
        user.groups.iter().map(String::as_str).collect();
    let mut primary_added = false;

    for line in raw.lines() {
        let fields: Vec<&str> = line.splitn(4, ':').collect();
        if fields.len() < 4 {
            out.push_str(line);
            out.push('\n');
            continue;
        }
        let gname = fields[0];

        // The skeleton ships a "writeonce" group at gid 1000. Replace
        // with the chosen username at the same gid.
        if gname == "writeonce" {
            out.push_str(&format!("{}:x:{}:\n", user.name, user.gid));
            primary_added = true;
            continue;
        }

        // For groups in the user's supplementary list, add the username
        // to the members field (last colon-separated field).
        if group_set.contains(gname) {
            let members: Vec<&str> = fields[3]
                .split(',')
                .filter(|s| !s.is_empty() && *s != "writeonce")
                .collect();
            let mut members_owned: Vec<String> = members.iter().map(|s| s.to_string()).collect();
            if !members_owned.iter().any(|m| m == &user.name) {
                members_owned.push(user.name.clone());
            }
            out.push_str(&format!(
                "{}:{}:{}:{}\n",
                fields[0],
                fields[1],
                fields[2],
                members_owned.join(",")
            ));
        } else {
            // Strip any pre-existing "writeonce" placeholder membership.
            let members: Vec<&str> = fields[3]
                .split(',')
                .filter(|s| !s.is_empty() && *s != "writeonce")
                .collect();
            out.push_str(&format!(
                "{}:{}:{}:{}\n",
                fields[0],
                fields[1],
                fields[2],
                members.join(",")
            ));
        }
    }
    if !primary_added {
        out.push_str(&format!("{}:x:{}:\n", user.name, user.gid));
    }
    std::fs::write(&path, out).context("write /etc/group")?;
    log::info!("/etc/group: groups {:?} updated for '{}'", user.groups, user.name);
    Ok(())
}

// ---- /home rename ---------------------------------------------------------

fn rename_home(root: &Path, user: &ResolvedUser) -> Result<()> {
    let skeleton = root.join("home/writeonce");
    let target = root.join(format!("home/{}", user.name));
    if !skeleton.exists() {
        // Skeleton tree did not ship a /home/writeonce — create the
        // target dir from scratch.
        std::fs::create_dir_all(&target).context("create /home dir")?;
    } else if skeleton == target {
        // Username is literally "writeonce" — nothing to do.
    } else {
        std::fs::rename(&skeleton, &target)
            .with_context(|| format!("rename {} → {}", skeleton.display(), target.display()))?;
    }
    chown_recursive(&target, user.uid, user.gid)?;
    log::info!("/home/{}: created + chowned to uid {}", user.name, user.uid);
    Ok(())
}

fn chown_recursive(path: &Path, uid: u32, gid: u32) -> Result<()> {
    for entry in walkdir(path)? {
        let cstr = std::ffi::CString::new(entry.as_os_str().as_encoded_bytes())?;
        // SAFETY: chown is a standard libc call; cstr is a valid NUL-terminated
        // path; uid/gid are positive integers.
        let rc = unsafe { libc::chown(cstr.as_ptr(), uid, gid) };
        if rc != 0 {
            return Err(anyhow::anyhow!(
                "chown({}) failed: {}",
                entry.display(),
                std::io::Error::last_os_error()
            ));
        }
    }
    Ok(())
}

fn walkdir(root: &Path) -> Result<Vec<std::path::PathBuf>> {
    let mut out = vec![root.to_path_buf()];
    if root.is_dir() {
        for entry in std::fs::read_dir(root)? {
            let e = entry?;
            let p = e.path();
            if p.is_dir() && !p.is_symlink() {
                out.extend(walkdir(&p)?);
            } else {
                out.push(p);
            }
        }
    }
    Ok(out)
}

// ---- .xinitrc patching ----------------------------------------------------

fn patch_xinitrc(root: &Path, user: &ResolvedUser, kbd: &ResolvedKeyboard) -> Result<()> {
    let path = root.join(format!("home/{}/.xinitrc", user.name));
    if !path.exists() {
        log::warn!(".xinitrc not found at {}; skipping patch", path.display());
        return Ok(());
    }
    let raw = std::fs::read_to_string(&path)?;
    let setxkb = if let Some(v) = &kbd.variant {
        format!("setxkbmap -layout {} -variant {v}", kbd.layout)
    } else {
        format!("setxkbmap {}", kbd.layout)
    };
    let patched = raw
        .lines()
        .map(|line| {
            if line.trim_start().starts_with("setxkbmap ") {
                setxkb.clone()
            } else {
                line.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    std::fs::write(&path, format!("{patched}\n")).context("write .xinitrc")?;
    log::info!("{}: setxkbmap → '{}'", path.display(), setxkb);
    Ok(())
}

// ---- /etc/vconsole.conf ---------------------------------------------------

fn write_vconsole_conf(root: &Path, kbd: &ResolvedKeyboard) -> Result<()> {
    let path = root.join("etc/vconsole.conf");
    let content = match &kbd.variant {
        Some(v) => format!("KEYMAP={}-{}\n", kbd.layout, v),
        None => format!("KEYMAP={}\n", kbd.layout),
    };
    std::fs::write(&path, &content).context("write /etc/vconsole.conf")?;
    log::info!("/etc/vconsole.conf: {}", content.trim());
    Ok(())
}

// ---- network (enabled.d stubs + default.target retarget) ------------------

fn apply_network(root: &Path, net: &ResolvedNetwork) -> Result<()> {
    if !net.enabled_at_boot {
        log::info!("network.enabled_at_boot=false — leaving network opt-in");
        return Ok(());
    }
    log::info!("network.enabled_at_boot=true — pre-enabling network services");
    let enabled_d = root.join("etc/writeonce/enabled.d");
    std::fs::create_dir_all(&enabled_d)
        .with_context(|| format!("create {}", enabled_d.display()))?;
    // Stub schema must match what writeonce-svc::enabled::load parses
    // (a single `unit = "<name>"` key).
    for unit in &["iwd.service", "dhcpcd.service", "writeonce-modules-load.service"] {
        let path = enabled_d.join(format!("{unit}.toml"));
        let body = format!(
            "# Pre-enabled by writeonce-installer (network.enabled_at_boot=true).\n\
             unit = \"{unit}\"\n"
        );
        std::fs::write(&path, body)
            .with_context(|| format!("write {}", path.display()))?;
        log::info!("enabled.d: pre-enabled {unit}");
    }
    // Headless boot wants enabled.d entries to fire at boot, which
    // means default.target needs to require multi-user.target rather
    // than just console.target. Rewrite the staged unit file in place.
    let default_target = root.join("etc/writeonce/services/default.target.toml");
    if default_target.exists() {
        let headless_body = "\
# Rewritten by writeonce-installer (network.enabled_at_boot=true).\n\
# default.target requires multi-user.target so enabled.d entries\n\
# (iwd, dhcpcd, modules-load) fire at boot — necessary for SSH/headless.\n\
\n\
[unit]\n\
description = \"Default supervisor target — headless (network at boot)\"\n\
requires    = [\"multi-user.target\"]\n\
after       = [\"multi-user.target\"]\n";
        std::fs::write(&default_target, headless_body)
            .with_context(|| format!("rewrite {}", default_target.display()))?;
        log::info!("default.target retargeted to multi-user.target for headless boot");
    } else {
        log::warn!(
            "default.target.toml not found at {}; skipping retarget",
            default_target.display()
        );
    }
    Ok(())
}

// ---- helper: days since epoch (for shadow's "last change" field) ----------

fn days_since_epoch() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() / 86_400)
        .unwrap_or(19_500)
}
