//! Discover the root block device given a `RootSpec`.
//!
//! Strategy:
//!   - `Device("/dev/sdaN")` — return it directly (assuming devtmpfs has
//!     populated /dev).
//!   - `PartUuid(u)` — walk `/sys/class/block/*`, read each entry's
//!     `uuid` file (GPT partition UUID exposed by recent kernels).
//!   - `Uuid(u)` / `Label(l)` — walk `/sys/class/block/*`, open the
//!     corresponding `/dev/<name>`, read the ext4 superblock at
//!     offset 1024, match the volume label (offset 1144 abs) or the
//!     fs UUID (offset 1128 abs). ext2/3/4 use the same layout.
//!     Other filesystems (vfat, xfs) are not probed — WriteOnce's
//!     root is always ext4.

use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::cmdline::RootSpec;

#[derive(Debug)]
pub enum DiscoverError {
    Unsupported(String),
    NotFound(String),
    Io(std::io::Error),
}

impl std::fmt::Display for DiscoverError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DiscoverError::Unsupported(s) => write!(f, "unsupported root spec: {s}"),
            DiscoverError::NotFound(s)    => write!(f, "no block device matched: {s}"),
            DiscoverError::Io(e)          => write!(f, "io: {e}"),
        }
    }
}

impl std::error::Error for DiscoverError {}

impl From<std::io::Error> for DiscoverError {
    fn from(e: std::io::Error) -> Self { DiscoverError::Io(e) }
}

/// Resolve a root device, polling for up to `timeout_secs` seconds if
/// the device hasn't appeared yet.
///
/// Why poll: on hardware where the root partition lives on a USB stick,
/// the kernel's USB stack enumerates asynchronously (`drivers/usb/core/
/// hub.c` defers per-port init onto a `delayed_work` queue with a
/// 100 ms minimum power-on delay; Lenovo Aptio V firmware adds more on
/// top). Our PID 1 races ahead of that — without a wait, we hit
/// `/sys/class/block/*` before the USB partition is in there and drop
/// to the recovery shell every boot.
///
/// systemd's `device.target` defaults to a 90 s timeout; dracut's
/// `rootfs-block/mount-root.sh` uses 30 s. We default to 30 s
/// (configurable via `writeonce.rootwait=N` on the kernel cmdline).
///
/// On first miss we print a single "waiting…" message so the operator
/// sees progress; on success after waiting we report how many polls it
/// took, useful for tuning. On final timeout the underlying
/// `NotFound` propagates and the caller drops to the recovery shell.
pub fn locate_root(spec: &RootSpec, timeout_secs: u64) -> Result<PathBuf, DiscoverError> {
    let deadline = Instant::now() + Duration::from_secs(timeout_secs);
    let mut attempt: u32 = 0;
    loop {
        match locate_root_once(spec) {
            Ok(p) => {
                if attempt > 0 {
                    eprintln!(
                        "writeonce-initramfs: root device found after {attempt} polls (~{} ms)",
                        attempt * 100
                    );
                }
                return Ok(p);
            }
            Err(DiscoverError::NotFound(_)) if Instant::now() < deadline => {
                if attempt == 0 {
                    eprintln!(
                        "writeonce-initramfs: waiting up to {timeout_secs}s for root device to appear..."
                    );
                }
                attempt += 1;
                std::thread::sleep(Duration::from_millis(100));
            }
            Err(e) => return Err(e),
        }
    }
}

