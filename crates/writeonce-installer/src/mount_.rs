// mount_.rs — mount + umount wrappers.
//
// Named with trailing underscore so it doesn't shadow std::mount.
// Shells out to mount(8) and umount(8) since those handle the kernel
// API + /etc/mtab dance + retries cleanly.

use anyhow::{Context, Result};
use std::path::Path;
use tokio::process::Command;

pub async fn mount(source: &Path, target: &Path, fs_type: &str) -> Result<()> {
    log::info!("mount -t {fs_type} {} {}", source.display(), target.display());
    std::fs::create_dir_all(target).context("create mount point")?;
    let status = Command::new("mount")
        .args([
            "-t",
            fs_type,
            source.to_str().unwrap(),
            target.to_str().unwrap(),
        ])
        .status()
        .await
        .context("spawn mount")?;
    if !status.success() {
        anyhow::bail!(
            "mount -t {fs_type} {} {} failed: {status}",
            source.display(),
            target.display()
        );
    }
    Ok(())
}

pub async fn umount(target: &Path) -> Result<()> {
    log::info!("umount {}", target.display());
    let status = Command::new("umount").arg(target).status().await?;
    if !status.success() {
        log::warn!("umount {} returned {status} — retrying with -l", target.display());
        let lazy = Command::new("umount").args(["-l", target.to_str().unwrap()]).status().await?;
        if !lazy.success() {
            anyhow::bail!("umount -l {} failed: {lazy}", target.display());
        }
    }
    Ok(())
}

/// RAII guard: unmounts on drop. Useful for ensuring the ESP comes
/// down even if subsequent steps fail.
pub struct MountGuard {
    pub target: std::path::PathBuf,
    active: bool,
}

impl MountGuard {
    pub fn new(target: std::path::PathBuf) -> Self {
        Self { target, active: true }
    }
    pub fn forget(mut self) {
        self.active = false;
    }
}

impl Drop for MountGuard {
    fn drop(&mut self) {
        if !self.active {
            return;
        }
        // We can't .await in drop. Spawn a blocking thread that runs
        // umount synchronously. Errors are logged, not propagated.
        let target = self.target.clone();
        let _ = std::process::Command::new("umount").arg(&target).status();
    }
}
