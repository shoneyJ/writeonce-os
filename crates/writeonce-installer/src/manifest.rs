// manifest.rs — the artifacts bundle metadata.
//
// The Docker-driven build pipeline produces a directory containing:
//   bzImage           (kernel)
//   initramfs.img     (initramfs)
//   BOOTX64.EFI       (writeonce-bootloader)
//   sysroot.tar.zst   (root filesystem, zstd-compressed tar)
//   manifest.toml     (this struct, serialized)

use anyhow::{Context, Result};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize)]
pub struct Manifest {
    pub schema_version: String,
    pub image: ImageSection,
    pub verification: VerificationSection,
    #[serde(default)]
    pub metadata: MetadataSection,
}

#[derive(Debug, Deserialize)]
pub struct ImageSection {
    pub kernel: String,
    pub initramfs: String,
    pub bootloader: String,
    pub sysroot: String,
    /// Kernel command line template. Use `__ROOT_UUID__` as the
    /// placeholder for the root partition UUID; the installer
    /// substitutes after mkfs.
    pub cmdline: String,
}

#[derive(Debug, Deserialize)]
pub struct VerificationSection {
    pub kernel_sha256: String,
    pub initramfs_sha256: String,
    pub bootloader_sha256: String,
    pub sysroot_sha256: String,
}

#[derive(Debug, Default, Deserialize)]
#[allow(dead_code)] // Fields are human-inspected via the manifest itself, not by code.
pub struct MetadataSection {
    #[serde(default)]
    pub build_key: String,
    #[serde(default)]
    pub built_at: String,
    #[serde(default)]
    pub writeonce_git_sha: String,
}

impl Manifest {
    pub fn load(dir: &Path) -> Result<(Self, PathBuf)> {
        let path = dir.join("manifest.toml");
        let raw =
            std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        let manifest: Self = toml::from_str(&raw).context("parse manifest.toml")?;
        Ok((manifest, path))
    }

    pub fn resolve(&self, dir: &Path) -> ResolvedArtifacts {
        ResolvedArtifacts {
            kernel: dir.join(&self.image.kernel),
            initramfs: dir.join(&self.image.initramfs),
            bootloader: dir.join(&self.image.bootloader),
            sysroot: dir.join(&self.image.sysroot),
            cmdline_template: self.image.cmdline.clone(),
            kernel_sha: self.verification.kernel_sha256.clone(),
            initramfs_sha: self.verification.initramfs_sha256.clone(),
            bootloader_sha: self.verification.bootloader_sha256.clone(),
            sysroot_sha: self.verification.sysroot_sha256.clone(),
        }
    }
}

#[derive(Debug)]
pub struct ResolvedArtifacts {
    pub kernel: PathBuf,
    pub initramfs: PathBuf,
    pub bootloader: PathBuf,
    pub sysroot: PathBuf,
    pub cmdline_template: String,
    pub kernel_sha: String,
    pub initramfs_sha: String,
    pub bootloader_sha: String,
    pub sysroot_sha: String,
}

impl ResolvedArtifacts {
    /// Verify each artifact's SHA-256 against the manifest before any
    /// destructive operation. If this fails the installer aborts
    /// before touching the USB.
    pub async fn verify_against_manifest(&self) -> Result<()> {
        for (label, path, expected) in [
            ("kernel", &self.kernel, &self.kernel_sha),
            ("initramfs", &self.initramfs, &self.initramfs_sha),
            ("bootloader", &self.bootloader, &self.bootloader_sha),
            ("sysroot", &self.sysroot, &self.sysroot_sha),
        ] {
            let actual = sha256_file_async(path).await?;
            if !actual.eq_ignore_ascii_case(expected) {
                anyhow::bail!(
                    "SHA-256 mismatch on {label}: manifest says {expected}, got {actual}"
                );
            }
            log::info!("{label} SHA-256 verified ({})", path.display());
        }
        Ok(())
    }
}

pub async fn sha256_file_async(path: &Path) -> Result<String> {
    let path = path.to_path_buf();
    tokio::task::spawn_blocking(move || sha256_file(&path))
        .await
        .context("spawn_blocking join")?
}

pub fn sha256_file(path: &Path) -> Result<String> {
    use std::io::Read;
    let mut f = std::fs::File::open(path)
        .with_context(|| format!("open {}", path.display()))?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 1 << 16];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(format!("{:x}", hasher.finalize()))
}
