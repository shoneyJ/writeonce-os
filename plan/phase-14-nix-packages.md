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

## Apps not in nixpkgs — pinned flakes (escape hatch)

Not every app lives in nixpkgs. **Zen Browser** is the canonical example:
as of 2026 it is *not* in nixpkgs and is distributed only through
community Nix flakes (e.g. `github:youwen5/zen-browser-flake`, which
wraps Zen's official binary via `wrapFirefox`). The "consume nixpkgs
as-is" rule (subtask 6) can't cover these, so the policy is:

- **Default path stays nixpkgs.** `nix profile install nixpkgs#<pkg>`
  for anything in the pinned channel (ripgrep, firefox, librewolf,
  chromium, …). This is the supply-chain-locked happy path.
- **Escape hatch — a *pinned* third-party flake** when, and only when,
  an app is absent from nixpkgs:
  ```sh
  nix profile install github:youwen5/zen-browser-flake
  ```
  This is still **consumption, not authoring** — no `*.nix` is written
  in this repo, so the acceptance criterion below holds. But it widens
  the trust set beyond the single pinned nixpkgs rev (now also: the
  flake repo, its update bot, and the app's binary CDN). Therefore:
  - **Pin the flake rev** (record it in a documented list, the same
    spirit as `checksums.txt` for source tarballs) so reinstalls are
    reproducible and an upstream change can't silently alter the build.
  - **Treat each pinned flake as a tracked supply-chain entry** — it
    gets the same bump-and-verify cadence as the nixpkgs pin (subtask 3)
    and the Nix tarball (subtask 2).
- **Non-NixOS note.** WriteOnce is an FHS/LFS host, not NixOS. Nix apps
  still run because their closure is self-contained and `autoPatchelf`'d
  against store libs — they do **not** depend on WriteOnce's source-built
  system GTK/X11. The one runtime caveat is GPU/GL accel: a Nix browser
  may need the host Mesa (Phase 8) reached via FHS paths, or a `nixGL`
  wrapper for hardware video decode. Basic rendering works without it.

**Tier boundary (made explicit).** WriteOnce has two package tiers, and
this plan governs only tier 2:
- **Tier 1 — substrate** (kernel, glibc, Xorg, systemd, i3/i3More, the
  Rust crates, *and the fallback terminal*): source-built by
  `build/0N-*.sh`, never from Nix. This is LFS's "rebuild the system
  from source" technique, mechanized.
- **Tier 2 — user apps** (editors, browsers, CLI tools): Nix, per this
  plan — nixpkgs first, pinned flake as the documented exception.

A useful litmus test: if pressing `mod+Return` must open a terminal on
a *fresh* install (before any Nix app exists), that terminal is tier 1.
Anything the user chooses to add afterward is tier 2.

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
- No `*.nix` package definitions exist in this repo (consumption only) —
  pinned third-party flakes count as consumption and are permitted.
- An app absent from nixpkgs installs from a pinned flake and runs under
  i3More: `nix profile install github:youwen5/zen-browser-flake` →
  `zen` launches on the X11 desktop.

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
