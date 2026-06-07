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
   (registers Nix) + `avahi-daemon` (advertises `writeonce.local`) → tty1 **greeter**
   (`writeonce-greeter`): sign in (password) and pick a session — **Shell (bash)** or an
   installed Wayland compositor. A fresh boot offers only Shell; install one via
   `nix profile add path:/etc/writeonce/sessions/sway` (or `sudo wo-sshd-setup` then drive
   it over SSH), log out, and the session appears in the ⚙ list. tty2–6 give a password
   getty. First Nix fetch is multi-hundred-MB; later cached.
   See [`../../docs/learning/ssh-and-byo-compositor.md`](../../docs/learning/ssh-and-byo-compositor.md).

## Acceptance / verification

- `nix --version` works (W3); a default route exists (W4); `ping writeonce.local`
  resolves from the workstation (mDNS / avahi).
- `sudo wo-sshd-setup` → `systemctl status sshd` active → `ssh writeonce@writeonce.local`
  logs in (key provisioned at install, or password).
- Boot lands at the **greeter** (password required). The Shell session works before any
  compositor is installed; after `nix profile add path:/etc/writeonce/sessions/sway`,
  "Sway" appears in the ⚙ list and launches → `echo $WAYLAND_DISPLAY` non-empty. tty2–6 →
  password getty → bash.

## Debugging (boots to the greeter by design, never hangs)

- `journalctl -b -u writeonce-greeter` (greeter/session) / `-u writeonce-nix-init` (Nix) /
  `-u iwd -u dhcpcd` (network) / `-u avahi-daemon` (mDNS) / `-u sshd` (after `wo-sshd-setup`).
- Greeter misbehaving? Ctrl+Alt+F2 → password getty → bash; or SSH in. Pick "Shell (bash)"
  at the greeter to debug a failing compositor (`loginctl`, the compositor's own logs).
- `nix profile add github:NixOS/nixpkgs/nixos-unstable#sway` to reproduce a package-install
  error directly (full flakeref — the bare `nixpkgs` alias won't resolve).
- Compositor logs are wherever your chosen compositor writes them. `just pull-logs /dev/sdX`
  from the workstation.

## Optional cheaper pre-flight (validate the risky new code before flashing)

QEMU on the full disk artifact with `-device virtio-gpu-pci -display gtk,gl=on -vga none` +
KVM + a NAT NIC: confirms **Nix registers, `nix profile install` works, Hyprland reaches
DRM + opens a Wayland socket** — without a T450 round-trip. It will *not* render the
Quickshell bar reliably (iris-only Mesa, no virgl), so the bar stays T450-authoritative.

## Risks

- Front-loaded on W3 (Nix bootstrap) and the first-boot network (W4); expect a few
  target iterations. Pre-baking the closure (W6) removes the first-boot-network dependency.
