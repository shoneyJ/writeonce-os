# Login shell profile. getty autologin → login → bash (this file).
#
# WriteOnce is a compositor-agnostic base: it boots to a console with the network
# up, mDNS advertising `writeonce.local`, and Nix ready. It does NOT ship or launch
# a desktop — you install the Wayland compositor of your choice (sway / hyprland /
# wayfire) via Nix and launch it yourself, bringing your own config.
[ -f ~/.bashrc ] && . ~/.bashrc

# Single-user Nix profile on PATH (present once Nix is bootstrapped).
if [ -e "$HOME/.nix-profile/etc/profile.d/nix.sh" ]; then
    . "$HOME/.nix-profile/etc/profile.d/nix.sh"
elif [ -e /etc/profile.d/nix.sh ]; then
    . /etc/profile.d/nix.sh
fi
[ -d "$HOME/.nix-profile/bin" ] && PATH="$HOME/.nix-profile/bin:$PATH"

# First console login on tty1: print a short orientation, then drop to the shell
# (no auto-launch — the compositor is your choice).
if [ -z "${WAYLAND_DISPLAY:-}" ] && [ -z "${DISPLAY:-}" ] && [ "$(tty)" = "/dev/tty1" ]; then
    cat <<'EOF'
WriteOnce — compositor-agnostic console base.
  SSH in from your workstation:   run `sudo wo-sshd-setup` once, then
                                  `ssh writeonce@writeonce.local` from the LAN.
  Install a Wayland compositor:   nix profile add github:NixOS/nixpkgs/nixos-unstable#sway
                                  (or #hyprland / #wayfire), then launch it: `exec sway`.
EOF
fi
