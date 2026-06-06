# Login shell profile. getty autologin → login → bash (this file). On tty1 we
# source the single-user Nix profile and launch the Hyprland Wayland session.
# Hyprland, Quickshell and the terminal are Tier-2 Nix packages (see
# /etc/writeonce/desktop-packages); wo-session falls back to this shell on any
# failure, so a GPU/compositor problem can never take the only console with it.
[ -f ~/.bashrc ] && . ~/.bashrc

# Single-user Nix profile on PATH (present once Phase 14 has bootstrapped /nix).
if [ -e "$HOME/.nix-profile/etc/profile.d/nix.sh" ]; then
    . "$HOME/.nix-profile/etc/profile.d/nix.sh"
elif [ -e /etc/profile.d/nix.sh ]; then
    . /etc/profile.d/nix.sh
fi
[ -d "$HOME/.nix-profile/bin" ] && PATH="$HOME/.nix-profile/bin:$PATH"

if [ -z "${WAYLAND_DISPLAY:-}" ] && [ -z "${DISPLAY:-}" ] && [ "$(tty)" = "/dev/tty1" ]; then
    /usr/local/bin/wo-session \
        || echo "WriteOnce: desktop launch failed — see messages above; you are at a shell."
fi
