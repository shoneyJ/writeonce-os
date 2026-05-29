# Phase 1 — T450 prep: rescue USB, disk plan, serial/netconsole

**Goal.** Make it safe to wipe Ubuntu. After this phase, a borked WriteOnce install must be recoverable in under 10 minutes without losing the workstation's ability to keep building.

## Subtasks

1. **Inventory current T450 state.** Already done in `../.agents/target-machine.md`. Note: current partitioning is GPT, ESP at 1 GB, /boot at 2 GB, LVM holding root.

2. **Capture anything worth keeping from Ubuntu before wipe.**
   - `/etc/fstab`, `/boot/grub/grub.cfg`, `dmesg` boot log, `lspci -nnk`, `/boot/config-$(uname -r)` (already in target-machine.md — but archive a full copy at `../.agents/ubuntu-baseline/`).
   - Ensure SSH keys / personal files on the T450 are pulled back to workstation if any.

3. **Build a rescue USB** (one-time, on workstation).
   - Recommend Arch Linux ISO or SystemRescue on a 8 GB+ stick — full toolchain, networking, GPT/parted, chroot capable.
   - Test that it boots the T450 in UEFI mode (F12 boot menu).

4. **Capture iwlwifi firmware blobs.** Wireless 7265 needs `iwlwifi-7265D-29.ucode` (or matching). Copy from `/lib/firmware/` of running Ubuntu **before wipe** into `build/firmware/`. Same for Intel microcode (`/lib/firmware/intel-ucode/`).

5. **Set up a serial / netconsole channel.** T450 doesn't have a physical serial port. Choose:
   - **Netconsole** (recommended) — `modprobe netconsole netconsole=@$T450_IP/eth0,6666@$WORKSTATION_IP/$WS_MAC` on the T450; `nc -ul 6666` on workstation. Logs kernel oops over Ethernet.
   - **USB-serial dongle** — only if netconsole proves unreliable during early-boot debug.
   - Document the chosen approach in this file.

6. **Decide final partition layout for WriteOnce** (single-OS, GPT, UEFI):
   - `sda1` ESP — FAT32, 512 MB (room for multiple kernel/initramfs versions).
   - `sda2` `/boot` — ext4, 1 GB (kernels + initramfs; separate from ESP because the Rust bootloader will read from here).
   - `sda3` root — ext4, 100 GB (system + Rust source trees).
   - `sda4` `/home` — ext4, remaining ~398 GB.
   - **No LVM, no swap-partition** (swap file on root if needed; the goal is to learn primitives, not LVM).

7. **Document the wipe-and-restore drill** in this file: rescue USB boot → `wipefs -a /dev/sda` → `parted` script that recreates the layout → reinstall procedure if a future phase requires it.

8. **Do NOT wipe yet.** Wipe happens at the start of Phase 2 when there's something to install.

## Deliverable

Rescue USB tested, firmware blobs archived, netconsole verified end-to-end (boot the current Ubuntu with `netconsole` module and confirm kernel messages reach the workstation).

## Acceptance criteria

- Booting the rescue USB on the T450 lands at a root shell with internet over the Intel wireless (or wired).
- `dmesg | grep netconsole` on the T450 confirms registration; workstation `nc -ul 6666` shows live boot logs from the T450.
- `build/firmware/iwlwifi-*.ucode` and `build/firmware/intel-ucode/` populated.

## References

- `../.agents/target-machine.md` for exact device IDs.
- `../.agents/reference/linux/Documentation/networking/netconsole.rst`.

## Risks

- Wiping before having a working WriteOnce kernel = T450 is a brick until Phase 2 lands. Mitigation: Phase 2 entirely on workstation first, only wipe when artifacts are ready and tested in QEMU.
