# Login shell profile, sourced for interactive logins — the greeter's "Shell
# (bash)" session and the tty2–tty6 getty logins. Graphical session selection
# happens at the greeter (writeonce-greeter); this file just sets up the
# environment for a console shell. It is also sourced when the greeter launches
# a compositor via `bash -lc 'exec <compositor>'`, which is how the Nix profile
# lands on PATH so a bare command (e.g. `sway`) resolves.
[ -f ~/.bashrc ] && . ~/.bashrc

# Single-user Nix profile on PATH (present once Nix is bootstrapped).
if [ -e "$HOME/.nix-profile/etc/profile.d/nix.sh" ]; then
    . "$HOME/.nix-profile/etc/profile.d/nix.sh"
elif [ -e /etc/profile.d/nix.sh ]; then
    . /etc/profile.d/nix.sh
fi
[ -d "$HOME/.nix-profile/bin" ] && PATH="$HOME/.nix-profile/bin:$PATH"
