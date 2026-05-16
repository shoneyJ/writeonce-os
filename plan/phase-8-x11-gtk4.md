# Phase 8 — X11/i3 userspace + GTK4 stack (i3More substrate)

**Goal.** Provide the X11 + i3 + GTK4 + D-Bus + PAM + audio stack that i3More requires (per the i3More OS-dep survey). This is *infrastructure* — i3More itself comes in Phase 9.

## Subtasks

1. **Build the X11 stack from source** (curated, LFS-style, into the sysroot):
   - `libXau`, `libxcb`, `xtrans`, `libX11`, `libXext`, `libXft`, `libXrender`, `libXrandr`, `libXinerama`, `libxkbcommon`.
   - `xorg-server` configured for the i915 KMS driver only (drop unused video drivers: nvidia, ati, vesa-only).
   - `xf86-input-libinput` (touchpad + keyboard).
   - `xf86-video-intel` is deprecated upstream — use the modesetting driver bundled with `xorg-server` against `i915`.
   - `xkeyboard-config`, `xset`, `setxkbmap`, `xrandr` utilities.

2. **Build i3 window manager from source.** From `../.agents/reference/i3More/vendor/i3/` if i3More vendors a patched copy; otherwise mainline i3. Dependencies: `libev`, `yajl`, `xcb-util-*`, `pcre2`.

3. **Build GTK4 + dependencies** (`glib`, `cairo`, `pango`, `gdk-pixbuf`, `graphene`, `harfbuzz`, `fontconfig`, `freetype`).
   - Match versions to `../.agents/reference/i3More/Cargo.toml` (gtk4 0.10.3 → GTK ≥ 4.14 runtime).

4. **Build D-Bus** (`dbus-1`, `libdbus-1`). System bus runs as a WriteOnce service; per-user session bus spawned by Phase 9 login flow.

5. **Build PAM** (`linux-pam`). Configure `/etc/pam.d/` with `login`, `passwd`, `writeonce-lock` (for `i3more-lock`).

6. **Audio: build PipeWire** (i3More prefers it per Cargo.toml's `pipewire 0.8.0`). Also `wireplumber` for session management. Skip PulseAudio compatibility unless `pactl` is genuinely required — re-check `i3more-audio` source.

7. **Fonts and icon themes.**
   - Adwaita icon theme (i3More dep).
   - hicolor-icon-theme.
   - DejaVu fonts as a baseline; let the user add more later.

8. **Write supervisor service files** (in the Phase-4 TOML format) for:
   - `dbus.service`, `seatd.service` (or skip seatd if logind shim is enough), `pipewire.service`, `wireplumber.service`.
   - `xorg.service` — starts `Xorg :0 vt7 -keeptty` against the active TTY.

9. **Iterate.** Boot, log in over SSH, start Xorg manually, then i3, then `xterm`. Confirm visible on T450 screen.

## Deliverable

Logging in on tty1 starts X11 + i3 + a terminal automatically (or via a manual `startx` for now).

## Acceptance criteria

- `Xorg -version` runs.
- `i3` launches with a config file; `Mod+Enter` opens a terminal.
- `gtk4-demo` shows the GTK4 demo window.
- `pactl info` (or `pw-cli info`) returns valid output.
- `busctl --system` and `busctl --user` both list buses.

## References

- LFS BLFS book (Beyond LFS) for Xorg/GTK build orders.
- `../.agents/reference/i3More/README*` and `../.agents/reference/i3More/Cargo.toml` for exact runtime expectations.

## Risks

- This phase has the most third-party C code to build. Time-box; if a dep is fighting back, accept a prebuilt from a clean source (e.g. snapshot Debian's source package).
