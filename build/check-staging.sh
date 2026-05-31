#!/usr/bin/env bash
# build/check-staging.sh — pre-flight validation of the staged sysroot.
#
# Runs every check that *would* have caught one of the boot bugs from
# this session's bring-up rounds. Catches missing libs / unit files /
# skeleton entries on the workstation, before USB flash + boot.
#
# Exit 0 = clean (safe to flash). Exit 1 = at least one check failed
# (do not flash; fix first).
#
# Invoke via `just check-staging`; `just usb-install` runs this first
# and refuses to flash on failure.
#
# To add a new check: append a `check_*` function and a call site
# below. Each function prints `[FAIL]` / `[PASS]` and increments the
# fail counter on failure.

set -u

STAGING="${STAGING:-build/staging/sysroot}"
FAIL=0
TOTAL=0

red()    { printf '\033[31m%s\033[0m' "$1"; }
green()  { printf '\033[32m%s\033[0m' "$1"; }
yellow() { printf '\033[33m%s\033[0m' "$1"; }

pass() { TOTAL=$((TOTAL+1)); printf "  [%s] %s\n" "$(green PASS)" "$1"; }
fail() { TOTAL=$((TOTAL+1)); FAIL=$((FAIL+1)); printf "  [%s] %s\n" "$(red FAIL)" "$1"; }

# ---------------------------------------------------------------------------
# Bedrock — the staging dir itself
# ---------------------------------------------------------------------------

if [ ! -d "$STAGING" ]; then
    echo "$(red ERROR): staging dir $STAGING does not exist."
    echo "  Run \`just stage\` first."
    exit 2
fi

echo "writeonce check-staging — auditing $STAGING"

# ---------------------------------------------------------------------------
# Required files (the long checklist)
# ---------------------------------------------------------------------------

echo
echo "== files =="

REQUIRED_FILES=(
    # /etc — essentials read by libc / login / systemd
    etc/passwd
    etc/group
    etc/shadow
    etc/hostname
    etc/hosts
    etc/fstab
    etc/os-release

    # systemd — PID 1 + service manager + logind + udev + journald
    usr/lib/systemd/systemd
    usr/bin/systemctl
    usr/bin/journalctl
    usr/bin/loginctl
    usr/lib/systemd/systemd-logind
    usr/lib/systemd/systemd-udevd
    usr/lib/systemd/system/graphical.target
    usr/lib/systemd/system/getty@.service

    # systemd userspace config (skeleton overlay)
    etc/systemd/system/default.target
    etc/systemd/system/getty@tty1.service.d/autologin.conf

    # PAM: shadow login auth + pam_systemd (logind session registration)
    etc/pam.d/login
    usr/lib/security/pam_unix.so
    usr/lib/security/pam_systemd.so

    # Session entry → X → i3 + i3More
    home/writeonce/.bash_profile
    home/writeonce/.xinitrc

    # (No bootloader in the rootfs: the EFI-stub kernel is the loader, staged
    #  to the ESP by 18-make-artifacts / install.sh — validated there, not here.)

    # D-Bus system bus (systemd-logind speaks to it)
    usr/bin/bash
    usr/bin/dbus-daemon
    usr/sbin/dbus-daemon

    # X11 session launcher (startx → X(org) → ~/.xinitrc → i3 + i3More)
    usr/bin/startx
    usr/bin/X
    usr/bin/Xorg

    # Required shared libs
    usr/lib/libpam.so.0
    usr/lib/libc.so.6
    usr/lib/libgcc_s.so.1
)

# Some entries are "either /usr/bin or /usr/sbin" — handled below as
# a special case (dbus-daemon ships in both on different distros).

for f in "${REQUIRED_FILES[@]}"; do
    if [ -e "$STAGING/$f" ] || [ -L "$STAGING/$f" ]; then
        pass "$f"
    else
        # dbus-daemon special case: pass if either /usr/bin or
        # /usr/sbin variant exists.
        case "$f" in
            usr/bin/dbus-daemon|usr/sbin/dbus-daemon)
                if [ -e "$STAGING/usr/bin/dbus-daemon" ] || [ -e "$STAGING/usr/sbin/dbus-daemon" ]; then
                    pass "$f (alt path present)"
                else
                    fail "$f"
                fi
                ;;
            *)
                fail "$f"
                ;;
        esac
    fi
done

