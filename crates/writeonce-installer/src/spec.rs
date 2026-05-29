// spec.rs — target-os.json schema.
//
// All fields are optional; missing or null values trigger interactive
// prompts. Once the operator confirms an installation, the resolved
// plan (no Nones, all values present) is passed to partition.rs +
// customize.rs.

use anyhow::{Context, Result};
use serde::Deserialize;
use std::path::Path;

#[derive(Debug, Deserialize, Default)]
#[allow(dead_code)] // schema_version is for human inspection of the JSON file.
pub struct TargetOsSpec {
    #[serde(default)]
    pub schema_version: Option<String>,
    #[serde(default)]
    pub partitions: Option<PartitionsSpec>,
    #[serde(default)]
    pub user: Option<UserSpec>,
    #[serde(default)]
    pub keyboard: Option<KeyboardSpec>,
    #[serde(default)]
    pub network: Option<NetworkSpec>,
}

#[derive(Debug, Deserialize, Default, Clone)]
pub struct NetworkSpec {
    /// When `true`, the installer writes
    /// `/etc/writeonce/enabled.d/{iwd,dhcpcd,writeonce-modules-load}.toml`
    /// before the artifact is sealed, AND replaces `default.target` so
    /// it requires `multi-user.target` (so the enabled.d entries fire
    /// at boot). Use for headless / SSH-only profiles where the user
    /// can't log in without network.
    ///
    /// Default `false`. Desktop installs leave network opt-in:
    /// `wo-ctl enable iwd dhcpcd` after first login.
    #[serde(default)]
    pub enabled_at_boot: Option<bool>,
}

#[derive(Debug, Deserialize, Default, Clone)]
pub struct PartitionsSpec {
    /// EFI System Partition size in mebibytes. Default 512.
    #[serde(default)]
    pub esp_mib: Option<u32>,
    /// Root partition size in gibibytes. None = consume rest of disk.
    #[serde(default)]
    pub root_gib: Option<u32>,
}

#[derive(Debug, Deserialize, Default, Clone)]
pub struct UserSpec {
    /// Username. Must not be "root". Lowercase alphanumeric + underscore.
    #[serde(default)]
    pub name: Option<String>,
    /// GECOS field (real name). Optional.
    #[serde(default)]
    pub real_name: Option<String>,
    /// **Ignored.** Kept on the schema only so existing JSON files that
    /// include this field still parse cleanly. The installer always
    /// prompts for the password interactively — a JSON file on disk
    /// is the wrong place to store a credential, and a stale hash
    /// invites the operator to use the wrong password later.
    #[serde(default)]
    #[allow(dead_code)]
    pub password_hash: Option<String>,
    /// Login shell. Default /bin/bash.
    #[serde(default)]
    pub shell: Option<String>,
    /// Supplementary groups. Default [wheel, video, audio, input, plugdev].
    #[serde(default)]
    pub groups: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, Default, Clone)]
pub struct KeyboardSpec {
    /// X11/console keymap layout (e.g. "us", "uk", "de", "fr").
    #[serde(default)]
    pub layout: Option<String>,
    /// Optional layout variant ("dvorak", "intl", …).
    #[serde(default)]
    pub variant: Option<String>,
}

impl TargetOsSpec {
    pub fn load(path: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("read {}", path.display()))?;
        // Strip JSON-with-comments style _comment fields are handled by
        // serde's #[serde(default)] — we just deserialize directly.
        // serde_json is more standard for .json but we already have
        // toml; let's stick with serde_json since it's the right format.
        let spec: Self = serde_json::from_str(&raw)
            .with_context(|| format!("parse {} as JSON", path.display()))?;
        Ok(spec)
    }
}

/// The resolved plan — all values present, no Nones. Produced by
/// merging the optional spec with interactive prompts for missing
/// fields. Consumed by partition.rs (sizes) and customize.rs (user +
/// keyboard + network).
#[derive(Debug, Clone)]
pub struct InstallationPlan {
    pub partition: PartitionPlan,
    pub user: ResolvedUser,
    pub keyboard: ResolvedKeyboard,
    pub network: ResolvedNetwork,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ResolvedNetwork {
    /// When true, pre-enable iwd + dhcpcd + writeonce-modules-load
    /// via enabled.d stubs and point default.target at multi-user.target.
    pub enabled_at_boot: bool,
}

#[derive(Debug, Clone)]
pub struct PartitionPlan {
    pub esp_mib: u32,
    /// None = use the rest of the disk after ESP.
    pub root_gib: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct ResolvedUser {
    pub name: String,
    pub real_name: String,
    /// Pre-hashed shadow entry ($6$…).
    pub password_hash: String,
    pub shell: String,
    pub groups: Vec<String>,
    /// UID. Always 1000 for the primary user.
    pub uid: u32,
    /// Primary GID. Always matches uid.
    pub gid: u32,
}

#[derive(Debug, Clone)]
pub struct ResolvedKeyboard {
    pub layout: String,
    pub variant: Option<String>,
}
