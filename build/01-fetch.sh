#!/usr/bin/env bash
# build/fetch.sh — download, GPG-verify, and SHA-256-verify all upstream sources.
#
# Idempotent: if a file is already present and its SHA-256 matches checksums.txt,
# it is skipped. If checksums.txt lacks an entry, the file is downloaded and
# its hash is written to sources/<basename>.next-lock; the script then refuses
# to proceed until you copy the hash into checksums.txt after spot-checking it.

set -euo pipefail

cd "$( dirname "${BASH_SOURCE[0]}" )"
# shellcheck disable=SC1091
source ./setup-env.sh

CHECKSUMS="$BUILD_ROOT/checksums.txt"
KEYRING="$BUILD_ROOT/gnupg"

# ---- per-package URL table --------------------------------------------------
# Each entry: <tarball-basename> <fetch-url>
#
# Signatures are inferred by appending ".sig" or ".sign" to the URL where the
# upstream provides one; absence is fine.

declare -A URLS=(
    [binutils-${BINUTILS_VERSION}.tar.xz]="https://ftpmirror.gnu.org/binutils/binutils-${BINUTILS_VERSION}.tar.xz"
    [gcc-${GCC_VERSION}.tar.xz]="https://ftpmirror.gnu.org/gcc/gcc-${GCC_VERSION}/gcc-${GCC_VERSION}.tar.xz"
    [glibc-${GLIBC_VERSION}.tar.xz]="https://ftpmirror.gnu.org/glibc/glibc-${GLIBC_VERSION}.tar.xz"
    [mpfr-${MPFR_VERSION}.tar.xz]="https://ftpmirror.gnu.org/mpfr/mpfr-${MPFR_VERSION}.tar.xz"
    [gmp-${GMP_VERSION}.tar.xz]="https://ftpmirror.gnu.org/gmp/gmp-${GMP_VERSION}.tar.xz"
    [mpc-${MPC_VERSION}.tar.gz]="https://ftpmirror.gnu.org/mpc/mpc-${MPC_VERSION}.tar.gz"
    [isl-${ISL_VERSION}.tar.xz]="https://libisl.sourceforge.io/isl-${ISL_VERSION}.tar.xz"

    [linux-${LINUX_VERSION}.tar.xz]="https://cdn.kernel.org/pub/linux/kernel/v${LINUX_MAJOR}/linux-${LINUX_VERSION}.tar.xz"

    # Intel 7265 wifi firmware — both stepping variants. The T450 ships either
    # the original 7265 or the D-step revision; we fetch both since they're
    # ~1 MiB each. linux-firmware reorganised in 2024 — files live under
    # intel/iwlwifi/ now, no longer at repo root.
    [iwlwifi-7265-17.ucode]="https://git.kernel.org/pub/scm/linux/kernel/git/firmware/linux-firmware.git/plain/intel/iwlwifi/iwlwifi-7265-17.ucode?id=${LINUX_FIRMWARE_COMMIT}"
    [iwlwifi-7265D-29.ucode]="https://git.kernel.org/pub/scm/linux/kernel/git/firmware/linux-firmware.git/plain/intel/iwlwifi/iwlwifi-7265D-29.ucode?id=${LINUX_FIRMWARE_COMMIT}"

    [m4-${M4_VERSION}.tar.xz]="https://ftpmirror.gnu.org/m4/m4-${M4_VERSION}.tar.xz"
    [ncurses-${NCURSES_VERSION}.tar.gz]="https://invisible-mirror.net/archives/ncurses/ncurses-${NCURSES_VERSION}.tar.gz"
    [bash-${BASH_VERSION}.tar.gz]="https://ftpmirror.gnu.org/bash/bash-${BASH_VERSION}.tar.gz"
    [coreutils-${COREUTILS_VERSION}.tar.xz]="https://ftpmirror.gnu.org/coreutils/coreutils-${COREUTILS_VERSION}.tar.xz"
    [diffutils-${DIFFUTILS_VERSION}.tar.xz]="https://ftpmirror.gnu.org/diffutils/diffutils-${DIFFUTILS_VERSION}.tar.xz"
    [file-${FILE_VERSION}.tar.gz]="https://astron.com/pub/file/file-${FILE_VERSION}.tar.gz"
    [findutils-${FINDUTILS_VERSION}.tar.xz]="https://ftpmirror.gnu.org/findutils/findutils-${FINDUTILS_VERSION}.tar.xz"
    [gawk-${GAWK_VERSION}.tar.xz]="https://ftpmirror.gnu.org/gawk/gawk-${GAWK_VERSION}.tar.xz"
    [grep-${GREP_VERSION}.tar.xz]="https://ftpmirror.gnu.org/grep/grep-${GREP_VERSION}.tar.xz"
    [gzip-${GZIP_VERSION}.tar.xz]="https://ftpmirror.gnu.org/gzip/gzip-${GZIP_VERSION}.tar.xz"
    [make-${MAKE_VERSION}.tar.gz]="https://ftpmirror.gnu.org/make/make-${MAKE_VERSION}.tar.gz"
    [patch-${PATCH_VERSION}.tar.xz]="https://ftpmirror.gnu.org/patch/patch-${PATCH_VERSION}.tar.xz"
    [sed-${SED_VERSION}.tar.xz]="https://ftpmirror.gnu.org/sed/sed-${SED_VERSION}.tar.xz"
    [tar-${TAR_VERSION}.tar.xz]="https://ftpmirror.gnu.org/tar/tar-${TAR_VERSION}.tar.xz"
    [xz-${XZ_VERSION}.tar.xz]="https://github.com/tukaani-project/xz/releases/download/v${XZ_VERSION}/xz-${XZ_VERSION}.tar.xz"

    [busybox-${BUSYBOX_VERSION}.tar.bz2]="https://busybox.net/downloads/busybox-${BUSYBOX_VERSION}.tar.bz2"

    # --- Phase 8 / Round 1 — base substrate ---
    # zlib.net rotates download URLs aggressively (older versions move
    # to /archive/); github madler/zlib release tags are stable.
    [zlib-${ZLIB_VERSION}.tar.xz]="https://github.com/madler/zlib/releases/download/v${ZLIB_VERSION}/zlib-${ZLIB_VERSION}.tar.xz"
    [brotli-${BROTLI_VERSION}.tar.gz]="https://github.com/google/brotli/archive/refs/tags/v${BROTLI_VERSION}.tar.gz"
    [expat-${EXPAT_VERSION}.tar.xz]="https://github.com/libexpat/libexpat/releases/download/R_$(echo ${EXPAT_VERSION}|tr . _)/expat-${EXPAT_VERSION}.tar.xz"
    [libffi-${LIBFFI_VERSION}.tar.gz]="https://github.com/libffi/libffi/releases/download/v${LIBFFI_VERSION}/libffi-${LIBFFI_VERSION}.tar.gz"
    [libxml2-${LIBXML2_VERSION}.tar.xz]="https://download.gnome.org/sources/libxml2/$(echo ${LIBXML2_VERSION}|cut -d. -f1-2)/libxml2-${LIBXML2_VERSION}.tar.xz"
    [util-macros-${UTIL_MACROS_VERSION}.tar.xz]="https://www.x.org/releases/individual/util/util-macros-${UTIL_MACROS_VERSION}.tar.xz"
    [libpng-${LIBPNG_VERSION}.tar.xz]="https://downloads.sourceforge.net/libpng/libpng-${LIBPNG_VERSION}.tar.xz"
    # sourceforge mirror redirects unreliably; use github releases.
    [libjpeg-turbo-${LIBJPEG_TURBO_VERSION}.tar.gz]="https://github.com/libjpeg-turbo/libjpeg-turbo/releases/download/${LIBJPEG_TURBO_VERSION}/libjpeg-turbo-${LIBJPEG_TURBO_VERSION}.tar.gz"
    [freetype-${FREETYPE_VERSION}.tar.xz]="https://downloads.sourceforge.net/freetype/freetype-${FREETYPE_VERSION}.tar.xz"
    [fontconfig-${FONTCONFIG_VERSION}.tar.xz]="https://www.freedesktop.org/software/fontconfig/release/fontconfig-${FONTCONFIG_VERSION}.tar.xz"
    [Linux-PAM-${LINUX_PAM_VERSION}.tar.xz]="https://github.com/linux-pam/linux-pam/releases/download/v${LINUX_PAM_VERSION}/Linux-PAM-${LINUX_PAM_VERSION}.tar.xz"
    [libxcrypt-${LIBXCRYPT_VERSION}.tar.xz]="https://github.com/besser82/libxcrypt/releases/download/v${LIBXCRYPT_VERSION}/libxcrypt-${LIBXCRYPT_VERSION}.tar.xz"
    [dbus-${DBUS_VERSION}.tar.xz]="https://dbus.freedesktop.org/releases/dbus/dbus-${DBUS_VERSION}.tar.xz"
    [sudo-${SUDO_VERSION}.tar.gz]="https://www.sudo.ws/dist/sudo-${SUDO_VERSION}.tar.gz"
    [zstd-${ZSTD_VERSION}.tar.gz]="https://github.com/facebook/zstd/releases/download/v${ZSTD_VERSION}/zstd-${ZSTD_VERSION}.tar.gz"

    # --- Phase 8 / Round 2 — X11 stack ---
    # Layer 1 — protocol headers
    [xorgproto-${XORGPROTO_VERSION}.tar.xz]="https://www.x.org/releases/individual/proto/xorgproto-${XORGPROTO_VERSION}.tar.xz"
    [xcb-proto-${XCB_PROTO_VERSION}.tar.xz]="https://xorg.freedesktop.org/archive/individual/proto/xcb-proto-${XCB_PROTO_VERSION}.tar.xz"
    # Layer 2 — core libs
    [libXau-${LIBXAU_VERSION}.tar.xz]="https://www.x.org/releases/individual/lib/libXau-${LIBXAU_VERSION}.tar.xz"
    [xtrans-${XTRANS_VERSION}.tar.xz]="https://www.x.org/releases/individual/lib/xtrans-${XTRANS_VERSION}.tar.xz"
    [libxcb-${LIBXCB_VERSION}.tar.xz]="https://xorg.freedesktop.org/archive/individual/lib/libxcb-${LIBXCB_VERSION}.tar.xz"
    [libX11-${LIBX11_VERSION}.tar.xz]="https://www.x.org/releases/individual/lib/libX11-${LIBX11_VERSION}.tar.xz"
    # Layer 3 — extension libs
    [libXext-${LIBXEXT_VERSION}.tar.xz]="https://www.x.org/releases/individual/lib/libXext-${LIBXEXT_VERSION}.tar.xz"
    [libICE-${LIBICE_VERSION}.tar.xz]="https://www.x.org/releases/individual/lib/libICE-${LIBICE_VERSION}.tar.xz"
    [libSM-${LIBSM_VERSION}.tar.xz]="https://www.x.org/releases/individual/lib/libSM-${LIBSM_VERSION}.tar.xz"
    [libXfixes-${LIBXFIXES_VERSION}.tar.xz]="https://www.x.org/releases/individual/lib/libXfixes-${LIBXFIXES_VERSION}.tar.xz"
    [libXdamage-${LIBXDAMAGE_VERSION}.tar.xz]="https://www.x.org/releases/individual/lib/libXdamage-${LIBXDAMAGE_VERSION}.tar.xz"
    [libXcomposite-${LIBXCOMPOSITE_VERSION}.tar.xz]="https://www.x.org/releases/individual/lib/libXcomposite-${LIBXCOMPOSITE_VERSION}.tar.xz"
    [libXcursor-${LIBXCURSOR_VERSION}.tar.xz]="https://www.x.org/releases/individual/lib/libXcursor-${LIBXCURSOR_VERSION}.tar.xz"
    [libXrender-${LIBXRENDER_VERSION}.tar.xz]="https://www.x.org/releases/individual/lib/libXrender-${LIBXRENDER_VERSION}.tar.xz"
    [libXft-${LIBXFT_VERSION}.tar.xz]="https://www.x.org/releases/individual/lib/libXft-${LIBXFT_VERSION}.tar.xz"
    [libXrandr-${LIBXRANDR_VERSION}.tar.xz]="https://www.x.org/releases/individual/lib/libXrandr-${LIBXRANDR_VERSION}.tar.xz"
    [libXinerama-${LIBXINERAMA_VERSION}.tar.xz]="https://www.x.org/releases/individual/lib/libXinerama-${LIBXINERAMA_VERSION}.tar.xz"
    [libXi-${LIBXI_VERSION}.tar.xz]="https://www.x.org/releases/individual/lib/libXi-${LIBXI_VERSION}.tar.xz"
    [libXtst-${LIBXTST_VERSION}.tar.xz]="https://www.x.org/releases/individual/lib/libXtst-${LIBXTST_VERSION}.tar.xz"
    [libxkbcommon-${LIBXKBCOMMON_VERSION}.tar.xz]="https://xkbcommon.org/download/libxkbcommon-${LIBXKBCOMMON_VERSION}.tar.xz"
    # Layer 4 — xcb-util collection
    [xcb-util-${XCB_UTIL_VERSION}.tar.xz]="https://xorg.freedesktop.org/archive/individual/xcb/xcb-util-${XCB_UTIL_VERSION}.tar.xz"
    [xcb-util-image-${XCB_UTIL_IMAGE_VERSION}.tar.xz]="https://xorg.freedesktop.org/archive/individual/xcb/xcb-util-image-${XCB_UTIL_IMAGE_VERSION}.tar.xz"
    [xcb-util-keysyms-${XCB_UTIL_KEYSYMS_VERSION}.tar.xz]="https://xorg.freedesktop.org/archive/individual/xcb/xcb-util-keysyms-${XCB_UTIL_KEYSYMS_VERSION}.tar.xz"
    [xcb-util-wm-${XCB_UTIL_WM_VERSION}.tar.xz]="https://xorg.freedesktop.org/archive/individual/xcb/xcb-util-wm-${XCB_UTIL_WM_VERSION}.tar.xz"
    [xcb-util-renderutil-${XCB_UTIL_RENDERUTIL_VERSION}.tar.xz]="https://xorg.freedesktop.org/archive/individual/xcb/xcb-util-renderutil-${XCB_UTIL_RENDERUTIL_VERSION}.tar.xz"
    [xcb-util-cursor-${XCB_UTIL_CURSOR_VERSION}.tar.xz]="https://xorg.freedesktop.org/archive/individual/xcb/xcb-util-cursor-${XCB_UTIL_CURSOR_VERSION}.tar.xz"
    # Layer 5 — keymap data
    [xkeyboard-config-${XKEYBOARD_CONFIG_VERSION}.tar.xz]="https://www.x.org/releases/individual/data/xkeyboard-config/xkeyboard-config-${XKEYBOARD_CONFIG_VERSION}.tar.xz"

    # --- Phase 8 / Round 3 — xorg-server + drivers ---
    [eudev-${EUDEV_VERSION}.tar.gz]="https://github.com/eudev-project/eudev/releases/download/v${EUDEV_VERSION}/eudev-${EUDEV_VERSION}.tar.gz"
    [libdrm-${LIBDRM_VERSION}.tar.xz]="https://dri.freedesktop.org/libdrm/libdrm-${LIBDRM_VERSION}.tar.xz"
    [libpciaccess-${LIBPCIACCESS_VERSION}.tar.xz]="https://www.x.org/releases/individual/lib/libpciaccess-${LIBPCIACCESS_VERSION}.tar.xz"
    [libXfont2-${LIBXFONT2_VERSION}.tar.xz]="https://www.x.org/releases/individual/lib/libXfont2-${LIBXFONT2_VERSION}.tar.xz"
    [libfontenc-${LIBFONTENC_VERSION}.tar.xz]="https://www.x.org/releases/individual/lib/libfontenc-${LIBFONTENC_VERSION}.tar.xz"
    [libxshmfence-${LIBXSHMFENCE_VERSION}.tar.xz]="https://www.x.org/releases/individual/lib/libxshmfence-${LIBXSHMFENCE_VERSION}.tar.xz"
    [libXxf86vm-${LIBXXF86VM_VERSION}.tar.xz]="https://www.x.org/releases/individual/lib/libXxf86vm-${LIBXXF86VM_VERSION}.tar.xz"
    # libepoxy stopped publishing release tarballs after moving to
    # gitlab; fall back to the github archive of the git tag.
    [libepoxy-${LIBEPOXY_VERSION}.tar.gz]="https://github.com/anholt/libepoxy/archive/refs/tags/${LIBEPOXY_VERSION}.tar.gz"
    [pixman-${PIXMAN_VERSION}.tar.gz]="https://cairographics.org/releases/pixman-${PIXMAN_VERSION}.tar.gz"
    [libxkbfile-${LIBXKBFILE_VERSION}.tar.xz]="https://www.x.org/releases/individual/lib/libxkbfile-${LIBXKBFILE_VERSION}.tar.xz"
    [font-util-${FONT_UTIL_VERSION}.tar.xz]="https://www.x.org/releases/individual/font/font-util-${FONT_UTIL_VERSION}.tar.xz"
    [libxcvt-${LIBXCVT_VERSION}.tar.xz]="https://www.x.org/releases/individual/lib/libxcvt-${LIBXCVT_VERSION}.tar.xz"
    [libmd-${LIBMD_VERSION}.tar.xz]="https://archive.hadrons.org/software/libmd/libmd-${LIBMD_VERSION}.tar.xz"
    [libXdmcp-${LIBXDMCP_VERSION}.tar.xz]="https://www.x.org/releases/individual/lib/libXdmcp-${LIBXDMCP_VERSION}.tar.xz"
    [pcre2-${PCRE2_VERSION}.tar.bz2]="https://github.com/PCRE2Project/pcre2/releases/download/pcre2-${PCRE2_VERSION}/pcre2-${PCRE2_VERSION}.tar.bz2"
    [fribidi-${FRIBIDI_VERSION}.tar.xz]="https://github.com/fribidi/fribidi/releases/download/v${FRIBIDI_VERSION}/fribidi-${FRIBIDI_VERSION}.tar.xz"
    [tiff-${LIBTIFF_VERSION}.tar.xz]="https://download.osgeo.org/libtiff/tiff-${LIBTIFF_VERSION}.tar.xz"
    [libevdev-${LIBEVDEV_VERSION}.tar.xz]="https://www.freedesktop.org/software/libevdev/libevdev-${LIBEVDEV_VERSION}.tar.xz"
    [mtdev-${MTDEV_VERSION}.tar.bz2]="https://bitmath.org/code/mtdev/mtdev-${MTDEV_VERSION}.tar.bz2"
    # GitLab redirects .tar.xz to an auth page; .tar.gz works directly.
    [libinput-${LIBINPUT_VERSION}.tar.gz]="https://gitlab.freedesktop.org/libinput/libinput/-/archive/${LIBINPUT_VERSION}/libinput-${LIBINPUT_VERSION}.tar.gz"
    [mesa-${MESA_VERSION}.tar.xz]="https://archive.mesa3d.org/mesa-${MESA_VERSION}.tar.xz"
    [xorg-server-${XORG_SERVER_VERSION}.tar.xz]="https://www.x.org/releases/individual/xserver/xorg-server-${XORG_SERVER_VERSION}.tar.xz"
    [xf86-input-libinput-${XF86_INPUT_LIBINPUT_VERSION}.tar.xz]="https://www.x.org/releases/individual/driver/xf86-input-libinput-${XF86_INPUT_LIBINPUT_VERSION}.tar.xz"
    [xinit-${XINIT_VERSION}.tar.xz]="https://www.x.org/releases/individual/app/xinit-${XINIT_VERSION}.tar.xz"

    # --- Phase 8 / Round 4 — GTK4 stack ---
    [glib-${GLIB_VERSION}.tar.xz]="https://download.gnome.org/sources/glib/$(echo ${GLIB_VERSION}|cut -d. -f1-2)/glib-${GLIB_VERSION}.tar.xz"
    [gobject-introspection-${GOBJECT_INTROSPECTION_VERSION}.tar.xz]="https://download.gnome.org/sources/gobject-introspection/$(echo ${GOBJECT_INTROSPECTION_VERSION}|cut -d. -f1-2)/gobject-introspection-${GOBJECT_INTROSPECTION_VERSION}.tar.xz"
    [harfbuzz-${HARFBUZZ_VERSION}.tar.xz]="https://github.com/harfbuzz/harfbuzz/releases/download/${HARFBUZZ_VERSION}/harfbuzz-${HARFBUZZ_VERSION}.tar.xz"
    [cairo-${CAIRO_VERSION}.tar.xz]="https://cairographics.org/releases/cairo-${CAIRO_VERSION}.tar.xz"
    [pango-${PANGO_VERSION}.tar.xz]="https://download.gnome.org/sources/pango/$(echo ${PANGO_VERSION}|cut -d. -f1-2)/pango-${PANGO_VERSION}.tar.xz"
    [gdk-pixbuf-${GDK_PIXBUF_VERSION}.tar.xz]="https://download.gnome.org/sources/gdk-pixbuf/$(echo ${GDK_PIXBUF_VERSION}|cut -d. -f1-2)/gdk-pixbuf-${GDK_PIXBUF_VERSION}.tar.xz"
    [graphene-${GRAPHENE_VERSION}.tar.xz]="https://download.gnome.org/sources/graphene/$(echo ${GRAPHENE_VERSION}|cut -d. -f1-2)/graphene-${GRAPHENE_VERSION}.tar.xz"
    # GitLab archive endpoint returns an HTML login page for the .tar.xz
    # form (auth flow / rate limit); .tar.gz is served correctly.
    [shared-mime-info-${SHARED_MIME_INFO_VERSION}.tar.gz]="https://gitlab.freedesktop.org/xdg/shared-mime-info/-/archive/${SHARED_MIME_INFO_VERSION}/shared-mime-info-${SHARED_MIME_INFO_VERSION}.tar.gz"
    [hicolor-icon-theme-${HICOLOR_ICON_THEME_VERSION}.tar.xz]="https://icon-theme.freedesktop.org/releases/hicolor-icon-theme-${HICOLOR_ICON_THEME_VERSION}.tar.xz"
    # adwaita-icon-theme uses single-major-component path under
    # download.gnome.org (47, not 47.0) — unlike glib/gtk (2.82, 4.16).
    [adwaita-icon-theme-${ADWAITA_ICON_THEME_VERSION}.tar.xz]="https://download.gnome.org/sources/adwaita-icon-theme/$(echo ${ADWAITA_ICON_THEME_VERSION}|cut -d. -f1)/adwaita-icon-theme-${ADWAITA_ICON_THEME_VERSION}.tar.xz"
    [gtk-${GTK4_VERSION}.tar.xz]="https://download.gnome.org/sources/gtk/$(echo ${GTK4_VERSION}|cut -d. -f1-2)/gtk-${GTK4_VERSION}.tar.xz"

    # --- Phase 8 / Round 5 — audio stack ---
    [lua-${LUA_VERSION}.tar.gz]="https://www.lua.org/ftp/lua-${LUA_VERSION}.tar.gz"
    [alsa-lib-${ALSA_LIB_VERSION}.tar.bz2]="https://www.alsa-project.org/files/pub/lib/alsa-lib-${ALSA_LIB_VERSION}.tar.bz2"
    [pipewire-${PIPEWIRE_VERSION}.tar.gz]="https://gitlab.freedesktop.org/pipewire/pipewire/-/archive/${PIPEWIRE_VERSION}/pipewire-${PIPEWIRE_VERSION}.tar.gz"
    [wireplumber-${WIREPLUMBER_VERSION}.tar.gz]="https://gitlab.freedesktop.org/pipewire/wireplumber/-/archive/${WIREPLUMBER_VERSION}/wireplumber-${WIREPLUMBER_VERSION}.tar.gz"

    # NOTE: Phase 9 (i3 + i3More) packages are NOT fetched here. i3 and
    # i3More are built externally via i3More's own Dockerfile.i3 +
    # justfile pipeline. 17-stage-sysroot.sh copies their artifacts
    # straight into the WriteOnce sysroot. See versions.env header
    # comment for Phase 9 + docs/learning/phase-9-desktop-bringup.md.

    # --- Phase 8 / Round 6 — network stack ---
    [readline-${READLINE_VERSION}.tar.gz]="https://ftp.gnu.org/gnu/readline/readline-${READLINE_VERSION}.tar.gz"
    [libcap-${LIBCAP_VERSION}.tar.xz]="https://mirrors.edge.kernel.org/pub/linux/libs/security/linux-privs/libcap2/libcap-${LIBCAP_VERSION}.tar.xz"
    [ell-${ELL_VERSION}.tar.xz]="https://mirrors.edge.kernel.org/pub/linux/libs/ell/ell-${ELL_VERSION}.tar.xz"
    [iwd-${IWD_VERSION}.tar.xz]="https://mirrors.edge.kernel.org/pub/linux/network/wireless/iwd-${IWD_VERSION}.tar.xz"
    [iproute2-${IPROUTE2_VERSION}.tar.xz]="https://mirrors.edge.kernel.org/pub/linux/utils/net/iproute2/iproute2-${IPROUTE2_VERSION}.tar.xz"
    [iputils-${IPUTILS_VERSION}.tar.gz]="https://github.com/iputils/iputils/archive/refs/tags/${IPUTILS_VERSION}.tar.gz"
    [dhcpcd-${DHCPCD_VERSION}.tar.xz]="https://github.com/NetworkConfiguration/dhcpcd/releases/download/v${DHCPCD_VERSION}/dhcpcd-${DHCPCD_VERSION}.tar.xz"
)

