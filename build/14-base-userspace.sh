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
    # xz (built in 03) for .ko.xz; openssl off (no module-sig verification);
    # --disable-manpages (scdoc isn't in the container). zstd left to
    # auto-detect (don't force --with-zstd if liblzma-only). build_pkg adds
    # --disable-static. `|| return $?` so a configure/make failure propagates
    # (don't let the symlink loop below mask it).
    build_pkg kmod "kmod-${KMOD_VERSION}.tar.xz" \
        --sysconfdir=/etc \
        --disable-manpages \
        --with-xz \
        --without-openssl \
        || return $?
    # kmod ships one binary + tool symlinks (modprobe/insmod/lsmod/depmod/
    # rmmod/modinfo). Mirror the conventional /usr/sbin names so modprobe
    # resolves on the sbin PATH too.
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
    # No --enable-watch8bit: it forces <ncursesw/ncurses.h>, but 03 installs
    # ncurses headers flat (/usr/include/ncurses.h), so watch builds against
    # plain <ncurses.h> instead.
    build_pkg procps-ng "procps-ng-${PROCPS_NG_VERSION}.tar.xz" \
        --disable-kill \
        --without-systemd
}

# ============================================================================
# shadow — login/su/passwd/useradd. The systemd branch needs /bin/login for
# the getty autologin + passwd to set the user password. PAM-aware (libpam
# from Phase 8a). Don't install groups(1) — coreutils owns it.
# ============================================================================
step_shadow() {
    build_pkg shadow "shadow-${SHADOW_VERSION}.tar.xz" \
        --without-libbsd \
        --without-selinux \
        --without-audit \
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
# tzdata (LFS Ch8) — timezone DB compiled with zic. Data only, no configure.
# The container's zic produces portable TZif files; target into $LFS.
# ============================================================================
step_tzdata() {
    local sentinel="$LOGS/.done-blfs-tzdata"
    [[ -f "$sentinel" ]] && { echo "skip tzdata (already built)"; return 0; }
    # zic is in the container but /usr/sbin may not be on the build PATH.
    local ZIC; ZIC="$(command -v zic || true)"; [[ -x "$ZIC" ]] || ZIC=/usr/sbin/zic
    [[ -x "$ZIC" ]] || { echo "ERROR: zic not found in container (tzdata)" >&2; return 1; }
    local work="$BUILD_ROOT/work/tzdata"
    rm -rf "$work"; mkdir -p "$work"
    # tzdata tarballs are flat (no top-level dir) → no --strip-components.
    tar -xf "$SOURCES/tzdata${TZDATA_VERSION}.tar.gz" -C "$work"
    pushd "$work" >/dev/null
        local ZI="$LFS/usr/share/zoneinfo"
        mkdir -pv "$ZI"/{posix,right}
        for tz in etcetera southamerica northamerica europe africa antarctica \
                  asia australasia backward; do
            "$ZIC" -L /dev/null   -d "$ZI"       "${tz}" && \
            "$ZIC" -L /dev/null   -d "$ZI/posix" "${tz}" && \
            "$ZIC" -L leapseconds -d "$ZI/right" "${tz}" \
                || { popd >/dev/null; echo "ERROR: tzdata zic ${tz} failed" >&2; return 1; }
        done
        cp -v zone.tab zone1970.tab iso3166.tab "$ZI"
        "$ZIC" -d "$ZI" -p America/New_York
    popd >/dev/null
    touch "$sentinel"
    echo "<<< tzdata done"
}

# ============================================================================
# kbd (LFS Ch8) — setfont/loadkeys/kbd_mode. Needed so systemd-vconsole-setup
# runs (un-mask it). Manual build: LFS drops the deprecated resizecons first.
# ============================================================================
step_kbd() {
    local sentinel="$LOGS/.done-blfs-kbd"
    [[ -f "$sentinel" ]] && { echo "skip kbd (already built)"; return 0; }
    local work="$BUILD_ROOT/work/kbd"
    rm -rf "$work"; mkdir -p "$work"
    tar -xf "$SOURCES/kbd-${KBD_VERSION}.tar.xz" -C "$work" --strip-components=1
    pushd "$work" >/dev/null
        # LFS: remove the deprecated resizecons program + its manpage.
        sed -i '/RESIZECONS_PROGS=/s/yes/no/' configure
        sed -i 's/resizecons.8 //' docs/man/man8/Makefile.in
        local cfg_guess; cfg_guess="$(_find_config_guess)"
        ./configure --prefix=/usr --sysconfdir=/etc --localstatedir=/var \
            --host="$LFS_TGT" --build="$("$cfg_guess")" \
            --disable-static --disable-vlock \
            2>&1 | tee "$LOGS/blfs-kbd-configure.log" && \
        make -j"$(nproc)"          2>&1 | tee "$LOGS/blfs-kbd-make.log" && \
        make DESTDIR="$LFS" install 2>&1 | tee "$LOGS/blfs-kbd-install.log" \
            || { popd >/dev/null; echo "ERROR: kbd failed" >&2; return 1; }
    popd >/dev/null
    touch "$sentinel"
    echo "<<< kbd done"
}

# ============================================================================
# e2fsprogs (LFS Ch8) — mkfs.ext4 / fsck.ext4. LFS disables libblkid/libuuid/
# uuidd/fsck (util-linux provides those). build_pkg configures in-tree.
# ============================================================================
step_e2fsprogs() {
    build_pkg e2fsprogs "e2fsprogs-${E2FSPROGS_VERSION}.tar.gz" \
        --enable-elf-shlibs \
        --disable-libblkid \
        --disable-libuuid \
        --disable-uuidd \
        --disable-fsck
}

# ============================================================================
# DejaVu fonts (BLFS x/installing/TTF-and-OTF-fonts.xml) — data only, no build.
# Installs the TTF files so fontconfig resolves monospace/sans/serif; without
# ANY font i3 (font pango:monospace) exits at startup. fontconfig builds its
# cache on first boot (/var/cache/fontconfig is writable) — no build-time fc-cache.
# ============================================================================
step_dejavu() {
    local sentinel="$LOGS/.done-blfs-dejavu"
    [[ -f "$sentinel" ]] && { echo "skip dejavu (already built)"; return 0; }
    local work="$BUILD_ROOT/work/dejavu"
    rm -rf "$work"; mkdir -p "$work"
    tar -xf "$SOURCES/dejavu-fonts-ttf-${DEJAVU_VERSION}.tar.bz2" -C "$work" --strip-components=1
    pushd "$work" >/dev/null
        install -v -d -m755 "$LFS/usr/share/fonts/dejavu"
        install -v -m644 ttf/*.ttf "$LFS/usr/share/fonts/dejavu/" \
            || { popd >/dev/null; echo "ERROR: dejavu install failed" >&2; return 1; }
    popd >/dev/null
    touch "$sentinel"
    echo "<<< dejavu done ($(ls "$LFS"/usr/share/fonts/dejavu/*.ttf 2>/dev/null | wc -l) ttf files)"
}

# ============================================================================
# vim (LFS Ch8) — the tier-1 base editor. Console build (no GUI / no X);
# runs inside xterm. Needed so a fresh system can edit configs before Nix
# exists (e.g. /etc/nix/nix.conf). Heavier editors (neovim/emacs/VS Code)
# stay tier-2 via Nix.
#
# Custom step (not build_pkg) for three vim-specific reasons:
#   1. Pre-configure edit of src/feature.h so the system vimrc is /etc/vimrc
#      (shipped via build/skeleton/etc/vimrc, NOT written here — $LFS/etc is
#      not copied into staging; only $LFS/usr is).
#   2. vim's configure runs target binaries for feature detection, which
#      can't execute under cross-compile. Pre-seed the vim_cv_* + ac_cv_*
#      cache vars (the standard cross-vim set) so configure trusts them.
#   3. Post-install `vi` symlink.
# ============================================================================
step_vim() {
    local sentinel="$LOGS/.done-blfs-vim"
    [[ -f "$sentinel" ]] && { echo "skip vim (already built)"; return 0; }
    local work="$BUILD_ROOT/work/vim"
    rm -rf "$work"; mkdir -p "$work"
    tar -xf "$SOURCES/vim-${VIM_VERSION}.tar.gz" -C "$work" --strip-components=1
    pushd "$work" >/dev/null
        # 1. System vimrc lives in /etc (LFS recipe).
        echo '#define SYS_VIMRC_FILE "/etc/vimrc"' >> src/feature.h
        local cfg_guess; cfg_guess="$(_find_config_guess)"
        # 2. Cross-compile feature-detection cache (vim can't run target
        #    test binaries). These are the canonical cross-vim answers for
        #    a glibc/ncurses Linux target.
        env \
            vim_cv_toupper_broken=no \
            vim_cv_terminfo=yes \
            vim_cv_tgetent=zero \
            vim_cv_getcwd_broken=no \
            vim_cv_stat_ignores_slash=no \
            vim_cv_memmove_handles_overlap=yes \
            vim_cv_bcopy_handles_overlap=yes \
            vim_cv_memcpy_handles_overlap=yes \
            ac_cv_sizeof_int=4 \
        ./configure --prefix=/usr --sysconfdir=/etc --localstatedir=/var \
            --host="$LFS_TGT" --build="$("$cfg_guess")" \
            --with-tlib=ncurses \
            --enable-multibyte \
            --enable-gui=no \
            --without-x \
            --disable-gpm \
            --disable-gtktest \
            2>&1 | tee "$LOGS/blfs-vim-configure.log" && \
        make -j"$(nproc)"           2>&1 | tee "$LOGS/blfs-vim-make.log" && \
        make DESTDIR="$LFS" install 2>&1 | tee "$LOGS/blfs-vim-install.log" \
            || { popd >/dev/null; echo "ERROR: vim failed" >&2; return 1; }
        # 3. vi → vim convenience symlink (LFS recipe).
        ln -sfv vim "$LFS/usr/bin/vi"
    popd >/dev/null
    touch "$sentinel"
    echo "<<< vim done"
}

# ============================================================================
# Driver
# ============================================================================
STEPS=(kmod util_linux procps_ng shadow bzip2 tzdata kbd e2fsprogs dejavu vim)

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
