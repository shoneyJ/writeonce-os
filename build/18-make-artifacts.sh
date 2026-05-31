#!/usr/bin/env bash
# build/18-make-artifacts.sh — bundle the staged sysroot + kernel into the
# installer artifacts directory (with-systmed branch).
#
# Boot model on this branch: the kernel's own EFI stub IS the bootloader.
# GRUB is rejected by the T450's Aptio-V firmware (loads → instant exit, no
# diagnostic — the reason the now-retired Rust bootloader existed), and we ship
# no Rust bootloader here. So the firmware loads the EFI-stub kernel directly as
# \EFI\BOOT\BOOTX64.EFI; the kernel command line is baked in via CONFIG_CMDLINE
# (build/kernel-config-additions.fragment), and the kernel mounts root itself
# (built-in AHCI + ext4) — NO initramfs.
#
# Output: build/artifacts/
#   bzImage           ← build/kernel/arch/x86_64/boot/bzImage
#   BOOTX64.EFI       ← identical to bzImage (the EFI-stub kernel = the loader)
#   sysroot.tar.zst   ← tar + zstd of build/staging/sysroot/
#   manifest.toml     ← SHA-256s + metadata
#
# Run AFTER ./17-stage-sysroot.sh + a kernel build. Consumed by build/install.sh.

set -euo pipefail

cd "$( dirname "${BASH_SOURCE[0]}" )/.."
# shellcheck disable=SC1091
source ./build/setup-env.sh

STAGING="${STAGING:-build/staging/sysroot}"
OUT="${OUT:-build/artifacts}"

echo "==== writeonce make-artifacts (systemd / EFI-stub) ===="
echo " STAGING: $STAGING"
echo " OUT:     $OUT"

[[ -d "$STAGING" ]] || { echo "error: $STAGING missing. Run ./17-stage-sysroot.sh first." >&2; exit 1; }
mkdir -p "$OUT"

# ---- 1. kernel (= bootloader) ----------------------------------------------
echo
echo "==== [1/3] Staging kernel as bzImage + BOOTX64.EFI (EFI stub)"
KERNEL_SRC="${KERNEL_BZIMAGE:-}"
if [[ -z "$KERNEL_SRC" ]]; then
    # 04-kernel.sh builds under build/work/linux-<ver>/; pick the newest bzImage.
    KERNEL_SRC=$(ls -t build/work/linux-*/arch/x86/boot/bzImage 2>/dev/null | head -1)
fi
[[ -f "$KERNEL_SRC" ]] || { echo "error: no bzImage found — build the kernel (just kernel / 04-kernel.sh)." >&2; exit 1; }
install -m644 "$KERNEL_SRC" "$OUT/bzImage"
install -m644 "$KERNEL_SRC" "$OUT/BOOTX64.EFI"
echo "    $KERNEL_SRC → $OUT/bzImage + $OUT/BOOTX64.EFI ($(du -h "$OUT/bzImage" | awk '{print $1}'))"

# ---- 2. tar + zstd the sysroot --------------------------------------------
echo
echo "==== [2/3] Compressing sysroot (tar + zstd -19) ..."
# Force uid/gid 0 (the container builds unprivileged as 1000); install.sh
# extracts as root and then chowns /home/writeonce back to 1000:1000.
tar --owner=0 --group=0 --numeric-owner -cf - -C "$STAGING" . | \
    zstd -19 -T0 -f -q -o "$OUT/sysroot.tar.zst"
ls -lh "$OUT/sysroot.tar.zst"

# ---- 3. manifest -----------------------------------------------------------
echo
echo "==== [3/3] Writing manifest.toml"
sha() { sha256sum "$1" | awk '{print $1}'; }
GIT_SHA=$(git rev-parse --short HEAD 2>/dev/null || echo unknown)
BUILT_AT=$(date -u +%FT%TZ)

cat > "$OUT/manifest.toml" <<EOF
schema_version = "0.2.0"
init = "systemd"

[image]
kernel     = "bzImage"
bootloader = "BOOTX64.EFI"   # identical to the kernel (EFI stub)
sysroot    = "sysroot.tar.zst"
# The kernel command line is baked into the image via CONFIG_CMDLINE
# (root=PARTUUID + init=/usr/lib/systemd/systemd). install.sh assigns that
# fixed PARTUUID to the root partition; there is no separate cmdline file.

[verification]
kernel_sha256     = "$(sha "$OUT/bzImage")"
bootloader_sha256 = "$(sha "$OUT/BOOTX64.EFI")"
sysroot_sha256    = "$(sha "$OUT/sysroot.tar.zst")"

[metadata]
built_at          = "$BUILT_AT"
writeonce_git_sha = "$GIT_SHA"
EOF

echo
echo "Artifacts ready at $OUT/"
ls -lh "$OUT/"
echo
echo "Next: sudo ./build/install.sh /dev/sdX   (or: just install /dev/sdX)"
