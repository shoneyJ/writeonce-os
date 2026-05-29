#!/usr/bin/env bash
# build/08-base-substrate.sh — Phase 8 round 1: foundation libraries.
#
# Cross-builds the 12 packages that Xorg, GTK4, D-Bus, PipeWire, PAM
# transitively depend on. Each step is idempotent via
# logs/.done-blfs-<name> sentinels — re-run is cheap, full re-do is
# `rm logs/.done-blfs-*`.
#
# Build order matters: each entry depends only on earlier entries plus
# the Phase 0 sysroot (glibc, gcc, binutils).
#
#   zlib             ← decompression for libpng / freetype
#   brotli           ← woff2 fonts via freetype
#   expat            ← XML parsing for fontconfig / dbus
#   libffi           ← cffi for libxml2 / dbus / later glib
#   libxml2          ← XML parsing for dbus, GTK, etc.
#   util-macros      ← autoconf macros every X11 lib uses
#   libpng           ← needs zlib
#   libjpeg-turbo    ← image decoding (independent)
#   freetype         ← needs zlib, libpng, brotli; rebuilt later after harfbuzz
#   fontconfig       ← needs freetype + expat + libxml2
#   linux-pam        ← needs libxml2 (for docs)
#   dbus             ← needs expat + libxml2 (later: pam, libsystemd)
#
# Round 8b will follow with the X11 protocol layer (09-x11-stack.sh).

set -uo pipefail

cd "$( dirname "${BASH_SOURCE[0]}" )"
# shellcheck disable=SC1091
source ./setup-env.sh
# shellcheck disable=SC1091
source ./blfs-pkg.sh

# Sanity: cross-toolchain present?
[[ -x "$LFS/tools/bin/$LFS_TGT-gcc" ]] || {
    echo "Cross-toolchain not found. Run ./02-cross-toolchain.sh first." >&2
    exit 1
}

# ---- 1. zlib -----------------------------------------------------------------
# Note: zlib's configure is a shell script (not autoconf-generated), so it
# does not accept --host / --build. Cross-compile via CC env var instead.
step_zlib() {
    local name=zlib
    local sentinel="$LOGS/.done-blfs-$name"
    [[ -f "$sentinel" ]] && { echo "skip $name"; return 0; }
    echo; echo "==== blfs: $name ===="
    rm -rf "$BUILD_ROOT/work/$name"; mkdir -p "$BUILD_ROOT/work/$name"
    tar -xf "$SOURCES/zlib-${ZLIB_VERSION}.tar.xz" \
        -C "$BUILD_ROOT/work/$name" --strip-components=1
    pushd "$BUILD_ROOT/work/$name" >/dev/null
        CC="$LFS/tools/bin/${LFS_TGT}-gcc" \
            ./configure --prefix=/usr 2>&1 | tee "$LOGS/blfs-$name-configure.log"
        make -j"$(nproc)"             2>&1 | tee "$LOGS/blfs-$name-make.log"
        make DESTDIR="$LFS" install   2>&1 | tee "$LOGS/blfs-$name-install.log"
    popd >/dev/null
    touch "$sentinel"
}

