# T450 boot bring-up — root-cause log

> Running notes on what's blocking first-boot of WriteOnce OS on the
> Lenovo ThinkPad T450 (Intel i5-5300U Broadwell, Intel HD 5500, Intel
> I218-LM, Intel 7265, UEFI Aptio V firmware, Secure Boot disabled).
> Updated as new evidence arrives.

## Latest known state

| Layer | Status | Notes |
|---|---|---|
| USB partition layout (GPT + ESP + ext4) | ✅ works | `blkid` confirms FAT32 ESP (`WRITEONCE`) + ext4 root (`writeonce-root`) |
| Firmware → our Rust bootloader | ✅ works | `[1/7] … [7/7]` progress visible on screen |
| Rust bootloader → kernel `StartImage` | ✅ works | EFI stub message `EFI stub: Loaded initrd from command line option` confirmed |
| Kernel post-`ExitBootServices` | ❌ blackout | Screen goes dark immediately after stub message; kernel reboots after panic= timeout |
| Firmware → GRUB | ❌ rejected | T450 firmware refuses `grub-mkstandalone` output ("screen blink" + auto-fallback to internal disk) |

## Bugs surfaced and fixed along the way

### 1. EFI stub: `failed to handle fs_proto`
**Cause.** Our Rust bootloader used `LoadImage(FromBuffer, file_path: None)`,
which leaves the kernel's `LoadedImage.device_handle` null. The Linux
EFI stub then can't open `initrd=\EFI\WriteOnce\initramfs.img` —
there's no FS to open relative to.

**Fix.** After `LoadImage`, patch the kernel's `LoadedImage.device_handle`
to our boot device's handle. `LoadedImage` is `#[repr(transparent)]`
over `uefi-raw::protocol::loaded_image::LoadedImageProtocol`, so a
pointer cast is safe. See `crates/writeonce-bootloader/src/main.rs`
step [6/7]. systemd-boot's `boot.c` does the same.

### 2. `StartImage returned (kernel rejected image) status=INVALID_PARAMETER`
**Cause.** Same as #1 — the EFI stub bailed out of cmdline parsing
because it couldn't materialise the device referenced by `initrd=`.
Same fix.

### 3. `writeonce-initramfs: unsupported root spec: filesystem LABEL probes not yet implemented`
**Cause.** The initramfs only resolved `Device`, `PartUuid` root specs.
The grub.cfg used `root=LABEL=writeonce-root`.

**Fix.** Added `read_ext4_label()` / `read_ext4_uuid()` in
`crates/writeonce-initramfs/src/discover.rs`. Walks
`/sys/class/block/*`, reads each device's ext4 superblock at offset
1024+120 for label and 1024+104 for fs-UUID (magic check at 1024+56).

### 4. Phantom sentinel: `step_kernel-build` rubber-stamped failures
**Cause.** `make … | tee` only propagates the make exit via PIPESTATUS;
the trailing `cp arch/x86/boot/bzImage …` silently no-op'd on a
missing bzImage. Stub bzImage persisted in `build/artifacts/`.

**Fix.** Chained the three commands with `&&` in
`build/04-kernel.sh`; failure now actually fails.

### 5. Phantom sentinel: `step_libjpeg-turbo` (Phase 8a)
**Cause.** Same un-chained `cmake -S/-B/install` pattern. Missing
`CMAKE_SYSTEM_PROCESSOR=x86_64` made the configure silently fail
(SIMD detection); the `touch sentinel` still ran. Downstream
gdk-pixbuf failed to find `libjpeg.pc` four phases later.

**Fix.** Chained the cmake calls; added `CMAKE_SYSTEM_PROCESSOR`.

### 6. Container missing `bc`, `libssl-dev` for kernel build
**Cause.** Kernel 6.12 needs `bc` for `timeconst.h` generation and
`openssl/bio.h` for `certs/extract-cert`.

