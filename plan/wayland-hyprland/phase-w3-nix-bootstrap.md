# Phase W3 — Nix bootstrap (offline single-user `/nix`)

**Status: build-time staging VERIFIED (2026-06-06); first-boot registration target-gated.**
`17a-install-nix.sh` was run on the workstation and stages `/nix` correctly (store + the
576-line `.reginfo` + the bundled `nix-2.34.7` + `nss-cacert`). The first-boot
`wo-nix-init` ops (`nix-store --load-db`, default-profile install) still need the target
(store paths are absolute → must run on `/nix`). Realises roadmap Phase 14 for this flavor.

## Goal

Get a working single-user Nix into the image, offline and supply-chain-locked, so the
first boot can `nix profile install` the desktop from `cache.nixos.org`. No `*.nix`
package definitions authored — consumption only.

## Subtasks

- [x] **Pin + fetch** — `NIX_VERSION=2.34.7` (latest official, verified against the
  `nix-releases` S3 bucket); `01-fetch.sh` URL (`releases.nixos.org` static x86_64-linux
  tarball); `checksums.txt` carries the upstream-published sha256 so the fetch self-verifies.
- [x] **Build-time staging** `build/17a-install-nix.sh` (runs after `17-stage-sysroot.sh`,
  before `18-make-artifacts.sh`): unpacks the tarball into `$STAGING/nix` (store + `.reginfo`
  + single-user `var` skeleton). Gated on `FLAVOR_PKG=nix`. Store-DB load is deferred to boot
  (store paths are absolute).
- [x] **First-boot registration** `wo-nix-init` + `writeonce-nix-init.service` (oneshot,
  guarded by the absent store DB, ordered `Before=getty@tty1.service`): chown `/nix`,
  `nix-store --load-db < /nix/.reginfo`, install the bundled nix into the default profile.
- [x] **Environment** `etc/profile.d/nix.sh` (profiles on PATH + `NIX_SSL_CERT_FILE` from the
  bundled `nss-cacert`); `etc/nix/nix.conf` (flakes, cache, single-user).
- [x] **Desktop set** `etc/writeonce/desktop/flake.nix` — `nixpkgs#{hyprland,quickshell,kitty,
  xdg-desktop-portal-hyprland,wl-clipboard}`, consumed by `wo-session`.

## Deliverable

An image that, on first boot, registers Nix and makes `nix profile install nixpkgs#<pkg>`
work — with no account/registration (the public binary cache + key are in `nix.conf`).

## Acceptance / verification (on the target — W5)

- `writeonce-nix-init.service` runs once → `/nix/var/nix/db/db.sqlite` exists; `nix --version`
  works; `/nix` owned by the primary user.
- `nix profile install nixpkgs#ripgrep` → `rg` runs (the roadmap Phase-14 criterion).

## Risks (the likely first-boot failure points)

1. **Tarball layout / `.reginfo`** — `17a` assumes the static tarball ships `store/` +
   `.reginfo`; confirm on first unpack.
2. **`nix-store --load-db` as the store owner** + the modern `nix profile` path — the steps
   in `wo-nix-init` are faithful to the official single-user installer but untested here.
3. **Footprint** — the base closure adds ~hundreds of MB (`du -sh $STAGING/nix`).
4. **nixpkgs pin** — built-in registry resolves `nixpkgs` unpinned; pin a rev for
   reproducibility (`nix registry pin`).
