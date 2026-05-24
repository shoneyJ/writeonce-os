#!/usr/bin/env bash
# build/18-make-artifacts.sh — bundle the staged sysroot + kernel +
# initramfs + EFI bootloader into the installer artifacts directory.
#
# Output: build/artifacts/
#   bzImage           ← from build/kernel/arch/x86_64/boot/bzImage
#   initramfs.img     ← from staging; built by Rust writeonce-initramfs OR busybox
#   BOOTX64.EFI       ← from target/x86_64-unknown-uefi/release/writeonce-bootloader.efi
#   sysroot.tar.zst   ← tar + zstd of build/staging/sysroot/
#   manifest.toml     ← SHA-256s of all the above + metadata
#
# Run AFTER ./17-stage-sysroot.sh completes. Output is what
# writeonce-installer reads via --from.

set -euo pipefail

cd "$( dirname "${BASH_SOURCE[0]}" )/.."
# shellcheck disable=SC1091
source ./build/setup-env.sh

STAGING="${STAGING:-build/staging/sysroot}"
OUT="${OUT:-build/artifacts}"

echo "==== writeonce make-artifacts ===="
echo " STAGING: $STAGING"
echo " OUT:     $OUT"

[[ -d "$STAGING" ]] || {
    echo "error: $STAGING does not exist. Run ./17-stage-sysroot.sh first." >&2
    exit 1
}

mkdir -p "$OUT"

# ---- 1. kernel -------------------------------------------------------------
echo
echo "==== [1/5] Staging kernel"
KERNEL_SRC="${KERNEL_BZIMAGE:-build/kernel/arch/x86_64/boot/bzImage}"
if [[ -f "$KERNEL_SRC" ]]; then
    install -m644 "$KERNEL_SRC" "$OUT/bzImage"
    echo "    $KERNEL_SRC → $OUT/bzImage"
else
    echo "    WARN: $KERNEL_SRC not present; reusing existing $OUT/bzImage if any"
    [[ -f "$OUT/bzImage" ]] || { echo "    error: no bzImage anywhere" >&2; exit 1; }
fi

# ---- 2. initramfs ----------------------------------------------------------
echo
echo "==== [2/5] Staging initramfs"
INITRAMFS_SRC="${INITRAMFS_IMG:-build/initramfs.img}"
if [[ -f "$INITRAMFS_SRC" ]]; then
    install -m644 "$INITRAMFS_SRC" "$OUT/initramfs.img"
    echo "    $INITRAMFS_SRC → $OUT/initramfs.img"
else
    echo "    WARN: $INITRAMFS_SRC not present; reusing existing $OUT/initramfs.img if any"
    [[ -f "$OUT/initramfs.img" ]] || { echo "    error: no initramfs.img anywhere" >&2; exit 1; }
fi

# ---- 3. UEFI bootloader ----------------------------------------------------
echo
echo "==== [3/5] Staging UEFI bootloader"
BOOTLOADER_SRC="${BOOTLOADER_EFI:-target/x86_64-unknown-uefi/release/writeonce-bootloader.efi}"
if [[ -f "$BOOTLOADER_SRC" ]]; then
    install -m644 "$BOOTLOADER_SRC" "$OUT/BOOTX64.EFI"
    echo "    $BOOTLOADER_SRC → $OUT/BOOTX64.EFI"
else
    echo "    error: writeonce-bootloader.efi not built. Run cargo build --release --target x86_64-unknown-uefi -p writeonce-bootloader first." >&2
    exit 1
fi

# ---- 4. tar + zstd the sysroot --------------------------------------------
echo
echo "==== [4/5] Compressing sysroot (tar + zstd -19) ..."
tar -cf - -C "$STAGING" . | zstd -19 -T0 -q -o "$OUT/sysroot.tar.zst"
ls -lh "$OUT/sysroot.tar.zst"

# ---- 5. compute SHA-256s + write manifest.toml ----------------------------
echo
echo "==== [5/5] Writing manifest.toml"
sha() { sha256sum "$1" | awk '{print $1}'; }

GIT_SHA=$(git rev-parse --short HEAD 2>/dev/null || echo unknown)
BUILT_AT=$(date -u +%FT%TZ)
BUILD_KEY=$(printf '%s\n%s\n%s\n%s\n%s\n' \
    "$(sha "$OUT/bzImage")" \
    "$(sha "$OUT/initramfs.img")" \
    "$(sha "$OUT/BOOTX64.EFI")" \
    "$(sha "$OUT/sysroot.tar.zst")" \
    "$GIT_SHA" | sha256sum | awk '{print $1}')

cat > "$OUT/manifest.toml" <<EOF
schema_version = "0.1.0"

[image]
kernel     = "bzImage"
initramfs  = "initramfs.img"
bootloader = "BOOTX64.EFI"
sysroot    = "sysroot.tar.zst"
cmdline    = "console=tty0 root=UUID=__ROOT_UUID__ rw quiet init=/usr/sbin/writeonce-pid1"

[verification]
kernel_sha256     = "$(sha "$OUT/bzImage")"
initramfs_sha256  = "$(sha "$OUT/initramfs.img")"
bootloader_sha256 = "$(sha "$OUT/BOOTX64.EFI")"
sysroot_sha256    = "$(sha "$OUT/sysroot.tar.zst")"

[metadata]
build_key          = "${BUILD_KEY:0:16}"
built_at           = "$BUILT_AT"
writeonce_git_sha  = "$GIT_SHA"
EOF

echo
echo "Artifacts ready at $OUT/"
ls -lh "$OUT/"
echo
echo "Next: sudo target/release/writeonce-installer install \\"
echo "          --from $OUT --target /dev/sdX"
