#!/usr/bin/env bash
# build/14-base-userspace.sh — the LFS Chapter-8 runtime essentials that the
# boot chain + desktop need but 03-sysroot-temp-tools.sh doesn't build.
#
# 03 already builds coreutils/sed/grep/gzip/tar/diffutils/findutils/m4/bash
# into $LFS/usr (the Ch6 set). This round adds the rest of the base system:
#
#   kmod       — modprobe/insmod/lsmod/depmod  (writeonce-modules-load.service)
#   util-linux — mount/umount/lsblk/blkid/...  (filesystem + block tooling)
#   procps-ng  — ps/free/top/uptime/pgrep      (process tooling)
#   shadow     — login/su/passwd/useradd/...   (account tooling; PAM-aware)
#   bzip2      — bzip2/bunzip2 + libbz2.so      (least critical; xz covers most)
#
# Cross-built into $LFS/usr via blfs-pkg.sh's build_pkg (same machinery as the
# Phase-8 stacks 08–13). Sentinel-driven (logs/.done-blfs-<name>); per-step
# logs at logs/blfs-<name>-{configure,make,install}.log.
#
#   ./14-base-userspace.sh              # build every step in order
#   ./14-base-userspace.sh kmod         # build just one
#
# Flag sets below are a reasonable first cut; expect 1–2 `just audit-last`
# iterations on util-linux/shadow as with the rest of Phase 8.

set -uo pipefail

cd "$( dirname "${BASH_SOURCE[0]}" )"
# shellcheck disable=SC1091
source ./setup-env.sh
# shellcheck disable=SC1091
source ./blfs-pkg.sh

# Sanity: coreutils (from 03) should be in place — the base system builds on it.
[[ -x "$LFS/usr/bin/true" ]] || {
    echo "warn: \$LFS/usr/bin/true missing — run 03-sysroot-temp-tools.sh (coreutils) first." >&2
}

# ============================================================================
# kmod — module loader. modprobe is what writeonce-modules-load.service runs.
# ============================================================================
step_kmod() {
    # Auto-detects xz (built in 03) for .ko.xz; openssl off (no module-sig
    # verification needed for our use). build_pkg adds --disable-static.
    build_pkg kmod "kmod-${KMOD_VERSION}.tar.xz" \
        --sysconfdir=/etc \
        --with-xz \
        --without-openssl \
        --with-zstd
    # kmod ships one binary + tool symlinks (modprobe/insmod/lsmod/depmod/
    # rmmod/modinfo). Some installs only create them under /usr/bin; mirror
    # the conventional /usr/sbin names so modprobe resolves on the sbin PATH.
    for t in modprobe insmod lsmod depmod rmmod modinfo; do
        if [[ -e "$LFS/usr/bin/$t" && ! -e "$LFS/usr/sbin/$t" ]]; then
            ln -sf ../bin/"$t" "$LFS/usr/sbin/$t"
        fi
    done
}

# ============================================================================
# util-linux — mount/umount/lsblk/blkid/findmnt/... (no login/su/runuser:
# shadow owns those; avoids duplicate PAM-aware tools).
# ============================================================================
step_util_linux() {
    build_pkg util-linux "util-linux-${UTIL_LINUX_VERSION}.tar.xz" \
        --libdir=/usr/lib \
        --disable-chfn-chsh \
        --disable-login \
        --disable-nologin \
        --disable-su \
        --disable-setpriv \
        --disable-runuser \
        --disable-pylibmount \
        --disable-liblastlog2 \
        --disable-makeinstall-chown \
        --disable-makeinstall-setuid \
        --without-python \
        --without-systemd \
        --without-systemdsystemunitdir
}

# ============================================================================
# procps-ng — ps/free/top/uptime/pgrep/pkill/watch. Needs ncurses (from 03).
# ============================================================================
step_procps_ng() {
    # --disable-kill: coreutils/util-linux provide kill; avoid the clash.
    build_pkg procps-ng "procps-ng-${PROCPS_NG_VERSION}.tar.xz" \
        --disable-kill \
        --without-systemd \
        --enable-watch8bit
}

# ============================================================================
# shadow — login/su/passwd/useradd/groupadd. PAM-aware (libpam from Phase 8a).
# Not boot-blocking (writeonce-login is the PAM login), but completes the base.
# ============================================================================
step_shadow() {
    # Don't install groups(1)/nologin(8): coreutils/util-linux own them.
    build_pkg shadow "shadow-${SHADOW_VERSION}.tar.xz" \
        --without-libbsd \
        --with-group-name-max-length=32
}

# ============================================================================
# bzip2 — Makefile-based (no configure); cross-build by hand. Least critical.
# ============================================================================
step_bzip2() {
    local sentinel="$LOGS/.done-blfs-bzip2"
    [[ -f "$sentinel" ]] && { echo "skip bzip2 (already built)"; return 0; }
    local work="$BUILD_ROOT/work/bzip2"
    rm -rf "$work"; mkdir -p "$work"
    tar -xf "$SOURCES/bzip2-${BZIP2_VERSION}.tar.gz" -C "$work" --strip-components=1
    pushd "$work" >/dev/null
        # Shared library first, then the static-linked tools.
        make -f Makefile-libbz2_so CC="${LFS_TGT}-gcc"            2>&1 | tee "$LOGS/blfs-bzip2-make-so.log" && \
        make clean                                                                                          && \
        make CC="${LFS_TGT}-gcc" AR="${LFS_TGT}-ar" RANLIB="${LFS_TGT}-ranlib" \
                                                                  2>&1 | tee "$LOGS/blfs-bzip2-make.log"    && \
        make PREFIX="$LFS/usr" install                            2>&1 | tee "$LOGS/blfs-bzip2-install.log" \
            || { popd >/dev/null; echo "ERROR: bzip2 failed" >&2; return 1; }
        # Install the shared lib + the so-linked bzip2 binary (LFS recipe).
        cp -av libbz2.so.* "$LFS/usr/lib/" 2>/dev/null || true
        ln -sf "libbz2.so.${BZIP2_VERSION}" "$LFS/usr/lib/libbz2.so"
        cp -av bzip2-shared "$LFS/usr/bin/bzip2" 2>/dev/null || true
    popd >/dev/null
    touch "$sentinel"
    echo "<<< bzip2 done"
}

# ============================================================================
# Driver
# ============================================================================
STEPS=(kmod util_linux procps_ng shadow bzip2)

if [[ $# -eq 0 ]]; then
    for s in "${STEPS[@]}"; do
        "step_$s" || { echo "stopping at $s (see logs/blfs-*-*.log; try \`just audit-last\`)"; exit 1; }
    done
else
    for s in "$@"; do
        # accept both `util-linux` and `util_linux` spellings.
        s="${s//-/_}"
        if [[ ! " ${STEPS[*]} " == *" $s "* ]]; then
            echo "unknown step: $s"; echo "valid: ${STEPS[*]}"; exit 1
        fi
        "step_$s" || exit 1
    done
fi

echo
echo "Base userspace: $(count_done_packages) blfs packages built (cumulative)."
echo "Next: ./build/17-stage-sysroot.sh && ./build/check-staging.sh"
