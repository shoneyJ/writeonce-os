#!/usr/bin/env bash
# build/16a-wayland-substrate.sh — Phase 8 round 7a: Wayland substrate.
#
# ┌─ PARKED (not on the active critical path) ────────────────────────────────┐
# │ The desktop (Hyprland + Qt6 + Quickshell) is delivered via Nix — see       │
# │ /etc/writeonce/desktop/flake.nix + plan/phase-14-nix-packages.md. A Nix     │
# │ Hyprland closure is self-contained (its own Mesa/wayland/seatd from        │
# │ nixpkgs) and only needs the kernel's DRM, so this from-source substrate is │
# │ NOT required for it. Retained as the from-source fallback; its packages    │
# │ are NOT yet wired into versions.env / 01-fetch.sh. To activate: add the    │
# │ version pins + fetch URLs, then run after 16-systemd.sh.                   │
# └────────────────────────────────────────────────────────────────────────────┘
#
# Lays the Wayland foundation under the Hyprland + Qt6 + Quickshell desktop
# (16b-16d). The X11/i3 stack (09-11) stays in the sysroot only as reusable
# libraries + an optional future XWayland; nothing here is X-specific.
#
# Build order:
#   wayland           ← libwayland-client/server (core protocol)
#   wayland-protocols ← xdg-shell, layer-shell, … (data + .pc)
#   libxkbcommon (WL) ← rebuild of the 09 lib with -Denable-wayland=true
#   mesa (WL)         ← rebuild of the 10 lib with -Dplatforms=x11,wayland
#   seatd / libseat   ← seat management via systemd-logind (DRM master + input)
#   hwdata            ← pnp.ids for EDID vendor lookup
#   libdisplay-info   ← EDID/DisplayID parsing (aquamarine, Hyprland outputs)
#   libliftoff        ← KMS plane offload helper (aquamarine)
#
# Reused as-is from 10: libdrm, pixman, libinput, libevdev, mtdev, eudev.
#
# Run AFTER ./16-systemd.sh. Sentinels at logs/.done-blfs-<name>; the two
# in-place rebuilds use logs/.done-blfs-<name>-wl markers so re-running this
# script is idempotent (the ~1h mesa rebuild does not repeat).

set -uo pipefail

cd "$( dirname "${BASH_SOURCE[0]}" )"
# shellcheck disable=SC1091
source ./setup-env.sh
# shellcheck disable=SC1091
source ./blfs-pkg.sh

# Gate: systemd (16) provides logind, which libseat's logind backend talks to.
[[ -f "$LOGS/.done-blfs-systemd" ]] || {
    echo "systemd (Phase 8 / round 6) not complete. Run ./16-systemd.sh first." >&2
    exit 1
}

# ---- 1. wayland (libwayland) ------------------------------------------------
# Target libwayland-client/server. The scanner it also builds is a target
# binary we ignore — downstream codegen uses the HOST wayland-scanner from the
# Containerfile (libwayland-bin), found via meson `native: true`.
step_wayland() {
    build_meson wayland "wayland-${WAYLAND_VERSION}.tar.xz" \
        -Ddocumentation=false \
        -Dtests=false \
        -Ddtd_validation=false
}

# ---- 2. wayland-protocols ----------------------------------------------------
# Pure data: protocol XML + wayland-protocols.pc. No compilation.
step_wayland-protocols() {
    build_meson wayland-protocols "wayland-protocols-${WAYLAND_PROTOCOLS_VERSION}.tar.xz" \
        -Dtests=false
}

# ---- 3. libxkbcommon rebuild (Wayland enabled) -------------------------------
# 09-x11-stack.sh built this with -Denable-wayland=false. Qt6 + Hyprland need
# the Wayland integration. Additive: keep -Denable-x11=true so the X11 stack
# still links the same lib.
step_libxkbcommon-wl() {
    local marker="$LOGS/.done-blfs-libxkbcommon-wl"
    [[ -f "$marker" ]] && { echo "skip libxkbcommon (wayland rebuild already done)"; return 0; }
    rm -f "$LOGS/.done-blfs-libxkbcommon"
    build_meson libxkbcommon "libxkbcommon-${LIBXKBCOMMON_VERSION}.tar.xz" \
        -Denable-wayland=true \
        -Denable-docs=false \
        -Denable-x11=true \
        -Denable-tools=false \
        || return 1
    touch "$marker"
}

