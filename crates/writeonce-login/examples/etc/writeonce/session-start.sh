#!/bin/sh
# /etc/writeonce/session-start.sh — invoked by writeonce-login after PAM
# authentication succeeds, running as the authenticated user with HOME,
# USER, XDG_RUNTIME_DIR, PATH already set.
#
# Decide whether to launch a graphical session (i3 + i3More) or a
# straight login shell, based on the requested target and the tty.

set -e

mkdir -p "$XDG_RUNTIME_DIR"
chmod 0700 "$XDG_RUNTIME_DIR" 2>/dev/null || true

# If we're on tty1 and the user has an .xinitrc, launch X.
if [ "$(tty)" = "/dev/tty1" ] && [ -x /usr/bin/startx ]; then
    exec /usr/bin/startx /etc/writeonce/xinitrc -- :0 vt1 -keeptty
fi

# Otherwise, drop to the user's shell.
exec "$SHELL" -l
