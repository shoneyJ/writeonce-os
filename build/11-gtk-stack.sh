#!/usr/bin/env bash
# build/11-gtk-stack.sh — Phase 8 round 4: GTK4 toolkit + dependencies.
#
# Cross-builds glib + the text-rendering chain (harfbuzz → cairo → pango)
# + gdk-pixbuf + graphene + icon themes + GTK4 itself.
#
# Run AFTER ./10-xorg-server.sh completes.
#
# Build order:
#   Foundation:     glib, gobject-introspection
#   Text rendering: harfbuzz → cairo → pango
#   Image loading:  gdk-pixbuf
#   Math:           graphene
#   Runtime data:   shared-mime-info, hicolor-icon-theme, adwaita-icon-theme
#   Toolkit:        gtk4
#
# Scoping notes (vs a "full" GTK4 build):
#   - No glib-networking (no TLS in GIO; we don't have OpenSSL/GnuTLS yet
#     and i3More doesn't need GIO HTTPS)
#   - No librsvg (huge Rust dep; Adwaita ships PNGs alongside SVGs)
#   - No introspection (i3More uses gtk4-rs static FFI, not runtime)
#   - No gtk-doc anywhere
#   - GTK4: x11-backend only, no wayland/vulkan/broadway/gst/cups/sysprof

set -uo pipefail

cd "$( dirname "${BASH_SOURCE[0]}" )"
# shellcheck disable=SC1091
source ./setup-env.sh
# shellcheck disable=SC1091
source ./blfs-pkg.sh

# Sanity: Phase 8c complete?
[[ -f "$LOGS/.done-blfs-xorg-server" ]] || {
    echo "Phase 8c (xorg-server) not complete. Run ./10-xorg-server.sh first." >&2
    exit 1
}

# ============================================================================
# Foundation
# ============================================================================

step_glib() {
    # The GLib platform: types, signals, GIO, GVariant, dbus client lib.
    # Don't enable introspection yet — gobject-introspection needs glib
    # to exist first. We could rebuild glib after gobject-introspection
    # is up, but i3More's gtk4-rs binding stack doesn't need runtime
    # introspection so we skip the second pass.
    build_meson glib "glib-${GLIB_VERSION}.tar.xz" \
        -Dman-pages=disabled \
        -Dtests=false \
        -Ddocumentation=false \
        -Dlibmount=disabled \
        -Dxattr=true \
        -Dselinux=disabled \
        -Dintrospection=disabled \
        -Dinstalled_tests=false \
        -Dsystemtap=disabled \
        -Ddtrace=disabled
}

step_gobject-introspection() {
    # The .gir/.typelib generator. Many GNOME packages probe for it at
    # build time even when not actually used. Cheap to install.
    build_meson gobject-introspection "gobject-introspection-${GOBJECT_INTROSPECTION_VERSION}.tar.xz" \
        -Dbuild_introspection_data=false \
        -Ddoctool=disabled \
        -Dcairo=disabled \
        -Dpython=python3
}

# ============================================================================
# Text rendering chain: harfbuzz → cairo → pango
# ============================================================================

step_harfbuzz() {
    # Modern text shaping (OpenType + complex scripts). Used by pango.
    build_meson harfbuzz "harfbuzz-${HARFBUZZ_VERSION}.tar.xz" \
        -Ddocs=disabled \
        -Dtests=disabled \
        -Dchafa=disabled \
        -Dintrospection=disabled \
        -Dicu=disabled \
        -Dfreetype=enabled \
        -Dgobject=enabled \
        -Dcairo=disabled
}

step_cairo() {
    # 2D vector graphics. Used by pango + gtk4. Build with X11 backends
    # so X server clients can draw via cairo-xlib / cairo-xcb.
    build_meson cairo "cairo-${CAIRO_VERSION}.tar.xz" \
        -Dxlib=enabled \
        -Dxcb=enabled \
        -Dxlib-xcb=enabled \
        -Dpng=enabled \
        -Dfreetype=enabled \
        -Dfontconfig=enabled \
        -Dquartz=disabled \
        -Ddwrite=disabled \
        -Dsymbol-lookup=disabled \
        -Dtests=disabled
}

