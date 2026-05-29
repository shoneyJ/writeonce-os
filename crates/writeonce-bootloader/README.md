# writeonce-bootloader

The WriteOnce OS **UEFI bootloader** (Phase 6). A `no_std`, `x86_64-unknown-uefi`
application that the T450's UEFI firmware loads as `\EFI\BOOT\BOOTX64.EFI`. It is
deliberately thin: it locates the boot ESP, reads the kernel command line, and
hands the kernel to **Linux's own EFI stub** (`CONFIG_EFI_STUB=y`). It does *not*
reimplement `boot_params`, the EFI-memory-map→e820 translation, initrd loading,
or `ExitBootServices()` — the kernel stub does all of that. Same pattern as
systemd-boot.

```
firmware → \EFI\BOOT\BOOTX64.EFI (this crate) → bzImage (EFI stub) → initramfs → writeonce-pid1
```

## What it does

`try_boot()` runs 7 numbered steps (each logged to screen *and* to a buffer that
is persisted to the ESP — see below):

1. **Locate the ESP** — via the loaded-image protocol on its own image handle (the
   device it was loaded from *is* the boot ESP).
2. **Open the ESP filesystem** (`SimpleFileSystem`) and its root volume.
3. **Read `\EFI\WriteOnce\cmdline.txt`** and, unless it already contains `initrd=`,
   append `initrd=\EFI\WriteOnce\initramfs.img` so the kernel's EFI stub loads the
   initramfs itself (kernel ≥ 5.7).
4. **Verify** the kernel image `\EFI\WriteOnce\bzImage` is present (stat its size).
5. **Verify** `\EFI\WriteOnce\initramfs.img` is present (warn-only — boot may still proceed).
6. **Load the kernel** via `LoadImage(FromDevicePath)`, then set its `load_options`
   to the UTF-16-encoded full command line.
7. **Hand off** with `StartImage(kernel)` — never returns on success.

### ESP layout it expects

Written by `writeonce-installer` (`crates/writeonce-installer/src/bootloader.rs`):

```
\EFI\BOOT\BOOTX64.EFI          ← this bootloader (UEFI default boot path)
\EFI\WriteOnce\bzImage         ← kernel (CONFIG_EFI_STUB=y)
\EFI\WriteOnce\initramfs.img   ← initramfs
\EFI\WriteOnce\cmdline.txt     ← kernel command line (root=UUID=… already substituted)
\EFI\WriteOnce\boot.log        ← written by this bootloader on every boot
```

## Build

`no_std`, targets `x86_64-unknown-uefi`, output is a PE/COFF `.efi` the firmware
loads directly. There is no `.cargo` alias — build it inside the container with
the UEFI target:

```bash
./build/in-container.sh cargo build -p writeonce-bootloader \
    --release --target x86_64-unknown-uefi
# → target/x86_64-unknown-uefi/release/writeonce-bootloader.efi
```

The build pipeline ships that `.efi` as the `BOOTX64.EFI` artifact in the build
bundle; `writeonce-installer` copies it to `\EFI\BOOT\BOOTX64.EFI` on the target USB.

Dependencies: `uefi` (uefi-rs), `uefi-raw`, `log`.

## Diagnostics

Boot is hard to debug on real hardware, so the bootloader is loud and persistent:

- **Dual logging:** every step is printed to the UEFI console *and* appended to an
  in-memory buffer that is flushed to `\EFI\WriteOnce\boot.log` on the ESP — before
  handoff and again on failure. If the machine never reaches kernel messages, mount
  the USB on another machine and read `boot.log`.
- **Halt-on-failure:** any failing step returns `Err((step_name, Status))`. The
  bootloader prints a `FATAL` banner with the step name + UEFI status and then
  **halts forever** (a `stall` loop) so the message can't scroll off or drop back to
  the firmware boot menu — photograph the screen or read `boot.log`, then reset.
- Before `StartImage` it prints a notice that the screen will go blank for 10–30s
  during `ExitBootServices()` on older hardware, with recovery instructions.

## Design notes

- **Why `LoadImage(FromDevicePath)` and not `FromBuffer`:** the loader builds the
  kernel's device path by copying its *own* loader device path and swapping the
  trailing `MEDIA_FILE_PATH` node for `\EFI\WriteOnce\bzImage`, so the firmware sets
  both `LoadedImage.device_handle` and `file_path` correctly. The earlier approach
  (`FromBuffer` + manually patching `device_handle`) worked on lenient QEMU/OVMF but
  produced a *silent* failure after `StartImage` on the T450's strict Aptio V
  firmware. This is the idiom systemd-boot / rEFInd / the uefi-rs `shell_launcher`
  example all use.
- **EFI-stub delegation:** initrd loading, `boot_params`, e820, and
  `ExitBootServices()` are the kernel stub's job; keeping them out of this binary is
  what makes it small and robust. See
  `docs/learning/phase-6-bootloader-efi-stub-delegation.md`.

## Source layout

Everything is in [`src/main.rs`](src/main.rs):

| Piece | Responsibility |
| --- | --- |
| `main` / `try_boot` | Entry point, the 7-step boot sequence, failure banner + halt. |
| `say!` | Macro that logs to the console and the persisted buffer at once. |
| `build_kernel_device_path` | Derive the kernel's `DevicePath` from the loader's own. |
| `persist_log` | Write the buffered log to `\EFI\WriteOnce\boot.log`. |
| `read_file` / `read_file_size` / `open_path` / `read_full` | ESP file helpers. |

## See also

- `crates/writeonce-installer` — lays out the ESP and installs this as `BOOTX64.EFI`.
- `crates/writeonce-initramfs` — what the kernel runs once the EFI stub has loaded it.
- `crates/writeonce-pid1` — PID 1, exec'd after `switch_root`.
- `docs/learning/phase-6-bootloader-efi-stub-delegation.md` — the rationale.
