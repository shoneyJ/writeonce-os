# Phase 8b — the X11 protocol stack

> Companion to [`../../build/09-x11-stack.sh`](../../build/09-x11-stack.sh).
> Explains what each X11 layer provides, the dependencies between them,
> and why each package is included for the i3 / i3More desktop target.

## The X11 system at a glance

```
                  ┌─────────────────────────────────┐
                  │   Application (i3More, GTK4)    │
                  └────┬───────────────────┬────────┘
                       │                   │
                       │ classic Xlib      │ modern XCB
                       ▼                   ▼
                  ┌───────────┐       ┌──────────────────────┐
                  │  libX11   │──────▶│       libxcb         │
                  │  (Xlib)   │       │ (binary X11 protocol)│
                  └───────────┘       └──────────┬───────────┘
                                                 │
                                                 │ X11 wire protocol over Unix-socket
                                                 ▼
                                        ┌─────────────────┐
                                        │   Xorg server   │  (Phase 8c)
                                        └─────────────────┘
```

`libX11` ("Xlib") is the historical X client library — synchronous,
string-formatted, every app has linked against it for 30+ years. `libxcb`
("X C Bindings") is the modern binary-protocol replacement; smaller, async,
faster. Today's apps usually link against **both**: GTK uses XCB directly,
old programs use Xlib (which itself uses XCB under the hood since
libX11 1.4.0).

## Layer 1 — protocol headers (no compiled code)

| Package | What it ships |
| --- | --- |
| **xorgproto** | C headers + XML descriptions for every X11 protocol (core + extensions). No `.so`. Every Layer 2/3 library `#include`s these. |
| **xcb-proto** | XML descriptions of the X11 protocol in a machine-readable form. `libxcb`'s build runs a Python script that reads these XMLs and generates `xcb.h`, `xcb_xproto.c`, etc. |

These are install-only packages — they exist so later packages can
compile.

## Layer 2 — core libraries

| Package | Role |
| --- | --- |
| **libXau** | Reads `$XAUTHORITY` cookies used for client authentication when connecting to the X server. Tiny; ~10 KB shared lib. |
| **xtrans** | Source-only transport abstraction (Unix sockets / TCP / SSH-forwarded sockets). Installed as headers + autoconf macros — no compiled library. |
| **libxcb** | The binary X protocol library. ~500 KB. All modern X clients depend on it transitively. Reads xcb-proto's XMLs to generate its API at build time. |
| **libX11** | Xlib. ~2 MB. Legacy API kept for backwards compatibility; internally talks XCB. |

After Layer 2 we have everything needed to talk to a hypothetical Xorg
server — except we haven't compiled extension support yet.

## Layer 3 — extension libraries

X11's core protocol from 1987 wasn't enough for modern GUIs. Every
modern X capability is an "extension" that the server advertises and
clients use via a separate library.

