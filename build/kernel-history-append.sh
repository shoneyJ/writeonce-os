#!/usr/bin/env bash
# build/kernel-history-append.sh — append one entry to
# docs/kernel-build-history.md describing the kernel we just built.
#
# Called automatically by `just kernel` after 04-kernel.sh +
# 05-initramfs.sh complete successfully. Manual use:
#   ./build/kernel-history-append.sh "reason for this rebuild"

set -euo pipefail

cd "$( dirname "${BASH_SOURCE[0]}" )/.."

REASON="${1:-${KERNEL_REBUILD_REASON:-(no reason supplied)}}"
HIST="docs/kernel-build-history.md"
ART="build/artifacts"

[[ -f "$ART/bzImage" ]]       || { echo "no bzImage at $ART/bzImage — skipping history append" >&2; exit 0; }
[[ -f "$ART/initramfs.img" ]] || { echo "no initramfs at $ART/initramfs.img — skipping" >&2; exit 0; }

sha() { sha256sum "$1" | awk '{print $1}'; }
sz()  { stat -c%s "$1"; }

TS="$(date -u +%FT%TZ)"
GIT_SHA="$(git rev-parse --short HEAD 2>/dev/null || echo unknown)"
GIT_SUBJECT="$(git log -1 --pretty=%s 2>/dev/null || echo '(no git)')"
FRAG_SHA="$(sha build/kernel-config-additions.fragment 2>/dev/null | head -c 12)"
BZ_SHA="$(sha "$ART/bzImage")"
INI_SHA="$(sha "$ART/initramfs.img")"
BZ_SZ="$(sz "$ART/bzImage")"
INI_SZ="$(sz "$ART/initramfs.img")"

{
    echo
    echo "## $TS"
    printf 'bzImage    : %12s bytes  sha256=%s\n' "$BZ_SZ"  "$BZ_SHA"
    printf 'initramfs  : %12s bytes  sha256=%s\n' "$INI_SZ" "$INI_SHA"
    printf 'git        : %s  %s\n' "$GIT_SHA" "$GIT_SUBJECT"
    printf 'fragment   : sha256=%s…\n' "$FRAG_SHA"
    printf 'reason     : %s\n' "$REASON"
} >> "$HIST"

echo "kernel-history: appended entry to $HIST (reason: $REASON)"
