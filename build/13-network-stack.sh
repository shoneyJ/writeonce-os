#!/usr/bin/env bash
# build/13-network-stack.sh — Phase 8 round 6: network stack.
#
# Cross-builds the userspace networking pieces:
#   ell       — Intel's "Embedded Linux Library" (iwd dep)
#   iwd       — wireless daemon (replaces wpa_supplicant; smaller, no OpenSSL)
#   iproute2  — ip / tc / ss / bridge — modern net config tools
#   iputils   — ping / traceroute / arping / tracepath — diagnostics
#   dhcpcd    — DHCP client for the ethernet interface
#
# Run AFTER ./12-audio-stack.sh completes.
#
# Build order matters:
#   ell must come before iwd (iwd links libell).
#   iproute2, iputils, dhcpcd are independent and can come in any order
#   after ell; we keep them in functional groupings.
#
# Scope notes:
#   - No wpa_supplicant: replaced by iwd.
#   - No NetworkManager / systemd-networkd: writeonce-svc orchestrates
#     iwd + dhcpcd directly via service units in Phase 9.
#   - No openvpn / wireguard userspace: kernel WireGuard is in-tree;
#     userspace (wg-tools) can come from nixpkgs if needed.
#   - No bluez (Bluetooth) — separate future round.

set -uo pipefail

cd "$( dirname "${BASH_SOURCE[0]}" )"
# shellcheck disable=SC1091
source ./setup-env.sh
# shellcheck disable=SC1091
source ./blfs-pkg.sh

# Sanity: Phase 8e complete?
[[ -f "$LOGS/.done-blfs-wireplumber" ]] || {
    echo "Phase 8e (audio stack) not complete. Run ./12-audio-stack.sh first." >&2
    exit 1
}

# ============================================================================
# ell — Intel's Embedded Linux Library
# ============================================================================

step_libcap() {
    # POSIX capabilities (libcap.so + setcap/getcap binaries). iputils
    # links libcap for CAP_NET_RAW so ping can run unprivileged. Uses a
    # hand-rolled Makefile, not autotools; cross via env overrides.
    local name=libcap
    local sentinel="$LOGS/.done-blfs-$name"
    if [[ -f "$sentinel" ]]; then echo "skip $name"; return 0; fi
    echo; echo "==== blfs: $name ===="
    rm -rf "$BUILD_ROOT/work/$name"; mkdir -p "$BUILD_ROOT/work/$name"
    tar -xf "$SOURCES/libcap-${LIBCAP_VERSION}.tar.xz" \
        -C "$BUILD_ROOT/work/$name" --strip-components=1
    pushd "$BUILD_ROOT/work/$name" >/dev/null
        # GOLANG=no disables Go bindings (we don't have a Go toolchain).
        # BUILD_CC must be the HOST cc (not the cross gcc) because the
        # build step compiles a small `_makenames` helper that runs on
        # the build machine to generate cap_names.h.
        make CC="$LFS/tools/bin/${LFS_TGT}-gcc" \
             AR="$LFS/tools/bin/${LFS_TGT}-ar" \
             RANLIB="$LFS/tools/bin/${LFS_TGT}-ranlib" \
             BUILD_CC=/usr/bin/cc                       \
             GOLANG=no                                   \
             SHARED=yes                                  \
             -j"$(nproc)" \
            2>&1 | tee "$LOGS/blfs-$name-make.log" && \
        make DESTDIR="$LFS"                             \
             prefix=/usr                                 \
             lib=lib                                     \
             RAISE_SETFCAP=no                            \
             GOLANG=no                                   \
             install                                     \
            2>&1 | tee "$LOGS/blfs-$name-install.log" \
            || { popd >/dev/null; echo "ERROR: $name failed" >&2; return 1; }
    popd >/dev/null
    find "$LFS/usr/lib" -name '*.la' -delete 2>/dev/null
    touch "$sentinel"
}

