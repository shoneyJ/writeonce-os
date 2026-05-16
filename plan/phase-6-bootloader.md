# Phase 6 — Rust UEFI bootloader (uefi-rs)

**Goal.** Replace GRUB with a Rust EFI app that reads `bzImage` + initramfs from the ESP, sets up `boot_params`, calls `ExitBootServices`, and jumps to the kernel.

## Subtasks

1. **Scaffold `src/bootloader/`.** `no_std`, target `x86_64-unknown-uefi`, dependencies: `uefi`, `uefi-services`.

2. **Implement the EFI app skeleton** — UEFI image entry, get the System Table, get the loaded image protocol, locate the ESP.

3. **Implement config parsing.** `\EFI\WriteOnce\boot.toml` on the ESP — lists kernel/initramfs paths, cmdline. Multiple entries with arrow-key selection.

4. **Implement kernel + initramfs loading.** Use the EFI Simple File System protocol to read into allocated pool memory.

5. **Build `boot_params`** for x86_64 Linux. Reference: `../.agents/reference/linux/Documentation/arch/x86/boot.rst` (the *exact* zero page layout). EFI memmap → e820, set `setup_header` fields, command line pointer, ramdisk image/size.

6. **Decide: use the EFI handover protocol or the legacy 16-bit boot?** Use the **EFI handover protocol** (`xloadflags & XLF_KERNEL_64`) — the kernel exposes a 64-bit handover entry that takes the System Table + boot_params directly, no real-mode dance.

7. **`ExitBootServices`** and jump to the handover entry. After this point, no UEFI services available.

8. **Test in QEMU + OVMF** (UEFI firmware): `qemu-system-x86_64 -bios OVMF.fd -drive file=esp.img,format=raw`. Build `esp.img` as a FAT32 image with the bootloader at `/EFI/BOOT/BOOTX64.EFI`.

9. **Install on the T450.** `cp target/.../bootloader.efi /boot/efi/EFI/BOOT/BOOTX64.EFI`. Use `efibootmgr` (last GRUB-era invocation) to set BootOrder to point at it. Keep a GRUB entry around as fallback for one boot, then remove.

10. **Decision log.** Document why custom uefi-rs instead of limine (open question from session notes): more ownership of the boot path; limine is solid but it's another author's binary. Note this here.

## Deliverable

T450 boots from press-power through a Rust EFI app to the kernel, no GRUB anywhere.

## Acceptance criteria

- `efibootmgr -v` on the T450 shows only WriteOnce in BootOrder.
- `dmesg | grep "EFI stub"` shows the handover happened.
- Pressing Esc at boot brings up the WriteOnce bootloader menu with multiple kernel entries.

## Risks

- `boot_params` layout drifts across kernel versions. Mitigation: pin to kernel 6.12 LTS and consult `arch/x86/include/uapi/asm/bootparam.h` in `../.agents/reference/linux/`.
