// prompt.rs — interactive collection of the InstallationPlan.
//
// Called once at the start of install(). Reads target-os.json if
// given; prompts for any field not present. Returns a fully-resolved
// InstallationPlan with no Nones.

use anyhow::{anyhow, Context, Result};
use std::io::{BufRead, Write};

use crate::detect::UsbDevice;
use crate::spec::{
    InstallationPlan, KeyboardSpec, PartitionPlan, PartitionsSpec,
    ResolvedKeyboard, ResolvedNetwork, ResolvedUser, TargetOsSpec, UserSpec,
};

const DEFAULT_ESP_MIB: u32 = 512;
const DEFAULT_SHELL: &str = "/bin/bash";
const DEFAULT_GROUPS: &[&str] = &["wheel", "video", "audio", "input", "plugdev"];
const DEFAULT_KEYBOARD_LAYOUT: &str = "us";

pub fn gather_plan(spec: &TargetOsSpec, device: &UsbDevice) -> Result<InstallationPlan> {
    let partition = gather_partition(spec.partitions.as_ref(), device)?;
    let user = gather_user(spec.user.as_ref())?;
    let keyboard = gather_keyboard(spec.keyboard.as_ref())?;
    // Network defaults to off (desktop opt-in). The spec can flip it
    // to `true` for headless / SSH-only profiles where the user can't
    // get back in without a network on first boot.
    let network = ResolvedNetwork {
        enabled_at_boot: spec.network.as_ref()
            .and_then(|n| n.enabled_at_boot)
            .unwrap_or(false),
    };
    Ok(InstallationPlan {
        partition,
        user,
        keyboard,
        network,
    })
}

// ---- partition ------------------------------------------------------------

fn gather_partition(spec: Option<&PartitionsSpec>, device: &UsbDevice) -> Result<PartitionPlan> {
    println!();
    println!("============================================================");
    println!(" Partition layout for {} ({:.2} GB)", device.path.display(), device.size_gb());
    println!("============================================================");

    let esp_mib = match spec.and_then(|s| s.esp_mib) {
        Some(v) => {
            println!(" ESP size:  {v} MiB (from spec)");
            v
        }
        None => prompt_u32(
            &format!(" ESP size in MiB [{DEFAULT_ESP_MIB}]: "),
            DEFAULT_ESP_MIB,
        )?,
    };

    if esp_mib < 100 || esp_mib > 4096 {
        return Err(anyhow!("ESP size {esp_mib} MiB out of sane range [100, 4096]"));
    }

    let disk_mib = (device.size_bytes / 1_048_576) as u32;
    let max_root_gib = (disk_mib.saturating_sub(esp_mib + 8)) / 1024; // 8 MiB padding for GPT

    let root_gib = match spec.and_then(|s| s.root_gib) {
        Some(0) | None => {
            // Either explicit "use rest" or no spec: prompt.
            let default = max_root_gib; // sentinel meaning "rest"
            let chosen = prompt_u32(
                &format!(" Root partition size in GiB (0 = use rest, max {max_root_gib}) [{default}]: "),
                default,
            )?;
            if chosen == max_root_gib || chosen == 0 {
                None
            } else {
                Some(chosen)
            }
        }
        Some(v) => {
            println!(" Root size: {v} GiB (from spec)");
            if v > max_root_gib {
                return Err(anyhow!(
                    "Root size {v} GiB exceeds available {max_root_gib} GiB after ESP"
                ));
            }
            Some(v)
        }
    };

    println!();
    println!(" → ESP:  {esp_mib} MiB");
    match root_gib {
        Some(v) => println!(" → root: {v} GiB"),
        None => println!(" → root: rest of disk (~{} GiB)", max_root_gib),
    }

    Ok(PartitionPlan { esp_mib, root_gib })
}

// ---- user -----------------------------------------------------------------

fn gather_user(spec: Option<&UserSpec>) -> Result<ResolvedUser> {
    println!();
    println!("============================================================");
    println!(" Primary user account");
    println!("============================================================");

    let name = match spec.and_then(|s| s.name.clone()) {
        Some(n) => {
            println!(" Username: {n} (from spec)");
            n
        }
        None => prompt_string(" Username (not root): ", None)?,
    };
    if name == "root" || name.is_empty() {
        return Err(anyhow!("username must be set and must not be 'root'"));
    }
    if !name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
    {
        return Err(anyhow!(
            "username '{name}' must be lowercase alphanumeric + underscore only"
        ));
    }

    let real_name = match spec.and_then(|s| s.real_name.clone()) {
        Some(rn) => {
            println!(" Real name: {rn} (from spec)");
            rn
        }
        None => prompt_string(" Real name (optional, Enter to skip): ", Some(""))?,
    };

    // Password is ALWAYS prompted interactively, even if `password_hash`
    // is present in target-os.json. Reasons:
    //   - A JSON file on disk is a poor place for a credential.
    //   - The hash format / crypt parameters can drift between hosts.
    //   - Interactive prompt makes the operator type the password they
    //     just chose, which catches typos.
    // We ignore `spec.password_hash` here on purpose.
    let password_hash = prompt_password_and_hash(&name)?;

    let shell = spec
        .and_then(|s| s.shell.clone())
        .unwrap_or_else(|| DEFAULT_SHELL.to_string());

    let groups = spec.and_then(|s| s.groups.clone()).unwrap_or_else(|| {
        DEFAULT_GROUPS.iter().map(|s| s.to_string()).collect()
    });

    println!();
    println!(" → user:  {name} (uid 1000)");
    println!(" → shell: {shell}");
    println!(" → groups: {}", groups.join(","));

    Ok(ResolvedUser {
        name,
        real_name,
        password_hash,
        shell,
        groups,
        uid: 1000,
        gid: 1000,
    })
}

