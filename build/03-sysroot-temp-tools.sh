#!/usr/bin/env bash
# build/03-sysroot-temp-tools.sh — LFS chapter 6 temporary tools.
#
# Builds the 17 packages that turn $LFS from "just the cross-toolchain + libc"
# into "a chroot-capable userspace". Every package is cross-built using the
# pass-1 toolchain from $LFS/tools and installed into $LFS/usr via DESTDIR.
#
# Same operating model as 02-cross-toolchain.sh:
#   ./03-sysroot-temp-tools.sh                 # run every step in order
#   ./03-sysroot-temp-tools.sh ncurses bash    # run just these
#   delete logs/.done-temp-<step> to force a redo
#
# References (sections in .agents/reference/lfs-rendered.txt):
#   M4 §6.3, Ncurses §6.4, Bash §6.5, Coreutils §6.6, Diffutils §6.7,
#   File §6.8, Findutils §6.9, Gawk §6.10, Grep §6.11, Gzip §6.12,
#   Make §6.13, Patch §6.14, Sed §6.15, Tar §6.16, Xz §6.17,
#   Binutils-Pass2 §6.18, GCC-Pass2 §6.19.

# NOTE: -e omitted deliberately, same reason as import-keys.sh — explicit
# error accounting per step is clearer than relying on early exit.
set -uo pipefail

cd "$( dirname "${BASH_SOURCE[0]}" )"
# shellcheck disable=SC1091
source ./setup-env.sh

# The cross-toolchain must be done.
[[ -x "$LFS/tools/bin/$LFS_TGT-gcc" ]] || {
    echo "Cross-toolchain not found. Run ./02-cross-toolchain.sh first." >&2
    exit 1
}

# Add the cross-toolchain to PATH so $LFS_TGT-gcc and friends resolve.
export PATH="$LFS/tools/bin:$PATH"

# Steps in order. Most are simple; ncurses, file, binutils-2, gcc-2 are custom.
STEPS=(
    m4
    ncurses
    bash
    coreutils
    diffutils
    file
    findutils
    gawk
    grep
    gzip
    make
    patch
    sed
    tar
    xz
    binutils-2
    gcc-2
)

# ---- driver framework -------------------------------------------------------

do_step() {
    local step="$1"
    local sentinel="$LOGS/.done-temp-$step"
    if [[ -f "$sentinel" ]]; then
        echo ">>> $step already complete (delete $sentinel to redo)"
        return 0
    fi
    echo
    echo "============================================================"
    echo " Step: $step"
    echo "============================================================"
    if "step_$step"; then
        touch "$sentinel"
        echo "<<< $step done"
    else
        echo "!!! $step FAILED"
        return 1
    fi
}

# Generic cross-build helper for the boring packages.
# Usage:
#   simple_build <pkg> <archive-basename> [<extra-configure-flags...>]
simple_build() {
    local pkg="$1" archive="$2"; shift 2
    local work="$BUILD_ROOT/work/$pkg"
    rm -rf "$work"; mkdir -p "$work"
    tar -xf "$SOURCES/$archive" -C "$work" --strip-components=1
    pushd "$work" >/dev/null
        local cfg_guess="./config.guess"
        [[ -x build-aux/config.guess ]] && cfg_guess="build-aux/config.guess"
        ./configure --prefix=/usr \
                    --host="$LFS_TGT" \
                    --build="$($cfg_guess)" \
                    "$@" \
            2>&1 | tee "$LOGS/temp-$pkg-configure.log"
        make -j"$(nproc)"              2>&1 | tee "$LOGS/temp-$pkg-make.log"
        make DESTDIR="$LFS" install    2>&1 | tee "$LOGS/temp-$pkg-install.log"
    popd >/dev/null
}

# ---- step functions ---------------------------------------------------------

step_m4()        { simple_build m4        "m4-${M4_VERSION}.tar.xz"; }

# Ncurses (§6.4) — needs `tic` built on the host first so the cross-build
# can use it for terminfo compilation.
step_ncurses() {
    local src="ncurses-${NCURSES_VERSION}"
    local work="$BUILD_ROOT/work/ncurses"
    rm -rf "$work"; mkdir -p "$work"
    tar -xf "$SOURCES/${src}.tar.gz" -C "$work" --strip-components=1
    pushd "$work" >/dev/null
        sed -i s/mawk// configure

        mkdir build
        pushd build >/dev/null
            ../configure 2>&1 | tee "$LOGS/temp-ncurses-host-configure.log"
            make -C include 2>&1 | tee "$LOGS/temp-ncurses-host-include.log"
            make -C progs tic 2>&1 | tee "$LOGS/temp-ncurses-host-tic.log"
        popd >/dev/null

        ./configure --prefix=/usr \
                    --host="$LFS_TGT" \
                    --build="$(./config.guess)" \
                    --mandir=/usr/share/man \
                    --with-manpage-format=normal \
                    --with-shared \
                    --without-normal \
                    --with-cxx-shared \
                    --without-debug \
                    --without-ada \
                    --disable-stripping \
                    --enable-widec \
            2>&1 | tee "$LOGS/temp-ncurses-configure.log"
        make -j"$(nproc)" 2>&1 | tee "$LOGS/temp-ncurses-make.log"
        make DESTDIR="$LFS" TIC_PATH="$(pwd)/build/progs/tic" install \
            2>&1 | tee "$LOGS/temp-ncurses-install.log"

        ln -sv libncursesw.so "$LFS/usr/lib/libncurses.so"
        sed -e 's/^#if.*XOPEN.*$/#if 1/' -i "$LFS/usr/include/curses.h"
    popd >/dev/null
}

