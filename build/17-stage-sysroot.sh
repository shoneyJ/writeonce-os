#!/usr/bin/env bash
# build/17-stage-sysroot.sh — assemble the final installable sysroot.
#
# Pulls together:
#   - $LFS/usr (the Phase 0-8 source-built userspace)
#   - target/.../release/writeonce-{pid1,svc,login,logind,initramfs} +
#     wo-ctl (the per-Rust-crate boot-path binaries)
#   - target/x86_64-unknown-uefi/release/writeonce-bootloader.efi
#   - i3 from i3More's meson install-root (built via i3More's own tooling)
#   - i3More from /opt/i3more/bin (built via i3More's own dev container)
#   - build/skeleton/ overlay (/etc/*, /home/writeonce/* defaults)
#   - crates/writeonce-svc/examples/services/*.toml → /etc/writeonce/services/
#
# Output: $STAGING (default: build/staging/sysroot/) — a complete root
# filesystem ready to be tar+zstd'd into the installer artifact.
#
# RUNS ON THE HOST DIRECTLY (not inside wo-builder). It needs to read
# the operator's /opt/i3more/bin/ and the i3More symlink target, both
# of which are outside the wo-builder Docker container's /work mount.
#
# Prerequisite ARTIFACTS:
#   - Phase 0-8 built ($LFS/usr populated)
#   - Rust crates built (cargo build-pid1, build-svc, build-login,
#     build-logind, build-initramfs + UEFI bootloader)
#   - i3 built via i3More's `just i3-build && just i3-stage` (install-root
#     populated at .agents/reference/i3More/vendor/i3/build/install-root/)
#   - i3More built via `docker compose run dev cargo build --release ...`
#     and copied to /opt/i3more/bin/ (or override I3MORE_BIN_DIR)

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
        cp -a "$src"/. "$STAGING/usr/lib/" 2>/dev/null || true
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

# ---- 3. install Rust boot-path binaries ------------------------------------
echo
echo "==== [3a/8] Installing Rust crate binaries"
mkdir -p "$STAGING/sbin" "$STAGING/usr/sbin" "$STAGING/usr/bin"
install_if_present() {
    local src="$1" dst="$2"
    if [[ -f "$src" ]]; then
        install -Dm755 "$src" "$dst"
        echo "    $src → $dst"
    else
        echo "    skip $src (not built)"
    fi
}
install_if_present target/x86_64-unknown-linux-musl/release/writeonce-pid1       "$STAGING/usr/sbin/writeonce-pid1"
install_if_present target/x86_64-unknown-linux-musl/release/writeonce-svc        "$STAGING/usr/sbin/writeonce-svc"
install_if_present target/x86_64-unknown-linux-musl/release/wo-ctl               "$STAGING/usr/bin/wo-ctl"
install_if_present target/release/writeonce-login                                "$STAGING/usr/sbin/writeonce-login"
install_if_present target/release/writeonce-logind                               "$STAGING/usr/sbin/writeonce-logind"
install_if_present target/release/writeonce-session-create                       "$STAGING/usr/sbin/writeonce-session-create"

# ---- 3b. install i3 from i3More's meson install-root -----------------------
echo
echo "==== [3b/8] Installing i3 (from i3More's meson install-root)"
# i3 is the user's fork at github.com:shoneyJ/i3, built via i3More's
# Dockerfile.i3 + justfile pipeline (just i3-build && just i3-stage).
# The staged install lives at vendor/i3/build/install-root/usr/local/.
# We copy it into our sysroot under /usr/ (NOT /usr/local) so it's on
# the default PATH for everyone.
#
# Override I3_INSTALL_ROOT to point at a different install-root.
I3_INSTALL_ROOT="${I3_INSTALL_ROOT:-.agents/reference/i3More/vendor/i3/build/install-root}"
I3_SRC_USR="$I3_INSTALL_ROOT/usr/local"
if [[ -d "$I3_SRC_USR/bin" ]]; then
    # Copy /usr/local/{bin,etc,share,lib} into staging /usr/{bin,etc,share,lib}.
    for sub in bin etc share lib; do
        if [[ -d "$I3_SRC_USR/$sub" ]]; then
            cp -av "$I3_SRC_USR/$sub/." "$STAGING/usr/$sub/" 2>/dev/null \
                | tail -3 | sed 's/^/    /'
        fi
    done
    echo "    i3 installed from $I3_SRC_USR"
