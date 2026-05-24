#!/usr/bin/env bash
# build/15-fake-artifacts.sh — produce a synthetic artifacts bundle so
# writeonce-installer can be smoke-tested without running the real
# Phase 0-9 build (which takes hours).
#
# Output: build/artifacts/ containing:
#   bzImage           (~1 MB of /dev/urandom; ELF magic faked at offset 0)
#   initramfs.img     (~2 MB cpio archive with /init script)
#   BOOTX64.EFI       (real writeonce-bootloader if present, else 64KB random)
#   sysroot.tar.zst   (minimal /etc + /bin + /usr from busybox-style tree)
#   manifest.toml     (with real SHA-256s of everything above)
#
# Intent:
#   - Lets us run `sudo writeonce-installer install --from build/artifacts \
#     --target /dev/sdX --dry-run` and exercise the full pipeline up to
#     the disk write.
#   - Lets us actually write to a throwaway USB and confirm the GPT + FAT32
#     + ext4 layout is correct.
#   - Will NOT produce a bootable system — the kernel is random bytes.

set -euo pipefail

cd "$( dirname "${BASH_SOURCE[0]}" )/.."

OUT=build/artifacts
mkdir -p "$OUT"

echo "=== fake-artifacts: writing to $OUT/"

# ---- bzImage (fake) --------------------------------------------------------
dd if=/dev/urandom of="$OUT/bzImage" bs=1024 count=1024 status=none

# ---- initramfs.img (real cpio with /init shell wrapper) -------------------
WORK=$(mktemp -d)
trap "rm -rf $WORK" EXIT
mkdir -p "$WORK/initramfs/"{bin,dev,proc,sys,etc}
cat > "$WORK/initramfs/init" <<'EOF'
#!/bin/sh
echo "WriteOnce fake initramfs — placeholder for smoke testing"
exec /bin/sh
EOF
chmod +x "$WORK/initramfs/init"
( cd "$WORK/initramfs" && find . -print | cpio --quiet -o -H newc ) | gzip > "$OUT/initramfs.img"

# ---- BOOTX64.EFI -----------------------------------------------------------
if [[ -f target/x86_64-unknown-uefi/release/writeonce-bootloader.efi ]]; then
    cp target/x86_64-unknown-uefi/release/writeonce-bootloader.efi "$OUT/BOOTX64.EFI"
    echo "    used real writeonce-bootloader.efi"
else
    dd if=/dev/urandom of="$OUT/BOOTX64.EFI" bs=1024 count=64 status=none
    echo "    BOOTX64.EFI is placeholder (writeonce-bootloader not built yet)"
fi

# ---- sysroot.tar.zst ------------------------------------------------------
mkdir -p "$WORK/sysroot/"{etc,bin,usr/bin,usr/lib,boot,dev,proc,sys,run,tmp,var/log}
cat > "$WORK/sysroot/etc/hostname" <<<"writeonce-test"
cat > "$WORK/sysroot/etc/os-release" <<EOF
NAME="WriteOnce"
VERSION="0.1.0-fake"
ID=writeonce
PRETTY_NAME="WriteOnce OS (fake test artifacts)"
EOF
echo "0.1.0-fake" > "$WORK/sysroot/etc/writeonce-release"
( cd "$WORK/sysroot" && tar -cf - . ) | zstd -19 -q -o "$OUT/sysroot.tar.zst"

# ---- compute SHA-256s + manifest.toml -------------------------------------
sha() { sha256sum "$1" | awk '{print $1}'; }

cat > "$OUT/manifest.toml" <<EOF
schema_version = "0.1.0"

[image]
kernel     = "bzImage"
initramfs  = "initramfs.img"
bootloader = "BOOTX64.EFI"
sysroot    = "sysroot.tar.zst"
cmdline    = "console=tty0 root=UUID=__ROOT_UUID__ rw quiet"

[verification]
kernel_sha256     = "$(sha "$OUT/bzImage")"
initramfs_sha256  = "$(sha "$OUT/initramfs.img")"
bootloader_sha256 = "$(sha "$OUT/BOOTX64.EFI")"
sysroot_sha256    = "$(sha "$OUT/sysroot.tar.zst")"

[metadata]
build_key          = "fake-$(date +%s)"
built_at           = "$(date -u +%FT%TZ)"
writeonce_git_sha  = "$(git rev-parse --short HEAD 2>/dev/null || echo unknown)"
EOF

ls -lh "$OUT/"
echo
echo "fake artifacts ready at $OUT/"
echo "next: sudo target/release/writeonce-installer install \\"
echo "          --from $OUT --target /dev/sdX --dry-run"