**Fix.** Added both to `build/Containerfile`.

### 7. GRUB doesn't load on T450 Aptio-V firmware
**Cause.** Unknown — `grub-mkstandalone --format=x86_64-efi` output
is rejected by the firmware. F12 boot menu shows the USB, selecting
it produces a single screen flash and falls back to the next boot
entry. Our Rust bootloader loads on the same firmware fine.

**Workaround.** Use the Rust bootloader as primary `BOOTX64.EFI`.
GRUB is staged as `\EFI\grub\grubx64.efi` on the ESP (still works in
QEMU) but not on the firmware's default load path.

**Real fix.** Not yet diagnosed. Suspects: missing module in our
`grub-mkstandalone --modules=…` list, GOP-init ordering quirk
specific to this firmware. Not blocking.

## Open issue: post-`ExitBootServices` screen blackout

**Symptom.** Kernel logs `EFI stub: Loaded initrd from command line
option`, then screen goes black. Auto-reboot after `panic=N` confirms
the kernel is alive — it's panicking with no visible console.

### What we've tried

| Attempt | Result |
|---|---|
| `quiet` removed, `loglevel=7 ignore_loglevel` added | No change |
| `earlyprintk=efi` added | Active only until `ExitBootServices`; gap to next driver |
| `earlycon=efifb,keep` added | No change |
| `CONFIG_DRM_FBDEV_EMULATION=y` added to kernel | No change |
| `i915.modeset=0` added (block i915, leave efifb) | No change (no efifb fallback driver) |
| `CONFIG_DRM_SIMPLEDRM=y` + `CONFIG_SYSFB_SIMPLEFB=y` added | No change |
| Adopt Ubuntu's full config as base instead of `defconfig` | No change — Ubuntu's own kernel binary (`/boot/vmlinuz-6.8.0-117-generic`) ALSO went silent after our bootloader handoff. **Rules out kernel build as the cause.** |
| **Switch `LoadImage(FromBuffer)` → `LoadImage(FromDevicePath)` in our Rust bootloader** | ✅ **Fixed the post-EFI-stub blackout.** Kernel now boots through. Identified by reading uefi-rs's `shell_launcher.rs` example: their idiom builds a DevicePath that points at the kernel file on the boot device and lets firmware load it. Firmware then sets both `LoadedImage.device_handle` AND `LoadedImage.file_path` correctly. Our previous `FromBuffer` approach left `file_path` null; T450 Aptio V firmware validated this at `StartImage` and quietly bailed (QEMU/OVMF was lenient and worked). systemd-boot / GRUB / rEFInd all use FromDevicePath. |
| **Next bug uncovered:** `CONFIG_USB_STORAGE=m` from Ubuntu's config — driver that exposes USB sticks as `/dev/sd*` is a module, but the module lives on the rootfs we can't mount without it (chicken-and-egg). Symptom: kernel boots into initramfs recovery shell; `blocks` shows only the internal Samsung SSD, no USB. Fix: pin `CONFIG_USB_STORAGE=y` + `CONFIG_USB_UAS=y` in `kernel-config-additions.fragment`, plus add `rootwait` to cmdline so kernel waits for USB enumeration. |
| **Race in `writeonce-initramfs::discover::locate_root()`** — even with the USB driver built-in, the kernel's hub init (`drivers/usb/core/hub.c:1077`) defers per-port enumeration onto a `delayed_work` queue (100 ms minimum power-on delay per port; Aptio V firmware adds more). Our Rust PID 1 races ahead: scans `/sys/class/block/*` once, doesn't find the USB yet, drops to recovery. The kernel's own `rootwait` flag only affects `init/do_mounts.c::wait_for_root()` which we never reach (we replace the kernel's mount logic entirely). ✅ **Fixed** by wrapping `locate_root` in a 30 s polling loop, gated by a new `writeonce.rootwait=N` cmdline knob (default 30). Same idiom systemd's `device.target` (90 s default) and dracut (30 s) use. Confirmed working on T450 — root device now mounts. |
| **`pivot_root` from initramfs returns EINVAL.** After locate_root succeeded and root mounted on `/sysroot`, `switch_root::switch_and_exec` called `pivot_root(., .)` and got `os error 22`. Kernel doc `Documentation/admin-guide/initrd.rst`: *"It is impossible to call pivot_root() from the initramfs because rootfs cannot be unmounted."* ✅ **Fixed** by replacing `pivot_root` with `mount --move /sysroot /` + `chroot .` — the same idiom busybox's `switch_root` and systemd's `initrd-switch-root.service` use. |
| **`writeonce-pid1` spawned `/bin/sh` instead of the service supervisor.** After switch_root succeeded we landed at PID 1, but it just forked a shell on tty1 and stopped. The full systemd-equivalent stack (writeonce-svc + unit graph + writeonce-login + PAM + writeonce-session-create) was already implemented, but no `/etc/writeonce/pid1.toml` was being shipped — so PID 1 used its Phase-3 prototype defaults (`/bin/sh`). ✅ **Fixed** by adding `build/skeleton/etc/writeonce/pid1.toml` that points `child = "/usr/sbin/writeonce-svc"`. PID 1 now hands off to the supervisor which brings up the dbus → logind → writeonce-login chain. |
| **`iwd.service` respawns forever, walking the PID counter.** After PID 1 was wired to writeonce-svc, `iwd.service` started failing on every boot — kernel CRYPTO user-API symbols were modular (Ubuntu base config) and not loaded by the time iwd probed `AF_ALG`. iwd then printed "WPS will not be available" and exited. writeonce-svc dutifully respawned it every 5 s with no rate limit. **Three coordinated fixes:** ✅ (a) added systemd-style `start-limit-burst` / `start-limit-interval-sec` to the supervisor (`crates/writeonce-svc/src/{config,state}.rs`); after 3 failures in 30 s for iwd the unit is marked `Failed` and respawning stops. ✅ (b) Pinned `CONFIG_CRYPTO_USER_API{,_HASH,_SKCIPHER,_RNG}` + `CMAC` + `CCM` + `CFG80211` + `MAC80211` as `=y` in `build/kernel-config-additions.fragment` so they're available at PID-1 time. ✅ (c) Added iwlwifi firmware fetch from kernel.org's `linux-firmware.git` (pinned commit `LINUX_FIRMWARE_COMMIT` in `build/versions.env`) — both `iwlwifi-7265-17.ucode` and `iwlwifi-7265D-29.ucode` cover the two 7265 stepping variants. Staged into `$STAGING/lib/firmware/` by a new step in `build/17-stage-sysroot.sh`. iwd now finds a phy and authenticates. |
| **Multiple services (dbus, dhcpcd, logind) spinning at boot** (photo `.agents/PXL_20260527_200326737.jpg`). Boot photo showed `mkdir: /var/lib/dhcpcd: Read-only file system`, `no such user dhcpcd`, `enp0s31f6: interface not found`, all the spinning services restarting forever. Root cause was structural, not per-service: too many things ran at boot, each with its own fragile assumption. ✅ **Bare-minimum-boot refit** (`plan/writeonce-svc-fix/fix-learn-from-scratch-boot.md`): introduced `console.target` as the unconditional boot path (sysinit → dbus → logind → writeonce-login) and made dhcpcd / iwd / modules-load *opt-in* via `/etc/writeonce/enabled.d/*.toml` stubs that the supervisor reads at startup (`crates/writeonce-svc/src/enabled.rs`). Users opt in via `wo-ctl enable <unit>` post-login; the installer's `network.enabled_at_boot = true` headless flag pre-writes the stubs for SSH-only profiles. Also: initramfs now parses `rw` / `ro` on cmdline (`crates/writeonce-initramfs/src/cmdline.rs` — previously hardcoded `MS_RDONLY`, killing every write); supervisor's burst-cap defaults tightened from 5/10s (mathematically unreachable) to 3/30s — same shape as iwd's per-unit override that already worked. End-to-end smoke test confirms bare boot reaches login with 6 jobs in plan + iwd Inactive; `wo-ctl enable/disable/enabled` round-trip works against the running supervisor. |

