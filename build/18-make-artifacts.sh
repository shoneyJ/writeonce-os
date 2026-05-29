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
# Our Rust writeonce-bootloader is the primary BOOTX64.EFI. GRUB was
# tried but `grub-mkstandalone`'s output is rejected by the T450's
# Aptio-V firmware (loads → instant exit → firmware falls back to next
# boot entry, no diagnostic). The Rust loader is known-good on this
# firmware (verified by its on-screen step progress).
#
# We also stage the GRUB binary at \EFI\grub\grubx64.efi on the ESP —
# accessible from the firmware F12 menu and still works in QEMU. Kept
# for the multi-entry menu UX when we eventually port to a firmware
# that accepts it.
echo
echo "==== [3/5] Staging UEFI bootloader"
RUST_BOOTLOADER="${BOOTLOADER_EFI:-target/x86_64-unknown-uefi/release/writeonce-bootloader.efi}"
if [[ -f "$RUST_BOOTLOADER" ]]; then
    install -m644 "$RUST_BOOTLOADER" "$OUT/BOOTX64.EFI"
    echo "    Rust bootloader → $OUT/BOOTX64.EFI ($(du -h "$OUT/BOOTX64.EFI" | awk '{print $1}'))"
else
    echo "    error: $RUST_BOOTLOADER not built — run cargo build --release --target x86_64-unknown-uefi -p writeonce-bootloader" >&2
    exit 1
fi

# Stage GRUB as a secondary boot option.
GRUB_CFG="${GRUB_CFG:-build/skeleton/boot/grub/grub.cfg}"
if [[ -f "$GRUB_CFG" ]] && command -v grub-mkstandalone >/dev/null; then
    grub-mkstandalone \
        --format=x86_64-efi \
        --output="$OUT/grubx64.efi" \
        --locales="" --themes="" --fonts="" \
        --modules="part_gpt fat ext2 normal configfile linux echo all_video efi_gop efi_uga search search_label search_fs_uuid loadenv test true font gettext gfxterm chain reboot halt" \
        "boot/grub/grub.cfg=$GRUB_CFG"
    echo "    GRUB (secondary) → $OUT/grubx64.efi (alt — F12 → \\EFI\\grub\\)"
fi

# ---- 4. tar + zstd the sysroot --------------------------------------------
echo
echo "==== [4/5] Compressing sysroot (tar + zstd -19) ..."
# --owner=0 --group=0 --numeric-owner: force every file in the tarball
# to uid/gid 0 regardless of the on-disk ownership (which is uid 1000
# because the build container runs unprivileged). The installer extracts
# with preserve_ownerships=true and lands files as root on the target.
# The few paths that need a non-root owner (/home/<user>, /var/cache/*)
# are fixed by writeonce-installer's customize.rs chown_recursive step
# AFTER extract.
tar --owner=0 --group=0 --numeric-owner -cf - -C "$STAGING" . | \
    zstd -19 -T0 -f -q -o "$OUT/sysroot.tar.zst"
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
# NOTE: This cmdline field is now LARGELY UNUSED — we ship GRUB which
# embeds its own grub.cfg with per-entry cmdlines (see
# build/skeleton/boot/grub/grub.cfg). The installer still substitutes
# __ROOT_UUID__ in case anything else reads this field.
# rootwait is essential when booting from removable media — USB
# enumeration can take a second or two and the kernel needs to wait.
# `rootwait` is the kernel's own flag; `writeonce.rootwait=N` is the
# initramfs's polling-timeout knob (see crates/writeonce-initramfs/src/
# cmdline.rs). Both belong here because writeonce-initramfs replaces
# the kernel's mount logic — without our flag the initramfs scans
# /sys/class/block once and gives up before USB enumeration completes.
cmdline    = "console=tty0 rootwait writeonce.rootwait=30 root=UUID=__ROOT_UUID__ rw init=/usr/sbin/writeonce-pid1"

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
