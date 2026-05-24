//! WriteOnce OS — UEFI bootloader (Phase 6 final).
//!
//! Delegates kernel handover to Linux's own EFI stub (compiled into the
//! kernel via `CONFIG_EFI_STUB=y`). The bootloader's job reduces to:
//!
//!   1. Locate the ESP via the loaded-image protocol.
//!   2. Read `\EFI\WriteOnce\cmdline.txt` from the ESP.
//!   3. Append `initrd=\EFI\WriteOnce\initramfs.img` so the EFI stub
//!      loads the initramfs itself (kernel 5.7+ supports this).
//!   4. Read `\EFI\WriteOnce\bzImage` into a buffer.
//!   5. `LoadImage(FromBuffer)` the bzImage as a regular EFI application.
//!   6. Set its load options to the UTF-16-encoded full cmdline.
//!   7. `StartImage` and never return.
//!
//! Same pattern as systemd-boot. The kernel's own EFI stub handles
//! `boot_params`, the EFI memory-map → e820 translation, and
//! `ExitBootServices()` — none of which we need to re-implement.

#![no_std]
#![no_main]

extern crate alloc;

use uefi::prelude::*;
use uefi::proto::loaded_image::LoadedImage;
use uefi::proto::media::file::{File, FileAttribute, FileInfo, FileMode, RegularFile};
use uefi::proto::media::fs::SimpleFileSystem;
use uefi::{CStr16, Status};

use alloc::format;
use alloc::vec::Vec;

const KERNEL_PATH:   &str = "EFI\\WriteOnce\\bzImage";
const INITRD_PATH:   &str = "EFI\\WriteOnce\\initramfs.img";
const CMDLINE_PATH:  &str = "EFI\\WriteOnce\\cmdline.txt";

#[entry]
fn main() -> Status {
    uefi::helpers::init().expect("init UEFI helpers");

    log::info!("=================================================");
    log::info!("  WriteOnce bootloader (Phase 6)");
    log::info!("=================================================");

    let image_handle = uefi::boot::image_handle();
    let device_handle = {
        let loaded_image = uefi::boot::open_protocol_exclusive::<LoadedImage>(image_handle)
            .expect("LoadedImage on our own handle");
        loaded_image.device().expect("loaded image has device handle")
    };

    let mut fs = uefi::boot::open_protocol_exclusive::<SimpleFileSystem>(device_handle)
        .expect("SimpleFileSystem on the boot device");
    let mut root = fs.open_volume().expect("open volume root");

    // 1. cmdline.txt (one line of kernel cmdline).
    let cmdline_bytes = read_file(&mut root, CMDLINE_PATH)
        .expect("read cmdline.txt");
    let cmdline_in = core::str::from_utf8(&cmdline_bytes)
        .unwrap_or("")
        .trim_end_matches(['\r', '\n', '\0']);

    // 2. Append `initrd=...` so the EFI stub loads the initramfs.
    let full_cmdline = if cmdline_in.contains("initrd=") {
        alloc::string::String::from(cmdline_in)
    } else {
        format!("{cmdline_in} initrd=\\{INITRD_PATH}")
    };
    log::info!("  cmdline:  {full_cmdline}");

    // 3. Load the kernel into a UEFI buffer.
    let kernel_bytes = read_file(&mut root, KERNEL_PATH)
        .expect("read bzImage");
    log::info!("  bzImage:  {} bytes", kernel_bytes.len());

    // Make sure initramfs.img exists too (the EFI stub will read it later).
    match read_file_size(&mut root, INITRD_PATH) {
        Ok(n)  => log::info!("  initrd:   {n} bytes (will be loaded by EFI stub)"),
        Err(s) => log::warn!("  initrd:   could not stat ({s:?}) — kernel boot will likely fail"),
    }

    // 4. Load the kernel as an EFI application.
    let kernel_handle = uefi::boot::load_image(
        image_handle,
        uefi::boot::LoadImageSource::FromBuffer {
            buffer:    &kernel_bytes,
            file_path: None,
        },
    ).expect("LoadImage(bzImage)");

    // 5. Set its load options (UTF-16-encoded cmdline; UEFI convention).
    let cmdline_utf16: Vec<u16> = full_cmdline.encode_utf16()
        .chain(core::iter::once(0u16))
        .collect();
    {
        let mut kernel_image = uefi::boot::open_protocol_exclusive::<LoadedImage>(kernel_handle)
            .expect("LoadedImage on kernel handle");
        // SAFETY: cmdline_utf16 outlives the kernel image (held in scope until
        // start_image, which transfers ownership of the load-options bytes).
        unsafe {
            kernel_image.set_load_options(
                cmdline_utf16.as_ptr() as *const u8,
                (cmdline_utf16.len() * 2) as u32,
            );
        }
    }

    // 6. Hand off. StartImage doesn't return on success.
    log::info!("  starting kernel...");
    let status = uefi::boot::start_image(kernel_handle);

    // If we got here, the kernel rejected its image (bad bzImage / wrong arch
    // / EFI stub disabled / etc.). Log and bail.
    log::error!("kernel start_image returned: {status:?}");
    uefi::boot::stall(5_000_000);
    Status::LOAD_ERROR
}

// ----------------------------------------------------------------------------
// Helpers
// ----------------------------------------------------------------------------

fn read_file(
    root: &mut uefi::proto::media::file::Directory,
    path_str: &str,
) -> Result<Vec<u8>, Status> {
    let mut file = open_path(root, path_str)?;

    let mut info_buf = [0u8; 512];
    let info = file
        .get_info::<FileInfo>(&mut info_buf)
        .map_err(|e| e.status())?;
    let size = info.file_size() as usize;

    let mut buf = alloc::vec![0u8; size];
    read_full(&mut file, &mut buf)?;
    Ok(buf)
}

fn read_file_size(
    root: &mut uefi::proto::media::file::Directory,
    path_str: &str,
) -> Result<u64, Status> {
    let mut file = open_path(root, path_str)?;
    let mut info_buf = [0u8; 512];
    let info = file
        .get_info::<FileInfo>(&mut info_buf)
        .map_err(|e| e.status())?;
    Ok(info.file_size())
}

fn open_path(
    root: &mut uefi::proto::media::file::Directory,
    path_str: &str,
) -> Result<RegularFile, Status> {
    let mut path_buf: [u16; 256] = [0; 256];
    let path_c16 = CStr16::from_str_with_buf(path_str, &mut path_buf)
        .map_err(|_| Status::INVALID_PARAMETER)?;
    let file = root
        .open(path_c16, FileMode::Read, FileAttribute::empty())
        .map_err(|e| e.status())?;
    file.into_regular_file().ok_or(Status::INVALID_PARAMETER)
}

fn read_full(file: &mut RegularFile, buf: &mut [u8]) -> Result<(), Status> {
    let mut total = 0;
    while total < buf.len() {
        let n = file.read(&mut buf[total..]).map_err(|e| e.status())?;
        if n == 0 { break; }
        total += n;
    }
    Ok(())
}
