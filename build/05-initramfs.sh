#!/usr/bin/env bash
# build/05-initramfs.sh — cross-build BusyBox and assemble the transitional
# initramfs that Phase 2's first boot will use. Replaced wholesale by the
# Rust initramfs in Phase 5; for now BusyBox does the job.
#
# Steps:
#   busybox             cross-build BusyBox statically
#   initramfs-root      author the /init tree (busybox applets, /etc, modules)
#   initramfs-pack      cpio + gzip into artifacts/initramfs.img

set -uo pipefail

cd "$( dirname "${BASH_SOURCE[0]}" )"
# shellcheck disable=SC1091
source ./setup-env.sh

[[ -x "$LFS/tools/bin/$LFS_TGT-gcc" ]] || {
    echo "Cross-toolchain not found. Run ./02-cross-toolchain.sh first." >&2
    exit 1
}
[[ -f "$BUILD_ROOT/artifacts/bzImage" ]] || {
    echo "bzImage not found. Run ./04-kernel.sh first." >&2
    exit 1
}

STEPS=( busybox initramfs-root initramfs-pack )

BUSYBOX_SRC="busybox-${BUSYBOX_VERSION}"
BUSYBOX_WORK="$BUILD_ROOT/work/$BUSYBOX_SRC"
INITRAMFS_ROOT="$BUILD_ROOT/work/initramfs-root"
MODULES_STAGE="$BUILD_ROOT/artifacts/modules-stage"

