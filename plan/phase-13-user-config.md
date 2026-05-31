# Phase 13 — Preconfigured `~/.config` + `/etc/skel`

> systemd branch (`with-systmed`), post-desktop. **Depends on the X/i3More desktop
> actually starting** — the current diagnostic image disables auto-`startx` pending the
> Xorg/i915 freeze fix; resolve that first.

**Goal.** A fresh user logs in and gets a complete, themed i3More desktop out of the box —
working terminal, launcher, lock, fonts, cursor, GTK theme, audio, and sane shell defaults —
delivered via `/etc/skel` so the Phase 11 installer's newly-created user inherits it all.

## Context

Today the seeded home (`build/skeleton/home/writeonce/`) has only `.bash_profile`,
`.xinitrc`, and a minimal `.config/i3/config` (keybindings → i3More applets). There is no
`/etc/skel`, so a user created by the installer would get nothing. The i3 config references
`alacritty` (`$mod+Return`) — that terminal's presence and config must be confirmed. This
phase turns the skeleton into a real, themed default environment and routes it through
`/etc/skel`.

## Subtasks

1. **Establish `/etc/skel`.** Move the user dotfiles from the hard-coded
   `home/writeonce/` into `build/skeleton/etc/skel/`, and have the installer copy
   `/etc/skel` → the new user's home (`cp -a` + `chown`). Keep a minimal `writeonce` home
   for the pre-seeded account, or generate it from skel at stage time.
2. **Terminal.** Verify a terminal emulator is built/staged (the i3 config assumes
   `alacritty`). If absent, either build it or pick a staged alternative (e.g. `xterm`
   already in the X stack) and fix the keybinding. Ship its config (`alacritty.toml`:
   font, colors, padding).
3. **i3 + i3More config polish.** Flesh out `~/.config/i3/config` (workspaces, sensible
   binds, autostart of i3More daemons matching `.xinitrc`); add any i3More config files
   the binaries read (`~/.config/i3more/…` — confirm against the i3More binaries).
4. **Shell defaults.** `.bashrc` (prompt, aliases, history, `PATH` including the future
   `~/.nix-profile/bin` from Phase 14), `.bash_profile` (restore the auto-`startx` block
   once X is fixed), `.inputrc`.
5. **Appearance.** GTK4 `~/.config/gtk-4.0/settings.ini` (theme, dark/light, icon theme,
   font), fontconfig defaults + at least one shipped UI font, cursor theme
   (`xsetroot`/`XCURSOR_THEME`), wallpaper / root-window color via `.xinitrc`.
6. **XDG + env.** `~/.config/user-dirs.dirs`, `XDG_*` defaults; `~/.profile`/`environment.d`
   for session env (`EDITOR`, `XCURSOR_*`, etc.).
7. **Audio per-user.** Confirm PipeWire/WirePlumber need no extra per-user config beyond
   the daemons launched in `.xinitrc`; add `~/.config/pipewire/…` only if a default is
   required.
8. **check-staging assertions.** Add checks that `/etc/skel` exists and contains the key
   dotfiles, and that the staged terminal binary referenced by the i3 config is present.

## Deliverable

A `/etc/skel` (mirrored to the pre-seeded `writeonce` home) that yields a polished,
ready-to-use i3More desktop on first login — terminal, launcher, lock, fonts, theme, audio
all working — with no per-user setup.

## Acceptance criteria

- A user created by the Phase 11 installer logs in → i3More desktop comes up themed, with a
  working terminal (`$mod+Return`), launcher (`$mod+d`), and lock (`$mod+l`).
- Fonts render (no boxes), cursor + GTK theme applied, audio applet functional.
- `ls -A ~` on the new user matches `/etc/skel` (dotfiles inherited).

## References

- `build/skeleton/home/writeonce/{.bash_profile,.xinitrc,.config/i3/config}` — current seed.
- `build/17-stage-sysroot.sh` (i3/i3More copy steps ~140–217) — where i3 + i3More land.
- `.agents/reference/i3More/` — definitive list of applets + any config they expect.
- Phase 11 (installer copies `/etc/skel`), Phase 14 (PATH adds Nix profile).

## Risks

- **Blocked by X.** Nothing here is verifiable until Xorg/i3 actually start; treat the
  Xorg/i915 freeze as the prerequisite.
- **i3More config drift.** The applets' expected config paths must be read from the i3More
  source, not assumed.
