#!/usr/bin/env bash
# build/10-xorg-server.sh — Phase 8 round 3: X.Org server + DRM/KMS + input.
#
# Cross-builds Mesa (Intel-only, no LLVM/Vulkan/Wayland), the X.Org server
# with the modesetting driver + glamor acceleration, libdrm for KMS
# access, and the libinput-based input driver chain.
#
# Run AFTER ./09-x11-stack.sh completes.
#
# This is the heaviest single round in Phase 8 (Mesa ~1 hour + xorg-server
# ~30 min). Sentinel files prevent redoing; per-step logs at
# logs/blfs-<name>-*.log. Use ./10-xorg-server.sh <step> to run a single
# step for debugging.
#
# Build order:
#   Foundation:   libdrm       (DRM/KMS API)
#                 libpciaccess  (Xorg's PCI bus probe)
#                 libXfont2    (server-side font loader)
#                 libepoxy     (OpenGL function loader; used by glamor)
#   Input chain:  libevdev → mtdev → libinput  (kernel evdev → high-level)
#   Heavy:        mesa-libs    (libGL/libEGL/libgbm — for glamor + DRI)
#   X server:     xorg-server  (modesetting + glamor + udev hotplug)
#   Input driver: xf86-input-libinput

set -uo pipefail

cd "$( dirname "${BASH_SOURCE[0]}" )"
# shellcheck disable=SC1091
source ./setup-env.sh
# shellcheck disable=SC1091
source ./blfs-pkg.sh

# Sanity: Phase 8b complete?
[[ -f "$LOGS/.done-blfs-xkeyboard-config" ]] || {
    echo "Phase 8b (X11 stack) not complete. Run ./09-x11-stack.sh first." >&2
    exit 1
}

# ============================================================================
# Foundation: eudev + libdrm + libpciaccess + libXfont2 + libepoxy
# ============================================================================

step_eudev() {
    # Device hotplug. The udev daemon + libudev. xorg-server's
    # -Dudev=true and libinput both need libudev at build time.
    # We use eudev (Gentoo's fork) so we don't have to drag in systemd.
    build_pkg eudev "eudev-${EUDEV_VERSION}.tar.gz" \
        --disable-manpages \
        --disable-hwdb-update \
        --sysconfdir=/etc \
        --localstatedir=/var
}

step_libdrm() {
    # KMS / DRM userspace library. Disable vendor-specific drivers we don't
    # need (AMD/NVIDIA/VC4/etc.) — Intel only.
    build_meson libdrm "libdrm-${LIBDRM_VERSION}.tar.xz" \
        -Dintel=enabled \
        -Damdgpu=disabled \
        -Dnouveau=disabled \
        -Dradeon=disabled \
        -Dvmwgfx=disabled \
        -Dfreedreno=disabled \
        -Dvc4=disabled \
        -Detnaviv=disabled \
        -Dman-pages=disabled \
        -Dvalgrind=disabled
}

step_libpciaccess() {
    build_meson libpciaccess "libpciaccess-${LIBPCIACCESS_VERSION}.tar.xz"
}

step_libfontenc() {
    build_pkg libfontenc "libfontenc-${LIBFONTENC_VERSION}.tar.xz" \
        --with-encodingsdir=/usr/share/fonts/X11/encodings
}

step_libxshmfence() {
    # Shared-memory fence sync between Mesa and the X server (DRI3 path).
    build_pkg libxshmfence "libxshmfence-${LIBXSHMFENCE_VERSION}.tar.xz"
}

step_libXxf86vm() {
    # XFree86 video-mode extension. Mesa references it for legacy DRI.
    build_pkg libXxf86vm "libXxf86vm-${LIBXXF86VM_VERSION}.tar.xz" \
        --disable-malloc0returnsnull
}

step_libXfont2() {
    # Server-side font loader. Xorg-server needs it for legacy font paths
    # even with modern Xft-based clients.
    build_pkg libXfont2 "libXfont2-${LIBXFONT2_VERSION}.tar.xz" \
        --disable-devel-docs
}

step_pixman() {
    # 2D rasteriser used by xorg-server (Composite/Render) and cairo. Pure
    # software lib, no external deps beyond libc + libm.
    build_meson pixman "pixman-${PIXMAN_VERSION}.tar.gz" \
        -Dtests=disabled \
        -Ddemos=disabled
}

step_libxkbfile() {
    # XKB keymap file parser — required by xorg-server's keyboard subsystem.
    build_pkg libxkbfile "libxkbfile-${LIBXKBFILE_VERSION}.tar.xz"
}

step_font-util() {
    # Build-time font helpers (bdftopcf, fontc, mkfontscale) referenced by
    # xorg-server's font-path setup. Provides the `fontutil` pkg-config
    # file that xorg-server's meson.build queries.
    build_pkg font-util "font-util-${FONT_UTIL_VERSION}.tar.xz"
}

step_libxcvt() {
    # CVT timing-formula library — xorg-server uses it for mode-line
    # computation. Replaces older libxf86config; required dep since 21.x.
    build_meson libxcvt "libxcvt-${LIBXCVT_VERSION}.tar.xz"
}

step_libmd() {
    # Portable BSD message-digest library (MD5/SHA1/SHA256). xorg-server
    # picks this over openssl/libgcrypt/nettle when present — smallest
    # crypto dep with no transitive baggage.
    build_pkg libmd "libmd-${LIBMD_VERSION}.tar.xz"
}