else
    cat <<EOF
    WARN: $I3_SRC_USR/bin does not exist — i3 not installed.

    Build it on the host via i3More's tooling:
        cd .agents/reference/i3More
        just i3-image
        just i3-build
        just i3-stage         # populates vendor/i3/build/install-root

    Or set I3_INSTALL_ROOT=<other-path> and re-run this script.

    Without i3, the resulting sysroot boots to writeonce-login but
    .xinitrc fails on 'exec i3' — user lands back at a re-prompted
    login.
EOF
fi

# ---- 3c. install pre-built i3More binaries from /opt/i3more/bin ------------
echo
echo "==== [3c/8] Installing i3More binaries"
# i3More is built out-of-tree on the workstation (docker compose run
# dev cargo build --release …). The operator's existing build installs
# to /opt/i3more/bin/ which we copy verbatim into the WriteOnce sysroot.
#
# ABI assumption: the workstation's glibc + GTK4 + libpipewire + libpam
# versions are forward-compatible with the WriteOnce sysroot versions
# (glibc 2.40, GTK4 4.16.7, pipewire 1.2.7). In practice this works
# because newer glibc + GTK4 are backward-compatible.
#
# Override I3MORE_BIN_DIR to point elsewhere (e.g. a CI cache or the
# i3More repo's dist/ directory).
I3MORE_BIN_DIR="${I3MORE_BIN_DIR:-/opt/i3more/bin}"
if [[ -d "$I3MORE_BIN_DIR" ]]; then
    copied=0
    for bin in "$I3MORE_BIN_DIR"/i3more*; do
        [[ -f "$bin" && -x "$bin" ]] || continue
        name="$(basename "$bin")"
        install -Dm755 "$bin" "$STAGING/usr/bin/$name"
        echo "    $bin → /usr/bin/$name"
        copied=$((copied + 1))
    done
    if [[ $copied -eq 0 ]]; then
        echo "    WARN: $I3MORE_BIN_DIR is empty — no i3More binaries installed"
    else
        echo "    installed $copied i3More binaries"
    fi
else
    cat <<EOF
    WARN: $I3MORE_BIN_DIR does not exist — skipping i3More install.
          The booted system will boot to i3 but lack the i3More UX
          layer (no launcher, lock, audio applet, etc.).
          To install: build i3More on the host (see i3More's README),
          ensure binaries land at /opt/i3more/bin/, then re-run.
          Or set I3MORE_BIN_DIR=<other-path> and re-run this script.
EOF
fi

# ---- 4. overlay the skeleton tree ------------------------------------------
echo
echo "==== [4/8] Overlaying build/skeleton/"
cp -a build/skeleton/. "$STAGING/"

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

# ---- 5. install service unit TOMLs -----------------------------------------
echo
echo "==== [5/8] Installing service units"
for unit in crates/writeonce-svc/examples/services/*.toml; do
    cp -v "$unit" "$STAGING/etc/writeonce/services/" | sed 's/^/    /'
done

# ---- 6. install dbus policy + final touch-ups ------------------------------
echo
echo "==== [6/8] D-Bus policy + final touch-ups"
mkdir -p "$STAGING/etc/dbus-1/system.d"
if [[ -f crates/writeonce-logind/examples/dbus-policy.conf ]]; then
    cp crates/writeonce-logind/examples/dbus-policy.conf \
       "$STAGING/etc/dbus-1/system.d/org.freedesktop.login1.conf"
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
echo "Next: ./build/18-make-artifacts.sh — bundles for the installer."