# Packages whose upstreams publish a detached GPG signature alongside the tarball.
# (For the others, only the SHA-256 is verified; the hash is the bridge to
#  trust, established once by a human against the upstream announcement.)
GPG_SIGNED=(
    binutils gcc glibc mpfr gmp mpc
    linux
    m4 bash coreutils diffutils findutils gawk grep gzip make sed tar xz
    # Phase 8 base substrate — most freedesktop / X.Org / GNOME projects
    # publish detached signatures, though the URL extension varies (.sig
    # vs .asc). The fetch loop below tries both.
    expat libpng libjpeg-turbo freetype fontconfig libxml2 dbus
    Linux   # matches "Linux-PAM-*" via the basename prefix
    # Phase 8 round 6 — kernel.org-hosted network stack pieces are signed.
    # iputils (github) + dhcpcd (github) are SHA-only.
    ell iwd iproute2
)

# ---- import keys from build/keys/ into project-local keyring ----------------
mkdir -p "$KEYRING"; chmod 700 "$KEYRING"
if compgen -G "$BUILD_ROOT/keys/*.asc" >/dev/null; then
    for k in "$BUILD_ROOT/keys/"*.asc; do
        GNUPGHOME="$KEYRING" gpg --quiet --import "$k" 2>/dev/null || true
    done
