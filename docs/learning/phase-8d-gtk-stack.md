# Phase 8d — the GTK4 stack

> Companion to [`../../build/11-gtk-stack.sh`](../../build/11-gtk-stack.sh).
> Explains the 11-package toolchain that lets i3More (a GTK4 app)
> render on top of the Phase 8c X server, and the deliberate trims
> that keep the build sane.

## The GTK4 layer cake

```
                  ┌───────────────────────────────────┐
                  │   i3More / GTK4 apps (Rust gtk4-rs│
                  │   bindings → libgtk-4.so via FFI) │
                  └───────────┬───────────────────────┘
                              │
                  ┌───────────▼───────────────────────┐
                  │              GTK4                  │
                  │  Widgets, layout, GTKSceneRenderer │
                  │  GskRenderer (GL / Vulkan / Cairo) │
                  └───┬───┬───┬───┬───┬───────────────┘
                      │   │   │   │   │
                  ┌───▼┐ ┌▼─┐ ┌▼─┐ ┌▼─┐ ┌▼──────────────┐
                  │GLib│ │HB │ │Pan│ │GdK │ │Graphene       │
                  │+   │ │+   │ │go │ │Pix │ │SIMD math      │
                  │GIO │ │Cai │ │   │ │buf │ │(matrices,vecs)│
                  │+   │ │ro  │ │   │ │    │ │               │
                  │GObj│ │    │ │   │ │    │ │               │
                  └─┬──┘ └─┬─┘ └─┬─┘ └─┬─┘ └───────────────┘
                    │      │     │     │
                    │      └─────┼────► fontconfig + freetype (Phase 8a)
                    │            │
                    └────────────┼─────► libxml2 + libffi (Phase 8a)
                                 │
                                 └──────► libpng + libjpeg-turbo (Phase 8a)
                                 │
                                 └──────► libX11 + libxcb + libxkbcommon + libepoxy (Phase 8b/c)
```

## Per-package role

### Foundation

| Package | Role |
| --- | --- |
| **glib** | The "GNOME C runtime": types (`GObject`), signals, properties, `GVariant`, `GIO` (async I/O, D-Bus client), `GError`, `GMainLoop`. **Everything in this round transitively links against glib.** |
| **gobject-introspection** | Generates `.gir` + `.typelib` files that describe GLib-based APIs in a machine-readable form. Used by language bindings that want runtime reflection (PyGObject, gjs). i3More uses `gtk4-rs` which is static FFI — doesn't need runtime introspection — but many GTK packages probe for `g-ir-scanner` during their own build, so we install it. |

### Text rendering chain

| Package | Role |
| --- | --- |
| **harfbuzz** | Modern text shaping. Turns "Hello" + a font into a list of glyph IDs + positions. Handles OpenType features (ligatures, kerning), Arabic / Indic / Hebrew scripts, RTL. |
| **cairo** | 2D vector graphics. Paths, fills, strokes, text via freetype, alpha compositing. Cairo can draw to PNG, SVG, PDF, or X11 surfaces directly. |
| **pango** | Text layout. Takes a paragraph + font + width and produces line-broken, justified, glyph-positioned output. Internally uses harfbuzz (shaping) + cairo (rendering) + fontconfig (font selection) + freetype (rasterisation). |

### Image loading + math

| Package | Role |
| --- | --- |
| **gdk-pixbuf** | Raster image loader. We enable only PNG + JPEG loaders (skip TIFF / WebP / SVG — `librsvg` is a huge Rust dep). Adwaita icon theme ships PNGs at common sizes so the lack of SVG support is fine. |
| **graphene** | SIMD-accelerated math: 4x4 matrices, 4D vectors, quaternions, frustum planes. GTK4's `GskRenderer` uses it for the 2D affine + 3D-projected transforms in scene-graph nodes. Without graphene, every GTK4 widget animation would fall back to scalar math. |

### Runtime data

| Package | Role |
| --- | --- |
| **shared-mime-info** | The freedesktop MIME database. GTK file choosers + many other apps look up "what's this file" via `g_content_type_*` which reads this database. |
| **hicolor-icon-theme** | The spec-mandated fallback icon theme. Every other icon theme inherits from hicolor. Pure data, ~5 MB. |
| **adwaita-icon-theme** | GNOME's standard icon set. **i3More specifies Adwaita in its Cargo.toml dep list.** Modern Adwaita ships PNGs at 16/24/32/48/64 px alongside SVGs, so we don't need librsvg for icon rendering — gdk-pixbuf's PNG loader is sufficient. |

### The toolkit

| Package | Role |
| --- | --- |
| **gtk4** | The widget toolkit. ~30 MB of compiled code. Provides `GtkWidget`, `GtkWindow`, `GtkLabel`, `GtkButton`, plus the scene-graph renderer (GskRenderer) that uses GL or Cairo to actually draw frames. |

## Deliberate trims

