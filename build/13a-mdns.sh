#!/usr/bin/env bash
# build/13a-mdns.sh — Phase 8f+: mDNS responder (avahi).
#
# Adds zero-config service discovery to the base so the workstation can reach
# the target by name — `ssh writeonce@writeonce.local` — instead of hunting for
# its DHCP address. Two packages:
#
#   libdaemon — avahi's small daemon-helper lib (fork/pidfile/signal plumbing).
#   avahi     — the mDNS/DNS-SD daemon. Built DAEMON-ONLY (the publish side):
#                 --disable-dbus  → no system-bus dependency (runs standalone)
#                 --disable-glib/gobject/qt/gtk/python/mono → no client bindings
#               The T450 only PUBLISHES writeonce.local; the workstation
#               RESOLVES it via its own avahi + nss-mdns. So we need neither dbus
#               nor the avahi-resolve/avahi-browse client tools on-target.
#
# Run AFTER ./13-network-stack.sh. avahi links expat (08-base-substrate) +
# libcap (13-network) + the libdaemon built first here.
#
# The systemd unit (etc/systemd/system/avahi-daemon.service + its .wants link),
# the config (etc/avahi/avahi-daemon.conf), and the avahi user/group ship in the
# skeleton (build/skeleton/...), staged onto this sysroot by 17-stage-sysroot.sh.
#
# Container note: avahi's autotools build wants `intltool`/`gettext`. If
# configure errors on those, add them to build/Containerfile (host-vs-target
# rule — fix the builder, not the target) and rerun.

set -uo pipefail

cd "$( dirname "${BASH_SOURCE[0]}" )"
# shellcheck disable=SC1091
source ./setup-env.sh
# shellcheck disable=SC1091
source ./blfs-pkg.sh

# Sanity: Phase 8f (network stack) complete? avahi links libcap from it.
[[ -f "$LOGS/.done-blfs-dhcpcd" ]] || {
    echo "Phase 8f (network stack) not complete. Run ./13-network-stack.sh first." >&2
    exit 1
}

# ============================================================================
# libdaemon — daemon-helper library (avahi dependency)
# ============================================================================
step_libdaemon() {
    # Plain autotools; no flags beyond build_pkg's defaults are needed.
    build_pkg libdaemon "libdaemon-${LIBDAEMON_VERSION}.tar.gz"
}

# ============================================================================
# avahi — mDNS/DNS-SD responder (daemon-only, publish side)
# ============================================================================
step_avahi() {
    # What remains after the disables is avahi-daemon + libavahi-core/common,
    # which publish the host A/AAAA record over multicast UDP 5353. XML service
    # parsing uses expat (already built). The daemon drops privileges to the
    # avahi:avahi user shipped in the skeleton passwd/group.
    build_pkg avahi "avahi-${AVAHI_VERSION}.tar.gz" \
        --with-distro=none \
        --with-xml=expat \
        --enable-libdaemon \
        --disable-dbus \
        --disable-glib --disable-gobject \
        --disable-qt3 --disable-qt4 --disable-qt5 \
        --disable-gtk --disable-gtk3 \
        --disable-python \
        --disable-mono --disable-monodoc \
        --disable-autoipd \
        --disable-libevent \
        --disable-manpages --disable-xmltoman \
        --disable-tests \
        --disable-nls \
        --with-avahi-user=avahi --with-avahi-group=avahi \
        --with-systemdsystemunitdir=no
}

STEPS=(
    libdaemon
    avahi
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
echo "Phase 8f+ mDNS: avahi-daemon built. It advertises <hostname>.local;"
echo "avahi-daemon.service (shipped in the skeleton) publishes writeonce.local."
