// mkfs.rs — format ESP + root partitions.
//
// Shells out to mkfs.vfat (dosfstools) and mkfs.ext4 (e2fsprogs).
// Captures the ext4 UUID so the bootloader cmdline can reference
// root=UUID=<uuid> rather than /dev/sdX2 which can change.

use anyhow::{Context, Result};
use std::path::Path;
use tokio::process::Command;

pub async fn format_esp(esp: &Path) -> Result<()> {
    log::info!("Formatting {} as FAT32 (label WRITEONCE-ESP)", esp.display());
    let status = Command::new("mkfs.vfat")
        .args(["-F", "32", "-n", "WRITEONCE", esp.to_str().unwrap()])
        .status()
        .await
        .context("spawn mkfs.vfat — install dosfstools")?;
    if !status.success() {
        anyhow::bail!("mkfs.vfat failed: {status}");
    }
    Ok(())
}

pub async fn format_root(root: &Path) -> Result<String> {
    log::info!("Formatting {} as ext4 (label writeonce-root)", root.display());
    let status = Command::new("mkfs.ext4")
        .args([
            "-F",
            "-L",
            "writeonce-root",
            "-E",
            "lazy_itable_init=0,lazy_journal_init=0", // pay cost now, not on first boot
            root.to_str().unwrap(),
        ])
        .status()
        .await
        .context("spawn mkfs.ext4 — install e2fsprogs")?;
    if !status.success() {
        anyhow::bail!("mkfs.ext4 failed: {status}");
    }
    read_ext4_uuid(root).await
}

async fn read_ext4_uuid(device: &Path) -> Result<String> {
    let out = Command::new("blkid")
        .args(["-s", "UUID", "-o", "value", device.to_str().unwrap()])
        .output()
        .await
        .context("spawn blkid — install util-linux")?;
    if !out.status.success() {
        anyhow::bail!(
            "blkid failed: {} {}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let uuid = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if uuid.is_empty() {
        anyhow::bail!("blkid returned empty UUID for {}", device.display());
    }
    log::info!("Root ext4 UUID: {uuid}");
    Ok(uuid)
}
