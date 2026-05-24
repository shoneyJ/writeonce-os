// extract.rs — stream a sysroot.tar.zst onto a mounted target.
//
// Pipeline: read tarball → zstd decompress → tar extract → write.
// All in-process so we can drive a progress bar from the tar reader.
//
// Uses spawn_blocking because tar + zstd are sync APIs; running them
// inside a tokio task without spawn_blocking would stall the runtime.

use anyhow::{Context, Result};
use indicatif::{ProgressBar, ProgressStyle};
use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

pub async fn extract_sysroot(tarball: &Path, target_root: &Path) -> Result<()> {
    let tarball = tarball.to_path_buf();
    let target_root = target_root.to_path_buf();
    let total = std::fs::metadata(&tarball)?.len();

    tokio::task::spawn_blocking(move || extract_sync(&tarball, &target_root, total))
        .await
        .context("spawn_blocking join")?
}

fn extract_sync(tarball: &Path, target_root: &Path, total: u64) -> Result<()> {
    let f = File::open(tarball).context("open sysroot tarball")?;

    // Wrap the file in a counting reader so we can drive a progress
    // bar based on bytes consumed from the input stream (close enough
    // to "how much have we done" without per-file accounting).
    let pb = ProgressBar::new(total);
    pb.set_style(
        ProgressStyle::with_template(
            "  {spinner} {wide_bar} {bytes:>10}/{total_bytes:<10} ({eta})",
        )
        .unwrap()
        .progress_chars("=> "),
    );
    let counting = CountingReader {
        inner: BufReader::new(f),
        pb: pb.clone(),
    };

    let zstd = zstd::stream::Decoder::new(counting).context("zstd::Decoder")?;
    let mut archive = tar::Archive::new(zstd);
    archive.set_preserve_permissions(true);
    archive.set_preserve_ownerships(true);
    archive
        .unpack(target_root)
        .context("tar::Archive::unpack")?;
    pb.finish_with_message("sysroot extracted");
    Ok(())
}

struct CountingReader<R: Read> {
    inner: R,
    pb: ProgressBar,
}

impl<R: Read> Read for CountingReader<R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let n = self.inner.read(buf)?;
        self.pb.inc(n as u64);
        Ok(n)
    }
}