fn prompt_password_and_hash(username: &str) -> Result<String> {
    use rpassword::prompt_password;
    loop {
        let p1 = prompt_password(&format!(" Password for {username}: "))?;
        if p1.is_empty() {
            println!(" (empty password rejected; try again)");
            continue;
        }
        if p1.len() < 6 {
            println!(" (password too short; minimum 6 characters)");
            continue;
        }
        let p2 = prompt_password(" Confirm password: ")?;
        if p1 != p2 {
            println!(" (passwords don't match; try again)");
            continue;
        }
        return hash_password_sha512(&p1);
    }
}

/// Shell out to openssl passwd -6 to produce a SHA-512 crypt hash.
/// Format: $6$<salt>$<hash> — directly usable in /etc/shadow.
pub fn hash_password_sha512(plaintext: &str) -> Result<String> {
    use std::io::Write as _;
    use std::process::{Command, Stdio};
    let mut child = Command::new("openssl")
        .args(["passwd", "-6", "-stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("spawn openssl — install openssl on the host")?;
    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(plaintext.as_bytes())?;
    stdin.write_all(b"\n")?;
    drop(stdin);
    let output = child.wait_with_output()?;
    if !output.status.success() {
        return Err(anyhow!(
            "openssl passwd -6 failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let hash = String::from_utf8(output.stdout)?.trim().to_string();
    validate_hash(&hash)?;
    Ok(hash)
}

pub fn validate_hash(hash: &str) -> Result<()> {
    if !hash.starts_with("$6$") {
        return Err(anyhow!(
            "password hash doesn't look like SHA-512 crypt ($6$...): {hash}"
        ));
    }
    if hash.len() < 20 {
        return Err(anyhow!("password hash unexpectedly short"));
    }
    Ok(())
}

// ---- keyboard -------------------------------------------------------------

fn gather_keyboard(spec: Option<&KeyboardSpec>) -> Result<ResolvedKeyboard> {
    println!();
    println!("============================================================");
    println!(" Keyboard layout");
    println!("============================================================");

    let layout = match spec.and_then(|s| s.layout.clone()) {
        Some(l) => {
            println!(" Layout: {l} (from spec)");
            l
        }
        None => {
            println!(" Common layouts: us, uk, de, fr, es, it, ru, jp, cn");
            let l = prompt_string(
                &format!(" Layout [{DEFAULT_KEYBOARD_LAYOUT}]: "),
                Some(DEFAULT_KEYBOARD_LAYOUT),
            )?;
            validate_keymap(&l)?;
            l
        }
    };

    let variant = match spec.and_then(|s| s.variant.clone()) {
        Some(v) if !v.is_empty() => {
            println!(" Variant: {v} (from spec)");
            Some(v)
        }
        _ => {
            let v = prompt_string(" Variant (Enter for none): ", Some(""))?;
            if v.is_empty() {
                None
            } else {
                validate_keymap(&v)?;
                Some(v)
            }
        }
    };

    println!();
    println!(" → layout: {layout}{}", variant.as_ref().map(|v| format!(" ({v})")).unwrap_or_default());

    Ok(ResolvedKeyboard { layout, variant })
}

fn validate_keymap(s: &str) -> Result<()> {
    if !s.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') {
        return Err(anyhow!(
            "keymap '{s}' must be lowercase alphanumeric + underscore only"
        ));
    }
    Ok(())
}

// ---- small helpers --------------------------------------------------------

fn prompt_u32(prompt: &str, default: u32) -> Result<u32> {
    print!("{prompt}");
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    let t = line.trim();
    if t.is_empty() {
        return Ok(default);
    }
    t.parse::<u32>()
        .with_context(|| format!("expected a non-negative integer, got '{t}'"))
}

fn prompt_string(prompt: &str, default: Option<&str>) -> Result<String> {
    print!("{prompt}");
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line)?;
    let t = line.trim().to_string();
    if t.is_empty() {
        if let Some(d) = default {
            return Ok(d.to_string());
        }
        return Err(anyhow!("input required"));
    }
    Ok(t)
}

/// Print a final summary of the resolved plan + target device so the
/// operator can review before the destructive "type yes" step.
pub fn summarize(device: &UsbDevice, plan: &InstallationPlan) {
    println!();
    println!("============================================================");
    println!(" Installation summary");
    println!("============================================================");
    println!(" Target device : {} ({:.2} GB)", device.path.display(), device.size_gb());
    println!("                 {} {}", device.vendor, device.model);
    println!(" ESP size      : {} MiB", plan.partition.esp_mib);
    println!(" Root size     : {}", match plan.partition.root_gib {
        Some(v) => format!("{v} GiB"),
        None    => "rest of disk".to_string(),
    });
    println!(" Username      : {} (uid 1000, real-name {:?})", plan.user.name, plan.user.real_name);
    println!(" Shell         : {}", plan.user.shell);
    println!(" Groups        : {}", plan.user.groups.join(","));
    println!(" Keyboard      : {}{}", plan.keyboard.layout,
        plan.keyboard.variant.as_ref().map(|v| format!(" ({v})")).unwrap_or_default());
}
