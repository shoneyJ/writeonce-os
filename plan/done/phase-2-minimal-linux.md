# Phase 2 — LFS-style minimal Linux (mainline kernel + transitional BusyBox init)

**Goal.** Prove the **whole chain works** on the actual T450: kernel boots, mounts root, networking up, root shell. Use mainline 6.12 and BusyBox temporarily — the Rust replacements come in phases 3–6.

## Subtasks

1. **Build Linux 6.12 with a target-tuned config.**
   - Start from `defconfig` (not `allnoconfig` — too slow to bring up enough drivers).
   - **Builtin (not modules)** these — driven by `../.agents/target-machine.md`:
     - `CONFIG_EFI_STUB`, `CONFIG_EFI_PARTITION`, `CONFIG_FB_EFI`
     - `CONFIG_SATA_AHCI` (storage controller is 8086:9c83 AHCI mode)
     - `CONFIG_EXT4_FS`, `CONFIG_VFAT_FS`, `CONFIG_TMPFS`, `CONFIG_DEVTMPFS`, `CONFIG_DEVTMPFS_MOUNT`
     - `CONFIG_E1000E` (ethernet I218-LM)
     - `CONFIG_DRM_I915` (HD 5500; KMS framebuffer needed for X11 later)
     - `CONFIG_USB_XHCI_HCD`, `CONFIG_USB_EHCI_HCD`, `CONFIG_HID_GENERIC`, `CONFIG_USB_HID`
     - `CONFIG_SND_HDA_INTEL` (audio — optional in this phase, needed by Phase 8)
     - `CONFIG_INTEL_IOMMU`, `CONFIG_MICROCODE_INTEL`
     - cgroup v2: `CONFIG_CGROUPS`, `CONFIG_MEMCG`, `CONFIG_CGROUP_PIDS`, `CONFIG_CGROUP_BPF`
   - **Modules** (loaded later by initramfs / supervisor):
     - `CONFIG_IWLWIFI`, `CONFIG_IWLMVM` (needs firmware blob from Phase 1)
     - `CONFIG_BT` (Bluetooth via xHCI), `CONFIG_RTSX_PCI` (card reader)
   - Disable irrelevant chunks aggressively: no NUMA, no ARM, no other SoC subsystems, no virtio drivers (T450 is bare metal), no Xen.

2. **Build the kernel** on the workstation: `make -j$(nproc) bzImage modules`. Artifact: `arch/x86/boot/bzImage`.

3. **Build BusyBox 1.37** statically against the cross-toolchain. Single binary, hardlinked to `/bin/{sh,ls,mount,cp,...}` inside the sysroot.

4. **Author a minimal sysroot** (`build/sysroot/`):
   - `/init` = transitional shell script that mounts proc/sys/dev, ifups eth0, drops to `/bin/sh`.
   - `/etc/passwd`, `/etc/group`, `/etc/fstab` (minimal — root + tmpfs).
   - `/lib/firmware/iwlwifi-*` from Phase 1 archive.

5. **Build an initramfs** containing kernel modules (`lib/modules/6.12.x/`), BusyBox, `/init`. CPIO + gzip: `find . | cpio -H newc -o | gzip > build/artifacts/initramfs.img`.

6. **Test in QEMU first** — *mandatory before touching the T450*.
   - `qemu-system-x86_64 -kernel build/artifacts/bzImage -initrd build/artifacts/initramfs.img -append "console=ttyS0" -nographic -m 2G`
   - Verify: boot proceeds to BusyBox shell; `cat /proc/cpuinfo`, `ls /sys/firmware/efi` work.

7. **Wipe the T450 and install.** Boot rescue USB → run the parted script from Phase 1 → format partitions → mount → install GRUB temporarily (we replace it in Phase 6; GRUB EFI is the simplest path right now) → copy `bzImage` and `initramfs.img` to `/boot` → write `/boot/grub/grub.cfg`.

8. **First T450 boot of WriteOnce.** Expect: GRUB menu → kernel decompress → mount root → BusyBox shell. Verify wired ethernet (`ip link`, `udhcpc enp0s25`), then wireless after loading `iwlwifi` module manually.

9. **Iterate kernel config** based on what's missing. Each iteration: rebuild on workstation, `scp` artifacts to T450's `/boot`, reboot.

## Deliverable

T450 boots from its SSD to a working BusyBox root shell, networking up, on a kernel built from `../.agents/reference/linux` cross-compiled by the Phase 0 toolchain.

## Acceptance criteria

- T450 powers on → GRUB → custom kernel → root shell, no Ubuntu artifacts touched.
- `ping 1.1.1.1` works over either wired or wireless.
- Reboot doesn't lose any state on `/home` (because there's nothing in it yet — but `/etc/fstab` mounts it cleanly).
- Netconsole from Phase 1 captures the boot log on the workstation.

## References

- `../.agents/reference/linux/init/main.c` — see `start_kernel()` and `rest_init()`, useful when reading boot logs.
- `../.agents/reference/linux/arch/x86/configs/x86_64_defconfig` — diff your config against this baseline.

## Risks

- iwlwifi firmware ABI mismatch — match the firmware version to the kernel-recommended one (check `../.agents/reference/linux/drivers/net/wireless/intel/iwlwifi/cfg/`).
- GRUB EFI install on a fresh ESP — practice in QEMU first.
