#!/usr/bin/env bash
# build/setup-env.sh — establish a hygienic LFS build environment.
# Sourced (not executed) by the other build scripts:
#     source "$(dirname "$0")/setup-env.sh"
#
# After sourcing, the following are in scope:
#   $BUILD_ROOT  = absolute path to writeonce-os/build/
#   $LFS         = $BUILD_ROOT/sysroot   (the in-progress target rootfs)
#   $LFS_TGT     = x86_64-lfs-linux-gnu  (the cross target triple)
#   $LFS_TOOLS   = $BUILD_ROOT/cross-tools (host-resident cross-toolchain)
#   $SOURCES     = $BUILD_ROOT/sources
#   $LOGS        = $BUILD_ROOT/logs
#   PATH         = scrubbed to $LFS_TOOLS/bin:/usr/bin:/bin
#   LC_ALL=POSIX, umask 022

set -euo pipefail

# Locate ourselves regardless of how we were sourced.
BUILD_ROOT="$( cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd )"
export BUILD_ROOT

# Load pinned versions and LFS_TGT.
# shellcheck disable=SC1091
source "$BUILD_ROOT/versions.env"

export LFS="$BUILD_ROOT/sysroot"
export LFS_TOOLS="$BUILD_ROOT/cross-tools"
export SOURCES="$BUILD_ROOT/sources"
export LOGS="$BUILD_ROOT/logs"

# Ensure the directory hierarchy exists.
mkdir -p "$LFS" "$LFS_TOOLS" "$SOURCES" "$LOGS" "$BUILD_ROOT/artifacts"

# The LFS book references $LFS/tools verbatim; expose our cross-tools there
# via a symlink so the book's commands and configure invocations Just Work.
if [[ ! -e "$LFS/tools" ]]; then
    ln -snf "$LFS_TOOLS" "$LFS/tools"
fi

# Scrub the environment for reproducibility. The two principal failure modes
# we are guarding against are (a) host LD_LIBRARY_PATH leaking into builds,
# and (b) the host's locale corrupting Make's parsing of build output.
unset CC CXX CPP CFLAGS CXXFLAGS LDFLAGS CPPFLAGS PKG_CONFIG_PATH
unset LD_LIBRARY_PATH LD_PRELOAD LD_RUN_PATH
export LC_ALL=POSIX
export LANG=POSIX
umask 022

# PATH: only the host basics plus our growing cross-toolchain.
export PATH="$LFS_TOOLS/bin:/usr/bin:/bin"

# Helper used by every script: log to both terminal and a per-step file.
# Usage: run_step "binutils-pass-1" command args...
LFS_log() {
    local step="$1"; shift
    local logfile="$LOGS/${step}.log"
    printf '\n=== [%s] %s ===\n' "$(date -Is)" "$step" | tee -a "$logfile"
    "$@" 2>&1 | tee -a "$logfile"
}
export -f LFS_log
