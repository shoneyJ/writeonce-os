#!/usr/bin/env bash
# build/17-stage-sysroot.sh — assemble the final installable sysroot.
#
# Pulls together:
#   - $LFS/usr (the Phase 0-8 source-built userspace)
#   - target/.../release/writeonce-{pid1,svc,login,logind,initramfs} +
#     wo-ctl (the per-Rust-crate boot-path binaries)
#   - target/x86_64-unknown-uefi/release/writeonce-bootloader.efi
#   - the Hyprland + Quickshell desktop is delivered via Nix at runtime
#     (see /etc/writeonce/desktop/flake.nix); its config rides in via the
#     build/skeleton overlay below — nothing is staged from a DE build here.
#   - build/skeleton/ overlay (/etc/*, /home/writeonce/* defaults:
#     .config/hypr, .config/quickshell, /usr/local/bin/wo-session, /etc/nix)
#   - crates/writeonce-svc/examples/services/*.toml → /etc/writeonce/services/
#
# Output: $STAGING (default: build/staging/sysroot/) — a complete root
# filesystem ready to be tar+zstd'd into the installer artifact.
#
# RUNS ON THE HOST DIRECTLY (not inside wo-builder) so it can read build
# artifacts + the skeleton overlay outside the container's /work mount.
#
# Prerequisite ARTIFACTS:
#   - Phase 0-8 built ($LFS/usr populated)
#   - Kernel modules + firmware staged (04-kernel.sh, 01-fetch.sh)
#   - Desktop (Hyprland + Quickshell): delivered via Nix at runtime, not staged
#     here (see /etc/writeonce/desktop/flake.nix + plan/phase-14-nix-packages.md)

set -euo pipefail

cd "$( dirname "${BASH_SOURCE[0]}" )/.."
# shellcheck disable=SC1091
source ./build/setup-env.sh

STAGING="${STAGING:-build/staging/sysroot}"

echo "==== writeonce stage-sysroot ===="
echo " LFS:     $LFS"
echo " STAGING: $STAGING"

# Sanity checks.
[[ -d "$LFS/usr" ]] || {
    echo "error: $LFS/usr does not exist. Run Phase 0-9 builds first." >&2
    exit 1
}
for bin in \
    target/x86_64-unknown-linux-musl/release/writeonce-pid1 \
    target/x86_64-unknown-linux-musl/release/writeonce-svc \
    target/x86_64-unknown-linux-musl/release/wo-ctl \
    target/release/writeonce-login \
    target/release/writeonce-logind \
    target/x86_64-unknown-linux-musl/release/writeonce-initramfs
do
    [[ -f "$bin" ]] || {
        echo "warn: $bin missing — staging will skip it"
    }
done

# ---- 1. fresh staging directory --------------------------------------------
echo
echo "==== [1/8] Resetting $STAGING"
rm -rf "$STAGING"
mkdir -p "$STAGING"/{boot,dev,etc,home,proc,root,run,sys,tmp,usr,var}
mkdir -p "$STAGING"/etc/writeonce/services
mkdir -p "$STAGING"/var/{lib,log,cache}
chmod 1777 "$STAGING"/tmp

# ---- 2. copy $LFS/usr into staging -----------------------------------------
echo
echo "==== [2/8] Copying \$LFS/usr (~hundreds of MB) ..."
cp -a "$LFS/usr"/. "$STAGING/usr/"

# Some Phase 8 packages whose --libdir defaults to /lib64 (linux-pam,
# possibly others) install OUTSIDE $LFS/usr/. The UsrMerge symlinks
# below (lib64 → usr/lib) don't help on their own — they only resolve
# if the files actually live at usr/lib. Merge those strays into the
# canonical location.
#
# Discovered via the May-2026 boot failure where writeonce-login died
# with `libpam.so.0: cannot open shared object file` — libpam was in
# $LFS/lib64/ but never copied into the staged artifact.
for src in "$LFS/lib64" "$LFS/lib"; do
    if [[ -d "$src" ]]; then
        echo "    merging $src/ → $STAGING/usr/lib/"
        # -n (--no-clobber): never overwrite a real file already staged from
        # $LFS/usr (copied above). $LFS/lib64 holds glibc's loader as a relative
        # symlink (ld-linux-x86-64.so.2 → ../lib/...) that is valid at /lib64 but
        # self-loops once dropped into /usr/lib — clobbering the real loader ELF
        # gives every binary ELOOP at execve (PID 1 panic). -n keeps the real
        # loader and still copies the stray libpam-class libs this merge is for.
        cp -a -n "$src"/. "$STAGING/usr/lib/" 2>/dev/null || true
    fi
done

