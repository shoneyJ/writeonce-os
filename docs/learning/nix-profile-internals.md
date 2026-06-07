# What `nix profile install` actually does

This note unpacks what `nix profile install <flakeref>` does behind the scenes,
single-user, no daemon. (WriteOnce has since moved the desktop install to **Home
Manager** — see [`home-manager-and-devshell.md`](home-manager-and-devshell.md) — but
HM builds a user profile generation by the *same* mechanism described here, so this
remains the reference for how Nix profiles work.)

## The store, in one paragraph

Everything Nix builds or downloads lands in `/nix/store/<hash>-<name>/`, where
`<hash>` is derived from *all inputs* (sources, deps, build flags). Same inputs →
same path; different inputs → different path. So paths are **immutable** and
**content-addressed-ish**: many versions coexist, nothing is overwritten, and
two packages sharing a dependency share one store path. A SQLite DB at
`/nix/var/nix/db/db.sqlite` records every valid path + its references (its
runtime closure).

## `nix profile install <flakeref>` step by step

1. **Resolve + evaluate.** The installable (`path:/etc/writeonce/desktop`, a
   flake) is fetched and its Nix expression *evaluated* to a **derivation** and
   its output store path(s). Our flake's output is a `buildEnv` that bundles
   hyprland/quickshell/alacritty/… into one path.

2. **Realize (build or substitute).** For each needed store path not already
   present, Nix asks the **substituters** in `nix.conf`
   (`substituters = https://cache.nixos.org`, verified by `trusted-public-keys`)
   for a prebuilt, signed **NAR** archive and unpacks it into `/nix/store`. Only
   if no substitute exists does it build from source. Each path is registered in
   the DB. (This is why "installing" is usually *downloading* — and why first
   boot needs network.)

3. **New profile generation.** A *profile* is a chain of generations. `install`
   computes a new set of installed elements, records them in the profile's
   **`manifest.json`**, and realizes a new **generation**: itself a store path —
   a `buildEnv` symlink-forest merging every installed package's `bin/`, `share/`,
   `lib/`, … into one tree.

4. **Atomic switch.** Nix creates `…/profiles/profile-<N>-link` → that generation
   store path, then atomically flips the `…/profiles/profile` symlink to it.
   Because it's a single `rename(2)` of a symlink, an interrupted install never
   leaves a half-state — you either get generation N or you don't. The user-facing
   `~/.nix-profile` (classic) / `$XDG_STATE_HOME/nix/profiles/profile`
   (`~/.local/state/nix/profiles/profile`, modern) points at the current
   generation; `/etc/profile.d/nix.sh` puts `…/profile/bin` on `PATH`, so
   `Hyprland`, `qs`, `alacritty` appear.

5. **GC roots + rollback.** Profile generations are **GC roots** (symlinked under
   `/nix/var/nix/gcroots/`), so `nix store gc` deletes only paths *no* generation
   references — your installed software is safe. Old generations are kept until
   GC'd, so `nix profile rollback` / `nix profile history` flip the symlink back:
   instant, atomic rollback.

## Why declarative (a flake) beats the imperative loop

The old `wo-session` looped `nix profile install nixpkgs#<pkg>` per line — N
separate generations, and "nixpkgs" resolved to *whatever* the registry pointed
at (unpinned). The flake (`/etc/writeonce/desktop/flake.nix`) is:
- **one** generation (atomic — all-or-nothing), and
- **pinned** by `flake.lock` (the exact nixpkgs commit + NAR hash), so the same
  command yields the same desktop on any machine, any day.

## Where it lives in WriteOnce

- `writeonce-nix-init.service` (first boot) seeds the single-user store DB
  (`nix-store --load-db < /nix/.reginfo`) and the **default** profile
  (`/nix/var/nix/profiles/default`, the bundled `nix` itself).
- `wo-session` then realizes the **desktop** flake into the *user* profile.
- `/nix` is owned by the `writeonce` user — single-user, daemonless; there is no
  `nix-daemon`, no build-users group.

## Inspecting it on the T450

```sh
nix profile list                 # installed elements + their store paths
nix profile history              # generations (rollback targets)
nix path-info -rsh ~/.nix-profile  # closure + sizes
ls -l ~/.local/state/nix/profiles/   # the generation symlinks + current `profile`
nix store gc                     # collect unreferenced paths (keeps profile roots)
```
