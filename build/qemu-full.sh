#!/usr/bin/env bash
# build/qemu-full.sh — boot the full bootloader→kernel→initramfs chain
# in QEMU under OVMF UEFI firmware, with the kernel console streamed
# to stdout via serial.
#
# Why: each USB-flash + T450-reboot iteration is 5–10 minutes. QEMU
# runs the same artifact set (BOOTX64.EFI, bzImage, initramfs.img,
# cmdline.txt) end-to-end in ~30 seconds, with text logs in your
# terminal — no photographing the screen.
#
# What this DOES exercise:
#   - Our writeonce-bootloader (loaded by OVMF from \EFI\BOOT\BOOTX64.EFI)
#   - Kernel EFI stub handoff (LoadImage / StartImage / device_handle
#     patch / load_options encoding)
#   - initramfs load via the stub's initrd= path
#   - writeonce-initramfs (Rust /init) PID-1 entry
#
# What this CAN'T catch:
#   - T450 i915 hardware quirks (QEMU uses cirrus/qxl, not i915)
#   - Lenovo firmware oddities
#   - Real disk geometry / GPT type-GUID issues
#
# Exit qemu: Ctrl-]  then  x

set -euo pipefail

cd "$( dirname "${BASH_SOURCE[0]}" )/.."

ARTIFACTS="build/artifacts"

for f in BOOTX64.EFI bzImage initramfs.img; do
    [[ -f "$ARTIFACTS/$f" ]] || {
        echo "missing $ARTIFACTS/$f — run \`just artifacts\` first" >&2
        exit 1
    }
done

command -v qemu-system-x86_64 >/dev/null \
    || { echo "qemu-system-x86_64 not installed (apt install qemu-system-x86)" >&2; exit 1; }
command -v mcopy >/dev/null \
    || { echo "mcopy not installed (apt install mtools)" >&2; exit 1; }

# Build a self-contained FAT32 image for the virtual ESP. mtools means
# no sudo / no losetup. The image is a temp file rebuilt every run, so
# you can edit cmdline.txt below and re-run with no extra ceremony.
ESP_DIR="$( mktemp -d -t wo-qemu-esp-XXXXXX )"
ESP_IMG="$ESP_DIR/esp.img"
trap 'rm -rf "$ESP_DIR"' EXIT

# 128 MiB is plenty for kernel + initramfs + bootloader.
dd if=/dev/zero of="$ESP_IMG" bs=1M count=128 status=none
mkfs.vfat -F32 -n WRITEONCE "$ESP_IMG" >/dev/null

# Mirror the on-USB layout: /EFI/BOOT/BOOTX64.EFI is the firmware's
# default-loaded path; /EFI/WriteOnce/ holds our payloads.
mmd -i "$ESP_IMG" ::EFI ::EFI/BOOT ::EFI/WriteOnce
mcopy -i "$ESP_IMG" "$ARTIFACTS/BOOTX64.EFI"   ::EFI/BOOT/BOOTX64.EFI
mcopy -i "$ESP_IMG" "$ARTIFACTS/bzImage"       ::EFI/WriteOnce/bzImage
mcopy -i "$ESP_IMG" "$ARTIFACTS/initramfs.img" ::EFI/WriteOnce/initramfs.img

# QEMU has no display in -nographic; route the kernel to ttyS0 (serial),
# which QEMU pipes to stdout via -serial mon:stdio. No root= because we
# don't attach a virtual rootfs — writeonce-initramfs will drop to its
# recovery shell, which is fine for exercising the boot path.
CMDLINE_FILE="$ESP_DIR/cmdline.txt"
cat > "$CMDLINE_FILE" <<EOF
console=ttyS0,115200 earlycon=ttyS0,115200 loglevel=7 ignore_loglevel panic=10
EOF
mcopy -i "$ESP_IMG" "$CMDLINE_FILE" ::EFI/WriteOnce/cmdline.txt

# OVMF firmware + a private writable copy of the var store. Ubuntu 24.04
# renamed these to *_4M.fd (the modern 4MB split firmware); older Ubuntu
# / Debian / Fedora use the *.fd names. Probe both.
if [[ -z "${OVMF_CODE:-}" ]]; then
    for cand in /usr/share/OVMF/OVMF_CODE_4M.fd /usr/share/OVMF/OVMF_CODE.fd \
                /usr/share/edk2/x64/OVMF_CODE.fd; do
        [[ -f "$cand" ]] && { OVMF_CODE="$cand"; break; }
    done
fi
if [[ -z "${OVMF_VARS_TEMPLATE:-}" ]]; then
    for cand in /usr/share/OVMF/OVMF_VARS_4M.fd /usr/share/OVMF/OVMF_VARS.fd \
                /usr/share/edk2/x64/OVMF_VARS.fd; do
        [[ -f "$cand" ]] && { OVMF_VARS_TEMPLATE="$cand"; break; }
    done
fi
[[ -n "${OVMF_CODE:-}" && -f "$OVMF_CODE" ]] || {
    echo "OVMF firmware not found (apt install ovmf). Searched:" >&2
    echo "  /usr/share/OVMF/OVMF_CODE_4M.fd" >&2
    echo "  /usr/share/OVMF/OVMF_CODE.fd" >&2
    echo "Override with OVMF_CODE=/path/to/OVMF_CODE.fd ./build/qemu-full.sh" >&2
    exit 1
}
[[ -n "${OVMF_VARS_TEMPLATE:-}" && -f "$OVMF_VARS_TEMPLATE" ]] || {
    echo "OVMF vars template not found (apt install ovmf)" >&2; exit 1;
}
OVMF_VARS="$ESP_DIR/vars.fd"
cp "$OVMF_VARS_TEMPLATE" "$OVMF_VARS"

echo "===================================================================="
echo "  QEMU full-stack boot — WriteOnce OS"
echo "===================================================================="
echo "  ESP image      : $ESP_IMG ($(du -h "$ESP_IMG" | awk '{print $1}'))"
echo "  cmdline        : $(cat "$CMDLINE_FILE")"
echo "  Exit            : Ctrl-]  x"
echo "===================================================================="
echo

exec qemu-system-x86_64 \
    -enable-kvm \
    -m 2G \
    -drive if=pflash,format=raw,readonly=on,file="$OVMF_CODE" \
    -drive if=pflash,format=raw,file="$OVMF_VARS" \
    -drive file="$ESP_IMG",format=raw,if=virtio,id=esp \
    -nographic \
    -no-reboot \
    -echr 29 \
    -serial mon:stdio