### Working hypotheses, ordered by likelihood

1. **Kernel ran but had no framebuffer driver bound** until simpledrm
   (added in this session). EFI stub's `efifb` is deprecated and
   doesn't survive `ExitBootServices` cleanly on some firmware; i915
   then either fails to bind on early init or takes a long time;
   between them no console exists.

2. **Triple fault very early** — bypasses any panic= handling, causes
   instant reset. Would explain "screen blink then reboot" if not for
   the fact that we know the kernel runs *some* code (we see EFI stub
   output before the blank).

3. **KASLR or ACPI interaction** — Broadwell + old Lenovo firmware
   has known issues with KASLR slot picking. Would be cured by
   `nokaslr noapic acpi=off pci=nocrs` cmdline additions.

### Next test plan

SIMPLEDRM didn't help — kernel doesn't survive long enough to reach
the PCI-probe stage where simpledrm activates. This points at a
**triple fault very early** in kernel init (before `start_kernel`
finishes), caused by something in: KASLR slot pick, ACPI table parse,
or APIC config.

**Cmdline bisection plan** (edit `\EFI\WriteOnce\cmdline.txt` on USB,
no rebuild):

| Test | cmdline (replace whole file) | If it boots, suspect... |
|---|---|---|
| 1 | `console=tty0 nokaslr panic=0` | KASLR slot conflict |
| 2 | `console=tty0 nokaslr noapic nolapic acpi=off pci=nocrs panic=0` | ACPI/APIC/PCI host bridge |
| 3 | Boot Ubuntu live USB on T450 as sanity | If Ubuntu fails too, hardware regression; otherwise our build is at fault |

