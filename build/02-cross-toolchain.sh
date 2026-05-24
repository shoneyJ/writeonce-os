#!/usr/bin/env bash
# build/cross-toolchain.sh — build the LFS-style cross-toolchain.
#
# Implements LFS chapter 5: binutils pass 1 → GCC pass 1 → Linux API headers →
# glibc → libstdc++ (from GCC). All output lands in $LFS_TOOLS (exposed at
# $LFS/tools), except glibc which installs into $LFS/usr because glibc is part
# of the target system, not the toolchain.
#
# Idempotent at the step level: each step writes a sentinel file on success
# and is skipped on subsequent runs.
#
# Run via:
#     ./cross-toolchain.sh                  # all steps
#     ./cross-toolchain.sh binutils-1       # one step
#     STEPS_REMAINING=1 ./cross-toolchain.sh ...  # for partial reruns

set -euo pipefail

cd "$( dirname "${BASH_SOURCE[0]}" )"
# shellcheck disable=SC1091
source ./setup-env.sh

# All steps in order.
STEPS=( binutils-1 gcc-1 linux-headers glibc gcc-2 libstdcxx )

# Run a single step.
do_step() {
    local step="$1"
    local sentinel="$LOGS/.done-$step"
    if [[ -f "$sentinel" ]]; then
        echo ">>> $step already complete (delete $sentinel to redo)"
        return 0
    fi
    echo
    echo "============================================================"
    echo " Step: $step"
    echo "============================================================"
    "step_$step"
    touch "$sentinel"
    echo "<<< $step done"
}

# ---- step 1: binutils pass 1 ------------------------------------------------
step_binutils-1() {
    local src="binutils-${BINUTILS_VERSION}"
    local work="$BUILD_ROOT/work/$src"
    rm -rf "$work"; mkdir -p "$work"
    tar -xf "$SOURCES/${src}.tar.xz" -C "$BUILD_ROOT/work/"
    pushd "$work" >/dev/null
        mkdir -p build && cd build
        ../configure --prefix="$LFS/tools"   \
                     --with-sysroot="$LFS"   \
                     --target="$LFS_TGT"     \
                     --disable-nls           \
                     --enable-gprofng=no     \
                     --disable-werror        \
                     --enable-new-dtags      \
                     --enable-default-hash-style=gnu \
                     2>&1 | tee "$LOGS/binutils-1-configure.log"
        make -j"$(nproc)" 2>&1 | tee "$LOGS/binutils-1-make.log"
        make install     2>&1 | tee "$LOGS/binutils-1-install.log"
    popd >/dev/null
}

# ---- step 2: GCC pass 1 -----------------------------------------------------
step_gcc-1() {
    local gcc_src="gcc-${GCC_VERSION}"
    local work="$BUILD_ROOT/work/$gcc_src"
    rm -rf "$work"; mkdir -p "$work"
    tar -xf "$SOURCES/${gcc_src}.tar.xz" -C "$BUILD_ROOT/work/"

    pushd "$work" >/dev/null
        # The GCC build expects mpfr/gmp/mpc/isl to live as siblings inside its
        # source tree, named without versions. LFS does this with `mv`.
        tar -xf "$SOURCES/mpfr-${MPFR_VERSION}.tar.xz" && mv "mpfr-${MPFR_VERSION}" mpfr
        tar -xf "$SOURCES/gmp-${GMP_VERSION}.tar.xz"   && mv "gmp-${GMP_VERSION}"   gmp
        tar -xf "$SOURCES/mpc-${MPC_VERSION}.tar.gz"   && mv "mpc-${MPC_VERSION}"   mpc
        tar -xf "$SOURCES/isl-${ISL_VERSION}.tar.xz"   && mv "isl-${ISL_VERSION}"   isl

        # Fix multilib lib path on 64-bit hosts so libgcc doesn't try /lib64.
        case "$(uname -m)" in
            x86_64) sed -e '/m64=/s/lib64/lib/' -i.orig gcc/config/i386/t-linux64 ;;
        esac

        mkdir -p build && cd build
        ../configure                                   \
            --target="$LFS_TGT"                        \
            --prefix="$LFS/tools"                      \
            --with-glibc-version="$GLIBC_VERSION"      \
            --with-sysroot="$LFS"                      \
            --with-newlib                              \
            --without-headers                          \
            --enable-default-pie                       \
            --enable-default-ssp                       \
            --disable-nls                              \
            --disable-shared                           \
            --disable-multilib                         \
            --disable-threads                          \
            --disable-libatomic                        \
            --disable-libgomp                          \
            --disable-libquadmath                      \
            --disable-libssp                           \
            --disable-libvtv                           \
            --disable-libstdcxx                        \
            --enable-languages=c,c++                   \
            2>&1 | tee "$LOGS/gcc-1-configure.log"
        make -j"$(nproc)" 2>&1 | tee "$LOGS/gcc-1-make.log"
        make install     2>&1 | tee "$LOGS/gcc-1-install.log"
        cd ..

        # Per LFS: install a fixed-up limits.h that pass-1 GCC will use until
        # the real glibc is built.
        cat gcc/limitx.h gcc/glimits.h gcc/limity.h > \
            "$($LFS/tools/bin/${LFS_TGT}-gcc -print-libgcc-file-name | sed 's@/libgcc.a@/install-tools/include/limits.h@')"
    popd >/dev/null
}

