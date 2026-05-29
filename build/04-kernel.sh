#!/usr/bin/env bash
# build/04-kernel.sh — cross-build Linux 6.12 for the ThinkPad T450.
#
# Steps:
#   kernel-extract         expand sources into work/
#   kernel-config          defconfig + merge WriteOnce fragment + olddefconfig
#   kernel-build           bzImage + modules (cross-compiled)
#   kernel-modules-stage   staged copy of modules under artifacts/modules-stage
#
# Usage:
#   ./04-kernel.sh                         # all steps
#   ./04-kernel.sh kernel-build            # just one
#   delete logs/.done-<step> to redo

set -uo pipefail

cd "$( dirname "${BASH_SOURCE[0]}" )"
# shellcheck disable=SC1091
source ./setup-env.sh

[[ -x "$LFS/tools/bin/$LFS_TGT-gcc" ]] || {
    echo "Cross-toolchain not found. Run ./02-cross-toolchain.sh first." >&2
    exit 1
}

STEPS=( kernel-extract kernel-config kernel-build kernel-modules-stage )
KERNEL_SRC="linux-${LINUX_VERSION}"
KERNEL_WORK="$BUILD_ROOT/work/$KERNEL_SRC"
FRAGMENT="$BUILD_ROOT/kernel-config-additions.fragment"

CROSS=(ARCH=x86_64 "CROSS_COMPILE=$LFS/tools/bin/$LFS_TGT-")

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
    if "step_$step"; then
        touch "$sentinel"
        echo "<<< $step done"
    else
        echo "!!! $step FAILED"
        return 1
    fi
}

step_kernel-extract() {
    rm -rf "$KERNEL_WORK"
    tar -xf "$SOURCES/${KERNEL_SRC}.tar.xz" -C "$BUILD_ROOT/work/"
    echo "    extracted: $KERNEL_WORK"
}

step_kernel-config() {
    [[ -f "$FRAGMENT" ]] || { echo "missing fragment: $FRAGMENT"; return 1; }
    # Base config: prefer build/kernel-base.config (Ubuntu's stable-LTS
    # config, stripped of distro-specific cert references). This gives
    # us thousands of hardware-compat options the Broadwell/Aptio-V
    # T450 needs that `defconfig` doesn't enable.
    # If kernel-base.config isn't present, fall back to defconfig — that
    # was the original path; useful for minimal builds + understanding
    # bare-defconfig deltas.
    local BASE="$BUILD_ROOT/kernel-base.config"
    pushd "$KERNEL_WORK" >/dev/null
        make "${CROSS[@]}" mrproper          2>&1 | tee "$LOGS/kernel-mrproper.log"
        if [[ -f "$BASE" ]]; then
            echo "    base: $BASE ($(wc -l < "$BASE") lines, distro-derived)"
            cp "$BASE" .config
        else
            echo "    base: defconfig (no kernel-base.config present)"
            make "${CROSS[@]}" defconfig     2>&1 | tee "$LOGS/kernel-defconfig.log"
        fi
        ./scripts/kconfig/merge_config.sh -m .config "$FRAGMENT" \
                                              2>&1 | tee "$LOGS/kernel-mergeconfig.log"
        make "${CROSS[@]}" olddefconfig      2>&1 | tee "$LOGS/kernel-olddefconfig.log"
        cp .config "$BUILD_ROOT/artifacts/kernel.config"
        echo "    config: $(wc -l < .config) lines, copy at artifacts/kernel.config"
    popd >/dev/null
}

step_kernel-build() {
    # `make … | tee` masks make's exit unless we check PIPESTATUS or
    # chain explicitly. Without this, a failing build (e.g. missing
    # `bc` in the container blowing up timeconst.h generation) gets
    # rubber-stamped by the `cp` afterwards, which silently no-ops on
    # a missing bzImage — leaving an ancient stub bzImage in
    # artifacts/ that boots into an old kernel.
    pushd "$KERNEL_WORK" >/dev/null
        make "${CROSS[@]}" -j"$(nproc)" bzImage modules \
                                              2>&1 | tee "$LOGS/kernel-build.log" && \
        cp arch/x86/boot/bzImage "$BUILD_ROOT/artifacts/bzImage" && \
        echo "    bzImage: $(du -h $BUILD_ROOT/artifacts/bzImage | awk '{print $1}')" \
            || { popd >/dev/null; echo "ERROR: kernel-build failed" >&2; return 1; }
    popd >/dev/null
}

step_kernel-modules-stage() {
    local stage="$BUILD_ROOT/artifacts/modules-stage"
    rm -rf "$stage"
    pushd "$KERNEL_WORK" >/dev/null
        make "${CROSS[@]}" \
             INSTALL_MOD_PATH="$stage" \
             INSTALL_MOD_STRIP=1 \
             modules_install                  2>&1 | tee "$LOGS/kernel-modules-install.log"
    popd >/dev/null
    echo "    modules staged: $(du -sh $stage | awk '{print $1}') -> $stage"
}

# ---- driver -----------------------------------------------------------------
if [[ $# -eq 0 ]]; then
    for s in "${STEPS[@]}"; do do_step "$s" || { echo "stopping"; exit 1; }; done
else
    for s in "$@"; do
        if [[ ! " ${STEPS[*]} " == *" $s "* ]]; then
            echo "unknown step: $s"; echo "valid: ${STEPS[*]}"; exit 1
        fi
        do_step "$s" || exit 1
    done
fi

echo
echo "Phase 2 kernel build complete."
echo "  bzImage:        $BUILD_ROOT/artifacts/bzImage"
echo "  modules-stage:  $BUILD_ROOT/artifacts/modules-stage"
echo "  Next:           ./05-initramfs.sh"
