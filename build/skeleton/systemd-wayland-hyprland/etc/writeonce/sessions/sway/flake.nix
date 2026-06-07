# /etc/writeonce/sessions/sway/flake.nix — WriteOnce "Sway" session template.
#
# A SELECTION MANIFEST (not a package definition): it picks Sway + a minimal,
# usable set out of nixpkgs and bundles them into one profile generation.
# Consistent with the project scope (consume nixpkgs as-is, no bespoke
# packaging). Install into your user profile, then pick "Sway" at the greeter:
#
#     nix profile add path:/etc/writeonce/sessions/sway
#
# Sway's nixpkgs package ships share/wayland-sessions/sway.desktop, which lands
# in ~/.nix-profile/share/wayland-sessions and is what makes "Sway" appear in
# the greeter's session list. Edit `paths` to taste. For reproducibility run
# `nix flake lock` in this directory (writes flake.lock pinning the nixpkgs rev).
{
  description = "WriteOnce session: Sway (Wayland)";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      system = "x86_64-linux";
      pkgs = nixpkgs.legacyPackages.${system};
    in {
      # `nix profile add path:/etc/writeonce/sessions/sway` installs this.
      packages.${system}.default = pkgs.buildEnv {
        name = "writeonce-session-sway";
        paths = with pkgs; [
          sway                       # the compositor (ships sway.desktop)
          swaybg swayidle swaylock   # wallpaper / idle / screen lock
          foot                       # terminal
          wmenu                      # dmenu-style launcher (sway's default $menu)
          wl-clipboard               # wl-copy / wl-paste
          grim slurp                 # screenshots + region select
          mako                       # notification daemon
          xdg-desktop-portal-wlr     # screenshare / file portals (wlroots)
        ];
      };
    };
}
