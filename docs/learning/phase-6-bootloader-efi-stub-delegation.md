# Phase 6 — bootloader design: delegate to the kernel's EFI stub

> Companion to [`../../crates/writeonce-bootloader/src/main.rs`](../../crates/writeonce-bootloader/src/main.rs)
> and [`../../plan/done/phase-6-bootloader.md`](../../plan/done/phase-6-bootloader.md).
> Captures the design choice that shrank the bootloader from "construct
> `boot_params` + e820 + handover-jump" to "`LoadImage` and let the
> kernel handle its own setup."

## The decision

When the WriteOnce kernel is configured with `CONFIG_EFI_STUB=y` (which
ours is — see `build/kernel-config-additions.fragment`), the resulting
`bzImage` is itself a valid PE/COFF UEFI application. A UEFI firmware
can load and start it directly the same way it would any other EFI app.

That means our bootloader does **not** need to:

- Construct the x86\_64 Linux `boot_params` "zero page" by hand.
- Translate the UEFI memory map into legacy e820 entries.
- Locate the kernel's 64-bit EFI handover entry by parsing
  `setup_header`.
- Call `ExitBootServices()` and jump to the handover entry.

All of that is performed by the kernel's own EFI stub — battle-tested
upstream code that GRUB, systemd-boot, and rEFInd all delegate to
today. There's no reason for WriteOnce to re-implement it.

## What the bootloader still does

```
1. uefi::helpers::init()                          ← allocator + log + panic handler
2. Locate the ESP via LoadedImage.device()         ← whatever volume we booted from
3. SimpleFileSystem.open_volume() → Directory root
4. Read /EFI/WriteOnce/cmdline.txt                ← one line of kernel cmdline
5. Append "initrd=\EFI\WriteOnce\initramfs.img"   ← if not already present
6. Read /EFI/WriteOnce/bzImage                    ← into a Vec<u8>
7. uefi::boot::load_image(FromBuffer { buffer })  ← UEFI loads it as an EFI app
8. LoadedImage::set_load_options(utf16(cmdline))  ← kernel reads cmdline from here
9. uefi::boot::start_image(handle)                ← kernel takes over; never returns
```

Result: ~100 LOC of actual logic, 31 KB PE32+ binary, zero direct
contact with the boot protocol's wire format.

## How the kernel finds the initramfs without us telling it

Linux 5.7 added a feature to the EFI stub: it parses its own command
line, looks for `initrd=<path>`, and loads the file from the same
SimpleFileSystem the kernel was loaded from. The path is a Windows-style
backslash-separated absolute path on that volume — e.g. `\EFI\WriteOnce\initramfs.img`.

Our bootloader appends `initrd=\EFI\WriteOnce\initramfs.img` to the
cmdline if it's not already there. The EFI stub does the rest. No need
for `LINUX_EFI_INITRD_MEDIA_GUID` / `LoadFile2` plumbing.

## What we give up

The trade is real but bounded:

- **Custom boot-time UX.** systemd-boot lets you press a key to edit the
  cmdline at boot. We don't have that yet; users edit `cmdline.txt` on
  the ESP between boots. Acceptable for a single-user laptop.
- **Multi-entry menus.** Our bootloader always boots one configuration.
  Adding a menu is a Phase 6c feature: read `entries/*.txt`, present a
  text menu, pick one. Mostly UEFI-Console I/O.
- **Direct control of EFI memory-map handling.** A bug or quirk in the
  EFI stub becomes a kernel-version-pinned problem. Mitigated by
  pinning Linux 6.12 LTS in `versions.env`.

For a developer workstation, none of these are dealbreakers.

## How the chain works end-to-end

```
                  T450 firmware (UEFI 64-bit, Secure Boot off)
                                  │
                                  │ /EFI/BOOT/BOOTX64.EFI
                                  ▼
                  ┌──────────────────────────────────┐
                  │      writeonce-bootloader.efi    │
                  │  - locate ESP via LoadedImage    │
                  │  - read cmdline.txt              │
                  │  - read bzImage                  │
                  │  - LoadImage(FromBuffer)         │
                  │  - set_load_options(cmdline)     │
                  │  - start_image()                 │
                  └──────────────────────────────────┘
                                  │
                                  ▼
                  ┌──────────────────────────────────┐
                  │  bzImage (CONFIG_EFI_STUB=y)     │
                  │  - parse own cmdline             │
                  │  - find "initrd=\path", load it  │
                  │  - construct boot_params + e820  │
                  │  - ExitBootServices()            │
                  │  - decompress, start_kernel()    │
                  └──────────────────────────────────┘
                                  │
                                  ▼
                  ┌──────────────────────────────────┐
                  │       Rust /init                 │
                  │       (writeonce-initramfs)      │
                  │  - mount /proc /sys /dev         │
                  │  - load configured modules       │
                  │  - discover root device          │
                  │  - pivot_root, execve PID 1      │
                  └──────────────────────────────────┘
                                  │
                                  ▼
                  ┌──────────────────────────────────┐
                  │       Rust PID 1                 │
                  │       (writeonce-pid1)           │
                  │  - mount essential FS            │
                  │  - signalfd + reaping loop       │
                  │  - exec writeonce-svc (PID 2)    │
                  └──────────────────────────────────┘
                                  │
                                  ▼
                              writeonce-svc
                              (Phase 4 supervisor)
                                  │
                                  ▼
                              writeonce-login (tty1)
                                  │
                                  ▼
                              Xorg + i3 + i3More (Phase 8/9)
```

Every box from `writeonce-bootloader.efi` downward is WriteOnce code.
The kernel itself is upstream Linux 6.12 LTS, unmodified.

## Staging the ESP for a real boot

The four files the bootloader expects:

```
ESP volume
├── EFI/BOOT/BOOTX64.EFI                ← target/x86_64-unknown-uefi/release/writeonce-bootloader.efi
└── EFI/WriteOnce/
    ├── bzImage                         ← build/artifacts/bzImage
    ├── initramfs.img                   ← build/artifacts/initramfs.img
    └── cmdline.txt                     ← e.g. "root=PARTUUID=... console=tty0"
```

The existing `build/07-bootable-usb.sh` lays out a GRUB-based USB
stick — it can be replaced (or augmented with a sibling
`build/08-esp-direct.sh`) that lays out the four files above when
ready to switch from GRUB to the WriteOnce bootloader.

## Iteration loop for the bootloader

```bash
# Edit src/main.rs in crates/writeonce-bootloader/, then:
./build/in-container.sh cargo build -p writeonce-bootloader \
                                    --release --target x86_64-unknown-uefi

# Copy onto a USB stick (or the QEMU ESP image):
cp target/x86_64-unknown-uefi/release/writeonce-bootloader.efi \
   /mnt/esp/EFI/BOOT/BOOTX64.EFI

# Boot and observe.
```

Compile-test cycle is ~2 seconds after the first build. QEMU-OVMF test
cycle (when OVMF is available) is ~30 seconds including boot.

## What still needs verification

- Round-trip in **QEMU + OVMF** (needs `apt install ovmf` on the host —
  or an OVMF blob fetched from upstream Tianocore's CI artifacts and
  dropped in `build/firmware/OVMF_CODE.fd`).
- Boot on the actual T450 via the existing
  [`build/07-bootable-usb.sh`](../../build/07-bootable-usb.sh) workflow,
  swapping its GRUB stage for the WriteOnce bootloader once we extend
  the script to do so.

Both are post-Round-6b verification work, not Round-6b deliverables.
