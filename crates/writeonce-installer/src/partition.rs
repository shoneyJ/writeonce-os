// partition.rs — GPT layout via sgdisk.
//
// Two-partition scheme:
//   /dev/sdX1   FAT32 ESP, 512 MiB, type EF00
//   /dev/sdX2   ext4 root, rest of disk, type 8300
//
// We deliberately shell out to sgdisk rather than writing the GPT
// header in-process — sgdisk is on every modern distro and gets the
// kernel's BLKRRPART re-read for free.

use anyhow::{Context, Result};
use std::path::Path;
use tokio::process::Command;

use crate::spec::PartitionPlan;

/// Wipe + repartition `device` with the WriteOnce GPT layout per `plan`.
pub async fn create_gpt(device: &Path, plan: &PartitionPlan) -> Result<()> {
    log::info!("Wiping existing partition table on {}", device.display());
    sgdisk(&["--zap-all", device.to_str().unwrap()]).await?;

    let esp_arg = format!("--new=1:0:+{}M", plan.esp_mib);
    let root_arg = match plan.root_gib {
        None => "--new=2:0:0".to_string(),         // use rest
        Some(g) => format!("--new=2:0:+{}G", g),    // explicit size
    };
    log::info!(
        "Creating ESP ({} MiB, FAT32) + root ({}) partitions",
        plan.esp_mib,
        plan.root_gib.map(|g| format!("{g} GiB")).unwrap_or_else(|| "rest".into())
    );
    sgdisk(&[
        &esp_arg,
        "--typecode=1:EF00",
        "--change-name=1:WRITEONCE-ESP",
        &root_arg,
        "--typecode=2:8300",
        "--change-name=2:writeonce-root",
        device.to_str().unwrap(),
    ])
    .await?;

    // Force kernel re-read of the new partition table.
    let _ = Command::new("partprobe")
        .arg(device)
        .status()
        .await;
    // Settle.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    Ok(())
}

async fn sgdisk(args: &[&str]) -> Result<()> {
    let status = Command::new("sgdisk")
        .args(args)
        .status()
        .await
        .context("spawn sgdisk — install gptfdisk on the host")?;
    if !status.success() {
        anyhow::bail!("sgdisk {:?} failed with status {status}", args);
    }
    Ok(())
}

/// Return device paths for the ESP and the root partition given the
/// parent disk. Handles both /dev/sdX1 (normal) and /dev/nvme0n1p1
/// (NVMe-style) naming.
pub fn partition_paths(device: &Path) -> (std::path::PathBuf, std::path::PathBuf) {
    let s = device.to_string_lossy();
    let (esp, root) = if s.ends_with(|c: char| c.is_ascii_digit()) {
        // nvme0n1 → nvme0n1p1, nvme0n1p2
        (format!("{}p1", s), format!("{}p2", s))
    } else {
        // sdb → sdb1, sdb2
        (format!("{}1", s), format!("{}2", s))
    };
    (esp.into(), root.into())
}
