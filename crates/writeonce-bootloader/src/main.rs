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
use uefi::proto::BootPolicy;
use uefi::proto::device_path::build::{self, DevicePathBuilder};
use uefi::proto::device_path::{DevicePath, DeviceSubType, DeviceType, LoadedImageDevicePath};
use uefi::proto::loaded_image::LoadedImage;
use uefi::proto::media::file::{File, FileAttribute, FileInfo, FileMode, RegularFile};
use uefi::proto::media::fs::SimpleFileSystem;
use uefi::{CStr16, Status, cstr16};

use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::fmt::Write;

const KERNEL_PATH:   &str = "EFI\\WriteOnce\\bzImage";
const INITRD_PATH:   &str = "EFI\\WriteOnce\\initramfs.img";
const CMDLINE_PATH:  &str = "EFI\\WriteOnce\\cmdline.txt";
const LOG_PATH:      &str = "EFI\\WriteOnce\\boot.log";

#[entry]
fn main() -> Status {
    // Run uefi init first so log:: works for everything else.
    let _ = uefi::helpers::init();

    let mut buf = String::new();
    let result = try_boot(&mut buf);

    // Persist the in-memory log to \EFI\WriteOnce\boot.log on the ESP.
    // If FS open failed this is a no-op (and the operator can still
    // photograph the screen). If FS open succeeded the file will hold
    // every step we logged, including the failure marker we add below.
    if let Err((step, status)) = result {
        let _ = writeln!(buf,
            "FATAL: step={step} status={status:?}");
    }
    let _ = persist_log(&buf);

    match result {
        Ok(()) => Status::LOAD_ERROR,    // unreachable on success path
        Err((step, status)) => {
            // Banner + step + status, then HALT FOREVER so the user can
            // photograph the screen. Firmware would otherwise scroll the
            // panic message off the display and return to its boot menu.
            log::error!("");
            log::error!("=================================================");
            log::error!("  WriteOnce bootloader: FATAL");
            log::error!("=================================================");
            log::error!("  step:    {step}");
            log::error!("  status:  {status:?}");
            log::error!("  log:     \\EFI\\WriteOnce\\boot.log (if FS opened)");
            log::error!("=================================================");
            log::error!("  Halted. Reset the machine to retry.");
            log::error!("=================================================");
            loop { uefi::boot::stall(10_000_000); }
        }
    }
}

/// say!: log to UEFI console AND append to the in-memory log buffer.
/// Format args identical to log::info!.
macro_rules! say {
    ($buf:expr, $($t:tt)*) => {{
        let s = alloc::format!($($t)*);
        log::info!("{}", &s);
        $buf.push_str(&s);
        $buf.push('\n');
    }};
}

/// Total numbered steps the user sees. Update when adding/removing steps.
const N_STEPS: u8 = 7;

/// Format human-readable byte counts (1234567 → "1.18 MiB").
fn fmt_bytes(n: u64) -> alloc::string::String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    if n >= MB { alloc::format!("{:.2} MiB ({} bytes)", n as f64 / MB as f64, n) }
    else if n >= KB { alloc::format!("{:.2} KiB ({} bytes)", n as f64 / KB as f64, n) }
    else { alloc::format!("{n} bytes") }
}