do_step() {
    local step="$1"
    local sentinel="$LOGS/.done-$step"
    if [[ -f "$sentinel" ]]; then
        echo ">>> $step already complete (delete $sentinel to redo)"
        return 0
    fi
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

step_busybox() {
    rm -rf "$BUSYBOX_WORK"
    tar -xf "$SOURCES/${BUSYBOX_SRC}.tar.bz2" -C "$BUILD_ROOT/work/"
    pushd "$BUSYBOX_WORK" >/dev/null
        make defconfig 2>&1 | tee "$LOGS/busybox-defconfig.log"
        # Force static linkage (smaller initramfs, no glibc deps at boot).
        sed -i 's/# CONFIG_STATIC is not set/CONFIG_STATIC=y/' .config
        # Drop `tc` (broken without iproute2 internals) and `inetd` (not needed).
        sed -i 's/^CONFIG_TC=y/# CONFIG_TC is not set/'        .config
        sed -i 's/^CONFIG_INETD=y/# CONFIG_INETD is not set/'  .config

        make CROSS_COMPILE="$LFS/tools/bin/$LFS_TGT-" -j"$(nproc)" \
            2>&1 | tee "$LOGS/busybox-build.log"
        cp busybox "$BUILD_ROOT/artifacts/busybox"
    popd >/dev/null
    echo "    BusyBox: $(du -h $BUILD_ROOT/artifacts/busybox | awk '{print $1}')"
}

step_initramfs-root() {
    rm -rf "$INITRAMFS_ROOT"
    mkdir -p "$INITRAMFS_ROOT"/{bin,sbin,etc,proc,sys,dev,run,tmp,root,old_root}
    mkdir -p "$INITRAMFS_ROOT"/{usr/bin,usr/sbin,lib/modules,lib/firmware}

    cp "$BUILD_ROOT/artifacts/busybox" "$INITRAMFS_ROOT/bin/busybox"
    chmod +x "$INITRAMFS_ROOT/bin/busybox"

    # Create symlinks for every applet. Use the host's stat of the binary,
    # since we may not be running on the same arch as the busybox we built.
    pushd "$INITRAMFS_ROOT/bin" >/dev/null
        # Standard applet set; busybox --list prints them. Run via qemu-user if
        # available, otherwise fall back to a curated list.
        if command -v qemu-x86_64-static >/dev/null 2>&1; then
            applets=$(qemu-x86_64-static "$BUILD_ROOT/artifacts/busybox" --list)
        else
            applets="sh bash ls cp mv rm cat echo mkdir mknod rmdir ln chmod chown
                     mount umount ifconfig ip route dmesg lsmod modprobe insmod rmmod
                     dd df du free hostname hwclock init kill killall login mdev nslookup
                     ping ps reboot poweroff sleep stat sync tar tee touch true false test
                     udhcpc vi which whoami yes wget"
        fi
        for cmd in $applets; do
            [[ "$cmd" == "busybox" ]] && continue
            ln -sf busybox "$cmd" 2>/dev/null || true
        done
    popd >/dev/null

    # Minimal /etc
    cat > "$INITRAMFS_ROOT/etc/passwd" <<'PASS'
root:x:0:0:root:/root:/bin/sh
PASS
    cat > "$INITRAMFS_ROOT/etc/group" <<'GROUP'
root:x:0:
GROUP
    cat > "$INITRAMFS_ROOT/etc/fstab" <<'FSTAB'
proc      /proc  proc      defaults  0 0
sysfs     /sys   sysfs     defaults  0 0
devtmpfs  /dev   devtmpfs  defaults  0 0
tmpfs     /tmp   tmpfs     defaults  0 0
tmpfs     /run   tmpfs     defaults  0 0
FSTAB

    # Transitional /init — replaced wholesale by Rust binary in Phase 5.
    cat > "$INITRAMFS_ROOT/init" <<'INIT'
#!/bin/sh
# WriteOnce OS — transitional initramfs /init (Phase 2 placeholder).
#
# Mounts the kernel-provided pseudo-filesystems, optionally brings up
# wired ethernet, and drops to a BusyBox shell. There is no PID 1 reaping
# beyond what BusyBox sh does — that's deliberate. Phase 5 will replace
# this entire binary with a Rust /init that hands off to /sbin/writeonce-pid1.

set -e

/bin/busybox mount -t proc     proc     /proc
/bin/busybox mount -t sysfs    sysfs    /sys
/bin/busybox mount -t devtmpfs devtmpfs /dev
/bin/busybox mount -t tmpfs    tmpfs    /run

echo
echo "================================================================"
echo "  WriteOnce OS — transitional initramfs (Phase 2)"
echo "================================================================"
echo "  Kernel:   $(/bin/busybox uname -r)"
echo "  Memory:   $(/bin/busybox free -h | /bin/busybox awk '/^Mem:/ {print $2 " total, " $7 " avail"}')"
echo "  Modules:  $(/bin/busybox ls /lib/modules 2>/dev/null | head -1 || echo 'none')"
echo "================================================================"
echo

# Best-effort: bring up wired ethernet via DHCP (won't fail boot if absent).
if /bin/busybox ip link show enp0s25 >/dev/null 2>&1; then
    /bin/busybox ip link set enp0s25 up
    /bin/busybox udhcpc -i enp0s25 -q -t 3 -n 2>/dev/null || true
fi

echo "Drop to BusyBox shell (PID $$). Type 'reboot' to restart."
exec /bin/busybox sh
INIT
    chmod +x "$INITRAMFS_ROOT/init"

    # Stage kernel modules
    if [[ -d "$MODULES_STAGE/lib/modules" ]]; then
        cp -r "$MODULES_STAGE/lib/modules" "$INITRAMFS_ROOT/lib/"
    fi

    # Stage iwlwifi firmware if Phase 1 captured it
    if compgen -G "$BUILD_ROOT/firmware/iwlwifi*" >/dev/null 2>&1; then
        cp "$BUILD_ROOT/firmware/iwlwifi"* "$INITRAMFS_ROOT/lib/firmware/" 2>/dev/null || true
    fi

    echo "    initramfs root: $(du -sh $INITRAMFS_ROOT | awk '{print $1}')"
}

step_initramfs-pack() {
    pushd "$INITRAMFS_ROOT" >/dev/null
        find . | cpio -H newc -o --quiet | gzip -9 \
            > "$BUILD_ROOT/artifacts/initramfs.img"
    popd >/dev/null
    echo "    initramfs.img: $(du -h $BUILD_ROOT/artifacts/initramfs.img | awk '{print $1}')"
}

# ---- driver -----------------------------------------------------------------
if [[ $# -eq 0 ]]; then
    for s in "${STEPS[@]}"; do do_step "$s" || exit 1; done
else
    for s in "$@"; do
        if [[ ! " ${STEPS[*]} " == *" $s "* ]]; then
            echo "unknown step: $s"; echo "valid: ${STEPS[*]}"; exit 1
        fi
        do_step "$s" || exit 1
    done
fi

echo
echo "Phase 2 initramfs complete."
echo "  bzImage:        $BUILD_ROOT/artifacts/bzImage"
echo "  initramfs.img:  $BUILD_ROOT/artifacts/initramfs.img"
echo "  Next:           ./06-qemu-smoke.sh   (boot the pair in QEMU)"
