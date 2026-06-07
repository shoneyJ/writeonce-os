# Phase W1 — Wayland substrate & delivery decision

**Status: done.** Decision: deliver the desktop via **Nix**; the from-source substrate is
**parked**.

## Goal

Decide how the Wayland compositor + shell reach the T450: source-built into the LFS sysroot
(like the X11 stack) vs delivered as a self-contained Nix closure. Confirm the kernel is
adequate (it is — no change).

## Context / findings

- **Kernel is over-spec, unchanged.** `with-systmed`'s 6.18.34 config already has
  `CONFIG_DRM=y`, `DRM_I915`, `DRM_FBDEV_EMULATION`, `INPUT_EVDEV`, `DEVTMPFS`; atomic
  modesetting is unconditional in modern DRM. Broadwell (Gen8) caveat: keep the cmdline
  `i915.enable_psr/fbc/dc=0` mitigations + software cursors.
- **From-source is high-risk** for our glibc-2.40 / GCC-14.2 sysroot: aquamarine-based
  Hyprland 0.44 hard-requires `hyprcursor` → **`librsvg` (Rust)**, and sets
  `CMAKE_CXX_STANDARD 26` (GCC 14.2 is only partial C++26); plus Qt6 + Quickshell cross.
- **A Nix Hyprland closure is self-contained** (its own Mesa/Wayland/seatd from nixpkgs)
  and only needs the kernel's DRM — sidestepping every cross-build risk, consistent with
  the project's "Nix for apps after login" scope.

## Outcome

- **Chosen:** Nix for Hyprland + Qt6 + Quickshell + terminal (W2–W5).
- **Parked:** the from-source Wayland substrate `build/16a-wayland-substrate.sh` (wayland,
  wayland-protocols, mesa+wayland rebuild, libxkbcommon+wayland, seatd, libdisplay-info,
  libliftoff) + the generic `build_cmake` helper in `build/blfs-pkg.sh` + Containerfile
  host code-gen deps. Retained as a fallback, clearly marked PARKED at the script head;
  its packages are **not** wired into `versions.env`/`01-fetch.sh`.

## Deliverable

A recorded decision (this doc + the learning doc) and a parked-but-correct from-source
script, so the choice is reversible without re-deriving it.

## Risks

- If Nix bootstrap (W3) proves intractable on the target, the fallback is the parked
  from-source path — at the cost of the librsvg/C++26/Qt6 cross-build effort.

## References

- `docs/learning/phase-8-wayland-hyprland-quickshell.md` (full rationale)
- `build/16a-wayland-substrate.sh` (parked from-source substrate)
- `plan/using-stock/modern_lfs_workstation_path.md` (the earlier Wayfire/Sway exploration)