# ---- 2. brotli ---------------------------------------------------------------
# brotli 1.1+ is CMake-only (no autotools). Custom cross-build step:
# toolchain file points cmake at $LFS/tools/bin/$LFS_TGT-gcc with sysroot,
# install lands at $LFS/usr via DESTDIR.
step_zstd() {
    # Facebook zstd — shader cache compression for Mesa, also used by
    # tar/many compressors. Uses cmake.
    local name=zstd
    local sentinel="$LOGS/.done-blfs-$name"
    [[ -f "$sentinel" ]] && { echo "skip $name"; return 0; }
    echo; echo "==== blfs (cmake): $name ===="
    rm -rf "$BUILD_ROOT/work/$name"; mkdir -p "$BUILD_ROOT/work/$name"
    tar -xf "$SOURCES/zstd-${ZSTD_VERSION}.tar.gz" \
        -C "$BUILD_ROOT/work/$name" --strip-components=1
    pushd "$BUILD_ROOT/work/$name/build/cmake" >/dev/null
        mkdir -p _build && cd _build
        cat > toolchain.cmake <<EOF
set(CMAKE_SYSTEM_NAME Linux)
set(CMAKE_SYSTEM_PROCESSOR x86_64)
set(CMAKE_C_COMPILER $LFS/tools/bin/${LFS_TGT}-gcc)
set(CMAKE_AR $LFS/tools/bin/${LFS_TGT}-ar)
set(CMAKE_RANLIB $LFS/tools/bin/${LFS_TGT}-ranlib)
set(CMAKE_C_FLAGS "--sysroot=$LFS")
set(CMAKE_FIND_ROOT_PATH $LFS)
set(CMAKE_FIND_ROOT_PATH_MODE_PROGRAM NEVER)
set(CMAKE_FIND_ROOT_PATH_MODE_LIBRARY ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_INCLUDE ONLY)
EOF
        cmake .. \
            -DCMAKE_TOOLCHAIN_FILE=toolchain.cmake \
            -DCMAKE_INSTALL_PREFIX=/usr \
            -DCMAKE_INSTALL_LIBDIR=lib \
            -DZSTD_BUILD_STATIC=OFF \
            -DZSTD_BUILD_PROGRAMS=OFF \
            -DZSTD_BUILD_TESTS=OFF \
            -DCMAKE_BUILD_TYPE=Release \
            2>&1 | tee "$LOGS/blfs-$name-cmake.log"  && \
        make -j"$(nproc)"             2>&1 | tee "$LOGS/blfs-$name-make.log" && \
        make DESTDIR="$LFS" install   2>&1 | tee "$LOGS/blfs-$name-install.log" \
            || { popd >/dev/null; echo "ERROR: $name failed" >&2; return 1; }
    popd >/dev/null
    find "$LFS/usr/lib" -name '*.la' -delete 2>/dev/null
    touch "$sentinel"
    echo "<<< $name done"
}

step_brotli() {
    local name=brotli
    local sentinel="$LOGS/.done-blfs-$name"
    [[ -f "$sentinel" ]] && { echo "skip $name"; return 0; }
    echo; echo "==== blfs (cmake): $name ===="
    rm -rf "$BUILD_ROOT/work/$name"; mkdir -p "$BUILD_ROOT/work/$name"
    tar -xf "$SOURCES/brotli-${BROTLI_VERSION}.tar.gz" \
        -C "$BUILD_ROOT/work/$name" --strip-components=1
    pushd "$BUILD_ROOT/work/$name" >/dev/null
        mkdir -p build && cd build
        cat > toolchain.cmake <<EOF
set(CMAKE_SYSTEM_NAME Linux)
set(CMAKE_SYSTEM_PROCESSOR x86_64)
set(CMAKE_C_COMPILER $LFS/tools/bin/${LFS_TGT}-gcc)
set(CMAKE_AR $LFS/tools/bin/${LFS_TGT}-ar)
set(CMAKE_RANLIB $LFS/tools/bin/${LFS_TGT}-ranlib)
set(CMAKE_C_FLAGS "--sysroot=$LFS")
set(CMAKE_EXE_LINKER_FLAGS "--sysroot=$LFS")
set(CMAKE_SHARED_LINKER_FLAGS "--sysroot=$LFS")
set(CMAKE_FIND_ROOT_PATH $LFS)
set(CMAKE_FIND_ROOT_PATH_MODE_PROGRAM NEVER)
set(CMAKE_FIND_ROOT_PATH_MODE_LIBRARY ONLY)
set(CMAKE_FIND_ROOT_PATH_MODE_INCLUDE ONLY)
EOF
        cmake .. \
            -DCMAKE_TOOLCHAIN_FILE=toolchain.cmake \
            -DCMAKE_INSTALL_PREFIX=/usr \
            -DCMAKE_BUILD_TYPE=Release \
            -DBUILD_SHARED_LIBS=ON \
            -DBROTLI_DISABLE_TESTS=ON \
            2>&1 | tee "$LOGS/blfs-$name-cmake.log"  && \
        make -j"$(nproc)"             2>&1 | tee "$LOGS/blfs-$name-make.log" && \
        make DESTDIR="$LFS" install   2>&1 | tee "$LOGS/blfs-$name-install.log" \
            || { popd >/dev/null; echo "ERROR: $name failed" >&2; return 1; }
    popd >/dev/null
    touch "$sentinel"
    echo "<<< $name done"
}

# ---- 3. expat ---------------------------------------------------------------
step_expat() {
    build_pkg expat "expat-${EXPAT_VERSION}.tar.xz" \
        --without-docbook
}

# ---- 4. libffi ---------------------------------------------------------------
step_libffi() {
    build_pkg libffi "libffi-${LIBFFI_VERSION}.tar.gz" \
        --with-gcc-arch=x86-64 \
        --disable-multi-os-directory
}

