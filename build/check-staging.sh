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
    # /etc — essentials read by libc / login / dbus
    etc/passwd
    etc/group
    etc/shadow
    etc/hostname
    etc/hosts
    etc/fstab

    # /etc/writeonce — pid1 + service supervisor configuration
    etc/writeonce/pid1.toml
    etc/writeonce/services/console.target.toml
    etc/writeonce/services/default.target.toml
    etc/writeonce/services/multi-user.target.toml
    etc/writeonce/services/sysinit.target.toml
    etc/writeonce/services/dbus.service.toml
    etc/writeonce/services/logind.service.toml
    etc/writeonce/services/writeonce-login.service.toml
    etc/writeonce/services/writeonce-bootstrap.service.toml

    # PAM
    etc/pam.d/writeonce-login
    etc/pam.d/sudo

    # WriteOnce binaries — the boot-chain set
    usr/sbin/writeonce-pid1
    usr/sbin/writeonce-svc
    usr/sbin/writeonce-login
    usr/sbin/writeonce-logind
    usr/sbin/writeonce-bootstrap
    usr/bin/wo-ctl

    # Userspace tools the bootstrap + services shell out to
    usr/bin/bash
    usr/bin/dbus-daemon
    usr/sbin/dbus-daemon

    # X11 session launcher (startx → X(org) → ~/.xinitrc → i3 + i3More).
    # writeonce-session-create execs /usr/bin/startx after login.
    usr/bin/startx
    usr/bin/X
    usr/bin/Xorg

    # Required shared libs (caught the May-2026 libpam regression)
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

# Dynamic deps of the glibc-linked boot-chain binaries must all resolve
# inside the staged /usr/lib. Caught the May-2026 libpam regression
# (writeonce-login) and guards the dbus → logind handshake — a missing
# transitive lib is a silent boot failure (service exits 127 / 1).
DBUS_BIN=""
for c in usr/bin/dbus-daemon usr/sbin/dbus-daemon; do
    [ -f "$STAGING/$c" ] && { DBUS_BIN="$c"; break; }
done
LDD_BINS=(usr/sbin/writeonce-login usr/sbin/writeonce-logind)
[ -n "$DBUS_BIN" ] && LDD_BINS+=("$DBUS_BIN")

for b in "${LDD_BINS[@]}"; do
    [ -f "$STAGING/$b" ] || { fail "$(basename "$b"): not staged (cannot ldd)"; continue; }
    missing=$(LD_LIBRARY_PATH="$STAGING/usr/lib" ldd "$STAGING/$b" 2>&1 | grep 'not found' || true)
    if [ -z "$missing" ]; then
        pass "$(basename "$b"): all shared libraries resolved"
    else
        fail "$(basename "$b"): missing shared libraries:"
        printf "%s\n" "$missing" | sed 's/^/        /'
    fi
done

# ---------------------------------------------------------------------------
# /run must be empty in staging — bootstrap creates content at boot
# ---------------------------------------------------------------------------

echo
echo "== /run =="

if [ -d "$STAGING/run" ]; then
    n=$(find "$STAGING/run" -mindepth 1 2>/dev/null | wc -l)
    if [ "$n" -eq 0 ]; then
        pass "/run is empty in staging (correct — tmpfs at boot, populated by writeonce-bootstrap)"
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

# enabled.d directory exists (even if empty — bootstrap reads it)
if [ -d "$STAGING/etc/writeonce/enabled.d" ]; then
    pass "/etc/writeonce/enabled.d/ directory present"
else
    fail "/etc/writeonce/enabled.d/ directory missing — wo-ctl enable will fail"
fi

# bootstrap script is executable
if [ -x "$STAGING/usr/sbin/writeonce-bootstrap" ]; then
    pass "writeonce-bootstrap is executable"
else
    fail "writeonce-bootstrap is not executable (mode mismatch)"
fi

# /bin/sh symlink in place (writeonce-bootstrap shebang is #!/bin/sh)
if [ -e "$STAGING/usr/bin/sh" ] || [ -e "$STAGING/bin/sh" ]; then
    pass "/bin/sh present (bootstrap shebang resolvable)"
else
    fail "/bin/sh missing — writeonce-bootstrap's shebang #!/bin/sh fails"
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
