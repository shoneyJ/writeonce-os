# /etc/writeonce/desktop/flake.nix — WriteOnce Tier-2 desktop set, declarative.
#
# This is a SELECTION MANIFEST, not a package definition: it only picks packages
# out of nixpkgs and bundles them into one environment. No derivations are
# authored here — consistent with the project scope ("consume nixpkgs as-is, no
# bespoke packaging"). The dev-workstation plan sanctions exactly this
# (/etc/writeonce/profile.nix as a default-profile flake).
#
# Realized by wo-session on first boot:
#     nix profile install path:/etc/writeonce/desktop
# installing the default package (the buildEnv) as ONE atomic profile generation.
#
# Pinning: the nixpkgs input ref below is locked precisely by flake.lock. No
# flake.lock is committed yet (generating one needs `nix` + network, absent on
# the build host); the first realize on the target writes one — run
# `nix flake lock` and commit the resulting flake.lock for full reproducibility.
{
  description = "WriteOnce desktop (Hyprland + Quickshell)";

  # hyprland + quickshell track nixpkgs-unstable. Pinned exactly by flake.lock.
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      system = "x86_64-linux";
      pkgs = nixpkgs.legacyPackages.${system};
    in {
      # `nix profile install path:/etc/writeonce/desktop` installs this default.
      packages.${system}.default = pkgs.buildEnv {
        name = "writeonce-desktop";
        paths = with pkgs; [
          hyprland                     # Wayland compositor
          quickshell                   # Qt/QML shell — the bar (qs -c wo)
          kitty                        # terminal ($term in hyprland.conf)
          xdg-desktop-portal-hyprland  # portals: screenshare / file pickers
          wl-clipboard                 # wl-copy / wl-paste
        ];
      };
    };
}