# ---- 5. libxml2 --------------------------------------------------------------
step_libxml2() {
    build_pkg libxml2 "libxml2-${LIBXML2_VERSION}.tar.xz" \
        --without-python \
        --without-icu
}

# ---- 6. util-macros ----------------------------------------------------------
step_util-macros() {
    build_pkg util-macros "util-macros-${UTIL_MACROS_VERSION}.tar.xz"
}

# ---- 7. libpng ---------------------------------------------------------------
step_libpng() {
    build_pkg libpng "libpng-${LIBPNG_VERSION}.tar.xz" \
        --enable-intel-sse=yes
}

# ---- 8. libjpeg-turbo --------------------------------------------------------
# Uses CMake exclusively. Add a thin wrapper when we get there.
step_libjpeg-turbo() {
    local name=libjpeg-turbo
    local sentinel="$LOGS/.done-blfs-$name"
    [[ -f "$sentinel" ]] && { echo "skip $name"; return 0; }
    command -v cmake >/dev/null || { echo "blfs: cmake not found (add to wo-builder Containerfile)"; return 1; }
    echo; echo "==== blfs (cmake): $name ===="
    rm -rf "$BUILD_ROOT/work/$name"; mkdir -p "$BUILD_ROOT/work/$name"
    tar -xf "$SOURCES/libjpeg-turbo-${LIBJPEG_TURBO_VERSION}.tar.gz" \
        -C "$BUILD_ROOT/work/$name" --strip-components=1
    pushd "$BUILD_ROOT/work/$name" >/dev/null
        # Chain with && so a configure / build / install failure aborts
        # before the sentinel is touched. Earlier this step produced a
        # phantom .done-blfs-libjpeg-turbo without ever installing
        # anything because the three cmake calls weren't chained and
        # the configure step silently failed (missing CMAKE_SYSTEM_PROCESSOR
        # → SIMD detection blew up → Makefile never generated).
        cmake -S . -B build \
            -DCMAKE_INSTALL_PREFIX=/usr \
            -DCMAKE_C_COMPILER="$LFS/tools/bin/${LFS_TGT}-gcc" \
            -DCMAKE_AR="$LFS/tools/bin/${LFS_TGT}-ar" \
            -DCMAKE_RANLIB="$LFS/tools/bin/${LFS_TGT}-ranlib" \
            -DENABLE_STATIC=OFF \
            -DCMAKE_SYSTEM_NAME=Linux \
            -DCMAKE_SYSTEM_PROCESSOR=x86_64 \
            -DCMAKE_FIND_ROOT_PATH="$LFS" \
            2>&1 | tee "$LOGS/blfs-$name-configure.log" && \
        cmake --build build -j"$(nproc)"           2>&1 | tee "$LOGS/blfs-$name-make.log" && \
        DESTDIR="$LFS" cmake --install build       2>&1 | tee "$LOGS/blfs-$name-install.log" \
            || { popd >/dev/null; echo "ERROR: $name failed" >&2; return 1; }
    popd >/dev/null
    touch "$sentinel"
}

# ---- 9. freetype (round 1 — no harfbuzz yet) --------------------------------
step_freetype() {
    build_pkg freetype "freetype-${FREETYPE_VERSION}.tar.xz" \
        --with-harfbuzz=no \
        --enable-freetype-config
}

# ---- 10. fontconfig ----------------------------------------------------------
step_fontconfig() {
    build_pkg fontconfig "fontconfig-${FONTCONFIG_VERSION}.tar.xz" \
        --sysconfdir=/etc \
        --localstatedir=/var \
        --disable-docs
}

# ---- 11. linux-pam -----------------------------------------------------------
# ---- 11a. libxcrypt ----------------------------------------------------------
# Provides libcrypt.so (DES/MD5/SHA-256/SHA-512 password hashing). glibc
# 2.38+ split crypt(3) out into this separate library; linux-pam's
# pam_unix + pam_pwhistory modules need it.
step_libxcrypt() {
    build_pkg libxcrypt "libxcrypt-${LIBXCRYPT_VERSION}.tar.xz" \
        --disable-static \
        --disable-obsolete-api
}

step_linux-pam() {
    build_pkg linux-pam "Linux-PAM-${LINUX_PAM_VERSION}.tar.xz" \
        --sysconfdir=/etc \
        --disable-doc \
        --disable-prelude \
        --disable-audit \
        --disable-selinux \
        --enable-securedir=/usr/lib/security
}

