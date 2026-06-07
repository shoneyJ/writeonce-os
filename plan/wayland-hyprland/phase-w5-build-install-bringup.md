# Phase W5 — Build → install → T450 first-boot bring-up

**Status: workstation-build half VERIFIED (2026-06-06); install + first boot target-gated.**
The build pipeline (steps 1–2 below) was run on the workstation and is green; the install +
first-boot bring-up (steps 3–4) need the T450.

## Verified on the workstation (2026-06-06)

Ran `01-fetch` (Nix tarball, sha256 matches) → `17-stage-sysroot.sh` → `17a-install-nix.sh`
→ `18-make-artifacts.sh`, all clean (EXIT 0). This is the first real run of the
profile-overlay refactor + the blind-written Nix staging:
- Skeleton overlay placed `common/` + `systemd-wayland-hyprland/` correctly (hypr/quickshell
  config, `wo-session`, `nix.conf`, iwd/dhcpcd units + `.wants` symlinks, common
  passwd/sudoers); no i3/`.xinitrc` leftovers; modes/symlinks intact.
- `17a` staged `/nix` (104M): `store/` (63 paths), `.reginfo` (576 lines — the load-db
  input), the bundled `nix-2.34.7` package (matches `wo-nix-init`'s glob), `nss-cacert`.
- Artifact: `sysroot.tar.zst` 572M (with `/nix`) + `BOOTX64.EFI`/`bzImage` (kernel 6.18.34).
  The image is ready to flash.

## Goal

Produce the image with the Nix store baked in, install it, and bring up the Hyprland +
Quickshell desktop on the T450 — fixing first-boot issues as they surface.

## Steps

1. **Fetch** — `cd build && ./01-fetch.sh` (self-verifies `nix-2.34.7` against the pinned
   sha256; substrate/kernel already built).
2. **Stage** — `./build/17-stage-sysroot.sh` → `./build/17a-install-nix.sh` (adds
   `/nix`, ~hundreds of MB) → `./build/18-make-artifacts.sh`. (Default `FLAVOR=systemd-wayland-hyprland`.)
3. **Install** — attach the target disk, `just install /dev/sdX` (= `check-staging` +
   `install.sh`); answer the Wi-Fi prompt (or skip for ethernet). Default login: `writeonce`.
4. **First boot (needs network)** — EFI-stub kernel → systemd → `writeonce-nix-init`
   (registers Nix) → tty1 autologin → `wo-session` → `nix profile install` the desktop
   (multi-hundred-MB download, minutes) → `exec Hyprland` → `quickshell -c wo`. Later boots
   are instant.

## Acceptance / verification

- `nix --version` works (W3); a default route exists before the install (W4).
- Hyprland starts; `echo $WAYLAND_DISPLAY` non-empty; `/run/user/1000/wayland-1` exists.
- Quickshell bar shows (clock ticking + ≥1 workspace pill); `Mod+Return` → kitty;
  `Mod+Q` close; `Mod+Shift+E` → back to shell.

## Debugging (it falls back to a shell, never hangs)

- `journalctl -b -u writeonce-nix-init` (Nix registration) / `-u iwd -u dhcpcd` (network).
- `nix profile install nixpkgs#hyprland` to reproduce a DE-install error directly.
- `cat $XDG_RUNTIME_DIR/hypr/*/hyprland.log` (compositor). `just pull-logs /dev/sdX` from the
  workstation.

## Optional cheaper pre-flight (validate the risky new code before flashing)

QEMU on the full disk artifact with `-device virtio-gpu-pci -display gtk,gl=on -vga none` +
KVM + a NAT NIC: confirms **Nix registers, `nix profile install` works, Hyprland reaches
DRM + opens a Wayland socket** — without a T450 round-trip. It will *not* render the
Quickshell bar reliably (iris-only Mesa, no virgl), so the bar stays T450-authoritative.

## Risks

- Front-loaded on W3 (Nix bootstrap) and the first-boot network (W4); expect a few
  target iterations. Pre-baking the closure (W6) removes the first-boot-network dependency.
