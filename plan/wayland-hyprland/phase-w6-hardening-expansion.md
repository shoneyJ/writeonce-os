# Phase W6 — Hardening & expansion

**Status: future.** Only after W5 reaches a working desktop. Each item is independent.

## Goal

Move from "minimal bar + terminal" to a comfortable daily desktop, and remove the
first-boot fragilities.

## Candidate work (pick as needed)

- **Pre-bake the desktop closure** into `/nix/store` at image-build (needs Nix on the
  workstation): `nix build`/`nix copy` the desktop set, stage it like `17a` does the base
  closure, point a profile at it. Removes the **first-boot-network dependency** — the
  compositor is present at first boot. Highest-value follow-up.
- **Pin nixpkgs** (`nix registry pin nixpkgs github:NixOS/nixpkgs/<rev>`) for reproducible
  desktop installs; track the rev like a supply-chain entry.
- **XWayland** for legacy X11 apps — add `nixpkgs#xwayland` (or enable it in the Hyprland
  package) so X-only apps run under Hyprland.
- **Richer Quickshell shell** — grow `shell.qml` toward the user's `~/dotfiles/quickshell`
  (notifications, OSD, launcher, battery via UPower, tray). Add the Qt/KDE modules each
  widget needs to the Nix set (Kirigami, Qt5Compat, QtPositioning) — incrementally, verifying
  imports resolve.
- **Idle / lock / screenshots** — `hypridle`, a locker, `grim`+`slurp`; from the Nix set.
- **Audio/portals polish** — `xdg-desktop-portal-hyprland` wired for screenshare/file-pickers;
  confirm PipeWire routing.
- **Broadwell perf** — re-evaluate animations/blur once running; keep software cursors if the
  Gen8 hardware-cursor glitch appears.
- **Fonts/cursors** — ship the fonts the config references (Nix or `/etc` fontconfig).

## Acceptance

Per item: the feature works on the T450 without regressing first boot; the Nix desktop set
stays declarative in `etc/writeonce/desktop/flake.nix`; no `*.nix` authored in-repo.

## Risk

- Scope creep toward the full "illogical-impulse" config (Kirigami + KDE Frameworks +
  Python venv) is heavy on Broadwell — add widgets incrementally, not wholesale.
