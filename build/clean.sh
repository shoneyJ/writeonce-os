#!/usr/bin/env bash
# build/clean.sh — wipe in-progress build state.
#
# Usage:
#     ./clean.sh              # remove cross-tools/, sysroot/, work/, artifacts/, logs/
#                             # (keeps sources/ and checksums.txt)
#     ./clean.sh --all        # also remove sources/ (forces redownload)
#     ./clean.sh --gnupg      # also remove the project-local GPG keyring
#
# After ./clean.sh, the next ./cross-toolchain.sh run starts from scratch.

set -euo pipefail

cd "$( dirname "${BASH_SOURCE[0]}" )"
mode="${1:-default}"

rm -rf cross-tools/ sysroot/ work/ artifacts/ logs/
echo "cleaned: cross-tools/ sysroot/ work/ artifacts/ logs/"

case "$mode" in
    --all)    rm -rf sources/ ; echo "cleaned: sources/" ;;
    --gnupg)  rm -rf gnupg/  ; echo "cleaned: gnupg/" ;;
esac
