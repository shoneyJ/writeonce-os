//! Serde types for the `probe` subcommand's JSON output.
//!
//! A probe is a flat dump of the host's hardware identity. Future
//! `resolve` (Phase 7b) reads this and walks the kernel's
//! `modules.alias` to derive `CONFIG_*` symbols. Keeping the schema
//! flat (no pre-interpretation) decouples the two phases so a probe
//! collected today survives kernel-version changes in the resolver.

use serde::{Deserialize, Serialize};

/// Bump when the JSON schema changes incompatibly so old probes can
/// be rejected (or migrated) by future `resolve`.
pub const PROBE_VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
pub struct Probe {
    pub probe_version: u32,
    pub probed_at_unix: u64,
    pub uname:    Uname,
    pub cpu:      CpuInfo,
    pub dmi:      DmiInfo,
    pub firmware: FirmwareInfo,
    pub devices:  Vec<Device>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct Uname {
    pub sysname: String,
    pub release: String,
    pub version: String,
    pub machine: String,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct CpuInfo {
    pub vendor_id:   String,
    pub cpu_family:  u32,
    pub model:       u32,
    pub model_name:  String,
    pub stepping:    u32,
    pub flags:       Vec<String>,
    /// Number of `processor :` blocks in /proc/cpuinfo (logical CPUs,
    /// including hyperthread siblings).
    pub cpu_count:   u32,
}

/// DMI strings from `/sys/class/dmi/id/*`. All fields are `Option` so
/// hardware that doesn't populate a given DMI string (or kernel that
/// hides it) round-trips as JSON `null` rather than an empty string.
#[derive(Debug, Serialize, Deserialize, Default)]
pub struct DmiInfo {
    pub sys_vendor:      Option<String>,
    pub product_name:    Option<String>,
    pub product_version: Option<String>,
    pub product_serial:  Option<String>,
    pub board_vendor:    Option<String>,
    pub board_name:      Option<String>,
    pub bios_vendor:     Option<String>,
    pub bios_version:    Option<String>,
    pub bios_date:       Option<String>,
}

#[derive(Debug, Serialize, Deserialize, Default)]
pub struct FirmwareInfo {
    pub efi: bool,
    /// 32 or 64; `None` on BIOS systems or when the sysfs entry is absent.
    pub fw_platform_size: Option<u32>,
    /// `None` when the SecureBoot EFI variable is unreadable (no efivars
    /// mount, no permission). `Some(true|false)` when readable.
    pub secure_boot: Option<bool>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Device {
    /// `"pci"`, `"usb"`, `"acpi"`, `"virtio"`, or `"platform"`.
    pub subsystem: String,
    /// Verbatim contents of `<syspath>/modalias`. Examples:
    ///   `pci:v00008086d00001616sv000017AAsd0000220Abc03sc00i00`
    ///   `usb:v8087p8000d0001dc09dsc00dp00ic09isc00ip00in00`
    ///   `acpi:LNXSYSTM:`
    pub modalias: String,
    /// Absolute path under `/sys/bus/<subsystem>/devices/`.
    pub syspath: String,
    /// Currently-bound driver, if any (resolved from the
    /// `<syspath>/driver` symlink basename). `None` if the device has
    /// no driver bound — either because none exists, or because it's
    /// claimed by a module that didn't load. Both cases are useful
    /// signals for `resolve`.
    pub driver: Option<String>,
}
