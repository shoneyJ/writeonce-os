// detect.rs — enumerate removable block devices.
//
// Reads /sys/block/* directly. Each block device has:
//   /sys/block/<name>/removable  → "1" if removable
//   /sys/block/<name>/size       → 512-byte sectors
//   /sys/block/<name>/device/model
//   /sys/block/<name>/device/vendor
//
// We deliberately do NOT use libudev here — it would add a libudev.so
// runtime dep and the /sys/block API is stable enough for our purpose.
// Refusing to install onto non-removable devices is the safety hook.

use anyhow::{Context, Result};
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct UsbDevice {
    /// Device node, e.g. /dev/sdb.
    pub path: PathBuf,
    /// Size in bytes.
    pub size_bytes: u64,
    /// Vendor string (may be empty).
    pub vendor: String,
    /// Model string (may be empty).
    pub model: String,
    /// True if /sys/block/<name>/removable reads "1".
    pub removable: bool,
}

impl UsbDevice {
    pub fn size_gb(&self) -> f64 {
        self.size_bytes as f64 / 1_000_000_000.0
    }
}

/// Enumerate all block devices that report `removable=1`. Filters out
/// loop devices, ramdisks, and non-disk types.
pub fn list_removable() -> Result<Vec<UsbDevice>> {
    let entries = std::fs::read_dir("/sys/block")
        .context("read /sys/block — is /sys mounted?")?;
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        // Skip ram*, loop*, dm-*, sr* (optical).
        if name.starts_with("ram")
            || name.starts_with("loop")
            || name.starts_with("dm-")
            || name.starts_with("sr")
            || name.starts_with("zram")
        {
            continue;
        }
        let block_dir = entry.path();
        if let Some(dev) = read_one(&block_dir, &name) {
            if dev.removable {
                out.push(dev);
            }
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

/// Look up one specific device by path (e.g. "/dev/sdb"). Used when
/// the user passes --target on the command line. Returns None if the
/// path isn't a block device under /sys/block.
pub fn lookup(device_path: &std::path::Path) -> Option<UsbDevice> {
    let name = device_path.file_name()?.to_string_lossy().to_string();
    let block_dir = PathBuf::from("/sys/block").join(&name);
    if !block_dir.exists() {
        return None;
    }
    read_one(&block_dir, &name)
}

fn read_one(block_dir: &std::path::Path, name: &str) -> Option<UsbDevice> {
    let removable = read_trimmed(&block_dir.join("removable")) == Some("1".into());
    let sectors: u64 = read_trimmed(&block_dir.join("size"))?.parse().ok()?;
    let size_bytes = sectors * 512;
    let vendor = read_trimmed(&block_dir.join("device/vendor")).unwrap_or_default();
    let model = read_trimmed(&block_dir.join("device/model")).unwrap_or_default();
    Some(UsbDevice {
        path: PathBuf::from(format!("/dev/{name}")),
        size_bytes,
        vendor,
        model,
        removable,
    })
}

fn read_trimmed(path: &std::path::Path) -> Option<String> {
    std::fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

/// Refuses if the requested device looks suspicious:
/// - doesn't exist
/// - not removable (system disk?)
/// - mounted (any partition currently in /proc/mounts)
pub fn safety_check(device: &UsbDevice) -> Result<()> {
    if !device.path.exists() {
        anyhow::bail!("device {} does not exist", device.path.display());
    }
    if !device.removable {
        anyhow::bail!(
            "device {} is NOT marked removable. Refusing to overwrite \
             — pass --force-non-removable if you really mean this",
            device.path.display()
        );
    }
    // Reject if any partition of this disk is currently mounted.
    let mounts = std::fs::read_to_string("/proc/mounts").context("read /proc/mounts")?;
    let prefix = format!("{} ", device.path.display());
    for line in mounts.lines() {
        if line.starts_with(&prefix)
            || line.starts_with(&format!("{}1 ", device.path.display()))
            || line.starts_with(&format!("{}2 ", device.path.display()))
        {
            anyhow::bail!(
                "device {} or one of its partitions is currently mounted. \
                 unmount before installing.",
                device.path.display()
            );
        }
    }
    Ok(())
}