# ---- 4. mesa rebuild (Wayland platform added) --------------------------------
# 10-xorg-server.sh built mesa with -Dplatforms=x11. Hyprland/aquamarine +
# Quickshell GL need the Wayland EGL platform (wayland-egl.pc, wl_* EGL syms).
# -Dplatforms=x11,wayland is ADDITIVE — the x11 platform (XWayland/glamor)
# keeps working. All other flags identical to 10's step_mesa.
step_mesa-wl() {
    local marker="$LOGS/.done-blfs-mesa-wl"
    [[ -f "$marker" ]] && { echo "skip mesa (wayland rebuild already done)"; return 0; }
    rm -f "$LOGS/.done-blfs-mesa"
    build_meson mesa "mesa-${MESA_VERSION}.tar.xz" \
        -Dgallium-drivers=iris \
        -Dvulkan-drivers= \
        -Dplatforms=x11,wayland \
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
        -Dtools= \
        || return 1
    touch "$marker"
}

# ---- 5. seatd / libseat ------------------------------------------------------
# libseat is how a non-root Hyprland acquires DRM master + opens input device
# fds. -Dlibseat-logind=systemd routes seat requests to systemd-logind (built
# in 16). We also build the standalone seatd daemon + builtin backend as
# fallbacks.
step_seatd() {
    build_meson seatd "seatd-${SEATD_VERSION}.tar.gz" \
        -Dlibseat-seatd=enabled \
        -Dlibseat-logind=systemd \
        -Dlibseat-builtin=enabled \
        -Dserver=enabled \
        -Dexamples=disabled \
        -Dman-pages=disabled
}

# ---- 6. hwdata ---------------------------------------------------------------
# Pure data (pci.ids/usb.ids/pnp.ids) + hwdata.pc. Its configure is a thin
# shell script that does not compile anything, so we invoke it directly rather
# than via build_pkg (which would pass --host/--disable-static it rejects).
step_hwdata() {
    local name=hwdata
    local sentinel="$LOGS/.done-blfs-$name"
    [[ -f "$sentinel" ]] && { echo "skip $name"; return 0; }
    echo; echo "==== blfs: $name ===="
    rm -rf "$BUILD_ROOT/work/$name"; mkdir -p "$BUILD_ROOT/work/$name"
    tar -xf "$SOURCES/hwdata-${HWDATA_VERSION}.tar.gz" \
        -C "$BUILD_ROOT/work/$name" --strip-components=1
    pushd "$BUILD_ROOT/work/$name" >/dev/null
        ./configure --prefix=/usr --datarootdir=/usr/share \
            2>&1 | tee "$LOGS/blfs-$name-configure.log" && \
        make DESTDIR="$LFS" install \
            2>&1 | tee "$LOGS/blfs-$name-install.log" \
            || { popd >/dev/null; echo "ERROR: $name failed" >&2; return 1; }
    popd >/dev/null
    touch "$sentinel"
    echo "<<< $name done"
}

# ---- 7. libdisplay-info ------------------------------------------------------
# EDID/DisplayID parsing; aquamarine + Hyprland output management depend on it.
# Reads hwdata's pnp.ids for vendor names → build after hwdata.
step_libdisplay-info() {
    build_meson libdisplay-info "libdisplay-info-${LIBDISPLAY_INFO_VERSION}.tar.gz" \
        -Dtests=false
}

# ---- 8. libliftoff -----------------------------------------------------------
# KMS plane-offloading helper aquamarine uses for hardware overlay/cursor
# planes. Cheap; aquamarine builds cleaner with it present.
step_libliftoff() {
    build_meson libliftoff "libliftoff-${LIBLIFTOFF_VERSION}.tar.gz" \
        -Dtests=false \
        -Dexamples=false
}

# ---- driver -----------------------------------------------------------------

STEPS=( wayland wayland-protocols libxkbcommon-wl mesa-wl seatd
        hwdata libdisplay-info libliftoff )

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
echo "Phase 8 round 7a Wayland substrate: $(count_done_packages) packages built (cumulative)."
echo "Next: ./16b-hyprland.sh — hypr* libs + aquamarine + Hyprland."
