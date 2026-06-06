#!/usr/bin/env bash
# build/16-systemd.sh — systemd (PID 1 + service manager + logind + udev +
# journald + tmpfiles) for the `with-systmed` branch.
#
# Replaces the custom Rust init layer (writeonce-pid1/svc/logind/...). Built
# cross via blfs-pkg.sh's build_meson (the same meson cross-file with
# needs_exe_wrapper=true that already tames Mesa). MINIMAL feature set: enable
# only the core init + logind + udev + the PAM module (pam_systemd, for session
# registration) + kmod/blkid; disable every optional daemon and library dep we
# don't ship. udev + journald are core (always built).
#
# Link deps come from $LFS (built earlier): libcap, libmount/libblkid/libuuid
# (util-linux), libkmod (kmod), libpam (Phase 8a), xz/zstd/zlib.
#
#   ./16-systemd.sh           # build systemd
# Sentinel: logs/.done-blfs-systemd. Logs: logs/blfs-systemd-{setup,compile,install}.log
#
# NOTE: option names + types are pinned to systemd 256.4's meson_options.txt
# (boolean → true/false, feature → enabled/disabled, mode/tests → combo).

set -uo pipefail

cd "$( dirname "${BASH_SOURCE[0]}" )"
# shellcheck disable=SC1091
source ./setup-env.sh
# shellcheck disable=SC1091
source ./blfs-pkg.sh

# Flavor gate: systemd is the init only for FLAVOR_INIT=systemd flavors. The rust
# flavor (writeonce-pid1) builds its init from the Rust crates instead.
[[ "${FLAVOR_INIT:-systemd}" == systemd ]] || {
    echo "16-systemd: skip — FLAVOR_INIT=$FLAVOR_INIT (flavor $FLAVOR uses a non-systemd init)"
    exit 0
}

# systemd's link deps must already be in $LFS.
for need in usr/lib/libcap.so usr/lib/libmount.so usr/lib/libkmod.so usr/lib/libpam.so; do
    [[ -e "$LFS/$need" ]] || echo "warn: $LFS/$need missing — systemd link may fail (build util-linux/libcap/kmod/pam first)" >&2
done

build_meson systemd "systemd-${SYSTEMD_VERSION}.tar.gz" \
    -Dmode=release \
    -Dversion-tag="${SYSTEMD_VERSION}" \
    -Ddefault-hierarchy=unified \
    \
    `# core daemons we KEEP` \
    -Dlogind=true \
    -Dhostnamed=true \
    -Dvconsole=true \
    -Dtmpfiles=true \
    -Dsysusers=true \
    -Dfirstboot=false \
    -Drandomseed=true \
    \
    `# library deps we KEEP (present in $LFS)` \
    -Dpam=enabled \
    -Dkmod=enabled \
    -Dblkid=enabled \
    -Dxz=enabled \
    -Dzstd=enabled \
    -Dzlib=enabled \
    \
    `# daemons/features we DROP (boolean)` \
    -Dnetworkd=false -Dresolve=false -Dtimesyncd=false -Dtimedated=false \
    -Dlocaled=false -Dmachined=false -Dportabled=false -Dnsresourced=false \
    -Dmountfsd=false -Dsysext=false -Duserdb=false -Doomd=false -Dpstore=false \
    -Dcoredump=false -Dbacklight=false -Drfkill=false -Dhibernate=false \
    -Dbinfmt=false -Dquotacheck=false -Dstoragetm=false -Dhwdb=false \
    -Dxdg-autostart=false -Defi=false -Dtpm=false -Denvironment-d=false \
    -Dnss-systemd=false -Dnss-myhostname=false -Dtranslations=false \
    -Dima=false -Dsmack=false -Didn=false \
    \
    `# library deps we DROP (feature; libs not built)` \
    -Dacl=disabled -Dseccomp=disabled -Dselinux=disabled -Dapparmor=disabled \
    -Daudit=disabled -Dpolkit=disabled -Dopenssl=disabled -Dgcrypt=disabled \
    -Dgnutls=disabled -Dp11kit=disabled -Dlibcryptsetup=disabled -Dlibcurl=disabled \
    -Dlibidn2=disabled -Dlibidn=disabled -Dlibiptc=disabled -Dqrencode=disabled \
    -Dmicrohttpd=disabled -Dremote=disabled -Dlibfido2=disabled -Dtpm2=disabled \
    -Delfutils=disabled -Dpwquality=disabled -Dpasswdqc=disabled -Dpcre2=disabled \
    -Dglib=disabled -Ddbus=disabled -Dlibarchive=disabled -Dlz4=disabled \
    -Dbzip2=disabled -Dfdisk=disabled -Dbootloader=disabled -Dukify=disabled \
    -Dhomed=disabled -Dimportd=disabled -Dsysupdate=disabled -Drepart=disabled \
    -Dvmspawn=disabled -Dxkbcommon=disabled -Dnss-mymachines=disabled \
    -Dnss-resolve=disabled \
    \
    `# tests/docs off (combo + feature)` \
    -Dtests=false -Dslow-tests=false -Dinstall-tests=false \
    -Dman=disabled -Dhtml=disabled

echo
echo "systemd built. Key artifacts in \$LFS:"
for a in usr/lib/systemd/systemd usr/bin/systemctl usr/bin/journalctl usr/bin/loginctl \
         usr/lib/systemd/systemd-logind usr/lib/systemd/systemd-udevd usr/lib/security/pam_systemd.so; do
    [[ -e "$LFS/$a" ]] && echo "  ok:   $a" || echo "  MISSING: $a"
done
echo "Next: stage + check-staging."