| Package | Provides | i3More uses? |
| --- | --- | --- |
| **libXext** | The X extension framework itself + a handful of older extensions (SHM, double-buffer). | Yes (transitively) |
| **libICE** | Inter-Client Exchange. Session management infrastructure. | Yes via libSM |
| **libSM** | Session Management. Tells apps "you're being logged out, save state." | Yes |
| **libXfixes** | Region operations, cursor manipulation. | Yes |
| **libXdamage** | Tracks which parts of a window changed. Composite managers need it. | Indirectly (xcompmgr-style) |
| **libXcomposite** | Lets a compositor capture every window into an offscreen buffer. Required by any compositing manager. | Yes (i3's xcompmgr support) |
| **libXrender** | Anti-aliased drawing primitives. The foundation of modern desktop rendering on X. | Yes |
| **libXcursor** | Themed mouse cursors (per-DPI, animated). Needs libXfixes + libXrender. | Yes |
| **libXft** | Anti-aliased text rendering bridging Xrender + fontconfig + freetype. **i3, dmenu, xterm, rofi all use this.** | Yes — directly |
| **libXrandr** | Resolution + multi-monitor configuration. | Yes — for laptop screen + external monitors |
| **libXinerama** | Older multi-monitor query API; deprecated in favor of XRandR but still queried by some apps. | Yes — defensive |
| **libXi** | X Input extension — the modern way to enumerate keyboards + mice + touchpads. **GTK4 reads input via XI2.** | Yes |
| **libXtst** | XTEST — synthetic event injection. Used by `xdotool`, accessibility tools, autotest frameworks. | Yes — i3More-launcher uses it |
| **libxkbcommon** | Modern keyboard handling: scancodes → keysyms via XKB rules. **Used by both Xorg and Wayland clients.** Meson-built; we disable Wayland support. | Yes |

## Layer 4 — xcb-util collection

Convenience libraries that build on `libxcb` for common patterns nobody
wants to re-implement.

| Package | Provides |
| --- | --- |
| **xcb-util** | Atom-name caching, error-code → string conversion. |
| **xcb-util-image** | Image format conversion (XCBImage compatible with old Xlib XImage). |
| **xcb-util-keysyms** | Keysym lookup tables (keycode 38 = 'a', etc.). |
| **xcb-util-wm** | EWMH + ICCCM helpers for window managers. **i3 uses this.** |
| **xcb-util-renderutil** | Xrender helper utilities. |
| **xcb-util-cursor** | Per-DPI themed-cursor loading. Replaces libXcursor for XCB-pure apps. |

## Layer 5 — keyboard data

| Package | Provides |
| --- | --- |
| **xkeyboard-config** | The actual keymap files (`us`, `de`, `dvorak`, `colemak`, …) consumed by xkbcommon and Xorg. Pure data, no libraries. ~5 MB installed. |

## What we deliberately don't build here

The plan stays disciplined about what's needed:

- **No Wayland.** i3More is X11-only (per the survey at `.agents/reference/i3More/`); libxkbcommon's Wayland support is explicitly disabled.
- **No XCB compositing extension** beyond what libXcomposite provides — the i3 model is "don't compose by default", just stack windows.
- **No XKB-X11 deprecated path.** Modern keyboard input is libxkbcommon directly; no need to install the legacy `libxkbfile`.
- **No XCMisc, XFontCache, or other deprecated extensions.** xorgproto headers are present (transitively); we just don't install separate libraries for them.
- **No Mesa.** Mesa is built in Round 8c alongside the X.Org server — the modesetting driver needs GBM from Mesa to talk to the kernel's DRM.

## Dependency graph (build order)

```
xorgproto ────────────────────────────────────────────────┐
                                                          │
xcb-proto ──── libxcb ─── (libxcb is the linchpin) ────┐ │
                  │                                      │ │
libXau ───────────┘                                      │ │
xtrans ───────────────── libX11 ──────────────────────┐  │ │
                                                      ▼  ▼ ▼
                                                  libXext, libICE, libSM, libXfixes,
                                                  libXdamage, libXcomposite,
                                                  libXrender, libXrandr, libXinerama,
                                                  libXi, libXtst
                                                          │
                                                          ▼
                                                  libXcursor ← libXfixes + libXrender
                                                  libXft     ← libXrender + freetype (Phase 8a) + fontconfig (Phase 8a)
                                                          │
                                                          ▼
                                                  libxkbcommon (meson; no waylanddep)
                                                  xcb-util, xcb-util-{image,keysyms,wm,renderutil}
                                                          │
                                                          ▼
                                                  xcb-util-cursor (needs the four above)
                                                          │
                                                          ▼
                                                  xkeyboard-config (meson; data-only)
```

The order in `build/09-x11-stack.sh`'s `STEPS=(…)` matches this.

## What's installed in $LFS/usr/lib after this round

Approximately:

```
libX11.so.6, libX11-xcb.so.1            (~2 MB)
libxcb.so.1                             (~500 KB)
libXau.so.6
libXext.so.6
libICE.so.6, libSM.so.6
libXfixes.so.3, libXdamage.so.1, libXcomposite.so.1
libXrender.so.1, libXft.so.2
libXrandr.so.2, libXinerama.so.1
libXi.so.6, libXtst.so.6
libXcursor.so.1
libxkbcommon.so.0, libxkbcommon-x11.so.0
libxcb-util.so.1, libxcb-image.so.0, libxcb-keysyms.so.1,
libxcb-icccm.so.4, libxcb-ewmh.so.2, libxcb-renderutil.so.0,
libxcb-cursor.so.0
```

Plus pkg-config files at `$LFS/usr/lib/pkgconfig/{xcb,x11,xext,...}.pc`
so downstream packages (Xorg server, GTK4, i3) can find them.

Plus headers at `$LFS/usr/include/{X11,xcb,xkbcommon}/...` and the
xkb data tree at `$LFS/usr/share/X11/xkb/`.

## After this round

The X.Org server itself comes in Round 8c, which adds:

- mesa-libs + libdrm (modesetting needs GBM + libdrm)
- xkbcomp + xkeyboard-config (already in this round) wired into the server
- xorg-server (the binary)
- xf86-input-libinput (touchpad + keyboard event source)

Round 8d is GTK4 + glib + cairo + pango + harfbuzz + gdk-pixbuf, sitting
on top of everything in 8a + 8b + a subset of 8c's libdrm.

Round 8e is the audio stack (alsa-lib + pipewire + wireplumber).

Round 8f is iwd + iproute2 for network.

Round 9 is i3 + i3More themselves.