# Same for stray /bin and /sbin: util-linux, kmod, and shadow install some
# tools to $LFS/sbin (and /bin depending on --bindir/--sbindir). The UsrMerge
# symlinks below only resolve if the files actually live under usr/{bin,sbin},
# so fold them in. `! -L` skips the case where $LFS already UsrMerged these to
# symlinks (avoids copying a dir onto itself).
for pair in bin:bin sbin:sbin; do
    src="$LFS/${pair%%:*}"; dst="$STAGING/usr/${pair##*:}"
    if [[ -d "$src" && ! -L "$src" ]]; then
        echo "    merging $src/ → $dst/"
        cp -a -n "$src"/. "$dst/" 2>/dev/null || true
    fi
done

# Symlinks /bin and /sbin to /usr/bin per modern UsrMerge convention.
ln -sf usr/bin  "$STAGING/bin"
ln -sf usr/sbin "$STAGING/sbin"
ln -sf usr/lib  "$STAGING/lib"

# POSIX requires /bin/sh. Point it at bash (the only shell we ship).
# Without this, anything that execve's /bin/sh fails with ENOENT —
# including writeonce-pid1's prototype placeholder and shebangs in
# /etc/init.d scripts a sysadmin might add later.
if [[ -e "$STAGING/usr/bin/bash" && ! -e "$STAGING/usr/bin/sh" ]]; then
    ln -sf bash "$STAGING/usr/bin/sh"
fi
ln -sf usr/lib  "$STAGING/lib64"

# ---- 3. (with-systmed branch) NO Rust boot-path binaries -------------------
# This branch uses systemd (built into $LFS/usr by 16-systemd.sh) as PID 1 +
# service manager + logind + udev, and shadow for login. The custom Rust init
# crates (writeonce-pid1/svc/logind/login/session-create) are NOT built or
# staged here — they live on `master`. systemd's /sbin/init symlink + its units
# are already under $LFS/usr (copied by step 2 above).
echo
echo "==== [3a/8] Rust crate binaries: skipped (systemd branch — none staged)"
mkdir -p "$STAGING/sbin" "$STAGING/usr/sbin" "$STAGING/usr/bin"
# Ensure /sbin/init resolves (systemd installs /usr/lib/systemd/systemd; some
# firmware/GRUB configs default init=/sbin/init). Create the symlink if the
# systemd build didn't already. Only for the systemd init flavor.
if [[ "${FLAVOR_INIT:-systemd}" == systemd \
      && -e "$STAGING/usr/lib/systemd/systemd" && ! -e "$STAGING/usr/sbin/init" ]]; then
    ln -sf ../lib/systemd/systemd "$STAGING/usr/sbin/init"
    echo "    symlinked /usr/sbin/init → ../lib/systemd/systemd"
fi

# ---- 3b. desktop environment: delivered via Nix, not staged here -----------
echo
echo "==== [3b/8] Desktop (Hyprland + Quickshell): via Nix at runtime"
# The X11/i3 + i3More desktop was replaced by a Wayland desktop (Hyprland
# compositor + Quickshell shell). Per the project scope these Tier-2 packages
# come from Nix — see /etc/writeonce/desktop/flake.nix + the wo-session launcher,
# both staged by the build/skeleton overlay in step [4/8]. A Nix Hyprland
# closure is self-contained (its own Mesa/Wayland/seatd), so there is nothing
# to copy from a DE build at this stage. (Prerequisite: the Phase 14 Nix
# bootstrap — plan/phase-14-nix-packages.md.)
echo "    desktop config staged via build/skeleton; binaries via Nix profile"

# ---- 3c. stage package /etc config -----------------------------------------
# LFS treats the whole $LFS (including /etc) as the system. Package configs
# (login.defs, shadow's pam.d, /etc/security for PAM, /etc/fonts for fontconfig,
# /etc/dbus-1, udev rules, dhcpcd.conf, …) install into $LFS/etc and MUST be
# staged — otherwise they vanish and login/PAM/fontconfig silently fall back to
# compiled defaults. The skeleton overlay below runs AFTER this, so our own
# files (passwd, group, shadow, hostname, pam.d/login, default.target, …) win.
echo
echo "==== [3c/8] Staging \$LFS/etc (package config)"
if [[ -d "$LFS/etc" ]]; then
    cp -a "$LFS/etc"/. "$STAGING/etc/"
    echo "    staged $(find "$LFS/etc" -mindepth 1 -maxdepth 1 | wc -l) top-level /etc entries"
fi

# ---- 4. overlay the skeleton tree (common + active flavor) -----------------
# Build-time profile: skeleton/common/ is shared by all flavors; skeleton/<FLAVOR>/
# carries the init/display/DE/pkg-specific files and is applied second so it wins.
echo
echo "==== [4/8] Overlaying build/skeleton: common + flavor ($FLAVOR)"
cp -a build/skeleton/common/. "$STAGING/"
if [[ -d "build/skeleton/$FLAVOR" ]]; then
    cp -a "build/skeleton/$FLAVOR/." "$STAGING/"
    echo "    overlaid skeleton/common + skeleton/$FLAVOR"
else
    echo "error: build/skeleton/$FLAVOR/ missing — flavor '$FLAVOR' is declared but not" >&2
    echo "       yet populated in this tree (see docs/learning). Only its config exists." >&2
    exit 1