Once a passing cmdline is found, bisect within it to find the
single offending flag, then either bake it into `cmdline.txt`/grub.cfg
or rebuild the kernel with the matching `CONFIG_*` setting.

## Things that are NOT the cause

Tested + cleared:

- USB partition / GPT layout — verified by blkid
- Bootloader cmdline parsing — verified by boot.log
- EFI stub initrd discovery — verified by the `LINUX_EFI_INITRD_MEDIA_GUID`
  message we got via GRUB before we lost it
- Secure Boot — confirmed disabled in BIOS
- UEFI/Legacy mode — confirmed "UEFI Only"
- Hardware itself — pending Ubuntu live USB confirmation, but the
  T450 boots Ubuntu from internal disk fine after fallback, so the
  display + CPU + ESP-reader path all work

## Future work motivated by this

- **Hardware-probe-driven kernel config.** The "Ubuntu-grade defconfig +
  hand-curated fragment" approach we settled for is a pragmatic
  workaround. The right WriteOnce-shaped solution is a tool that
  derives a minimal kernel config from the target's hardware probe.
  Design captured in [`plan/phase-7-kerngen.md`](../../plan/phase-7-kerngen.md).

## Operational notes

- **Kernel build history**: `docs/kernel-build-history.md` (auto-updated
  by `just kernel`)
- **Boot log on the USB**: `\EFI\WriteOnce\boot.log` — written by the
  Rust bootloader on every boot attempt, contains the `[1/7]…[7/7]`
  trace. Read after a failed boot: `sudo mount /dev/sda1 /mnt/wo-esp
  && cat /mnt/wo-esp/EFI/WriteOnce/boot.log`.
- **QEMU iteration loop**: `just qemu-full` — proves the artifact set
  is correct in software. Won't reproduce T450 hardware issues but
  catches everything else.