# ---- step 3: Linux API headers ----------------------------------------------
step_linux-headers() {
    local src="linux-${LINUX_VERSION}"
    local work="$BUILD_ROOT/work/$src"
    rm -rf "$work"
    tar -xf "$SOURCES/${src}.tar.xz" -C "$BUILD_ROOT/work/"
    pushd "$work" >/dev/null
        make mrproper                                  2>&1 | tee "$LOGS/linux-headers-mrproper.log"
        make headers                                   2>&1 | tee "$LOGS/linux-headers-make.log"
        find usr/include -type f ! -name '*.h' -delete
        mkdir -p "$LFS/usr"
        cp -rv usr/include "$LFS/usr"                  >>"$LOGS/linux-headers-install.log"
    popd >/dev/null
}

# ---- step 4: glibc ----------------------------------------------------------
step_glibc() {
    local src="glibc-${GLIBC_VERSION}"
    local work="$BUILD_ROOT/work/$src"
    rm -rf "$work"
    tar -xf "$SOURCES/${src}.tar.xz" -C "$BUILD_ROOT/work/"

    pushd "$work" >/dev/null
        # LFS-style /lib64 symlink on x86_64 (the cross-glibc will install
        # things in /usr/lib but expects a /lib64 dynamic-linker path).
        case "$(uname -m)" in
            x86_64)
                mkdir -p "$LFS/lib64"
                ln -sfn ../lib/ld-linux-x86-64.so.2 "$LFS/lib64/ld-linux-x86-64.so.2"
                ln -sfn ../lib/ld-linux-x86-64.so.2 "$LFS/lib64/ld-lsb-x86-64.so.3" ;;
        esac

        mkdir -p build && cd build
        # LFS additionally tells the build to use rpc/ from glibc's source.
        echo "rootsbindir=/usr/sbin" > configparms
        ../configure                                   \
            --prefix=/usr                              \
            --host="$LFS_TGT"                          \
            --build="$(../scripts/config.guess)"       \
            --enable-kernel=5.10                       \
            --with-headers="$LFS/usr/include"          \
            --disable-nscd                             \
            libc_cv_slibdir=/usr/lib                   \
            2>&1 | tee "$LOGS/glibc-configure.log"
        make -j"$(nproc)" 2>&1 | tee "$LOGS/glibc-make.log"
        make DESTDIR="$LFS" install 2>&1 | tee "$LOGS/glibc-install.log"

        # LFS fix-up: ldd's RTLDLIST has the wrong path after install.
        sed '/RTLDLIST=/s@/usr@@g' -i "$LFS/usr/bin/ldd"
    popd >/dev/null

    # Smoke test: cross-compile a hello-world and verify it links cleanly
    # against the new glibc.
    local tmp; tmp="$(mktemp -d)"
    cat > "$tmp/dummy.c" <<'C'
#include <stdio.h>
int main(void) { puts("hello, cross-glibc"); return 0; }
C
    "$LFS/tools/bin/$LFS_TGT-gcc" "$tmp/dummy.c" -o "$tmp/dummy"
    local interp
    # readelf prints "[Requesting program interpreter: /lib64/ld-linux-x86-64.so.2]";
    # extract just the path after "interpreter: ", stop before the trailing ']'.
    interp="$(readelf -l "$tmp/dummy" | grep -oP '(?<=interpreter: )[^]]+')"
    if [[ "$interp" == "/lib64/ld-linux-x86-64.so.2" ]]; then
        echo "    cross-glibc smoke test: OK (interp = $interp)"
    else
        echo "    cross-glibc smoke test: FAILED — interp = $interp"
        exit 5
    fi
    rm -rf "$tmp"
}