fi

# /root home directory (root user).
mkdir -p "$STAGING/root"
chmod 700 "$STAGING/root"

# /home/writeonce ownership (uid 1000, gid 1000 from /etc/passwd).
chown -R 1000:1000 "$STAGING/home/writeonce" 2>/dev/null || true
chmod 700 "$STAGING/home/writeonce/.config" 2>/dev/null || true

# Generate /etc/shadow from .template (no real hashes — install-time
# step prompts for passwords or accepts pre-set ones).
cp "$STAGING/etc/shadow.template" "$STAGING/etc/shadow"
chmod 640 "$STAGING/etc/shadow"

# ---- 5. service units: provided by systemd itself --------------------------
# (with-systmed branch) No writeonce-svc *.service.toml units, and no
# writeonce-logind D-Bus policy: systemd ships its own units under
# $LFS/usr/lib/systemd/system and its own org.freedesktop.login1 policy.
echo
echo "==== [5/8] Service units: provided by systemd (none copied)"

# ---- 6. final touch-ups ----------------------------------------------------
echo
echo "==== [6/8] Final touch-ups"

# startx (from xinit) launches the server named `X`; xorg-server installs
# the suid-wrapper script as /usr/bin/Xorg, not /usr/bin/X. Symlink the
# conventional name so `startx` (built --with-xserver=/usr/bin/X) finds it.
if [[ -e "$STAGING/usr/bin/Xorg" && ! -e "$STAGING/usr/bin/X" ]]; then
    ln -sf Xorg "$STAGING/usr/bin/X"
    echo "    symlinked /usr/bin/X → Xorg (startx default server)"
fi

# Create empty resolv.conf — dhcpcd will populate it at boot.
: > "$STAGING/etc/resolv.conf"

# /etc/fstab — minimal; root is mounted by initramfs via root= kernel arg.
cat > "$STAGING/etc/fstab" <<EOF
# /etc/fstab — populated by writeonce-installer at install time.
# root partition mount happens via kernel root= arg, not here.
proc        /proc     proc      defaults  0 0
sysfs       /sys      sysfs     defaults  0 0
devtmpfs    /dev      devtmpfs  defaults  0 0
devpts      /dev/pts  devpts    gid=5,mode=620  0 0
tmpfs       /tmp      tmpfs     defaults,nodev,nosuid  0 0
tmpfs       /run      tmpfs     defaults,nodev,nosuid  0 0
EOF


# ---- 6b. install kernel modules --------------------------------------------
# 04-kernel.sh stages the built =m drivers (iwlwifi, bt, mmc, rtsx, …) at
# build/artifacts/modules-stage/lib/modules/<ver>. Copy them into the rootfs
# (usr-merged: /lib → usr/lib) so modprobe + systemd-modules-load find them.
# Without this, wifi/bt/cardreader won't load — the built-in =y drivers
# (AHCI, ext4, i915, e1000e wired ethernet, USB) still boot the desktop.
echo
echo "==== [6b/8] Kernel modules"
MODSTAGE="${MODULES_STAGE:-build/artifacts/modules-stage/lib/modules}"
if compgen -G "$MODSTAGE/*" >/dev/null 2>&1; then
    mkdir -p "$STAGING/usr/lib/modules"
    cp -a "$MODSTAGE"/. "$STAGING/usr/lib/modules/"
    echo "    staged modules for kernel(s): $(ls "$STAGING/usr/lib/modules" | tr '\n' ' ')"
else
    echo "    WARN: $MODSTAGE absent — run 04-kernel.sh first; wifi/bt/mmc modules absent (built-ins still boot)."
fi

# ---- 7. install kernel firmware blobs --------------------------------------
# The kernel's iwlwifi driver issues request_firmware() AFTER switch_root, so
# the blobs must live in the *real* rootfs, not just the initramfs. 01-fetch.sh
# drops them at $BUILD_ROOT/firmware/. We copy verbatim into /lib/firmware/.
echo
echo "==== [7/8] Kernel firmware"
FW_SRC="${BUILD_ROOT:-build}/firmware"
if compgen -G "$FW_SRC/*" >/dev/null 2>&1; then
    mkdir -p "$STAGING/lib/firmware"
    for fw in "$FW_SRC"/*; do
        name="$(basename "$fw")"
        install -Dm644 "$fw" "$STAGING/lib/firmware/$name"
        echo "    $name"
    done
else
    echo "    WARN: $FW_SRC is empty — run \`./build/01-fetch.sh\` to populate it."
    echo "    Without firmware, iwlwifi will fail to bind to wifi hardware on boot."
fi

echo
echo "Staging complete. Size:"
du -sh "$STAGING"
echo
echo "Next: ./build/17a-install-nix.sh — stage the single-user Nix store (Phase 14),"
echo "      then ./build/18-make-artifacts.sh — bundles for the installer."
