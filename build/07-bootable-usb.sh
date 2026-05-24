#!/usr/bin/env bash
# build/07-bootable-usb.sh — write a bootable WriteOnce USB stick.
#
# Produces a UEFI-bootable FAT32 stick containing:
#   /EFI/BOOT/BOOTX64.EFI      standalone GRUB with grub.cfg embedded
#   /bzImage                   the WriteOnce kernel (Phase 2)
#   /initramfs.img             the WriteOnce initramfs (Phase 2)
#
# Boots on any UEFI-firmware machine with Secure Boot disabled (T450 ✓).
# Loaded entirely into RAM — does NOT touch the target machine's disk.
#
# DESTRUCTIVE: this script wipes the entire device. Several safety guards:
#   - Refuses /dev/sda (the workstation's main drive)
#   - Refuses any device with a mounted partition
#   - Shows the device's model + size and requires you to type "yes"
#   - Must be invoked with sudo
#
# Usage:
#     sudo ./07-bootable-usb.sh /dev/sdX
#         where sdX is the USB stick (NOT a real disk).
#         Find it with `lsblk` after plugging in.

set -uo pipefail

cd "$( dirname "${BASH_SOURCE[0]}" )"
# shellcheck disable=SC1091
source ./setup-env.sh

# --- argument and privilege checks ------------------------------------------

if [[ $# -ne 1 ]]; then
    cat <<EOF >&2
usage: sudo $0 /dev/sdX

  /dev/sdX must be a USB stick. Find candidates with:
      lsblk -d -o NAME,SIZE,MODEL,TRAN | grep -i usb
EOF
    exit 1
fi

if [[ $EUID -ne 0 ]]; then
    echo "Must run as root (use sudo). All operations below need privilege." >&2
    exit 1
fi

DEV="$1"
[[ -b "$DEV" ]] || { echo "$DEV is not a block device"; exit 1; }

case "$DEV" in
    /dev/sda|/dev/sda[0-9]*|/dev/nvme0*)
        echo "REFUSING to operate on $DEV — that's almost certainly the workstation's primary drive."
        echo "USB sticks usually show up as /dev/sdb, /dev/sdc, etc."
        exit 1
        ;;
esac

# Refuse if the device or any of its partitions is mounted.
if lsblk -no MOUNTPOINT "$DEV" | grep -q .; then
    echo "REFUSING: $DEV (or a partition of it) is currently mounted."
    echo "Unmount everything first:"
    lsblk -o NAME,MOUNTPOINT "$DEV"
    exit 1
fi

# --- artifact preflight -----------------------------------------------------

BZIMAGE="$BUILD_ROOT/artifacts/bzImage"
INITRAMFS="$BUILD_ROOT/artifacts/initramfs.img"

[[ -f "$BZIMAGE" ]]   || { echo "missing $BZIMAGE — run ./04-kernel.sh"; exit 1; }
[[ -f "$INITRAMFS" ]] || { echo "missing $INITRAMFS — run ./05-initramfs.sh"; exit 1; }

for t in grub-mkstandalone parted mkfs.vfat wipefs sync; do
    command -v "$t" >/dev/null || { echo "missing host tool: $t"; exit 1; }
done

# --- confirmation -----------------------------------------------------------

cat <<EOF

The following device will be COMPLETELY ERASED:
EOF
lsblk -d -o NAME,SIZE,MODEL,VENDOR,SERIAL,TRAN "$DEV"
echo

read -rp "Type the device path exactly to confirm wipe (or anything else to abort): " CONFIRM
if [[ "$CONFIRM" != "$DEV" ]]; then
    echo "Aborted."
    exit 1
fi

# --- wipe + partition + format ----------------------------------------------

echo
echo "==> Wiping existing partition tables on $DEV"
wipefs --all "$DEV"

echo "==> Creating GPT with a single ESP partition"
parted --script "$DEV" \
    mklabel gpt \
    mkpart ESP fat32 1MiB 100% \
    set 1 esp on

# Wait for the kernel to notice the new partition.
partprobe "$DEV"
sleep 1

PART1=$(lsblk -lnp -o NAME "$DEV" | tail -n +2 | head -n1)
[[ -b "$PART1" ]] || { echo "could not find partition device for $DEV"; exit 1; }

echo "==> Formatting $PART1 as FAT32"
mkfs.vfat -F 32 -n WRITEONCE "$PART1"

# --- stage the EFI tree -----------------------------------------------------

MOUNT="$(mktemp -d)"
trap 'umount "$MOUNT" 2>/dev/null; rmdir "$MOUNT" 2>/dev/null' EXIT

mount "$PART1" "$MOUNT"
mkdir -p "$MOUNT/EFI/BOOT"

echo "==> Copying kernel + initramfs to USB"
install -m644 "$BZIMAGE"   "$MOUNT/bzImage"
install -m644 "$INITRAMFS" "$MOUNT/initramfs.img"

echo "==> Generating grub.cfg"
GRUB_CFG="$(mktemp)"
cat > "$GRUB_CFG" <<'CFG'
set timeout=3
set default=0

menuentry "WriteOnce OS (Phase 2 transitional)" {
    set root=(hd0,gpt1)
    linux  /bzImage console=tty0 console=ttyS0,115200 panic=10
    initrd /initramfs.img
}

menuentry "WriteOnce OS (recovery shell)" {
    set root=(hd0,gpt1)
    linux  /bzImage console=tty0 console=ttyS0,115200 init=/bin/busybox sh
    initrd /initramfs.img
}

menuentry "Reboot"   { reboot }
menuentry "Shutdown" { halt }
CFG

echo "==> Building standalone GRUB EFI binary with grub.cfg embedded"
grub-mkstandalone \
    --format=x86_64-efi \
    --output="$MOUNT/EFI/BOOT/BOOTX64.EFI" \
    --modules="part_gpt part_msdos fat normal multiboot multiboot2 configfile linux echo all_video boot loadenv reboot halt" \
    "boot/grub/grub.cfg=$GRUB_CFG"

rm -f "$GRUB_CFG"

echo "==> Flushing writes"
sync
umount "$MOUNT"
rmdir "$MOUNT"
trap - EXIT

# --- summary ----------------------------------------------------------------

cat <<EOF

================================================================
  Bootable WriteOnce USB ready on $DEV
================================================================

Plug $DEV into the T450 and:
  1. Power on while pressing F12 (Lenovo boot menu).
  2. Select the USB stick (it'll show as the FAT32 label "WRITEONCE").
  3. GRUB menu appears; "WriteOnce OS (Phase 2 transitional)" is default.
  4. Expect kernel boot → initramfs → writeonce-pid1 (if cross-built and
     staged) → BusyBox shell.

To eject safely:
  sync && udisksctl power-off -b $DEV    # or just unplug if writes are flushed

To re-run with updated artifacts, just re-run this script — the GRUB EFI
binary and the kernel+initramfs files are overwritten cleanly.
EOF