# ---- step 5: GCC pass 2 (full, with threads) ---------------------------------
#
# gcc-pass1 was built with --disable-threads --without-headers --disable-libstdcxx
# because it predates glibc. The result is a single-threaded compiler whose
# libstdc++ headers (built by step_libstdcxx using gcc-1) leave _GLIBCXX_HAS_GTHREADS
# undefined — std::mutex, std::thread, std::lock_guard<std::mutex>, std::condition_variable
# all fail to compile. Mesa 24.3 uses std::mutex in src/util/texcompress_astc_luts.h,
# which is enough to break the cross-build.
#
# gcc-pass2 re-bootstraps the cross-gcc against the just-built glibc with
# --enable-threads=posix --enable-shared, replacing gcc-pass1's binaries in
# $LFS/tools. After this, libstdcxx must be re-run so it inherits the new gcc's
# threading model.
step_gcc-2() {
    local gcc_src="gcc-${GCC_VERSION}"
    local work="$BUILD_ROOT/work/$gcc_src"
    rm -rf "$work"; mkdir -p "$work"
    tar -xf "$SOURCES/${gcc_src}.tar.xz" -C "$BUILD_ROOT/work/"

    pushd "$work" >/dev/null
        tar -xf "$SOURCES/mpfr-${MPFR_VERSION}.tar.xz" && mv "mpfr-${MPFR_VERSION}" mpfr
        tar -xf "$SOURCES/gmp-${GMP_VERSION}.tar.xz"   && mv "gmp-${GMP_VERSION}"   gmp
        tar -xf "$SOURCES/mpc-${MPC_VERSION}.tar.gz"   && mv "mpc-${MPC_VERSION}"   mpc
        tar -xf "$SOURCES/isl-${ISL_VERSION}.tar.xz"   && mv "isl-${ISL_VERSION}"   isl

        case "$(uname -m)" in
            x86_64) sed -e '/m64=/s/lib64/lib/' -i.orig gcc/config/i386/t-linux64 ;;
        esac

        # Wipe gcc-pass1 binaries + the sysroot-wrapper trick. gcc-pass2 is
        # configured with --with-sysroot=$LFS so the baked-in default sysroot
        # replaces the need for a wrapper script.
        local td="$LFS/tools/bin"
        for prog in gcc g++ cpp c++; do
            rm -f "$td/${LFS_TGT}-${prog}.real" "$td/${LFS_TGT}-${prog}"
        done

        mkdir -p build-pass2 && cd build-pass2
        ../configure                                   \
            --target="$LFS_TGT"                        \
            --prefix="$LFS/tools"                      \
            --with-build-sysroot="$LFS"                \
            --with-sysroot="$LFS"                      \
            --enable-default-pie                       \
            --enable-default-ssp                       \
            --disable-nls                              \
            --disable-multilib                         \
            --disable-libatomic                        \
            --disable-libsanitizer                     \
            --disable-libssp                           \
            --disable-libvtv                           \
            --enable-languages=c,c++                   \
            --enable-threads=posix                     \
            --enable-shared                            \
            2>&1 | tee "$LOGS/gcc-2-configure.log"
        make -j"$(nproc)" 2>&1 | tee "$LOGS/gcc-2-make.log"
        make install      2>&1 | tee "$LOGS/gcc-2-install.log"
    popd >/dev/null

    # Smoke test: cross gcc-2 must now report Thread model: posix.
    local model
    model="$("$LFS/tools/bin/$LFS_TGT-gcc" -v 2>&1 | awk -F': ' '/Thread model/{print $2}')"
    if [[ "$model" != "posix" ]]; then
        echo "    gcc-2 smoke test: FAILED — Thread model = $model (expected posix)" >&2
        return 5
    fi
    echo "    gcc-2 smoke test: OK (Thread model = posix)"
}

# ---- step 6: libstdc++ from GCC ---------------------------------------------
step_libstdcxx() {
    local gcc_src="gcc-${GCC_VERSION}"
    local work="$BUILD_ROOT/work/$gcc_src"
    # GCC source may still be present from step 2; if not, re-extract.
    [[ -d "$work" ]] || tar -xf "$SOURCES/${gcc_src}.tar.xz" -C "$BUILD_ROOT/work/"
    # Standalone libstdc++ has no gthr-default.h — it's normally generated by
    # gcc's libgcc build (which we skipped for gcc-pass1). Without it, the
    # `gthreads library` configure probe fails and _GLIBCXX_HAS_GTHREADS
    # stays undefined → no std::mutex. Point gthr-default.h at the POSIX
    # implementation so libstdc++ detects threading is available.
    ln -sf gthr-posix.h "$work/libgcc/gthr-default.h"
    rm -rf "$work/build-libstdcxx"
    pushd "$work" >/dev/null
        mkdir -p build-libstdcxx && cd build-libstdcxx
        ../libstdc++-v3/configure                      \
            --host="$LFS_TGT"                          \
            --build="$(../config.guess)"               \
            --prefix=/usr                              \
            --disable-multilib                         \
            --disable-nls                              \
            --disable-libstdcxx-pch                    \
            --with-gxx-include-dir="/tools/$LFS_TGT/include/c++/${GCC_VERSION}" \
            2>&1 | tee "$LOGS/libstdcxx-configure.log"
        make -j"$(nproc)" 2>&1 | tee "$LOGS/libstdcxx-make.log"
        make DESTDIR="$LFS" install 2>&1 | tee "$LOGS/libstdcxx-install.log"
        # The libstdc++.la files are not needed and confuse later builds.
        rm -fv "$LFS"/usr/lib/lib{stdc++{,exp,fs},supc++}.la
    popd >/dev/null
}

# ---- driver -----------------------------------------------------------------
if [[ $# -eq 0 ]]; then
    for s in "${STEPS[@]}"; do do_step "$s"; done
else
    for s in "$@"; do
        if [[ ! " ${STEPS[*]} " == *" $s "* ]]; then
            echo "unknown step: $s"; echo "valid: ${STEPS[*]}"; exit 1
        fi
        do_step "$s"
    done
fi

echo
echo "Cross-toolchain steps complete:"
ls "$LFS/tools/bin/" | head -20
echo
echo "Cross-compiler version:"
"$LFS/tools/bin/$LFS_TGT-gcc" --version | head -n1