# ---------------------------------------------------------------------------
# GNU base userspace (LFS Ch8). Their absence is what halted sysinit.target's
# /bin/true and would break writeonce-bootstrap (mkdir/chown/tr/od) and the
# desktop .xinitrc (mkdir) — see the 2026-05-30 boot. This check makes the
# gap a workstation failure instead of a T450 surprise.
# ---------------------------------------------------------------------------

echo
echo "== base userspace =="

# coreutils + sed/grep/tar/gzip — install to /usr/bin.
for b in true false ls cat cp mkdir chmod chown ln tr od sleep env sed grep tar gzip; do
    if [ -x "$STAGING/usr/bin/$b" ]; then
        pass "usr/bin/$b"
    else
        fail "usr/bin/$b (base userspace missing — rebuild via 03 / 14-base-userspace)"
    fi
done

# util-linux / kmod / procps / shadow tools land in /usr/bin, /usr/sbin, or
# /sbin. login (shadow) + agetty (util-linux) drive the getty autologin.
for b in mount modprobe ps agetty login passwd; do
    if [ -x "$STAGING/usr/bin/$b" ] || [ -x "$STAGING/usr/sbin/$b" ] || [ -x "$STAGING/sbin/$b" ]; then
        pass "$b (base tool present)"
    else
        fail "$b missing (util-linux/kmod/procps/shadow not staged)"
    fi
done

# ---------------------------------------------------------------------------
# System users in passwd (writeonce-bootstrap needs messagebus = UID 99)
# ---------------------------------------------------------------------------

echo
echo "== users =="

for u in root messagebus; do
    if grep -q "^$u:" "$STAGING/etc/passwd" 2>/dev/null; then
        pass "user '$u' in /etc/passwd"
    else
        fail "user '$u' missing from /etc/passwd"
    fi
done

# ---------------------------------------------------------------------------
# Library resolution — writeonce-login dynamically links libpam etc.
# Confirm the dynamic loader finds everything via staging's /usr/lib.
# ---------------------------------------------------------------------------

echo
echo "== ldd =="

# Dynamic deps of the core boot binaries must all resolve inside the staged
# /usr/lib. systemd (PID 1) or login failing to link = an unbootable image.
DBUS_BIN=""
for c in usr/bin/dbus-daemon usr/sbin/dbus-daemon; do
    [ -f "$STAGING/$c" ] && { DBUS_BIN="$c"; break; }
done
LDD_BINS=(usr/lib/systemd/systemd usr/lib/systemd/systemd-logind)
[ -n "$DBUS_BIN" ] && LDD_BINS+=("$DBUS_BIN")
for c in usr/bin/login bin/login usr/sbin/login; do
    [ -f "$STAGING/$c" ] && { LDD_BINS+=("$c"); break; }
done

for b in "${LDD_BINS[@]}"; do
    [ -f "$STAGING/$b" ] || { fail "$(basename "$b"): not staged (cannot ldd)"; continue; }
    # Include /usr/lib/systemd — systemd's private libs (libsystemd-core/shared)
    # live there and the binaries RUNPATH to it (resolves on the target).
    missing=$(LD_LIBRARY_PATH="$STAGING/usr/lib:$STAGING/usr/lib/systemd" ldd "$STAGING/$b" 2>&1 | grep 'not found' || true)
    if [ -z "$missing" ]; then
        pass "$(basename "$b"): all shared libraries resolved"
    else
        fail "$(basename "$b"): missing shared libraries:"
        printf "%s\n" "$missing" | sed 's/^/        /'
    fi
done

# The dynamic loader (ELF interpreter) must resolve to a real file. A self-
# looping symlink here makes every binary fail to execve with ELOOP — the
# kernel panics on PID 1. The ldd checks above can't catch this (they run on
# the host loader via LD_LIBRARY_PATH), so assert it explicitly.
LOADER="$STAGING/usr/lib/ld-linux-x86-64.so.2"
if realpath -e "$LOADER" >/dev/null 2>&1; then
    pass "dynamic loader ld-linux-x86-64.so.2 resolves to a real file"
else
    fail "dynamic loader ld-linux-x86-64.so.2 does not resolve (ELOOP/dangling) — every binary would fail to exec (PID 1 panic, error -40)"
fi

# ---------------------------------------------------------------------------
# /run must be empty in staging — bootstrap creates content at boot
# ---------------------------------------------------------------------------

echo
echo "== /run =="

