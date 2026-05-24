#!/usr/bin/env bash
# build/09-x11-stack.sh — Phase 8 round 2: X11 protocol + libraries.
#
# Cross-builds the X11 protocol headers, the core libraries (libxcb,
# libX11), the X extension libraries that GTK4 / i3 / i3More
# transitively require, the xcb-util collection, and xkeyboard-config.
#
# The X.Org server itself + input drivers + modesetting come in
# Round 8c (10-xorg-server.sh).
#
# Run AFTER ./08-base-substrate.sh completes (the .done-blfs-dbus
# sentinel is the gate).
#
# Build order matters per layer:
#   Layer 1 — protocol headers (no deps beyond Phase 8a)
#   Layer 2 — core libs:         libXau, xtrans, libxcb (needs xcb-proto), libX11
#   Layer 3 — extensions:        each is small; libXft needs libXrender + freetype;
#                                libXcursor needs libXfixes + libXrender
#   Layer 4 — xcb-util:          all depend on libxcb; xcb-util-cursor needs more
#   Layer 5 — xkeyboard-config:  data only, no shared libs

set -uo pipefail

cd "$( dirname "${BASH_SOURCE[0]}" )"
# shellcheck disable=SC1091
source ./setup-env.sh
# shellcheck disable=SC1091
source ./blfs-pkg.sh

# Sanity: Phase 8a complete?
[[ -f "$LOGS/.done-blfs-dbus" ]] || {
    echo "Phase 8a (base substrate) not complete. Run ./08-base-substrate.sh first." >&2
    exit 1
}

# ============================================================================
# Layer 1 — protocol headers
# ============================================================================

step_xorgproto() {
    build_pkg xorgproto "xorgproto-${XORGPROTO_VERSION}.tar.xz"
}

step_xcb-proto() {
    # xcb-proto generates code at install time; needs python3 + libxml2.
    build_pkg xcb-proto "xcb-proto-${XCB_PROTO_VERSION}.tar.xz"
}

# ============================================================================
# Layer 2 — core libs
# ============================================================================

step_libXau() {
    build_pkg libXau "libXau-${LIBXAU_VERSION}.tar.xz"
}

step_xtrans() {
    # xtrans installs C headers + autoconf macros only — no compiled library.
    build_pkg xtrans "xtrans-${XTRANS_VERSION}.tar.xz"
}

step_libxcb() {
    # libxcb reads xcb-proto's XML files via Python codegen during build.
    build_pkg libxcb "libxcb-${LIBXCB_VERSION}.tar.xz" \
        --without-doxygen \
        --enable-xinput
}

step_libX11() {
    # libX11 is Xlib — the classic protocol library. Modern apps prefer
    # XCB, but a lot of legacy code still uses Xlib.
    # --disable-malloc0returnsnull: skip the AC_RUN_IFELSE in
    # XORG_CHECK_MALLOC_ZERO (can't run cross-compiled binary). glibc
    # malloc(0) returns non-NULL, matching the --disable answer.
    build_pkg libX11 "libX11-${LIBX11_VERSION}.tar.xz" \
        --disable-malloc0returnsnull \
        --disable-thread-safety-constructor
}

# ============================================================================
# Layer 3 — extension libraries
# ============================================================================