fn locate_root_once(spec: &RootSpec) -> Result<PathBuf, DiscoverError> {
    match spec {
        RootSpec::Device(p) => {
            let path = PathBuf::from(p);
            if path.exists() {
                Ok(path)
            } else {
                Err(DiscoverError::NotFound(p.clone()))
            }
        }
        RootSpec::PartUuid(uuid) => {
            // Check /sys/class/block/*/dm/uuid or /sys/dev/block/*/uuid.
            // GPT partition UUIDs are exposed via /sys/class/block/<name>/uuid
            // on recent kernels.
            for entry in fs::read_dir("/sys/class/block")? {
                let entry = entry?;
                let name = entry.file_name();
                let uuid_path = entry.path().join("uuid");
                if let Ok(body) = fs::read_to_string(&uuid_path) {
                    if body.trim().eq_ignore_ascii_case(uuid) {
                        let dev = PathBuf::from("/dev").join(&name);
                        if dev.exists() { return Ok(dev); }
                    }
                }
            }
            Err(DiscoverError::NotFound(uuid.clone()))
        }
        RootSpec::Label(label) => {
            for dev in candidate_block_devices()? {
                if let Some(found) = read_ext4_label(&dev) {
                    if found == *label {
                        return Ok(dev);
                    }
                }
            }
            Err(DiscoverError::NotFound(format!("LABEL={label}")))
        }
        RootSpec::Uuid(uuid) => {
            let want = uuid.to_ascii_lowercase();
            for dev in candidate_block_devices()? {
                if let Some(found) = read_ext4_uuid(&dev) {
                    if found == want {
                        return Ok(dev);
                    }
                }
            }
            Err(DiscoverError::NotFound(format!("UUID={uuid}")))
        }
    }
}

/// Enumerate every block device under `/sys/class/block/*` and map it
/// back to its `/dev/<name>` path. Returns only paths that actually
/// exist (devtmpfs has materialised them).
fn candidate_block_devices() -> Result<Vec<PathBuf>, DiscoverError> {
    let mut out = Vec::new();
    for entry in fs::read_dir("/sys/class/block")? {
        let entry = entry?;
        let dev = PathBuf::from("/dev").join(entry.file_name());
        if dev.exists() {
            out.push(dev);
        }
    }
    Ok(out)
}

/// Read the ext2/3/4 superblock label (`s_volume_name`, 16 bytes at
/// absolute offset 1024+120 = 1144). Verifies the magic number
/// `0xEF53` at offset 1080 first. Returns None for non-ext filesystems
/// or unreadable devices (e.g. opening /dev/sda whole-disk when only
/// /dev/sda2 has an ext4 fs — same code probes both, that's fine).
fn read_ext4_label(dev: &Path) -> Option<String> {
    let buf = read_ext4_superblock(dev)?;
    // s_volume_name lives at superblock offset 120 (post-1024 base).
    let label_bytes = &buf[120..120 + 16];
    let end = label_bytes.iter().position(|&b| b == 0).unwrap_or(16);
    let label = core::str::from_utf8(&label_bytes[..end]).ok()?;
    if label.is_empty() { return None; }
    Some(label.to_string())
}

/// Read the ext2/3/4 superblock UUID (`s_uuid`, 16 bytes at offset
/// 104 of the superblock = absolute offset 1128). Returns the
/// canonical 8-4-4-4-12 hex string in lowercase, matching what
/// `blkid` reports.
fn read_ext4_uuid(dev: &Path) -> Option<String> {
    let buf = read_ext4_superblock(dev)?;
    let u = &buf[104..104 + 16];
    Some(format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        u[0], u[1], u[2], u[3],   u[4], u[5],   u[6], u[7],
        u[8], u[9],   u[10], u[11], u[12], u[13], u[14], u[15],
    ))
}

/// Read the first 1024 bytes of an ext2/3/4 superblock (starting at
/// offset 1024 from the device, since the first 1024 bytes are
/// reserved for boot-sector use). Returns None unless the magic
/// number `0xEF53` is found at superblock offset 56.
fn read_ext4_superblock(dev: &Path) -> Option<Vec<u8>> {
    let mut f = fs::File::open(dev).ok()?;
    f.seek(SeekFrom::Start(1024)).ok()?;
    let mut buf = vec![0u8; 1024];
    f.read_exact(&mut buf).ok()?;
    // s_magic is at superblock offset 56, little-endian.
    let magic = u16::from_le_bytes([buf[56], buf[57]]);
    if magic != 0xEF53 { return None; }
    Some(buf)
}
