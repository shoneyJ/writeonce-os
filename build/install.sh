#!/usr/bin/env bash
# build/install.sh — install WriteOnce OS (systemd branch) to a block device.
#
# Replaces the Rust writeonce-installer on this branch. Happy path:
#   GPT (ESP + ext4 root) → mkfs → extract sysroot.tar.zst → install the
#   EFI-stub kernel as \EFI\BOOT\BOOTX64.EFI → set a password → done.
#
# Boot model: the kernel IS the bootloader (EFI stub), with root= + init baked
# into CONFIG_CMDLINE. The root partition is given a FIXED GPT PARTUUID that
# matches that baked cmdline, so this stays install-agnostic.
#
#   sudo ./build/install.sh /dev/sdX            (or: just install /dev/sdX)
#
# DESTROYS all data on the target device. Runs on the host/live-env (x86_64).

set -euo pipefail

DEV="${1:?usage: sudo ./build/install.sh /dev/sdX}"
OUT="${OUT:-build/artifacts}"
# Must match CONFIG_CMDLINE root=PARTUUID in kernel-config-additions.fragment.
ROOT_PARTUUID="b0007001-c0de-4f5e-9abc-000000000002"
DEFAULT_PASSWORD="${WRITEONCE_PASSWORD:-writeonce}"

[[ $(id -u) -eq 0 ]] || { echo "error: must run as root (sudo)." >&2; exit 1; }
[[ -b "$DEV" ]] || { echo "error: $DEV is not a block device." >&2; exit 1; }
for f in BOOTX64.EFI sysroot.tar.zst; do
    [[ -f "$OUT/$f" ]] || { echo "error: $OUT/$f missing — run ./build/18-make-artifacts.sh." >&2; exit 1; }
done
for t in sgdisk mkfs.vfat mkfs.ext4 zstd tar; do
    command -v "$t" >/dev/null || { echo "error: '$t' not found (need gptfdisk, dosfstools, e2fsprogs, zstd, tar)." >&2; exit 1; }
done

# Refuse a mounted target.
if grep -q "^$DEV" /proc/mounts; then
    echo "error: $DEV (or a partition of it) is mounted. Unmount first." >&2; exit 1
fi

MODEL=$(lsblk -ndo MODEL "$DEV" 2>/dev/null || true)
SIZE=$(lsblk -ndo SIZE "$DEV" 2>/dev/null || true)
echo "About to WIPE $DEV  [$MODEL  $SIZE]  and install WriteOnce OS (systemd)."
read -r -p "Type 'yes' to continue: " ans
[[ "$ans" == "yes" ]] || { echo "aborted."; exit 1; }

echo "==== [1/6] Partitioning GPT (ESP 512MiB + ext4 root) ===="
wipefs -a "$DEV" >/dev/null 2>&1 || true
sgdisk --zap-all "$DEV" >/dev/null
sgdisk \
    --new=1:0:+512MiB --typecode=1:EF00 --change-name=1:'WRITEONCE-ESP' \
    --new=2:0:0       --typecode=2:8300 --change-name=2:'writeonce-root' \
    --partition-guid=2:"$ROOT_PARTUUID" \
    "$DEV" >/dev/null
# Settle + locate the partition device nodes (sdX1/sdX2 or nvme..p1/p2).
partprobe "$DEV" 2>/dev/null || true; sleep 1
ESP=$(lsblk -lnpo NAME "$DEV" | sed -n '2p')
ROOT=$(lsblk -lnpo NAME "$DEV" | sed -n '3p')
echo "    ESP=$ESP  ROOT=$ROOT  (root PARTUUID=$ROOT_PARTUUID)"

echo "==== [2/6] Formatting (vfat ESP label WRITEONCE + ext4 root label writeonce-root) ===="
mkfs.vfat -F32 -n WRITEONCE "$ESP" >/dev/null
mkfs.ext4 -F -L writeonce-root "$ROOT" >/dev/null

echo "==== [3/6] Extracting sysroot ===="
MNT=$(mktemp -d)
mount "$ROOT" "$MNT"
zstd -d -c "$OUT/sysroot.tar.zst" | tar -x -C "$MNT" --numeric-owner
# /home/writeonce was forced to uid/gid 0 in the tar; restore to 1000:1000.
chown -R 1000:1000 "$MNT/home/writeonce" 2>/dev/null || true

# dbus's activation helper must be setuid root, group messagebus (the unprivileged
# build + uid-0 tar can't set this). Without it, dbus system-bus service activation
# fails ("permission of the setuid helper is not correct").
for h in "$MNT/usr/libexec/dbus-daemon-launch-helper" "$MNT/usr/lib/dbus-1.0/dbus-daemon-launch-helper"; do
    if [[ -f "$h" ]]; then
        chown root:messagebus "$h" && chmod 4750 "$h" && echo "    set setuid root:messagebus on $(basename "$h")"
    fi
done

echo "==== [4/6] Installing EFI-stub kernel → \\EFI\\BOOT\\BOOTX64.EFI ===="
mkdir -p "$MNT/boot/efi"
mount "$ESP" "$MNT/boot/efi"
mkdir -p "$MNT/boot/efi/EFI/BOOT"
install -m644 "$OUT/BOOTX64.EFI" "$MNT/boot/efi/EFI/BOOT/BOOTX64.EFI"