/// Every step that can fail returns `Err((step_name, uefi_status))`. The
/// step_name string ends up on screen on failure — keep it short + unique.
fn try_boot(buf: &mut String) -> Result<(), (&'static str, Status)> {
    say!(buf, "");
    say!(buf, "==========================================================");
    say!(buf, "  WriteOnce OS — UEFI bootloader (Phase 6)");
    say!(buf, "==========================================================");
    say!(buf, "");

    // ---- [1/N] Locate ESP -------------------------------------------------
    say!(buf, "[1/{N_STEPS}] Locating EFI System Partition (boot device)...");
    let image_handle = uefi::boot::image_handle();
    let device_handle = {
        let loaded_image = uefi::boot::open_protocol_exclusive::<LoadedImage>(image_handle)
            .map_err(|e| ("open LoadedImage(self)", e.status()))?;
        loaded_image.device().ok_or(("loaded image has no device", Status::UNSUPPORTED))?
    };
    say!(buf, "        ok — loaded from device handle {:p}",
        device_handle.as_ptr());

    // ---- [2/N] Open ESP filesystem ----------------------------------------
    say!(buf, "[2/{N_STEPS}] Opening ESP filesystem (SimpleFileSystem)...");
    let mut fs = uefi::boot::open_protocol_exclusive::<SimpleFileSystem>(device_handle)
        .map_err(|e| ("open SimpleFileSystem (boot device has no ESP?)", e.status()))?;
    let mut root = fs.open_volume()
        .map_err(|e| ("open ESP root volume", e.status()))?;
    say!(buf, "        ok");

    // ---- [3/N] Read cmdline.txt ------------------------------------------
    say!(buf, "[3/{N_STEPS}] Reading kernel cmdline from \\{CMDLINE_PATH}...");
    let cmdline_bytes = read_file(&mut root, CMDLINE_PATH)
        .map_err(|s| ("read \\EFI\\WriteOnce\\cmdline.txt", s))?;
    let cmdline_in = core::str::from_utf8(&cmdline_bytes)
        .unwrap_or("")
        .trim_end_matches(['\r', '\n', '\0']);
    let full_cmdline = if cmdline_in.contains("initrd=") {
        String::from(cmdline_in)
    } else {
        format!("{cmdline_in} initrd=\\{INITRD_PATH}")
    };
    say!(buf, "        ok — {} characters", full_cmdline.len());
    say!(buf, "        cmdline = {full_cmdline}");

    // ---- [4/N] Verify bzImage presence (firmware will load it later) -----
    say!(buf, "[4/{N_STEPS}] Verifying kernel image at \\{KERNEL_PATH}...");
    match read_file_size(&mut root, KERNEL_PATH) {
        Ok(n) => say!(buf, "        ok — {} (firmware will load it via DevicePath)", fmt_bytes(n)),
        Err(s) => return Err(("verify \\EFI\\WriteOnce\\bzImage", s)),
    }

    // ---- [5/N] Verify initramfs presence ---------------------------------
    say!(buf, "[5/{N_STEPS}] Verifying initramfs at \\{INITRD_PATH}...");
    match read_file_size(&mut root, INITRD_PATH) {
        Ok(n) => say!(buf, "        ok — {} (kernel EFI stub will load it)", fmt_bytes(n)),
        Err(s) => say!(buf, "        WARN — could not stat: {s:?} (boot may still fail)"),
    }

    // Drop the FS/root handles we held during reads. The later
    // `persist_log` call needs to re-open SimpleFileSystem.
    drop(root);
    drop(fs);
    // device_handle is no longer needed — firmware sets the kernel's
    // LoadedImage.device_handle directly when we use FromDevicePath.
    let _ = device_handle;

    // ---- [6/N] Load kernel via DevicePath + set cmdline ------------------
    // Build the DevicePath: take our own loader's device path (which
    // identifies this ESP) and replace the trailing file-path nodes
    // with `\EFI\WriteOnce\bzImage`. Firmware loads the file itself,
    // setting both LoadedImage.device_handle AND LoadedImage.file_path
    // correctly — matching what GRUB/systemd-boot/rEFInd do and what
    // strict UEFI firmware (Aptio V on T450) requires for `StartImage`
    // to succeed. Earlier `LoadImage(FromBuffer)` + manual device_handle
    // pointer-patch worked in QEMU/OVMF (lenient) but produced silent
    // failure on T450 after `StartImage`. Reference:
    // .agents/reference/uefi-rs/uefi-test-runner/src/bin/shell_launcher.rs
    say!(buf, "[6/{N_STEPS}] Building DevicePath for kernel + loading via firmware...");
    let mut dp_storage = Vec::new();
    let kernel_device_path = build_kernel_device_path(&mut dp_storage)
        .map_err(|e| ("build kernel DevicePath", e))?;
    let kernel_handle = uefi::boot::load_image(
        image_handle,
        uefi::boot::LoadImageSource::FromDevicePath {
            device_path: kernel_device_path,
            boot_policy: BootPolicy::ExactMatch,
        },
    ).map_err(|e| ("LoadImage(FromDevicePath) bzImage", e.status()))?;
    say!(buf, "        - LoadImage(FromDevicePath) ok — firmware set device_handle + file_path");

    // Set cmdline as UTF-16 load_options (UEFI convention).
    let cmdline_utf16: Vec<u16> = full_cmdline.encode_utf16()
        .chain(core::iter::once(0u16))
        .collect();
    {
        let mut kernel_image = uefi::boot::open_protocol_exclusive::<LoadedImage>(kernel_handle)
            .map_err(|e| ("open LoadedImage(kernel)", e.status()))?;
        // SAFETY: cmdline_utf16 outlives the kernel image (held in scope until
        // start_image, which transfers ownership of the load-options bytes).
        unsafe {
            kernel_image.set_load_options(
                cmdline_utf16.as_ptr() as *const u8,
                (cmdline_utf16.len() * 2) as u32,
            );
        }
        say!(buf, "        - set kernel load_options ({} UTF-16 chars)",
            cmdline_utf16.len() - 1);
    }
    say!(buf, "        ok");

    // ---- [7/N] Hand off to the kernel ------------------------------------
    say!(buf, "[7/{N_STEPS}] Handing off to kernel...");
    say!(buf, "");
    say!(buf, "  +---------------------------------------------------+");
    say!(buf, "  |  About to call StartImage(kernel).                |");
    say!(buf, "  |                                                   |");
    say!(buf, "  |  The screen will briefly go BLANK while the EFI   |");
    say!(buf, "  |  stub calls ExitBootServices() and the kernel     |");
    say!(buf, "  |  takes over the display. This can last 10-30 sec  |");
    say!(buf, "  |  on older hardware. Do not power off.             |");
    say!(buf, "  |                                                   |");
    say!(buf, "  |  After the blank, you should see kernel boot      |");
    say!(buf, "  |  messages starting with [    0.000000] ...        |");
    say!(buf, "  |                                                   |");
    say!(buf, "  |  If nothing appears within 60 seconds, hold the   |");
    say!(buf, "  |  power button 5 sec, mount the USB on your dev    |");
    say!(buf, "  |  machine and read \\EFI\\WriteOnce\\boot.log.      |");
    say!(buf, "  +---------------------------------------------------+");
    say!(buf, "");

    // Flush log to ESP — last chance before kernel takeover.
    say!(buf, "        flushing log to \\{LOG_PATH}...");
    let _ = persist_log(buf);

    // Give the user ~3 seconds to read the notice above. UEFI stall is
    // in microseconds.
    uefi::boot::stall(3_000_000);

    say!(buf, "        StartImage(kernel) — see you on the other side.");
    let status = uefi::boot::start_image(kernel_handle)
        .err()
        .map(|e| e.status())
        .unwrap_or(Status::ABORTED);

    // If we got here, the kernel rejected its image.
    Err(("start_image returned (kernel rejected image)", status))
}