step_readline() {
    # GNU readline — line editing + history. Required by iwd's iwctl CLI.
    # Two cross-build subtleties live here:
    #
    # 1. Hand-rolled ncurses.pc: LFS Ch.6 built ncurses without
    #    --enable-pc-files, so readline's auto-generated readline.pc
    #    lists `Requires.private: ncurses` and downstream
    #    `pkg-config --exists readline` fails (which is what iwd does,
    #    not direct linking). Six lines of static .pc + ncursesw / tinfo
    #    symlinks resolve it.
    #
    # 2. SHLIB_LIBS=-lncursesw override at install time: readline 8.2's
    #    shared-lib Makefile leaves SHLIB_LIBS empty even when configure
    #    detected the termcap functions in -lncurses. Without the
    #    override, libreadline.so has no DT_NEEDED on libncursesw, and
    #    consumers (iwctl) fail to link with undefined tgetent/tputs/etc.
    #    The LFS recipe for readline 8.2 patches this the same way.
    local name=readline
    local sentinel="$LOGS/.done-blfs-$name"
    if [[ -f "$sentinel" ]]; then echo "skip $name"; return 0; fi

    if [[ ! -f "$LFS/usr/lib/pkgconfig/ncurses.pc" ]]; then
        install -dDm755 "$LFS/usr/lib/pkgconfig"
        cat > "$LFS/usr/lib/pkgconfig/ncurses.pc" <<'PC'
prefix=/usr
exec_prefix=${prefix}
libdir=${exec_prefix}/lib
includedir=${prefix}/include

Name: ncurses
Description: ncurses 6.x wide-character terminal library
Version: 6.5
Libs: -L${libdir} -lncursesw
Cflags: -I${includedir} -D_XOPEN_SOURCE_EXTENDED
PC
        ln -sf ncurses.pc "$LFS/usr/lib/pkgconfig/ncursesw.pc"
        ln -sf ncurses.pc "$LFS/usr/lib/pkgconfig/tinfo.pc"
    fi

    echo; echo "==== blfs: $name ===="
    rm -rf "$BUILD_ROOT/work/$name"; mkdir -p "$BUILD_ROOT/work/$name"
    tar -xf "$SOURCES/readline-${READLINE_VERSION}.tar.gz" \
        -C "$BUILD_ROOT/work/$name" --strip-components=1
    pushd "$BUILD_ROOT/work/$name" >/dev/null
        ./configure                                       \
            --prefix=/usr                                 \
            --host="$LFS_TGT"                             \
            --build="$(./support/config.guess)"           \
            --disable-static                              \
            --with-curses                                 \
            bash_cv_termcap_lib=libncursesw               \
            2>&1 | tee "$LOGS/blfs-$name-configure.log" && \
        make -j"$(nproc)" SHLIB_LIBS="-lncursesw"          \
            2>&1 | tee "$LOGS/blfs-$name-make.log" && \
        make DESTDIR="$LFS" SHLIB_LIBS="-lncursesw" install \
            2>&1 | tee "$LOGS/blfs-$name-install.log" \
            || { popd >/dev/null; echo "ERROR: $name failed" >&2; return 1; }
    popd >/dev/null
    find "$LFS/usr/lib" -name '*.la' -delete 2>/dev/null
    touch "$sentinel"
}

step_ell() {
    # Async event loop + crypto + minimal D-Bus client. iwd is built on it.
    # Autoconf with --disable-glib (we don't want to link glib here).
    build_pkg ell "ell-${ELL_VERSION}.tar.xz" \
        --disable-glib
}

# ============================================================================
# iwd — iNet Wireless Daemon
# ============================================================================

step_iwd() {
    # iwd does WPA / 802.11 association + has a built-in DHCP client for
    # the wireless interface. iwctl is the user-facing CLI; iwmon is the
    # nl80211 packet sniffer for debugging.
    #
    # We disable:
    #   - systemd integration (we don't ship systemd)
    #   - ofono (telephony / SIM card auth)
    #   - sim-auth (EAP-SIM/AKA, cellular-style)
    #   - manual pages (no man infra in sysroot yet)
    #   - dbus-policy install (path conflict with our dbus dir layout;
    #     handled by writeonce-svc unit instead)
    #
    # We enable:
    #   - client (iwctl)
    #   - monitor (iwmon)
    #   - tools (extra debug binaries)
    #   - wired DISABLED — no 802.1X for home Ethernet
    build_pkg iwd "iwd-${IWD_VERSION}.tar.xz" \
        --disable-systemd-service \
        --disable-manual-pages \
        --disable-ofono \
        --disable-sim-auth \
        --disable-wired \
        --enable-client \
        --enable-monitor \
        --enable-tools \
        --localstatedir=/var \
        --sysconfdir=/etc \
        --disable-cmocka
}

# ============================================================================
# iproute2 — `ip`, `tc`, `ss`, `bridge`
# ============================================================================

