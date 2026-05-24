// verify.rs — post-write SHA reread.
//
// After unmounting, re-mount read-only and SHA the files we just
// wrote. Catches USB sticks that silently corrupted blocks during
// write — rare but does happen with cheap drives.

use anyhow::{Context, Result};
use std::path::Path;

use crate::manifest::{sha256_file_async, ResolvedArtifacts};
use crate::mount_;

pub async fn verify_post_write(
    artifacts: &ResolvedArtifacts,
    esp_part: &Path,
    workdir: &Path,
) -> Result<()> {
    let esp_dir = workdir.join("verify-esp");
    log::info!("Re-mounting ESP read-only for verification");
    mount_::mount(esp_part, &esp_dir, "vfat")
        .await
        .context("mount ESP for verification")?;

    let kernel = esp_dir.join("EFI/WriteOnce/bzImage");
    let initramfs = esp_dir.join("EFI/WriteOnce/initramfs.img");
    let bootloader = esp_dir.join("EFI/BOOT/BOOTX64.EFI");

    let result: Result<()> = async {
        let k = sha256_file_async(&kernel).await?;
        if !k.eq_ignore_ascii_case(&artifacts.kernel_sha) {
            anyhow::bail!("re-read SHA mismatch on kernel: expected {} got {}", artifacts.kernel_sha, k);
        }
        log::info!("✓ kernel re-read SHA matches");

        let i = sha256_file_async(&initramfs).await?;
        if !i.eq_ignore_ascii_case(&artifacts.initramfs_sha) {
            anyhow::bail!("re-read SHA mismatch on initramfs: expected {} got {}", artifacts.initramfs_sha, i);
        }
        log::info!("✓ initramfs re-read SHA matches");

        let b = sha256_file_async(&bootloader).await?;
        if !b.eq_ignore_ascii_case(&artifacts.bootloader_sha) {
            anyhow::bail!("re-read SHA mismatch on bootloader: expected {} got {}", artifacts.bootloader_sha, b);
        }
        log::info!("✓ bootloader re-read SHA matches");
        Ok(())
    }
    .await;

    let _ = mount_::umount(&esp_dir).await;
    result
}