step_libXdmcp() {
    # X Display Manager Control Protocol library — required by xorg-server
    # (auth/cookie subsystem references xdmcp.pc unconditionally; the
    # protocol itself can stay unused at runtime).
    build_pkg libXdmcp "libXdmcp-${LIBXDMCP_VERSION}.tar.xz"
}

step_libepoxy() {
    # OpenGL function loader. The X server's glamor acceleration uses it
    # to call into mesa-libs' libGL at runtime.
    build_meson libepoxy "libepoxy-${LIBEPOXY_VERSION}.tar.gz" \
        -Dtests=false \
        -Degl=yes \
        -Dglx=yes \
        -Dx11=true
}

# ============================================================================
# Input chain: libevdev → mtdev → libinput
# ============================================================================

step_libevdev() {
    # Thin wrapper around the kernel's input-event interface
    # (/dev/input/eventN). libinput uses it.
    build_pkg libevdev "libevdev-${LIBEVDEV_VERSION}.tar.xz" \
        --disable-tests \
        --disable-documentation
}

step_mtdev() {
    # Multitouch event decoder; older devices report multitouch via the
    # "MT protocol A" which mtdev translates into "MT protocol B".
    build_pkg mtdev "mtdev-${MTDEV_VERSION}.tar.bz2"
}

step_libinput() {
    # The modern unified input library used by Xorg and Wayland. Reads
    # /dev/input/event*, applies acceleration/palm-rejection/etc.
    build_meson libinput "libinput-${LIBINPUT_VERSION}.tar.gz" \
        -Dlibwacom=false \
        -Ddebug-gui=false \
        -Dtests=false \
        -Ddocumentation=false \
        -Dudev-dir=/usr/lib/udev
}

# ============================================================================
# Heavy: mesa-libs
# ============================================================================

step_mesa() {
    # Mesa 24.0.9, iris driver only. No LLVM, no Vulkan, no CLC kernels.
    # Iris (Broadwell HD 5500 / Gen8) uses Mesa's brw_compile (in-tree C
    # backend) for shader compilation — no LLVM dep needed. CLC kernels
    # (intel_clc / mesa_clc) are only required when explicitly enabled;
    # Mesa 24.1+ implicitly forced them for iris, which made same-arch
    # cross builds impossible — see plan/phase-8 notes. 24.0.9 has no
    # such requirement.
    build_meson mesa "mesa-${MESA_VERSION}.tar.xz" \
        -Dgallium-drivers=iris \
        -Dvulkan-drivers= \
        -Dplatforms=x11 \
        -Dllvm=disabled \
        -Dgallium-extra-hud=false \
        -Dgallium-va=disabled \
        -Dgallium-xa=disabled \
        -Dgallium-opencl=disabled \
        -Dgallium-rusticl=false \
        -Dgallium-vdpau=disabled \
        -Dmicrosoft-clc=disabled \
        -Dintel-clc=disabled \
        -Dvideo-codecs= \
        -Dosmesa=false \
        -Dglvnd=false \
        -Dgles1=disabled \
        -Dgles2=enabled \
        -Dopengl=true \
        -Degl=enabled \
        -Dglx=dri \
        -Dgbm=enabled \
        -Dshared-glapi=enabled \
        -Dvalgrind=disabled \
        -Dlibunwind=disabled \
        -Dperfetto=false \
        -Dtools=
}

# ============================================================================
# X server
# ============================================================================

step_xorg-server() {
    # Configure the server for:
    #   - just Xorg (no Xvfb, Xnest, Xephyr, Xwayland)
    #   - glamor acceleration (uses GBM + libepoxy + libGL via DRI3)
    #   - udev-driven device hotplug + KMS detection
    #   - the suid wrapper so non-root login users can start it via startx
    build_meson xorg-server "xorg-server-${XORG_SERVER_VERSION}.tar.xz" \
        -Dxorg=true \
        -Dxnest=false \
        -Dxvfb=false \
        -Dxephyr=false \
        -Dxwin=false \
        -Dglamor=true \
        -Ddri3=true \
        -Dudev=true \
        -Dudev_kms=true \
        -Dlibunwind=false \
        -Dsuid_wrapper=true \
        -Dxkb_dir=/usr/share/X11/xkb \
        -Dxkb_bin_dir=/usr/bin \
        -Dxkb_default_rules=base \
        -Ddocs=false \
        -Ddevel-docs=false \
        -Dsystemd_logind=false \
        -Dsecure-rpc=false \
        -Dxdm-auth-1=false
}

# ============================================================================
# Input driver
# ============================================================================

step_xf86-input-libinput() {
    # The Xorg input driver that binds libinput to the X core protocol.
    # Replaces the older xf86-input-evdev / xf86-input-synaptics drivers.
    build_pkg xf86-input-libinput "xf86-input-libinput-${XF86_INPUT_LIBINPUT_VERSION}.tar.xz"
}

# ============================================================================
# Driver
# ============================================================================

STEPS=(
    eudev
    libpciaccess libdrm libfontenc libxshmfence libXxf86vm libXfont2 libxkbfile font-util libxcvt libmd libXdmcp
    libevdev mtdev libinput
    mesa
    libepoxy
    pixman
    xorg-server
    xf86-input-libinput
)

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
echo "Phase 8c xorg-server stack: $(count_done_packages) packages built (cumulative)."
echo "Next: ./11-gtk-stack.sh (when populated) — glib + cairo + pango + gtk4."