/// Build the DevicePath for `\EFI\WriteOnce\bzImage` by taking our own
/// loader's device path (the firmware-built DevicePath that loaded
/// BOOTX64.EFI) and replacing its trailing MEDIA_FILE_PATH nodes with
/// the bzImage path. `storage` is reused as the backing buffer; the
/// returned `&DevicePath` borrows from it.
///
/// This is the same idiom uefi-rs's `shell_launcher.rs` uses to load
/// the EFI shell from the same ESP it was itself loaded from — and
/// the same idiom systemd-boot uses to load the kernel.
fn build_kernel_device_path(storage: &mut Vec<u8>) -> Result<&DevicePath, Status> {
    let our_path = uefi::boot::open_protocol_exclusive::<LoadedImageDevicePath>(
        uefi::boot::image_handle(),
    ).map_err(|e| e.status())?;

    let mut builder = DevicePathBuilder::with_vec(storage);
    // Copy every node from our own path EXCEPT the MEDIA_FILE_PATH
    // node(s) at the end (which point at BOOTX64.EFI). What remains
    // is the path to the ESP device itself.
    for node in our_path.node_iter() {
        if node.full_type() == (DeviceType::MEDIA, DeviceSubType::MEDIA_FILE_PATH) {
            break;
        }
        builder = builder.push(&node).map_err(|_| Status::OUT_OF_RESOURCES)?;
    }
    // Append the file-path node pointing at the kernel.
    builder = builder.push(&build::media::FilePath {
        path_name: cstr16!(r"\EFI\WriteOnce\bzImage"),
    }).map_err(|_| Status::OUT_OF_RESOURCES)?;
    builder.finalize().map_err(|_| Status::OUT_OF_RESOURCES)
}

/// Open the boot device's ESP, write `buf` to \EFI\WriteOnce\boot.log,
/// overwriting any previous content. Best-effort; errors are returned but
/// the caller is expected to ignore them (we're often in a halt path).
fn persist_log(buf: &str) -> Result<(), Status> {
    use uefi::proto::media::file::{File, FileAttribute, FileMode};
    let image_handle = uefi::boot::image_handle();
    let device_handle = {
        let loaded_image = uefi::boot::open_protocol_exclusive::<LoadedImage>(image_handle)
            .map_err(|e| e.status())?;
        loaded_image.device().ok_or(Status::UNSUPPORTED)?
    };
    let mut fs = uefi::boot::open_protocol_exclusive::<SimpleFileSystem>(device_handle)
        .map_err(|e| e.status())?;
    let mut root = fs.open_volume().map_err(|e| e.status())?;

    let mut path_buf = [0u16; 256];
    let path = CStr16::from_str_with_buf(LOG_PATH, &mut path_buf)
        .map_err(|_| Status::INVALID_PARAMETER)?;
    // CreateReadWrite opens-or-creates; we then truncate by deleting and
    // re-creating to keep file size in sync with buf.len(). Simpler than
    // SetInfo(EFI_FILE_INFO.FileSize).
    if let Ok(existing) = root.open(path, FileMode::CreateReadWrite, FileAttribute::empty()) {
        if let Some(f) = existing.into_regular_file() {
            let _ = f.delete();
        }
    }
    let file = root.open(path, FileMode::CreateReadWrite, FileAttribute::empty())
        .map_err(|e| e.status())?;
    let mut file = file.into_regular_file().ok_or(Status::INVALID_PARAMETER)?;
    file.write(buf.as_bytes()).map_err(|e| e.status())?;
    file.flush().map_err(|e| e.status())?;
    Ok(())
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