fi

# ---- per-file flow ----------------------------------------------------------
need_review=0
gpg_failures=()
sha_failures=()

for base in "${!URLS[@]}"; do
    url="${URLS[$base]}"
    out="$SOURCES/$base"

    # 1. download (if missing)
    if [[ ! -f "$out" ]]; then
        echo ">>> fetching $base"
        # --tries=2 + --timeout=15 fail fast on 404 / dead host; with --quiet
        # alone wget retries 20 times with backoff and hangs for minutes.
        if ! wget --tries=2 --timeout=15 --quiet --show-progress \
                  -O "$out.part" "$url"; then
            echo "    ERROR: wget failed for $url" >&2
            rm -f "$out.part"
            continue   # keep going so we surface ALL bad URLs in one run
        fi
        mv "$out.part" "$out"
    fi

    # 2. optional GPG signature
    pkg="${base%%-*}"
    if [[ " ${GPG_SIGNED[*]} " == *" $pkg "* ]]; then
        sig_url=""
        case "$pkg" in
            linux)              sig_url="${url%.xz}.sign" ;;
            busybox)            sig_url="$url.sig" ;;
            *)                  sig_url="$url.sig" ;;
        esac
        sig="$SOURCES/$base.sig"
        if [[ ! -f "$sig" ]]; then
            # Try .sig first; if 404, fall back to .asc (many GNOME /
            # GitHub-release upstreams use .asc instead of .sig).
            if wget --quiet -O "$sig.part" "$sig_url" 2>/dev/null; then
                mv "$sig.part" "$sig"
            elif wget --quiet -O "$sig.part" "${url}.asc" 2>/dev/null; then
                mv "$sig.part" "$sig"
            else
                rm -f "$sig.part"
                echo "    (no signature published for $pkg, skipping GPG)"
                sig=""
            fi
        fi
        if [[ -n "$sig" && -f "$sig" ]]; then
            # The kernel's .sign is over the *decompressed* tarball; everyone
            # else signs the compressed file. Handle both.
            verified=0
            if [[ "$pkg" == "linux" ]]; then
                xz -dc "$out" | GNUPGHOME="$KEYRING" gpg --verify "$sig" - 2>/dev/null \
                    && verified=1
            else
                GNUPGHOME="$KEYRING" gpg --verify "$sig" "$out" 2>/dev/null && verified=1
            fi
            if [[ $verified -eq 1 ]]; then
                echo "    gpg: OK"
            else
                echo "    gpg: FAILED — refusing to trust $base (key probably missing)"
                gpg_failures+=("$base")
                # Do NOT proceed to SHA-256: an unverified file's hash must not
                # be recorded into checksums.txt. Skip to next package.
                continue
            fi
        fi
    fi

    # 3. SHA-256
    actual="$(sha256sum "$out" | awk '{print $1}')"
    # Skip comment lines (which would otherwise return "#" as the hash).
    expected="$(awk -v f="$base" '$1!~/^#/ && $1!="" && $2==f {print $1}' "$CHECKSUMS")"

    if [[ -z "$expected" ]]; then
        echo "    sha256: NO ENTRY in checksums.txt — review required"
        echo "$actual  $base" > "$out.next-lock"
        need_review=1
    elif [[ "$actual" != "$expected" ]]; then
        echo "    sha256: MISMATCH"
        echo "      got      $actual"
        echo "      expected $expected"
        sha_failures+=("$base")
    else
        echo "    sha256: OK"
    fi