step_pango() {
    # Text layout. Depends on harfbuzz + cairo + fontconfig + freetype.
    build_meson pango "pango-${PANGO_VERSION}.tar.xz" \
        -Ddocumentation=false \
        -Dintrospection=disabled \
        -Dbuild-testsuite=false \
        -Dbuild-examples=false
}

# ============================================================================
# Image loading + math
# ============================================================================

step_gdk-pixbuf() {
    # Raster image loader. We enable only PNG + JPEG (Phase 8a builds);
    # skip TIFF (no libtiff) and WebP (no libwebp).
    build_meson gdk-pixbuf "gdk-pixbuf-${GDK_PIXBUF_VERSION}.tar.xz" \
        -Drelocatable=false \
        -Dinstalled_tests=false \
        -Dbuiltin_loaders=png,jpeg \
        -Dintrospection=disabled \
        -Dman=false \
        -Dgtk_doc=false \
        -Dgio_sniffing=false \
        -Dtests=false \
        -Dtiff=disabled \
        -Dothers=disabled
}

step_graphene() {
    # SIMD-accelerated math types (matrices, vectors). gtk4 uses it for
    # GskRenderer transforms.
    build_meson graphene "graphene-${GRAPHENE_VERSION}.tar.xz" \
        -Dgtk_doc=false \
        -Dintrospection=disabled \
        -Dtests=false \
        -Dgobject_types=true
}

# ============================================================================
# Runtime data: MIME types + icon themes
# ============================================================================

step_shared-mime-info() {
    # File-type detection by content + extension. Required at runtime by
    # GTK file choosers + many other apps.
    build_meson shared-mime-info "shared-mime-info-${SHARED_MIME_INFO_VERSION}.tar.xz" \
        -Dupdate-mimedb=true
}

step_hicolor-icon-theme() {
    # Spec-mandated fallback icon theme. Pure data, no library; autoconf.
    build_pkg hicolor-icon-theme "hicolor-icon-theme-${HICOLOR_ICON_THEME_VERSION}.tar.xz"
}

step_adwaita-icon-theme() {
    # i3More's icons resolve through Adwaita. Modern Adwaita uses meson
    # and ships both .svg and pre-rendered .png at common sizes — we
    # don't strictly need librsvg at runtime.
    build_meson adwaita-icon-theme "adwaita-icon-theme-${ADWAITA_ICON_THEME_VERSION}.tar.xz"
}

# ============================================================================
# The toolkit
# ============================================================================

step_gtk4() {
    # GTK4 — the toolkit i3More builds against. X11 backend only.
    # Disable every optional integration we don't ship.
    build_meson gtk4 "gtk-${GTK4_VERSION}.tar.xz" \
        -Dx11-backend=true \
        -Dwayland-backend=false \
        -Dbroadway-backend=false \
        -Dvulkan=disabled \
        -Dbuild-tests=false \
        -Dbuild-demos=false \
        -Dbuild-examples=false \
        -Dbuild-testsuite=false \
        -Dintrospection=disabled \
        -Ddocumentation=false \
        -Dman-pages=false \
        -Dgtk_doc=false \
        -Dmedia-gstreamer=disabled \
        -Dmedia-ffmpeg=disabled \
        -Dprint-cups=disabled \
        -Dprint-cpdb=disabled \
        -Dcolord=disabled \
        -Dsysprof=disabled \
        -Dtracker=disabled \
        -Dcloudproviders=disabled \
        -Df16c=disabled
}

# ============================================================================
# Driver
# ============================================================================

STEPS=(
    glib gobject-introspection
    harfbuzz cairo pango
    gdk-pixbuf graphene
    shared-mime-info hicolor-icon-theme adwaita-icon-theme
    gtk4
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
echo "Phase 8d GTK stack: $(count_done_packages) packages built (cumulative)."
echo "Next: ./12-audio-stack.sh (when populated) — alsa-lib + pipewire + wireplumber."
