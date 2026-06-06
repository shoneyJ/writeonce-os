#!/usr/bin/env bash
# build/17a-install-nix.sh — Phase 14: stage the single-user Nix store offline.
#
# Runs AFTER 17-stage-sysroot.sh (which resets $STAGING) and BEFORE
# 18-make-artifacts.sh — its name sorts between them ("17-" < "17a" < "18").
# Unpacks the hash-locked Nix static tarball (fetched by 01-fetch.sh) into
# $STAGING/nix and lays the single-user skeleton. The store-DB load +
# default-profile creation happen on FIRST BOOT (writeonce-nix-init.service),
# because Nix store paths are absolute (/nix) and must be registered on the
# target, not in $STAGING.
#
# RUNS ON THE HOST (not in wo-builder): pure untar + cp + mkdir.
# See plan/phase-14-nix-packages.md. UNVERIFIED — no Nix on this workstation.

set -euo pipefail
cd "$( dirname "${BASH_SOURCE[0]}" )/.."
# shellcheck disable=SC1091
source ./build/setup-env.sh

# Flavor gate: only flavors whose desktop is delivered via Nix bootstrap /nix.
# Source-built-desktop flavors (FLAVOR_PKG=source) skip this entirely.
[[ "${FLAVOR_PKG:-nix}" == nix ]] || {
    echo "17a-install-nix: skip — FLAVOR_PKG=$FLAVOR_PKG (flavor $FLAVOR ships no Nix store)"
    exit 0
}

STAGING="${STAGING:-build/staging/sysroot}"
TARBALL="$SOURCES/nix-${NIX_VERSION}-x86_64-linux.tar.xz"

echo "==== writeonce install-nix (Phase 14) ===="
echo " NIX_VERSION: $NIX_VERSION"
echo " STAGING:     $STAGING"

[[ -d "$STAGING/usr" ]] || { echo "error: run ./build/17-stage-sysroot.sh first ($STAGING/usr missing)" >&2; exit 1; }
[[ -f "$TARBALL" ]]     || { echo "error: $TARBALL missing — run ./build/01-fetch.sh" >&2; exit 1; }

# ---- 1. unpack -------------------------------------------------------------
work="$BUILD_ROOT/work/nix-bootstrap"
rm -rf "$work"; mkdir -p "$work"
echo
echo "==== [1/3] Unpacking $(basename "$TARBALL")"
tar -xf "$TARBALL" -C "$work" --strip-components=1   # → $work/{store,.reginfo,install,…}
[[ -d "$work/store" ]] || { echo "error: tarball has no store/ — unexpected layout" >&2; exit 1; }

# ---- 2. stage /nix ---------------------------------------------------------
echo
echo "==== [2/3] Staging /nix store (~hundreds of MB)"
mkdir -p "$STAGING/nix/store"
cp -a "$work/store/." "$STAGING/nix/store/"
# .reginfo: store-path DB dump consumed by first-boot `nix-store --load-db`.
if   [[ -f "$work/.reginfo" ]]; then cp -a "$work/.reginfo" "$STAGING/nix/.reginfo"
elif [[ -f "$work/reginfo"  ]]; then cp -a "$work/reginfo"  "$STAGING/nix/.reginfo"
else echo "    WARN: no .reginfo in tarball — first-boot DB load may fail"; fi

# single-user var skeleton
mkdir -p "$STAGING/nix/var/nix/profiles/per-user/writeonce"
mkdir -p "$STAGING/nix/var/nix/gcroots/per-user/writeonce"
mkdir -p "$STAGING/nix/var/nix/temproots" "$STAGING/nix/var/log/nix"

# ---- 3. ownership (best-effort; authoritative chown is first-boot) ---------
echo
echo "==== [3/3] Ownership"
chown -R 1000:1000 "$STAGING/nix" 2>/dev/null \
    || echo "    (not root: /nix is chowned to writeonce on first boot by writeonce-nix-init)"

echo
echo "Nix store staged:"; du -sh "$STAGING/nix" 2>/dev/null || true
echo "Next: ./build/18-make-artifacts.sh"