echo "==== [5/6] Setting passwords (root + writeonce = '$DEFAULT_PASSWORD') ===="
# Hash on the host (sha512crypt) and edit /etc/shadow directly. This avoids a
# chroot + chpasswd/PAM round-trip: shadow's PAM config cross-installed under
# /usr/etc (not /etc), so chpasswd-in-chroot is unreliable. A direct shadow
# edit has no such dependency.
HASH=$(openssl passwd -6 "$DEFAULT_PASSWORD" 2>/dev/null \
       || python3 -c 'import crypt,sys; print(crypt.crypt(sys.argv[1], crypt.mksalt(crypt.METHOD_SHA512)))' "$DEFAULT_PASSWORD")
if [[ -n "$HASH" ]] && grep -q '^root:' "$MNT/etc/shadow"; then
    for u in root writeonce; do
        # Replace the password field (2nd colon-field) of the user's line.
        sed -i "s|^\($u\):[^:]*:|\1:$HASH:|" "$MNT/etc/shadow"
    done
    echo "    passwords set (change after first boot: passwd)"
else
    echo "    FATAL: could not hash/set the password (need 'openssl' or 'python3' on the" >&2
    echo "           install host). WriteOnce boots to a password-gated greeter on tty1 —" >&2
    echo "           with no password the account is locked and local login is impossible." >&2
    echo "           Install openssl or python3 (or set WRITEONCE_PASSWORD) and re-run." >&2
    umount "$MNT/boot/efi" 2>/dev/null || true
    umount "$MNT" 2>/dev/null || true
    rmdir "$MNT" 2>/dev/null || true
    exit 1
fi

echo "==== [5b/6] Wi-Fi for first-boot internet (optional) ===="
# Your Wayland compositor + apps are fetched from Nix after first boot, which
# needs internet. Ethernet works with NO config (dhcpcd.service is enabled).
# For Wi-Fi, provision an iwd profile now; blank SSID = skip (ethernet only).
WIFI_SSID_HINT="${WIFI_SSID_HINT:-HOME_SA}"   # workstation's current network
if [[ -t 0 ]]; then
    read -r -p "Wi-Fi SSID (blank = skip, use ethernet; workstation is on '$WIFI_SSID_HINT'): " WSSID
    if [[ -n "${WSSID:-}" ]]; then
        read -r -s -p "Wi-Fi passphrase for '$WSSID': " WPSK; echo
        if [[ -n "$WPSK" ]]; then
            install -d -m700 "$MNT/var/lib/iwd"
            # iwd reads /var/lib/iwd/<SSID>.psk; Passphrase is plaintext (iwd derives the PSK).
            ( umask 077; cat > "$MNT/var/lib/iwd/${WSSID}.psk" <<EOF
[Security]
Passphrase=${WPSK}

[Settings]
AutoConnect=true
EOF
            )
            chmod 600 "$MNT/var/lib/iwd/${WSSID}.psk"
            echo "    iwd profile written for '$WSSID' (auto-connect on boot)"
        else
            echo "    empty passphrase — skipped Wi-Fi (ethernet only)"
        fi
    else
        echo "    skipped Wi-Fi (ethernet via dhcpcd works with no config)"
    fi
else
    echo "    non-interactive — skipped Wi-Fi (provision /var/lib/iwd/<SSID>.psk manually)"
fi

echo "==== [5c/6] SSH public key for remote access (optional) ===="
# WriteOnce enables sshd via Nix (run 'sudo wo-sshd-setup' once on the target).
# Authorizing your workstation key now makes that first SSH key-only + ready.
if [[ -t 0 ]]; then
    SSH_USER_HOME=$(getent passwd "${SUDO_USER:-root}" | cut -d: -f6)
    DEFAULT_PUBKEY=""
    for k in "$SSH_USER_HOME/.ssh/id_ed25519.pub" "$SSH_USER_HOME/.ssh/id_rsa.pub"; do
        if [[ -f "$k" ]]; then DEFAULT_PUBKEY="$k"; break; fi
    done
    read -r -p "SSH public key to authorize (blank = skip)${DEFAULT_PUBKEY:+ [$DEFAULT_PUBKEY]}: " PUBKEY_PATH
    PUBKEY_PATH="${PUBKEY_PATH:-$DEFAULT_PUBKEY}"
    if [[ -n "${PUBKEY_PATH:-}" && -f "$PUBKEY_PATH" ]]; then
        install -d -m700 "$MNT/home/writeonce/.ssh"
        cat "$PUBKEY_PATH" >> "$MNT/home/writeonce/.ssh/authorized_keys"
        chmod 600 "$MNT/home/writeonce/.ssh/authorized_keys"
        chown -R 1000:1000 "$MNT/home/writeonce/.ssh"
        echo "    authorized $(basename "$PUBKEY_PATH") (key-only SSH after 'sudo wo-sshd-setup')"
    elif [[ -n "${PUBKEY_PATH:-}" ]]; then
        echo "    no key file at '$PUBKEY_PATH' — skipped (password SSH still works)"
    else
        echo "    skipped SSH key (password SSH works after 'sudo wo-sshd-setup')"
    fi
else
    echo "    non-interactive — skipped SSH key"
fi

echo "==== [6/6] sync + unmount ===="
sync
umount "$MNT/boot/efi"
umount "$MNT"
rmdir "$MNT"

echo
echo "✓ Install complete on $DEV."
echo "  Remove the install medium and boot the target."
echo "  Expected: firmware → bzImage (EFI stub) → systemd → multi-user.target →"
echo "  writeonce-nix-init (registers Nix) → getty autologin (writeonce) →"
echo "  console (writeonce). WriteOnce ships no desktop — install a Wayland compositor"
echo "  via Nix and launch it. Enable SSH with 'sudo wo-sshd-setup', then from the"
echo "  workstation: ssh writeonce@writeonce.local"
