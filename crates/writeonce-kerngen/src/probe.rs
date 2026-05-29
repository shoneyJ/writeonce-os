//! Collect the hardware fingerprint of the running system into a
//! [`Probe`].
//!
//! Every read is best-effort. A missing `/sys/firmware/efi` yields
//! `firmware.efi = false`; a missing `modalias` skips that device.
//! The probe never fails the whole collection because one subsystem
//! is absent — the JSON's missing fields are themselves diagnostic.

use std::ffi::CStr;
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::types::*;

/// Top-level entry point. Returns a populated [`Probe`].
pub fn collect() -> Probe {
    Probe {
        probe_version: PROBE_VERSION,
        probed_at_unix: now_unix(),
        uname:    read_uname(),
        cpu:      read_cpu(),
        dmi:      read_dmi(),
        firmware: read_firmware(),
        devices:  read_devices(),
    }
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// uname (kernel identity)
// ---------------------------------------------------------------------------

fn read_uname() -> Uname {
    // SAFETY: uname(2) fills a caller-owned `utsname` struct; zeroed
    // is a valid initial state. Returns 0 on success; we tolerate
    // failure by emitting empty strings.
    let mut buf: libc::utsname = unsafe { std::mem::zeroed() };
    if unsafe { libc::uname(&mut buf) } != 0 {
        return Uname::default();
    }
    Uname {
        sysname: cstr_field(&buf.sysname),
        release: cstr_field(&buf.release),
        version: cstr_field(&buf.version),
        machine: cstr_field(&buf.machine),
    }
}

/// Convert a `[c_char; N]` from `utsname` into an owned `String`.
fn cstr_field(field: &[libc::c_char]) -> String {
    // Cast through *const c_char so we can use CStr::from_ptr safely.
    let ptr = field.as_ptr();
    // SAFETY: `utsname` fields are NUL-terminated per POSIX.
    unsafe { CStr::from_ptr(ptr) }.to_string_lossy().into_owned()
}

// ---------------------------------------------------------------------------
// CPU (/proc/cpuinfo)
// ---------------------------------------------------------------------------

fn read_cpu() -> CpuInfo {
    let body = match fs::read_to_string("/proc/cpuinfo") {
        Ok(s) => s,
        Err(_) => return CpuInfo::default(),
    };

    let mut cpu = CpuInfo::default();
    let mut in_first_block = true;

    for line in body.lines() {
        if line.is_empty() {
            // End of `processor : 0` block — stop parsing fields.
            in_first_block = false;
            continue;
        }
        let Some((key, val)) = line.split_once(':') else { continue };
        let key = key.trim();
        let val = val.trim();

        if key == "processor" {
            cpu.cpu_count += 1;
        }
        if !in_first_block {
            continue;
        }
        match key {
            "vendor_id"  => cpu.vendor_id  = val.to_string(),
            "cpu family" => cpu.cpu_family = val.parse().unwrap_or(0),
            "model"      => cpu.model      = val.parse().unwrap_or(0),
            "model name" => cpu.model_name = val.to_string(),
            "stepping"   => cpu.stepping   = val.parse().unwrap_or(0),
            "flags"      => cpu.flags      = val.split_whitespace()
                                                .map(str::to_string)
                                                .collect(),
            _ => {}
        }
    }
    cpu
}

// ---------------------------------------------------------------------------
// DMI (/sys/class/dmi/id/*)
// ---------------------------------------------------------------------------

fn read_dmi() -> DmiInfo {
    let base = Path::new("/sys/class/dmi/id");
    let read = |name: &str| -> Option<String> {
        let p = base.join(name);
        fs::read_to_string(&p).ok().map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    };
    DmiInfo {
        sys_vendor:      read("sys_vendor"),
        product_name:    read("product_name"),
        product_version: read("product_version"),
        product_serial:  read("product_serial"),
        board_vendor:    read("board_vendor"),
        board_name:      read("board_name"),
        bios_vendor:     read("bios_vendor"),
        bios_version:    read("bios_version"),
        bios_date:       read("bios_date"),
    }
}

// ---------------------------------------------------------------------------
// Firmware (/sys/firmware/efi/*)
// ---------------------------------------------------------------------------

fn read_firmware() -> FirmwareInfo {
    let efi_root = Path::new("/sys/firmware/efi");
    if !efi_root.exists() {
        return FirmwareInfo::default();
    }
    FirmwareInfo {
        efi: true,
        fw_platform_size: fs::read_to_string(efi_root.join("fw_platform_size"))
            .ok()
            .and_then(|s| s.trim().parse().ok()),
        secure_boot: read_secure_boot(),
    }
}

/// SecureBoot EFI variable layout: first 4 bytes are
/// `EFI_VARIABLE_ATTRIBUTES` (a u32), followed by the variable's
/// payload — for SecureBoot that's a single u8 (`0` or `1`).
/// We look for any file named `SecureBoot-*` under
/// `/sys/firmware/efi/efivars/` to avoid hardcoding the GUID.
fn read_secure_boot() -> Option<bool> {
    let dir = Path::new("/sys/firmware/efi/efivars");
    let entries = fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if !name.starts_with("SecureBoot-") {
            continue;
        }
        let bytes = fs::read(entry.path()).ok()?;
        // Need at least the 4-byte attribute prefix plus the value.
        if bytes.len() >= 5 {
            return Some(bytes[4] != 0);
        }
    }
    None
}

