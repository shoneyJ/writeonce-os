# /etc/writeonce/sessions/hyprland/flake.nix — WriteOnce "Hyprland" session template.
#
# A SELECTION MANIFEST (not a package definition): it picks Hyprland + a
# minimal, usable set out of nixpkgs and bundles them into one profile
# generation. Install into your user profile, then pick "Hyprland" at the
# greeter:
#
#     nix profile add path:/etc/writeonce/sessions/hyprland
#
# Hyprland's nixpkgs package ships share/wayland-sessions/hyprland.desktop,
# which lands in ~/.nix-profile/share/wayland-sessions and is what makes
# "Hyprland" appear in the greeter's session list. Edit `paths` to taste. For
# reproducibility run `nix flake lock` here (writes flake.lock pinning nixpkgs).
{
  description = "WriteOnce session: Hyprland (Wayland)";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      system = "x86_64-linux";
      pkgs = nixpkgs.legacyPackages.${system};
    in {
      # `nix profile add path:/etc/writeonce/sessions/hyprland` installs this.
      packages.${system}.default = pkgs.buildEnv {
        name = "writeonce-session-hyprland";
        paths = with pkgs; [
          hyprland                       # the compositor (ships hyprland.desktop)
          hyprpaper hyprlock             # wallpaper / screen lock
          kitty                          # terminal
          wofi                           # launcher
          wl-clipboard                   # wl-copy / wl-paste
          grim slurp                     # screenshots + region select
          mako                           # notification daemon
          xdg-desktop-portal-hyprland    # screenshare / file portals
        ];
      };
    };
}
