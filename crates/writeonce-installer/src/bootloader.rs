// bootloader.rs — populate the ESP with kernel + initramfs + bootloader.
//
// Final ESP layout:
//   /EFI/BOOT/BOOTX64.EFI            ← UEFI default boot path
//   /EFI/WriteOnce/bzImage           ← kernel
//   /EFI/WriteOnce/initramfs.img     ← initramfs
//   /EFI/WriteOnce/cmdline.txt       ← kernel command line (with root=UUID=…)
//
// The bootloader (writeonce-bootloader, the BOOTX64.EFI file) reads
// cmdline.txt + bzImage + initramfs.img and hands off to the kernel's
// EFI stub. See docs/learning/phase-6-bootloader-efi-stub-delegation.md.

use anyhow::{Context, Result};
use std::path::Path;

pub struct EspLayout<'a> {
    pub esp_mount: &'a Path,
    pub bootloader: &'a Path,
    pub kernel: &'a Path,
    pub initramfs: &'a Path,
    /// Already has root=UUID=… substituted.
    pub cmdline: &'a str,
}

pub async fn populate_esp(layout: &EspLayout<'_>) -> Result<()> {
    let efi_boot = layout.esp_mount.join("EFI/BOOT");
    let efi_wo = layout.esp_mount.join("EFI/WriteOnce");
    tokio::fs::create_dir_all(&efi_boot).await?;
    tokio::fs::create_dir_all(&efi_wo).await?;

    log::info!("Installing bootloader → /EFI/BOOT/BOOTX64.EFI");
    tokio::fs::copy(layout.bootloader, efi_boot.join("BOOTX64.EFI"))
        .await
        .context("copy bootloader")?;

    log::info!("Installing kernel → /EFI/WriteOnce/bzImage");
    tokio::fs::copy(layout.kernel, efi_wo.join("bzImage"))
        .await
        .context("copy kernel")?;

    log::info!("Installing initramfs → /EFI/WriteOnce/initramfs.img");
    tokio::fs::copy(layout.initramfs, efi_wo.join("initramfs.img"))
        .await
        .context("copy initramfs")?;

    log::info!("Writing /EFI/WriteOnce/cmdline.txt");
    tokio::fs::write(efi_wo.join("cmdline.txt"), layout.cmdline)
        .await
        .context("write cmdline.txt")?;

    Ok(())
}

/// Format the final kernel command line with the root partition UUID
/// substituted into the template from manifest.toml. Template uses
/// `__ROOT_UUID__` as the placeholder.
pub fn format_cmdline(template: &str, root_uuid: &str) -> String {
    template.replace("__ROOT_UUID__", root_uuid)
}
