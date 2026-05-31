#!/bin/sh
# ~/.xinitrc — invoked by startx after Xorg comes up.
#
# Starts per-user daemons (PipeWire + WirePlumber + i3More applets),
# then execs i3 as the X session leader. When i3 exits, the X session
# ends and the user returns to writeonce-login's prompt.

# ------- Graphics: point Mesa at the real DRI driver dir -------------------
# Xorg/Mesa baked the cross-build path ($LFS/usr/lib/dri) into libglx; override
# it so client GL (GTK4 / i3More) loads iris from the installed location.
# Proper fix is rebuilding Xorg/Mesa with dridriverdir=/usr/lib/dri.
export LIBGL_DRIVERS_PATH=/usr/lib/dri
# No accessibility bus (at-spi2) is running; tell GTK not to look for one
# (silences "Unable to acquire the address of the accessibility bus").
export GTK_A11Y=none

# ------- Audio -------------------------------------------------------------
pipewire    >"$HOME/.cache/pipewire.log"    2>&1 &
wireplumber >"$HOME/.cache/wireplumber.log" 2>&1 &
mkdir -p "$HOME/.cache"

# ------- Per-user D-Bus session bus ----------------------------------------
# Some i3More applets (audio, power) talk to the SYSTEM bus
# (writeonce-logind, pipewire) but a few (i3more-translate cache) also
# want a session bus. dbus-launch exports DBUS_SESSION_BUS_ADDRESS.
if command -v dbus-launch >/dev/null 2>&1; then
    eval "$(dbus-launch --sh-syntax --exit-with-session)"
fi

# ------- X input + cursor defaults -----------------------------------------
setxkbmap us
xset r rate 200 30
xsetroot -cursor_name left_ptr

# ------- i3More daemons (background) ---------------------------------------
# The "bar" replacement — main i3more binary positions itself on-screen.
i3more         >"$HOME/.cache/i3more.log"         2>&1 &
# Audio applet (volume / sink switch).
i3more-audio   >"$HOME/.cache/i3more-audio.log"   2>&1 &
# Optional applets — uncomment after install.
# i3more-power   >"$HOME/.cache/i3more-power.log"   2>&1 &
# i3more-keyhint >"$HOME/.cache/i3more-keyhint.log" 2>&1 &

# ------- Launch i3 (WM owns the rest of the X session) ---------------------
# Log i3's own output so a startup failure (e.g. font/config) is visible via
# `just pull-logs` instead of vanishing when the session tears down.
exec i3 >"$HOME/.cache/i3.log" 2>&1
