//! Discover the root block device given a `RootSpec`.
//!
//! Strategy:
//!   - `Device("/dev/sdaN")` — return it directly (assuming devtmpfs has
//!     populated /dev).
//!   - `Uuid(u)` / `Label(l)` — walk `/sys/class/block/*`, read each
//!     partition's `dev` major:minor, mknod the corresponding `/dev`
//!     node if missing, then run `blkid`-equivalent (read partition
//!     superblock magic bytes) to match UUID/LABEL.
//!   - `PartUuid(u)` — same, but using the partition table's UUID
//!     stored in `/sys/class/block/<name>/uuid`.
//!
//! For the WriteOnce minimum we implement `Device` directly and stub
//! the UUID/LABEL probes. The supervisor's first boot uses devtmpfs +
//! a `root=/dev/sda3` cmdline; the UUID/LABEL paths get filled in once
//! we ship a real initramfs that lacks devtmpfs autopopulation.

use std::fs;
use std::path::PathBuf;

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

pub fn locate_root(spec: &RootSpec) -> Result<PathBuf, DiscoverError> {
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
        RootSpec::Uuid(_) | RootSpec::Label(_) => {
            // Filesystem UUID / LABEL probes require reading the FS
            // superblock; deferred. For Phase 5 boot, prefer
            // root=PARTUUID=... or root=/dev/sdaN.
            Err(DiscoverError::Unsupported(
                "filesystem UUID / LABEL probes not yet implemented; use \
                 root=PARTUUID=... or root=/dev/sdaN".into()
            ))
        }
    }
}
