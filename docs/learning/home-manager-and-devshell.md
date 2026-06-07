# Home Manager (.config) + the dev-env devShell

The Wayland flavor's user environment is managed by **Home Manager** — the Nix
tool for declaratively managing `~/.config` and per-user packages. The development
toolchain (vscode/neovim/tmux) is separate: a **`nix develop` devShell**. Both
live under `/etc/writeonce/`.

## Can Nix manage hypr / the .config files? — yes, via Home Manager

Home Manager evaluates a Nix module (`home.nix`) into a **home generation** — a
store path — and on `switch`:
- installs the user's packages into a per-user Nix profile, and
- **symlinks `~/.config/<app>` into the Nix store** (`~/.config/hypr/hyprland.conf`
  → a read-only store path, etc.). Generations are atomic + rollbackable, just
  like `nix profile` (see `nix-profile-internals.md`).

So `~/.config/{hypr,quickshell,nvim,alacritty,fish,starship,tmux}` are all produced
from `/etc/writeonce/home/home.nix` — no hand-edited dotfiles on the target.

## Layout

```
/etc/writeonce/home/          # Home Manager flake (manages the desktop + .config)
  flake.nix                   # inputs: nixpkgs (pinned) + home-manager (follows it)
  home.nix                    # packages + programs.* + xdg.configFile
  hypr/hyprland.conf          # thin Hyprland config ($term = alacritty)  → ~/.config/hypr
  quickshell/wo/shell.qml     # thin bar                                  → ~/.config/quickshell/wo
  nvim/                       # vendored LazyVim tree                     → ~/.config/nvim
/etc/writeonce/devshell/
  flake.nix                   # `nix develop` → vscode + neovim + tmux + toolchain
```

`home.nix` ports the light dotfiles faithfully (alacritty, fish, starship, tmux —
via `programs.*`) and keeps Hyprland + Quickshell **thin** (the distro's minimal
versions, not the heavy upstream "ii"). This **supersedes** the earlier
`desktop/flake.nix` + the raw skeleton `.config` files.

## Activation (single-user, no daemon)

`wo-session` builds + activates the generation on first boot, pinned by the flake's
own `flake.lock` (no unpinned `home-manager/master`):
```sh
hm=$(nix build --no-link --print-out-paths \
       /etc/writeonce/home#homeConfigurations.writeonce.activationPackage)
"$hm/activate"        # installs packages + writes the ~/.config symlinks
```
Then it re-reads `~/.nix-profile/bin` into PATH and `exec Hyprland`. Re-runs are an
idempotent re-switch. (Manual: `home-manager switch --flake /etc/writeonce/home#writeonce`.)

## The dev environment is a devShell, not in the profile

`vscode`, `neovim`, `tmux` + the toolchain (`git`, `lazygit`, `fzf`, `ripgrep`,
`fd`, `eza`, `lua-language-server`, `nil`) come from
`/etc/writeonce/devshell/flake.nix`, entered on demand:
```sh
nix develop /etc/writeonce/devshell      # or copy the flake into a project
```
This keeps the daily desktop light (HM installs only the compositor/shell/terminal),
while the editors appear — fully configured, since nvim/tmux read the HM-managed
`~/.config` — only inside `nix develop`.

## Caveats (honest)

- **HM-managed configs are read-only** (store symlinks). Edit the source in
  `/etc/writeonce/home/` and re-`switch`; don't edit `~/.config/*` in place.
- **Neovim is impure**: the vendored LazyVim config is faithful, but lazy.nvim +
  mason still fetch plugins + LSP servers at runtime on first launch (network).
  `nvim`'s config dir being read-only means `:Lazy sync` can't rewrite
  `lazy-lock.json` — update the lock in the source tree instead.
- **vscode** is a heavy Electron app — slowest to fetch and run on the HD 5500;
  for Wayland it wants `--ozone-platform=wayland`.
- **Target-gated:** none of this was run here (no Nix on the build host). First
  `home-manager switch` / `nix develop` happens on the T450 (W5).
- **stateVersion** in `home.nix` may need bumping if `home-manager switch` warns
  about a mismatch with the pinned `home-manager` input.
