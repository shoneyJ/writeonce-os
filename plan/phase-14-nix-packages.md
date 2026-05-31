# Phase 14 — Package management via Nix (single-user)

> systemd branch (`with-systmed`), post-desktop. Per the project scope: **Nix for apps,
> adopted wholesale — no bespoke package manager, no hand-written package definitions.**

**Goal.** A booted WriteOnce system can install upstream applications with
`nix profile install <pkg>`, from a pinned `nixpkgs`, in **single-user** mode — keeping the
bespoke surface bounded to the boot path while userspace apps come from Nix.

## Context

WriteOnce's contribution is the boot path (kernel + systemd substrate + X11/i3More); the
scope memory (`project-writeonce-scope`) explicitly directs package management to **Nix,
single-user initially**. The source-built LFS substrate is *not* a general package manager —
it exists to bring up the boot path and desktop. Everything a user installs afterward
(editors, browsers, CLI tools) should come from Nix. Today there is **no Nix** anywhere in
the image.

## Subtasks

1. **Single-user store.** Provision `/nix` owned by the primary user (daemonless,
   single-user Nix — no `nix-daemon`, no build users group). Document the later upgrade
   path to multi-user if ever needed.
2. **Offline bootstrap (supply-chain-locked).** Do **not** `curl | sh` at build or first
   boot. Instead pre-stage a pinned Nix release into the rootfs via a new build step:
   fetch the official static/portable Nix tarball through `build/01-fetch.sh`, lock its
   hash via the `.next-lock` → `checksums.txt` flow (human-verified, as with systemd/shadow),
   and unpack `/nix` + the `nix` binaries during staging. This keeps Nix install fully
   offline and reproducible.
3. **Pin nixpkgs + flakes.** Ship `/etc/nix/nix.conf` with a pinned `nixpkgs` channel/flake
   revision and `experimental-features = nix-command flakes`. Document how to bump the pin.
4. **TLS + network.** Stage `cacert` (CA bundle) and point `NIX_SSL_CERT_FILE`/`SSL_CERT_FILE`
   at it so Nix can fetch from `cache.nixos.org`. **Depends on Phase 12** (working network +
   DNS) for substituters.
5. **User profile on PATH.** Add `~/.nix-profile/bin` to `PATH` via `/etc/skel`'s
   `.bash_profile`/`.profile` (Phase 13), and source `nix-daemon.sh`/`nix.sh` profile script
   as appropriate for single-user.
6. **Scope boundary (enforce in docs).** Nix installs **apps only**. The boot path, kernel,
   Xorg, i3More, and the systemd substrate stay source-built and are never replaced by Nix.
   No `default.nix`/derivations authored in-repo — consume `nixpkgs` as-is.
7. **Disk/footprint note.** The Nix store can grow large; document `nix store gc` and the
   interaction with the Phase 11 root/`/home` sizing (consider putting `/nix` on root or a
   dedicated dataset).

## Deliverable

A booted system where `nix --version` works and `nix profile install <pkg>` fetches from the
pinned nixpkgs and makes the program runnable from the desktop — Nix bootstrapped offline
from a hash-locked tarball, no bespoke packaging code.

## Acceptance criteria

- `nix --version` reports the pinned version; `/nix` is single-user owned.
- With network up (Phase 12): `nix profile install nixpkgs#ripgrep` succeeds and `rg` runs in
  a terminal under i3More.
- `nix.conf` shows the pinned nixpkgs rev + flakes enabled; reinstalling at the same pin is
  reproducible.
- No `*.nix` package definitions exist in this repo (consumption only).

## References

- Memory `project-writeonce-scope` — "Package management: Nix (single-user), adopted
  wholesale."
- `build/01-fetch.sh` + `build/checksums.txt` — the supply-chain `.next-lock` flow to reuse
  for the Nix tarball.
- Phase 12 (network/DNS prerequisite), Phase 13 (`/etc/skel` PATH wiring).
- upstream Nix manual — single-user install + `nix profile` + flakes.

## Risks

- **First-boot network dependency.** Nix is useless without Phase 12; gate accordingly.
- **Store permissions.** Single-user `/nix` ownership must match the installer-created user
  (UID 1000); the installer should `chown` `/nix` if the username differs from `writeonce`.
- **Pin staleness.** A pinned nixpkgs ages; document the bump-and-verify cadence.
