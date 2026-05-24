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
unset CC CXX CPP PKG_CONFIG_PATH
unset LD_LIBRARY_PATH LD_PRELOAD LD_RUN_PATH
export LC_ALL=POSIX
export LANG=POSIX
umask 022

# --sysroot points cross-gcc + cross-ld at $LFS so they find libc + crt*.o
# at $LFS/usr/lib/ instead of the toolchain-internal (and empty) sysroot.
# Required because our Phase 0 gcc was NOT built with --with-sysroot=$LFS;
# we compensate at every invocation via CFLAGS/CXXFLAGS/LDFLAGS.
export CFLAGS="--sysroot=$LFS -D_GNU_SOURCE -include linux/limits.h -DLOGIN_NAME_MAX=256 -DHOST_NAME_MAX=64 -DLINE_MAX=2048"
export CXXFLAGS="--sysroot=$LFS -D_GNU_SOURCE -include linux/limits.h -DLOGIN_NAME_MAX=256 -DHOST_NAME_MAX=64 -DLINE_MAX=2048"
export LDFLAGS="--sysroot=$LFS"
# CPPFLAGS = preprocessor flags. libpng's Makefile uses CPP directly
# (not CC) for pnglibconf.c → drops CFLAGS but honours CPPFLAGS, so
# without this the sysroot include path is lost.
# -D_GNU_SOURCE + -include linux/limits.h expose PATH_MAX +
# LOGIN_NAME_MAX which linux-pam (and others) reference at file scope
# without explicitly including limits.h.
export CPPFLAGS="--sysroot=$LFS -D_GNU_SOURCE -include linux/limits.h -DLOGIN_NAME_MAX=256 -DHOST_NAME_MAX=64 -DLINE_MAX=2048"

# Cross-compile cached answers for autoconf AC_TRY_RUN tests. Without
# these, libX11 (and many others) abort with "cannot run test program
# while cross compiling". Values reflect modern glibc + Linux behaviour.
# Exported directly so autoconf's cache-vars-from-env path picks them up
# (CONFIG_SITE-via-file occasionally isn't read; env vars always work).
export CONFIG_SITE="$BUILD_ROOT/cross-config.site"
export ac_cv_func_malloc_0_nonnull=yes
export ac_cv_func_realloc_0_nonnull=yes
export ac_cv_func_fork_works=yes
export ac_cv_func_vfork_works=yes
export ac_cv_func_mmap_fixed_mapped=yes
export ac_cv_func_chown_works=yes
export ac_cv_func_getpgrp_void=yes
export ac_cv_func_setvbuf_reversed=no
export ac_cv_func_strerror_r_char_p=yes
export ac_cv_func_stat_empty_string_bug=no
export ac_cv_func_lstat_dereferences_slashed_symlink=yes
export ac_cv_func_lstat_empty_string_bug=no
export ac_cv_func_stat_ignores_trailing_slash=no
export ac_cv_func_pread=yes
export ac_cv_func_pwrite=yes
export ac_cv_search_clock_gettime='none required'

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
