# Phase W2 — Desktop config & session launch

**Status: done** (committed `ff50800`). HOW-agnostic config; verifiable by inspection.

## Goal

Ship the Hyprland + Quickshell desktop configuration and wire the login → session path,
replacing the X11/i3 `startx` flow. Config only — the binaries come from Nix (W3–W5).

## Subtasks

- [x] **Session launcher** `build/skeleton/.../usr/local/bin/wo-session` — exports the
  Wayland/Qt env (`XDG_SESSION_TYPE=wayland`, `QT_QPA_PLATFORM=wayland`,
  `WLR_NO_HARDWARE_CURSORS=1` for Gen8), realizes the desktop from Nix on first boot, then
  `exec Hyprland`. Falls back to a shell on any failure (never traps the only console).
- [x] **`~/.bash_profile`** — on tty1 autologin: source the Nix profile, run `wo-session`.
- [x] **Hyprland config** `…/.config/hypr/hyprland.conf` — single file, no plugins;
  `kb_layout=de` (matches `/etc/vconsole.conf`); blur/shadows off + `vfr=true` (Broadwell);
  `exec-once` for dbus env, pipewire/wireplumber, `quickshell -c wo`; SUPER keybinds,
  `$term=alacritty`, `wpctl` volume keys.
- [x] **Quickshell bar** `…/.config/quickshell/wo/shell.qml` — thin top bar (Hyprland
  workspaces, clock, default-sink volume) using only standard Quickshell+Qt modules (no
  Kirigami/QtPositioning/Qt5Compat).
- [x] **`17-stage-sysroot.sh`** — dropped the i3/i3More staging; the config rides in via the
  skeleton overlay. **`motd`/`autologin.conf`** text updated; `.xinitrc` + `.config/i3/`
  removed.

## Deliverable

A staged rootfs whose `/home/writeonce` + `/etc` carry a complete, minimal Hyprland+Quickshell
desktop config, launched on tty1 autologin — inert until the binaries exist (W3–W5).

## Acceptance / verification

- `qmllint` / `qs -c wo --check` on `shell.qml` (where Quickshell is available); `hyprland.conf`
  has no plugin/source lines so it can't fail on missing plugins.
- Default-flavor stage places the files at the expected paths (confirmed by the skeleton
  overlay; see W5).

## Risks

- The thin `shell.qml` assumes the nixpkgs Quickshell build provides `Quickshell.Hyprland`
  + `Quickshell.Services.Pipewire` (it does). A very old nixpkgs pin could lack `quickshell`.
- Terminal is `alacritty` (from Nix); ensure it's in the desktop package set (W3 `desktop/flake.nix`).
