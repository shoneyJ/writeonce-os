#!/usr/bin/env bash
# build/12-audio-stack.sh — Phase 8 round 5: audio stack.
#
# Cross-builds Lua + ALSA library + PipeWire daemon + WirePlumber
# session manager.
#
# Run AFTER ./11-gtk-stack.sh completes.
#
# Build order:
#   lua          — script engine for wireplumber's policy rules
#   alsa-lib     — userspace bindings to the kernel's /dev/snd/* devices
#   pipewire     — modern audio + video server (daemon + libraries)
#   wireplumber  — session/policy manager that drives pipewire
#
# Scoping (vs a "full" pipewire build):
#   - No GStreamer integration (we don't ship GStreamer)
#   - No BlueZ / Bluetooth audio (defer; USB / built-in audio suffices)
#   - No Avahi (no mDNS audio discovery)
#   - No JACK API compatibility (pure ALSA + Pulse-compat is enough)
#   - No Vulkan in pipewire (only audio, no video acceleration)
#   - No libcamera (no webcam path in the developer-workstation MVP)

set -uo pipefail

cd "$( dirname "${BASH_SOURCE[0]}" )"
# shellcheck disable=SC1091
source ./setup-env.sh
# shellcheck disable=SC1091
source ./blfs-pkg.sh

# Sanity: Phase 8d complete?
[[ -f "$LOGS/.done-blfs-gtk4" ]] || {
    echo "Phase 8d (GTK4 stack) not complete. Run ./11-gtk-stack.sh first." >&2
    exit 1
}

# ============================================================================
# Lua — wireplumber's policy/scripting layer
# ============================================================================

step_lua() {
    # Lua's upstream uses a hand-rolled Makefile rather than autotools.
    # No --host / --build options. Cross-compile via CC/AR/RANLIB env
    # overrides and the "linux" Makefile target.
    local name=lua sentinel="$LOGS/.done-blfs-$name"
    if [[ -f "$sentinel" ]]; then
        echo "skip $name (already built)"
        return 0
    fi
    echo
    echo "============================================================"
    echo " blfs: $name"
    echo "============================================================"
    rm -rf "$BUILD_ROOT/work/$name"; mkdir -p "$BUILD_ROOT/work/$name"
    tar -xf "$SOURCES/lua-${LUA_VERSION}.tar.gz" \
        -C "$BUILD_ROOT/work/$name" --strip-components=1
    pushd "$BUILD_ROOT/work/$name" >/dev/null
        make \
            CC="$LFS/tools/bin/${LFS_TGT}-gcc" \
            AR="$LFS/tools/bin/${LFS_TGT}-ar rcu" \
            RANLIB="$LFS/tools/bin/${LFS_TGT}-ranlib" \
            MYCFLAGS="-fPIC" \
            linux \
            2>&1 | tee "$LOGS/blfs-$name-make.log"
        make \
            INSTALL_TOP="$LFS/usr" \
            INSTALL_MAN="$LFS/usr/share/man" \
            install \
            2>&1 | tee "$LOGS/blfs-$name-install.log"

        # Lua doesn't ship a pkg-config file; downstream consumers
        # (wireplumber, others) look for `lua5.4` via pkg-config. Write
        # one in place pointing at the freshly-installed Lua.
        mkdir -p "$LFS/usr/lib/pkgconfig"
        cat > "$LFS/usr/lib/pkgconfig/lua5.4.pc" <<EOF
prefix=/usr
exec_prefix=\${prefix}
libdir=\${exec_prefix}/lib
includedir=\${prefix}/include

Name: Lua
Description: An Extensible Extension Language
Version: ${LUA_VERSION}
Libs: -L\${libdir} -llua -lm
Libs.private: -ldl
Cflags: -I\${includedir}
EOF
        # Also expose it under the un-versioned name some packages probe for.
        ln -sf lua5.4.pc "$LFS/usr/lib/pkgconfig/lua.pc"
    popd >/dev/null
    touch "$sentinel"
    echo "<<< $name done"
}

# ============================================================================
# ALSA — kernel-side audio bindings
# ============================================================================

step_alsa-lib() {
    # Userspace library that wraps the kernel's /dev/snd/pcm* + /dev/snd/control*
    # interfaces. Every audio-producing/consuming app on Linux either talks
    # to this directly or via something (pipewire) that does.
    build_pkg alsa-lib "alsa-lib-${ALSA_LIB_VERSION}.tar.bz2" \
        --disable-static \
        --disable-python
}

# ============================================================================
# PipeWire — the modern audio + video server
# ============================================================================

step_pipewire() {
    # PipeWire replaces PulseAudio + JACK with a unified server. We
    # enable just the audio path; video/screen-capture/bluez/avahi are
    # all explicitly disabled to keep the build sane.
    build_meson pipewire "pipewire-${PIPEWIRE_VERSION}.tar.gz" \
        -Dexamples=disabled \
        -Dman=disabled \
        -Dtests=disabled \
        -Dgstreamer=disabled \
        -Dbluez5=disabled \
        -Dvulkan=disabled \
        -Dx11=disabled \
        -Dx11-xfixes=disabled \
        -Dlibcamera=disabled \
        -Decho-cancel-webrtc=disabled \
        -Daudiotestsrc=disabled \
        -Dvideotestsrc=disabled \
        -Davahi=disabled \
        -Djack=disabled \
        -Droc=disabled \
        -Dsndfile=disabled \
        -Dsession-managers= \
        -Dlegacy-rtkit=false \
        -Dpipewire-alsa=enabled \
        -Dpipewire-jack=disabled \
        -Dudev=enabled
}

# ============================================================================
# WirePlumber — session/policy manager
# ============================================================================

step_wireplumber() {
    # WirePlumber drives the PipeWire daemon: device hotplug routing,
    # default-sink/source policy, application stream rules. Policy is
    # expressed in Lua scripts shipped under /usr/share/wireplumber/.
    build_meson wireplumber "wireplumber-${WIREPLUMBER_VERSION}.tar.gz" \
        -Ddocumentation=disabled \
        -Dintrospection=disabled \
        -Dtests=false \
        -Dsystem-lua=true \
        -Delogind=disabled \
        -Dsystemd=disabled \
        -Dsystemd-system-service=false \
        -Dsystemd-user-service=false
}

# ============================================================================
# Driver
# ============================================================================

STEPS=(
    lua
    alsa-lib
    pipewire
    wireplumber
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
echo "Phase 8e audio stack: $(count_done_packages) packages built (cumulative)."
echo "Next: ./13-network-stack.sh (when populated) — iproute2 + iwd."
