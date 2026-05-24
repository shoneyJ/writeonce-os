#!/usr/bin/env bash
# build/blfs-pkg.sh — sourceable helper for BLFS-style package builds.
#
# Provides two main entry points that the per-stack scripts
# (08-base-substrate.sh, 09-x11-stack.sh, …) call to compile an upstream
# tarball into the WriteOnce sysroot:
#
#   build_pkg <name> <archive> [extra configure flags…]
#       Autoconf-style:  ./configure → make → make DESTDIR=$LFS install
#
#   build_meson <name> <archive> [extra meson options…]
#       Meson-style:     meson setup build → meson compile → meson install
#
# Both:
#   - Skip if logs/.done-blfs-<name> exists (delete sentinel to redo).
#   - Extract into work/<name>/ (wiped first).
#   - Stream stdout+stderr to logs/blfs-<name>-{configure,make,install}.log.
#   - Cross-compile against the Phase 0 toolchain ($LFS_TGT prefix).
#
# Do NOT execute this file directly. Source it:
#     source "$(dirname "${BASH_SOURCE[0]}")/blfs-pkg.sh"

# ---- preconditions ----------------------------------------------------------

[[ -n "${LFS:-}"        ]] || { echo "blfs-pkg.sh: \$LFS not set (source setup-env.sh first)" >&2; exit 1; }
[[ -n "${LFS_TGT:-}"    ]] || { echo "blfs-pkg.sh: \$LFS_TGT not set"                          >&2; exit 1; }
[[ -n "${SOURCES:-}"    ]] || { echo "blfs-pkg.sh: \$SOURCES not set"                          >&2; exit 1; }
[[ -n "${LOGS:-}"       ]] || { echo "blfs-pkg.sh: \$LOGS not set"                             >&2; exit 1; }
[[ -d "$LFS/tools/bin" ]] || { echo "blfs-pkg.sh: cross-toolchain not at \$LFS/tools/bin"     >&2; exit 1; }

# Make the cross-toolchain available to configure scripts.
export PATH="$LFS/tools/bin:$PATH"

# Common pkg-config / library search flags for cross-build into $LFS.
# Subpackages may override.
export PKG_CONFIG_PATH="$LFS/usr/lib/pkgconfig:$LFS/usr/share/pkgconfig"
export PKG_CONFIG_LIBDIR="$LFS/usr/lib/pkgconfig:$LFS/usr/share/pkgconfig"
export PKG_CONFIG_SYSROOT_DIR="$LFS"

# ---- helpers ---------------------------------------------------------------

# Locate config.guess regardless of where the package puts it.
_find_config_guess() {
    if   [[ -x ./config.guess           ]]; then echo "./config.guess"
    elif [[ -x build-aux/config.guess   ]]; then echo "build-aux/config.guess"
    elif [[ -x config/config.guess      ]]; then echo "config/config.guess"
    else echo "config.guess"   # last-ditch; configure will error
    fi
}

# Extract <archive> into work/<name>/, replacing any existing extract.
_extract() {
    local name="$1" archive="$2"
    local work="$BUILD_ROOT/work/$name"
    rm -rf "$work"; mkdir -p "$work"
    tar -xf "$SOURCES/$archive" -C "$work" --strip-components=1
}

# ---- build_pkg: autoconf-style --------------------------------------------