// ---------------------------------------------------------------------------
// Devices (/sys/bus/{pci,usb,acpi,virtio,platform}/devices/*/modalias)
// ---------------------------------------------------------------------------

const BUSES: &[&str] = &["pci", "usb", "acpi", "virtio", "platform"];

fn read_devices() -> Vec<Device> {
    let mut out = Vec::new();
    for &bus in BUSES {
        let dir = format!("/sys/bus/{bus}/devices");
        let entries = match fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let syspath = entry.path();
            // Read modalias if present. Devices without one are typically
            // bus glue (root hubs, ports) — uninteresting for resolve.
            let modalias = match fs::read_to_string(syspath.join("modalias")) {
                Ok(s) => s.trim().to_string(),
                Err(_) => continue,
            };
            // Some bus glue (e.g. acpi:bus/devices/ABCD0000) has the
            // file but no payload. No information to feed resolve.
            if modalias.is_empty() {
                continue;
            }
            out.push(Device {
                subsystem: bus.to_string(),
                modalias,
                syspath: syspath.to_string_lossy().into_owned(),
                driver: read_driver_symlink(&syspath),
            });
        }
    }
    out.sort_by(|a, b| {
        a.subsystem.cmp(&b.subsystem).then_with(|| a.syspath.cmp(&b.syspath))
    });
    out
}

/// `<syspath>/driver` is a symlink pointing into
/// `/sys/bus/<bus>/drivers/<driver-name>/`. We just want the basename.
fn read_driver_symlink(syspath: &Path) -> Option<String> {
    let target = fs::read_link(syspath.join("driver")).ok()?;
    target.file_name().map(|n| n.to_string_lossy().into_owned())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn now_unix_is_post_2024() {
        // Sanity: any modern test run is past 2024-01-01 (1704067200).
        // If the clock is wildly wrong the rest of the probe is suspect.
        assert!(now_unix() > 1_704_067_200);
    }

    #[test]
    fn probe_collect_returns_consistent_schema() {
        // Runs against the real /sys + /proc — verifies we don't panic
        // and the JSON round-trips. Skips field-content assertions
        // because they're host-specific.
        let p = collect();
        let json = serde_json::to_string(&p).expect("serialise");
        let back: Probe = serde_json::from_str(&json).expect("deserialise");
        assert_eq!(back.probe_version, PROBE_VERSION);
    }
}
