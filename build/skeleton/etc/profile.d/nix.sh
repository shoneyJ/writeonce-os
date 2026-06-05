# /etc/profile.d/nix.sh — single-user Nix environment for WriteOnce (Phase 14).
# /nix is bootstrapped offline (17a-install-nix.sh) and registered on first boot
# (writeonce-nix-init.service); this only puts the Nix profiles on PATH and
# points TLS at the cacert bundled in the Nix closure. Sourced by ~/.bash_profile.

if [ -e /nix/var/nix/profiles/default/bin ]; then
    PATH="/nix/var/nix/profiles/default/bin:$PATH"
fi
if [ -d "$HOME/.nix-profile/bin" ]; then
    PATH="$HOME/.nix-profile/bin:$PATH"
fi
if [ -d "$HOME/.local/state/nix/profiles/profile/bin" ]; then
    PATH="$HOME/.local/state/nix/profiles/profile/bin:$PATH"
fi
export PATH

# Substituter TLS: reuse the nss-cacert bundled in the Nix closure (its store
# path carries a hash, so resolve it at runtime rather than hard-coding).
if [ -z "${NIX_SSL_CERT_FILE:-}" ]; then
    for _c in /nix/store/*-nss-cacert-*/etc/ssl/certs/ca-bundle.crt; do
        [ -r "$_c" ] && { NIX_SSL_CERT_FILE="$_c"; export NIX_SSL_CERT_FILE; break; }
    done
    unset _c
fi