step_bash()      { simple_build bash      "bash-${BASH_VERSION}.tar.gz" \
                                          --without-bash-malloc \
                                          --bindir=/usr/bin; }

step_coreutils() { simple_build coreutils "coreutils-${COREUTILS_VERSION}.tar.xz" \
                                          --enable-install-program=hostname \
                                          --enable-no-install-program=kill,uptime; }

step_diffutils() { simple_build diffutils "diffutils-${DIFFUTILS_VERSION}.tar.xz"; }

# File (§6.8) — needs `file` itself built on the host so the cross-build can
# use it as a compiler for magic files.
step_file() {
    local src="file-${FILE_VERSION}"
    local work="$BUILD_ROOT/work/file"
    rm -rf "$work"; mkdir -p "$work"
    tar -xf "$SOURCES/${src}.tar.gz" -C "$work" --strip-components=1
    pushd "$work" >/dev/null
        mkdir build
        pushd build >/dev/null
            ../configure --disable-bzlib --disable-libseccomp \
                         --disable-xzlib --disable-zlib \
                2>&1 | tee "$LOGS/temp-file-host-configure.log"
            make -j"$(nproc)" 2>&1 | tee "$LOGS/temp-file-host-make.log"
        popd >/dev/null

        ./configure --prefix=/usr --host="$LFS_TGT" --build="$(./config.guess)" \
            2>&1 | tee "$LOGS/temp-file-configure.log"
        make FILE_COMPILE="$(pwd)/build/src/file" -j"$(nproc)" \
            2>&1 | tee "$LOGS/temp-file-make.log"
        make DESTDIR="$LFS" install 2>&1 | tee "$LOGS/temp-file-install.log"
        rm -fv "$LFS/usr/lib/libmagic.la"
    popd >/dev/null
}

step_findutils() { simple_build findutils "findutils-${FINDUTILS_VERSION}.tar.xz" \
                                          --localstatedir=/var/lib/locate; }

step_gawk() {
    # LFS removes the 'extras' directory before configure.
    local src="gawk-${GAWK_VERSION}"
    local work="$BUILD_ROOT/work/gawk"
    rm -rf "$work"; mkdir -p "$work"
    tar -xf "$SOURCES/${src}.tar.xz" -C "$work" --strip-components=1
    pushd "$work" >/dev/null
        sed -i 's/extras//' Makefile.in
        ./configure --prefix=/usr --host="$LFS_TGT" --build="$(./config.guess)" \
            2>&1 | tee "$LOGS/temp-gawk-configure.log"
        make -j"$(nproc)"              2>&1 | tee "$LOGS/temp-gawk-make.log"
        make DESTDIR="$LFS" install    2>&1 | tee "$LOGS/temp-gawk-install.log"
    popd >/dev/null
}

step_grep()      { simple_build grep      "grep-${GREP_VERSION}.tar.xz"; }
step_gzip()      { simple_build gzip      "gzip-${GZIP_VERSION}.tar.xz"; }

step_make()      { simple_build make      "make-${MAKE_VERSION}.tar.gz" \
                                          --without-guile; }

step_patch()     { simple_build patch     "patch-${PATCH_VERSION}.tar.xz"; }
step_sed()       { simple_build sed       "sed-${SED_VERSION}.tar.xz"; }
step_tar()       { simple_build tar       "tar-${TAR_VERSION}.tar.xz"; }

step_xz() {
    simple_build xz "xz-${XZ_VERSION}.tar.xz" --disable-static \
                                              "--docdir=/usr/share/doc/xz-${XZ_VERSION}"
    rm -fv "$LFS/usr/lib/liblzma.la"
}