# ---- 12. sudo ----------------------------------------------------------------
step_sudo() {
    # Privilege elevation for the wheel group. PAM-aware (links the same
    # libpam.so we built in step_linux-pam). Build-time options:
    #   --without-{lecture,sendmail,interfaces,passwd}: drop optional
    #     subsystems we don't ship. Keeps the binary small.
    #   --with-secure-path: hardcoded PATH used when sudo is invoked,
    #     so `sudo somecmd` always resolves against a known set.
    #   --enable-shell-sets-home: `sudo -i` sets HOME to root's home.
    #
    # /etc/sudoers ships from build/skeleton/etc/sudoers — the
    # %wheel ALL=(ALL) ALL line is what grants the install-time
    # user privilege escalation via their own password.
    # sudo's make install does `install -o root -g root -m 4755` to set
    # setuid root on the binary and root:root on /etc/sudoers. Both
    # chown operations fail with EPERM when the build runs as uid 1000.
    # The fakeroot wrapper intercepts chown/lchown and records the
    # intended uid/gid in its in-memory database; the file on disk
    # stays uid 1000 BUT subsequent fakeroot operations (tar, ls, stat)
    # report uid 0. step_make_artifacts pipes through `fakeroot tar`
    # so the tarball records the simulated root ownership, which the
    # installer (running as real root on the target) materialises.
    local name=sudo
    local sentinel="$LOGS/.done-blfs-$name"
    if [[ -f "$sentinel" ]]; then echo "skip $name"; return 0; fi
    echo; echo "==== blfs: $name (fakeroot) ===="
    rm -rf "$BUILD_ROOT/work/$name"; mkdir -p "$BUILD_ROOT/work/$name"
    tar -xf "$SOURCES/sudo-${SUDO_VERSION}.tar.gz" \
        -C "$BUILD_ROOT/work/$name" --strip-components=1
    pushd "$BUILD_ROOT/work/$name" >/dev/null
        ./configure \
            --prefix=/usr \
            --host="$LFS_TGT" \
            --build="$(./scripts/mkpkg --getbuild 2>/dev/null || ./config.guess)" \
            --sysconfdir=/etc \
            --with-secure-path="/usr/sbin:/usr/bin:/sbin:/bin" \
            --enable-shell-sets-home \
            --with-rundir=/run/sudo \
            --with-vardir=/var/db/sudo \
            --disable-static \
            --disable-zlib \
            --without-sendmail \
            --without-interfaces \
            2>&1 | tee "$LOGS/blfs-$name-configure.log" && \
        make -j"$(nproc)" 2>&1 | tee "$LOGS/blfs-$name-make.log" && \
        fakeroot -- make DESTDIR="$LFS" install \
                                      2>&1 | tee "$LOGS/blfs-$name-install.log" \
            || { popd >/dev/null; echo "ERROR: $name failed" >&2; return 1; }
    popd >/dev/null
    find "$LFS/usr/lib" -name '*.la' -delete 2>/dev/null
    touch "$sentinel"
}

# ---- 13. dbus ----------------------------------------------------------------
step_dbus() {
    # dbus 1.16+ uses meson, not autotools.
    build_meson dbus "dbus-${DBUS_VERSION}.tar.xz" \
        -Dsystem_pid_file=/run/dbus/pid \
        -Dsystem_socket=/run/dbus/system_bus_socket \
        -Dmodular_tests=disabled \
        -Dxml_docs=disabled \
        -Dx11_autolaunch=disabled \
        -Dsystemd=disabled \
        -Dapparmor=disabled \
        -Dselinux=disabled \
        -Dlibaudit=disabled \
        -Drelocation=disabled
}

# ---- driver -----------------------------------------------------------------

STEPS=( zlib zstd brotli expat libffi libxml2 util-macros libpng libjpeg-turbo libxcrypt
        freetype fontconfig linux-pam sudo dbus )

if [[ $# -eq 0 ]]; then
    for s in "${STEPS[@]}"; do
        "step_$s" || { echo "stopping at $s"; exit 1; }
    done
else
    for s in "$@"; do
        if [[ ! " ${STEPS[*]} " == *" $s "* ]]; then
            echo "unknown step: $s"; echo "valid: ${STEPS[*]}"; exit 1
        fi
        "step_$s" || exit 1
    done
fi

echo
echo "Phase 8 base substrate: $(count_done_packages) / ${#STEPS[@]} packages done."
echo "Next: ./09-x11-stack.sh (when populated)."