done

# --- post-loop: stage firmware blobs into $BUILD_ROOT/firmware/ -------------
# 05-initramfs.sh and 17-stage-sysroot.sh both look for fetched firmware at
# $BUILD_ROOT/firmware/<file>, not under sources/. Copy the .ucode files we
# verified above into that path so the downstream scripts see them.
FW_OUT="$BUILD_ROOT/firmware"
mkdir -p "$FW_OUT"
for fw in "$SOURCES"/iwlwifi-*.ucode; do
    [[ -f "$fw" ]] || continue
    name="$(basename "$fw")"
    if [[ ! -f "$FW_OUT/$name" ]] || ! cmp -s "$fw" "$FW_OUT/$name"; then
        cp "$fw" "$FW_OUT/$name"
        echo "    firmware -> $FW_OUT/$name"
    fi
done

# --- summary ----------------------------------------------------------------
if [[ ${#gpg_failures[@]} -gt 0 ]]; then
    echo
    echo "GPG verification failed for ${#gpg_failures[@]} file(s):"
    for f in "${gpg_failures[@]}"; do echo "  $f"; done
    echo
    echo "Likely cause: the matching public keys aren't in build/keys/ yet."
    echo "Bulk-import every key referenced by sources/*.sig and re-run:"
    echo "    ./import-keys.sh && ./01-fetch.sh"
fi

if [[ ${#sha_failures[@]} -gt 0 ]]; then
    echo
    echo "SHA-256 mismatch for ${#sha_failures[@]} file(s):"
    for f in "${sha_failures[@]}"; do echo "  $f"; done
    echo "Either the tarball was tampered with, or the entry in checksums.txt is wrong."
    echo "Investigate before proceeding. Do NOT silently update checksums.txt."
fi

if [[ $need_review -eq 1 ]]; then
    echo
    echo "Some files lack a SHA-256 entry in checksums.txt."
    echo "Review each sources/<file>.next-lock against the upstream announcement,"
    echo "then merge into checksums.txt:"
    echo
    echo "    cat sources/*.next-lock >> checksums.txt && rm sources/*.next-lock"
fi

# Non-zero exit if anything is unresolved.
if [[ ${#gpg_failures[@]} -gt 0 || ${#sha_failures[@]} -gt 0 ]]; then
    exit 2
elif [[ $need_review -eq 1 ]]; then
    exit 4
fi

echo
echo "All sources present and verified."