if [ -d "$STAGING/run" ]; then
    n=$(find "$STAGING/run" -mindepth 1 2>/dev/null | wc -l)
    if [ "$n" -eq 0 ]; then
        pass "/run is empty in staging (correct — tmpfs at boot, populated by systemd-tmpfiles)"
    else
        fail "/run is NOT empty in staging — content will be shadowed by tmpfs at boot"
        find "$STAGING/run" -mindepth 1 -maxdepth 2 | sed 's/^/        /'
    fi
else
    pass "/run absent (created at runtime via tmpfs mount)"
fi

# ---------------------------------------------------------------------------
# Skeleton hygiene — common one-line config files
# ---------------------------------------------------------------------------

echo
echo "== skeleton hygiene =="

# default.target must be a symlink to a real boot target. graphical.target =
# full desktop; multi-user.target = the diagnostic text-login image (startx run
# manually). Either is a valid, bootable default.
if [ -L "$STAGING/etc/systemd/system/default.target" ]; then
    tgt=$(readlink "$STAGING/etc/systemd/system/default.target")
    case "$tgt" in
        *graphical.target)  pass "default.target → $tgt (desktop)" ;;
        *multi-user.target) pass "default.target → $tgt (diagnostic text login)" ;;
        *)                  fail "default.target → $tgt (expected graphical.target or multi-user.target)" ;;
    esac
else
    fail "etc/systemd/system/default.target is not a symlink (systemd has no default boot target)"
fi

# getty autologin drop-in present (autologin writeonce on tty1)
if grep -q -- '--autologin' "$STAGING/etc/systemd/system/getty@tty1.service.d/autologin.conf" 2>/dev/null; then
    pass "getty@tty1 autologin drop-in present"
else
    fail "getty@tty1 autologin drop-in missing/empty"
fi

# /bin/sh in place (shebangs, agetty's login shell fallback, etc.)
if [ -e "$STAGING/usr/bin/sh" ] || [ -e "$STAGING/bin/sh" ]; then
    pass "/bin/sh present"
else
    fail "/bin/sh missing"
fi

# systemd-firstboot must be masked — otherwise it prompts interactively on the
# console (timezone/locale/root password) and blocks an unattended boot.
if [ "$(readlink "$STAGING/etc/systemd/system/systemd-firstboot.service" 2>/dev/null)" = "/dev/null" ]; then
    pass "systemd-firstboot.service masked (no interactive prompt)"
else
    fail "systemd-firstboot.service NOT masked — boot will hang on the firstboot prompt"
fi

# Empty /etc/machine-id present → systemd generates a unique id at first boot.
if [ -e "$STAGING/etc/machine-id" ]; then
    pass "/etc/machine-id present (generated at first boot)"
else
    fail "/etc/machine-id missing"
fi

# Persistent journal dir → journalctl -b -1 survives a freeze + reboot.
if [ -d "$STAGING/var/log/journal" ]; then
    pass "/var/log/journal present (persistent journald)"
else
    fail "/var/log/journal missing — boot logs are volatile (lost on reboot)"
fi

# D-Bus system bus units + enablement. systemd-logind (and i3More applets,
# pipewire) need the system bus; dbus was built without systemd unit files, so
# the skeleton ships them. Missing/unenabled → "Failed to start User Login
# Management" and no session/seat tracking.
if [ -f "$STAGING/etc/systemd/system/dbus.socket" ] && [ -f "$STAGING/etc/systemd/system/dbus.service" ]; then
    pass "dbus.socket + dbus.service present"
else
    fail "dbus.socket/dbus.service missing — systemd-logind cannot reach the system bus"
fi
if [ -L "$STAGING/etc/systemd/system/sockets.target.wants/dbus.socket" ]; then
    pass "dbus.socket enabled (sockets.target.wants)"
else
    fail "dbus.socket not enabled — system bus won't start at boot"
fi

# ---------------------------------------------------------------------------
# Verdict
# ---------------------------------------------------------------------------

echo
echo "== verdict =="
if [ $FAIL -eq 0 ]; then
    printf "  %s — %d/%d checks passed.\n" "$(green ALL-CLEAR)" $TOTAL $TOTAL
    printf "  Safe to \`just usb-install /dev/sdX\`.\n"
    exit 0
else
    printf "  %s — %d/%d checks failed.\n" "$(red FAIL)" $FAIL $TOTAL
    printf "  DO NOT flash to USB until these are resolved.\n"
    exit 1
fi