# Binutils Pass 2 (§6.18) — rebuilt against the freshly-built target libc.
step_binutils-2() {
    local src="binutils-${BINUTILS_VERSION}"
    local work="$BUILD_ROOT/work/binutils-pass2"
    rm -rf "$work"; mkdir -p "$work"
    tar -xf "$SOURCES/${src}.tar.xz" -C "$work" --strip-components=1
    pushd "$work" >/dev/null
        # Strip rpath embedding (LFS workaround in ltmain.sh).
        sed '6009s/$add_dir//' -i ltmain.sh

        mkdir build && cd build
        ../configure --prefix=/usr \
                     --build="$(../config.guess)" \
                     --host="$LFS_TGT" \
                     --disable-nls \
                     --enable-shared \
                     --enable-gprofng=no \
                     --disable-werror \
                     --enable-64-bit-bfd \
                     --enable-new-dtags \
                     --enable-default-hash-style=gnu \
            2>&1 | tee "$LOGS/temp-binutils-2-configure.log"
        make -j"$(nproc)"           2>&1 | tee "$LOGS/temp-binutils-2-make.log"
        make DESTDIR="$LFS" install 2>&1 | tee "$LOGS/temp-binutils-2-install.log"
        rm -fv "$LFS/usr/lib/lib"{bfd,ctf,ctf-nobfd,opcodes,sframe}.{a,la}
    popd >/dev/null
}

# GCC Pass 2 (§6.19) — rebuilt against the freshly-built target libc.
# This becomes the canonical compiler inside the chroot from this point on.
step_gcc-2() {
    local src="gcc-${GCC_VERSION}"
    local work="$BUILD_ROOT/work/gcc-pass2"
    rm -rf "$work"; mkdir -p "$work"
    tar -xf "$SOURCES/${src}.tar.xz" -C "$work" --strip-components=1
    pushd "$work" >/dev/null
        tar -xf "$SOURCES/mpfr-${MPFR_VERSION}.tar.xz" && mv "mpfr-${MPFR_VERSION}" mpfr
        tar -xf "$SOURCES/gmp-${GMP_VERSION}.tar.xz"   && mv "gmp-${GMP_VERSION}"   gmp
        tar -xf "$SOURCES/mpc-${MPC_VERSION}.tar.gz"   && mv "mpc-${MPC_VERSION}"   mpc

        case "$(uname -m)" in
            x86_64) sed -e '/m64=/s/lib64/lib/' -i.orig gcc/config/i386/t-linux64 ;;
        esac

        mkdir build && cd build
        # libgcc prerequisite: gthr-default.h symlink so libgcc builds before
        # libstdc++ exists in the target sysroot.
        mkdir -pv "$LFS_TGT/libgcc"
        ln -sf ../../../libgcc/gthr-posix.h "$LFS_TGT/libgcc/gthr-default.h"

        ../configure \
            --build="$(../config.guess)"               \
            --host="$LFS_TGT"                          \
            --target="$LFS_TGT"                        \
            LDFLAGS_FOR_TARGET="-L$PWD/$LFS_TGT/libgcc" \
            --prefix=/usr                              \
            --with-build-sysroot="$LFS"                \
            --enable-default-pie                       \
            --enable-default-ssp                       \
            --disable-nls                              \
            --disable-multilib                         \
            --disable-libatomic                        \
            --disable-libgomp                          \
            --disable-libquadmath                      \
            --disable-libsanitizer                     \
            --disable-libssp                           \
            --disable-libvtv                           \
            --enable-languages=c,c++                   \
            2>&1 | tee "$LOGS/temp-gcc-2-configure.log"
        make -j"$(nproc)"           2>&1 | tee "$LOGS/temp-gcc-2-make.log"
        make DESTDIR="$LFS" install 2>&1 | tee "$LOGS/temp-gcc-2-install.log"

        # `cc` alias so packages that hard-code CC=cc work in the chroot.
        ln -sv gcc "$LFS/usr/bin/cc"
    popd >/dev/null
}

# ---- driver -----------------------------------------------------------------

if [[ $# -eq 0 ]]; then
    for s in "${STEPS[@]}"; do
        do_step "$s" || { echo "stopping at first failure"; exit 1; }
    done
else
    for s in "$@"; do
        if [[ ! " ${STEPS[*]} " == *" $s "* ]]; then
            echo "unknown step: $s"
            echo "valid: ${STEPS[*]}"
            exit 1
        fi
        do_step "$s" || exit 1
    done
fi

echo
done_count=$(ls "$LOGS"/.done-temp-* 2>/dev/null | wc -l)
echo "Chapter 6 temporary tools: $done_count / ${#STEPS[@]} step(s) complete."
echo
echo "Summary of installed binaries:"
ls "$LFS/usr/bin"  2>/dev/null | wc -l | xargs printf "  /usr/bin     %d entries\n"
ls "$LFS/usr/sbin" 2>/dev/null | wc -l | xargs printf "  /usr/sbin    %d entries\n"
echo
echo "If this was the full sequence, you can now chroot into \$LFS for Phase 2."