step_iproute2() {
    # iproute2 uses a hand-rolled Makefile, not autotools. The included
    # configure script is a feature-probe (libbpf? libelf? libcap?) that
    # writes Config. Cross-compile via env overrides.
    local name=iproute2
    local sentinel="$LOGS/.done-blfs-$name"
    if [[ -f "$sentinel" ]]; then
        echo "skip $name (already built)"
        return 0
    fi
    echo
    echo "============================================================"
    echo " blfs: $name"
    echo "============================================================"
    rm -rf "$BUILD_ROOT/work/$name"; mkdir -p "$BUILD_ROOT/work/$name"
    tar -xf "$SOURCES/iproute2-${IPROUTE2_VERSION}.tar.xz" \
        -C "$BUILD_ROOT/work/$name" --strip-components=1
    pushd "$BUILD_ROOT/work/$name" >/dev/null
        # Feature probe. Honours CC.
        CC="$LFS/tools/bin/${LFS_TGT}-gcc" \
        PKG_CONFIG=pkg-config \
            ./configure 2>&1 | tee "$LOGS/blfs-$name-configure.log"

        make -j"$(nproc)" \
            CC="$LFS/tools/bin/${LFS_TGT}-gcc" \
            AR="$LFS/tools/bin/${LFS_TGT}-ar" \
            HOSTCC=cc \
            PKG_CONFIG=pkg-config \
            2>&1 | tee "$LOGS/blfs-$name-make.log"

        make install \
            DESTDIR="$LFS" \
            PREFIX=/usr \
            SBINDIR=/usr/sbin \
            LIBDIR=/usr/lib \
            CONFDIR=/etc/iproute2 \
            DOCDIR=/usr/share/doc/iproute2 \
            MANDIR=/usr/share/man \
            2>&1 | tee "$LOGS/blfs-$name-install.log"
    popd >/dev/null
    touch "$sentinel"
    echo "<<< $name done"
}

# ============================================================================
# iputils — ping / traceroute / arping
# ============================================================================

step_iputils() {
    # iputils switched from autotools to meson around 2019. ping needs
    # CAP_NET_RAW (set as a file capability post-install) or setuid root;
    # we'll handle that in the Phase 10 ISO step, not here.
    build_meson iputils "iputils-${IPUTILS_VERSION}.tar.gz" \
        -DBUILD_HTML_MANS=false \
        -DBUILD_MANS=false \
        -DSKIP_TESTS=true \
        -DNO_SETCAP_OR_SUID=true
}

# ============================================================================
# dhcpcd — DHCP client
# ============================================================================

step_dhcpcd() {
    # dhcpcd has a custom configure script (not autoconf). It honours
    # --prefix, --sysconfdir, --libexecdir; --host is accepted but cross
    # compilation needs CC/AR/RANLIB envs.
    local name=dhcpcd
    local sentinel="$LOGS/.done-blfs-$name"
    if [[ -f "$sentinel" ]]; then
        echo "skip $name (already built)"
        return 0
    fi
    echo
    echo "============================================================"
    echo " blfs: $name"
    echo "============================================================"
    rm -rf "$BUILD_ROOT/work/$name"; mkdir -p "$BUILD_ROOT/work/$name"
    tar -xf "$SOURCES/dhcpcd-${DHCPCD_VERSION}.tar.xz" \
        -C "$BUILD_ROOT/work/$name" --strip-components=1
    pushd "$BUILD_ROOT/work/$name" >/dev/null
        CC="$LFS/tools/bin/${LFS_TGT}-gcc" \
        AR="$LFS/tools/bin/${LFS_TGT}-ar" \
        RANLIB="$LFS/tools/bin/${LFS_TGT}-ranlib" \
            ./configure \
                --prefix=/usr \
                --sysconfdir=/etc \
                --libexecdir=/usr/lib/dhcpcd \
                --dbdir=/var/lib/dhcpcd \
                --rundir=/run \
                --build="$(./config.guess 2>/dev/null || gcc -dumpmachine)" \
                --host="$LFS_TGT" \
                --without-hooks=10-wpa_supplicant \
                --without-hooks=15-timezone \
                2>&1 | tee "$LOGS/blfs-$name-configure.log"

        make -j"$(nproc)" 2>&1 | tee "$LOGS/blfs-$name-make.log"
        make install DESTDIR="$LFS" 2>&1 | tee "$LOGS/blfs-$name-install.log"
    popd >/dev/null
    touch "$sentinel"
    echo "<<< $name done"
}

# ============================================================================
# Driver
# ============================================================================

STEPS=(
    libcap
    readline
    ell
    iwd
    iproute2
    iputils
    dhcpcd
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
echo "Phase 8f network stack: $(count_done_packages) packages built (cumulative)."
echo "Next: Phase 9 — cross-compile i3 + i3More, wire service units, boot the desktop."