# --disable-malloc0returnsnull / --disable-malloc0returnssuccess:
# xorg-macros's XORG_CHECK_MALLOC_ZERO runs an AC_RUN_IFELSE that
# can't execute under cross-compile. Disable forces "malloc(0) is non-NULL"
# (glibc behaviour), skipping the runtime check.
_XORG_CROSS="--disable-malloc0returnsnull"
step_libXext()        { build_pkg libXext        "libXext-${LIBXEXT_VERSION}.tar.xz"             $_XORG_CROSS; }
step_libICE()         { build_pkg libICE         "libICE-${LIBICE_VERSION}.tar.xz"               $_XORG_CROSS; }
step_libSM()          { build_pkg libSM          "libSM-${LIBSM_VERSION}.tar.xz"                 $_XORG_CROSS; }
step_libXfixes()      { build_pkg libXfixes      "libXfixes-${LIBXFIXES_VERSION}.tar.xz"         $_XORG_CROSS; }
step_libXdamage()     { build_pkg libXdamage     "libXdamage-${LIBXDAMAGE_VERSION}.tar.xz"       $_XORG_CROSS; }
step_libXcomposite()  { build_pkg libXcomposite  "libXcomposite-${LIBXCOMPOSITE_VERSION}.tar.xz" $_XORG_CROSS; }
step_libXrender()     { build_pkg libXrender     "libXrender-${LIBXRENDER_VERSION}.tar.xz"       $_XORG_CROSS; }
step_libXcursor()     { build_pkg libXcursor     "libXcursor-${LIBXCURSOR_VERSION}.tar.xz"       $_XORG_CROSS; }
step_libXft()         { build_pkg libXft         "libXft-${LIBXFT_VERSION}.tar.xz"               $_XORG_CROSS; }
step_libXrandr()      { build_pkg libXrandr      "libXrandr-${LIBXRANDR_VERSION}.tar.xz"         $_XORG_CROSS; }
step_libXinerama()    { build_pkg libXinerama    "libXinerama-${LIBXINERAMA_VERSION}.tar.xz"     $_XORG_CROSS; }
step_libXi()          { build_pkg libXi          "libXi-${LIBXI_VERSION}.tar.xz"                 $_XORG_CROSS; }
step_libXtst()        { build_pkg libXtst        "libXtst-${LIBXTST_VERSION}.tar.xz"             $_XORG_CROSS; }

step_libxkbcommon() {
    # Modern keyboard handling; meson-only. Disable Wayland support since
    # we don't ship Wayland (the X11 + i3More stack uses Xlib/XCB).
    # -Denable-tools=false: skip xkbcli demos (interactive-evdev tool
    # references LONG_BIT which requires _XOPEN_SOURCE; we don't need
    # the demos and i3/Xorg use only the lib).
    build_meson libxkbcommon "libxkbcommon-${LIBXKBCOMMON_VERSION}.tar.xz" \
        -Denable-wayland=false \
        -Denable-docs=false \
        -Denable-x11=true \
        -Denable-tools=false
}

# ============================================================================
# Layer 4 — xcb-util collection
# ============================================================================

step_xcb-util()             { build_pkg xcb-util             "xcb-util-${XCB_UTIL_VERSION}.tar.xz"; }
step_xcb-util-image()       { build_pkg xcb-util-image       "xcb-util-image-${XCB_UTIL_IMAGE_VERSION}.tar.xz"; }
step_xcb-util-keysyms()     { build_pkg xcb-util-keysyms     "xcb-util-keysyms-${XCB_UTIL_KEYSYMS_VERSION}.tar.xz"; }
step_xcb-util-wm()          { build_pkg xcb-util-wm          "xcb-util-wm-${XCB_UTIL_WM_VERSION}.tar.xz"; }
step_xcb-util-renderutil()  { build_pkg xcb-util-renderutil  "xcb-util-renderutil-${XCB_UTIL_RENDERUTIL_VERSION}.tar.xz"; }
step_xcb-util-cursor()      { build_pkg xcb-util-cursor      "xcb-util-cursor-${XCB_UTIL_CURSOR_VERSION}.tar.xz"; }

# ============================================================================
# Layer 5 — keymap data
# ============================================================================

step_xkeyboard-config() {
    # XML keymaps consumed by xkbcommon + Xorg. Pure data, no library.
    # Uses meson.
    build_meson xkeyboard-config "xkeyboard-config-${XKEYBOARD_CONFIG_VERSION}.tar.xz" \
        -Dxkb-base=/usr/share/X11/xkb \
        -Dcompat-rules=true
}

# ============================================================================
# Driver
# ============================================================================

STEPS=(
    # Layer 1
    xorgproto xcb-proto
    # Layer 2
    libXau xtrans libxcb libX11
    # Layer 3
    libXext libICE libSM
    libXfixes libXdamage libXcomposite
    libXrender libXcursor libXft
    libXrandr libXinerama libXi libXtst
    libxkbcommon
    # Layer 4
    xcb-util xcb-util-image xcb-util-keysyms xcb-util-wm
    xcb-util-renderutil xcb-util-cursor
    # Layer 5
    xkeyboard-config
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
echo "Phase 8b X11 stack: $(count_done_packages) packages built (cumulative)."
echo "Next: ./10-xorg-server.sh (when populated)."