build_pkg() {
    local name="$1" archive="$2"; shift 2
    local sentinel="$LOGS/.done-blfs-$name"
    if [[ -f "$sentinel" ]]; then
        echo "skip $name (already built)"
        return 0
    fi
    echo
    echo "============================================================"
    echo " blfs: $name"
    echo "============================================================"
    _extract "$name" "$archive"
    # Use `set -o pipefail` (already on from setup-env.sh) so a failing
    # tool piped through `tee` still produces a non-zero exit. Also wrap
    # the three steps in an explicit chain — without &&, a make failure
    # was followed by `touch $sentinel` which faked completion.
    pushd "$BUILD_ROOT/work/$name" >/dev/null
        local cfg_guess
        cfg_guess="$(_find_config_guess)"
        ./configure                                            \
            --prefix=/usr                                      \
            --host="$LFS_TGT"                                  \
            --build="$("$cfg_guess")"                          \
            --disable-static                                   \
            "$@"                                               \
            2>&1 | tee "$LOGS/blfs-$name-configure.log"     && \
        make -j"$(nproc)"             2>&1 | tee "$LOGS/blfs-$name-make.log" && \
        make DESTDIR="$LFS" install   2>&1 | tee "$LOGS/blfs-$name-install.log" \
            || { popd >/dev/null; echo "ERROR: $name failed" >&2; return 1; }
    popd >/dev/null
    # Purge libtool .la archives: they embed absolute /usr/lib/foo.la
    # paths that confuse libtool when downstream packages relink during
    # cross-compile. Modern distros (Arch, Alpine, NixOS) ship without
    # .la files; .pc pkg-config files carry the same info correctly.
    find "$LFS/usr/lib" -name '*.la' -delete 2>/dev/null
    touch "$sentinel"
    echo "<<< $name done"
}

# ---- build_meson: meson + ninja --------------------------------------------

build_meson() {
    local name="$1" archive="$2"; shift 2
    local sentinel="$LOGS/.done-blfs-$name"
    if [[ -f "$sentinel" ]]; then
        echo "skip $name (already built)"
        return 0
    fi
    command -v meson >/dev/null || { echo "blfs: meson not in PATH (apt install meson via container, or add to wo-builder)"; return 1; }
    command -v ninja >/dev/null || { echo "blfs: ninja not in PATH";                                                          return 1; }
    echo
    echo "============================================================"
    echo " blfs (meson): $name"
    echo "============================================================"
    _extract "$name" "$archive"

    # Write a meson cross-file describing the LFS target.
    local cross_file="$BUILD_ROOT/work/$name/cross-lfs.ini"
    cat > "$cross_file" <<EOF
[binaries]
c          = '$LFS/tools/bin/${LFS_TGT}-gcc'
cpp        = '$LFS/tools/bin/${LFS_TGT}-g++'
ar         = '$LFS/tools/bin/${LFS_TGT}-ar'
strip      = '$LFS/tools/bin/${LFS_TGT}-strip'
pkgconfig  = 'pkg-config'

[properties]
sys_root        = '$LFS'
pkg_config_libdir = '$LFS/usr/lib/pkgconfig:$LFS/usr/share/pkgconfig'
# Even though build_machine and host_machine are both x86_64-linux, the cross
# binaries are linked against \$LFS libraries (different libc/libstdc++/glibc
# soname-paths) and would fail to find their host shared deps at run time.
# Treat them as un-runnable so meson's can_run_host_binaries() returns false;
# this is the signal Mesa et al. use to build their build-time helpers
# (mesa_clc, intel_clc, glsl_compiler) as native binaries against the host
# toolchain instead of incorrectly linking cross binaries to host libLLVM.
needs_exe_wrapper = true

[host_machine]
system     = 'linux'
cpu_family = 'x86_64'
cpu        = 'x86_64'
endian     = 'little'
EOF

    pushd "$BUILD_ROOT/work/$name" >/dev/null
        meson setup build                                      \
            --cross-file=cross-lfs.ini                         \
            --prefix=/usr                                      \
            --buildtype=release                                \
            --default-library=shared                           \
            "$@"                                               \
            2>&1 | tee "$LOGS/blfs-$name-setup.log"          && \
        meson compile -C build         2>&1 | tee "$LOGS/blfs-$name-compile.log" && \
        DESTDIR="$LFS" meson install -C build \
                                         2>&1 | tee "$LOGS/blfs-$name-install.log" \
            || { popd >/dev/null; echo "ERROR: $name failed" >&2; return 1; }
    popd >/dev/null
    find "$LFS/usr/lib" -name '*.la' -delete 2>/dev/null
    touch "$sentinel"
    echo "<<< $name done"
}

# ---- introspection ---------------------------------------------------------

list_done_packages() {
    ls "$LOGS"/.done-blfs-* 2>/dev/null | sed 's|.*/.done-blfs-||' | sort
}

count_done_packages() {
    ls "$LOGS"/.done-blfs-* 2>/dev/null | wc -l
}