| Trim | Why |
| --- | --- |
| **No glib-networking** | Provides TLS for GIO HTTP/HTTPS. Would need OpenSSL or GnuTLS in our substrate (not yet built). i3More doesn't make HTTPS calls via GIO. If we add it later: build OpenSSL first, then `glib-networking` with the `gnutls=disabled openssl=enabled` flag, then GTK4 picks it up automatically at runtime via dlopen. |
| **No librsvg** | SVG rendering. librsvg switched to Rust around 2.41 — pulling it in here is a big surface area, plus Adwaita's PNGs cover our icon needs. Future Phase 9 work can add librsvg if i3More turns out to need SVG icons at runtime. |
| **No introspection at runtime** | `-Dintrospection=disabled` on every GTK package. i3More's bindings are `gtk4-rs` which is static FFI generated at compile time. Disabling introspection saves ~30 MB of `.typelib` files. |
| **No Wayland backend** | i3More is X11-only. `-Dwayland-backend=false` on GTK4 cuts the EGL+Wayland code paths. |
| **No Vulkan backend** | GTK4's GskRenderer can use Vulkan, GL, or Cairo. We disabled Vulkan in Mesa (Phase 8c), so disable here too. GL is the default backend; Cairo is the fallback. |
| **No broadway-backend** | Broadway lets GTK render to a remote browser. Useful for cloud-IDE scenarios; not relevant for a laptop desktop. |
| **No media-gstreamer / media-ffmpeg** | GTK4 can play video via GstPlay. The i3More desktop has no video player; disable both. |
| **No print-cups / print-cpdb** | Printing. The T450 is a developer workstation; printing can be added via CUPS-as-Nix-package in Phase 10 if needed. |
| **No colord** | Color profile management for ICC; relevant for graphic-designer workflows, not generic desktop. |
| **No sysprof** | Profiling integration. Useful for tuning, but adds build deps; future hardening pass. |
| **No tracker, no cloudproviders** | GNOME-specific integrations not applicable to the i3-based desktop. |

The result is a GTK4 that does exactly what i3More needs (X11 + GL accel + PNG/JPEG icons + Cairo fallback) and nothing else.

## Why the build order matters

```
glib                           [no deps beyond Phase 8a]
  ↓
gobject-introspection          [needs glib]
  ↓
harfbuzz                       [needs freetype from Phase 8a; can build before glib if needed]
  ↓
cairo                          [needs freetype + fontconfig + libpng (Phase 8a); X libs (Phase 8b)]
  ↓
pango                          [needs glib + harfbuzz + cairo + fontconfig + freetype]
  ↓
gdk-pixbuf                     [needs glib + libpng + libjpeg]
  ↓
graphene                       [needs glib]
  ↓
shared-mime-info               [needs glib + libxml2]
  ↓
hicolor-icon-theme             [pure data, no deps]
  ↓
adwaita-icon-theme             [data + meson; assumes hicolor present]
  ↓
gtk4                           [needs every previous + libX11 + libxcb + libxkbcommon + libepoxy]
```

If you build out of order: `meson setup` fails with "Dependency $X not found". The sentinels prevent reordering accidents because each step checks for its dependency-step's sentinel transitively — but explicitly running, e.g., `./11-gtk-stack.sh gtk4` before glib is built will fail at meson setup, not silently produce a broken GTK4.

## Expected compile times

Roughly, on a single-thread make / meson compile inside `wo-builder` on a recent workstation:

| Package | Time |
| --- | --- |
| glib                  | ~10 min |
| gobject-introspection | ~5 min  |
| harfbuzz              | ~5 min  |
| cairo                 | ~5 min  |
| pango                 | ~3 min  |
| gdk-pixbuf            | ~3 min  |
| graphene              | ~2 min  |
| shared-mime-info      | ~1 min  |
| hicolor-icon-theme    | <1 min  |
| adwaita-icon-theme    | ~2 min  (just data + image processing) |
| **gtk4**              | **~30–45 min** |
| **Total**             | **~60–80 min** |

Smaller than 8c (Mesa was the long pole) but still a sit-down round.
Use `--no-network` per the supply-chain doc.

## What's in $LFS/usr after this round

```
/usr/lib/
  libglib-2.0.so.0           libgobject-2.0.so.0           libgio-2.0.so.0
  libharfbuzz.so.0           libharfbuzz-gobject.so.0
  libcairo.so.2              libcairo-gobject.so.2
  libpango-1.0.so.0          libpangocairo-1.0.so.0
                             libpangoft2-1.0.so.0
                             libpangoxft-1.0.so.0
  libgdk_pixbuf-2.0.so.0
  libgraphene-1.0.so.0
  libgtk-4.so.1              ← ~30 MB
  libgirepository-1.0.so.1
/usr/share/icons/
  Adwaita/                   ~50 MB (icons at many sizes)
  hicolor/
/usr/share/mime/             shared-mime-info database
/usr/share/glib-2.0/schemas/ GSettings schemas (compiled at end of GTK4 install)
```

Approximately **150 MB** added on top of Phase 8c's substrate, bringing
the running total to ~350 MB of native userspace before Phase 8e
(audio) and Phase 9 (i3 + i3More).

## After this round

GTK4 is functional in the sysroot. To exercise it inside the running
system you'd need an X server running on a tty and at least one
GTK4 app. Phase 9 (i3 + i3More) is what brings up the actual desktop;
before then the sysroot is "complete" only in the sense that running
`gtk4-demo` from inside a chroot+X session would actually work.

Next round (8e, audio) adds the runtime PipeWire daemon + ALSA so
i3More's audio applet has something to talk to.
