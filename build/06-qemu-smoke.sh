#!/usr/bin/env bash
# build/06-qemu-smoke.sh — boot the freshly-built kernel + initramfs in QEMU.
#
# Two modes:
#   ./06-qemu-smoke.sh           interactive serial console (Ctrl+a x to quit)
#   ./06-qemu-smoke.sh --uefi    boot via OVMF (UEFI firmware) instead of legacy
#
# Success criterion: kernel decompresses, mounts pseudo-FS, and BusyBox sh
# prompt appears. If you can run `uname -a` and `ls /` inside, Phase 2's
# first artifact pair is validated and you can move on to T450 deployment.

set -uo pipefail

cd "$( dirname "${BASH_SOURCE[0]}" )"
# shellcheck disable=SC1091
source ./setup-env.sh

BZIMAGE="$BUILD_ROOT/artifacts/bzImage"
INITRAMFS="$BUILD_ROOT/artifacts/initramfs.img"

[[ -f "$BZIMAGE" ]]   || { echo "missing $BZIMAGE — run ./04-kernel.sh"; exit 1; }
[[ -f "$INITRAMFS" ]] || { echo "missing $INITRAMFS — run ./05-initramfs.sh"; exit 1; }

command -v qemu-system-x86_64 >/dev/null \
    || { echo "qemu-system-x86_64 not installed (apt install qemu-system-x86)"; exit 1; }

QEMU_ARGS=(
    -kernel  "$BZIMAGE"
    -initrd  "$INITRAMFS"
    -append  "console=ttyS0 panic=10"
    -nographic
    -m       2G
    -no-reboot
    -enable-kvm
)

if [[ "${1:-}" == "--uefi" ]]; then
    OVMF="/usr/share/OVMF/OVMF_CODE.fd"
    [[ -f "$OVMF" ]] || { echo "OVMF firmware not found at $OVMF (apt install ovmf)"; exit 1; }
    QEMU_ARGS+=( -bios "$OVMF" )
    echo "Booting WriteOnce kernel + initramfs in QEMU (UEFI / OVMF)…"
else
    echo "Booting WriteOnce kernel + initramfs in QEMU (legacy)…"
fi

echo "  Ctrl+a x to quit"
echo "  Expect: BusyBox banner, drop to /bin/sh prompt."
echo
exec qemu-system-x86_64 "${QEMU_ARGS[@]}"
