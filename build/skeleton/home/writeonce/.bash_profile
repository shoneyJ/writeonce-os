# DIAGNOSTIC image: auto-startx is disabled so a GPU/Xorg hang can't take the
# console with it. getty autologin → login → bash (this file) → shell prompt.
# Launch the desktop manually with `startx` to capture its log/error.
# Once X is confirmed working, re-enable the auto-startx below and flip
# default.target → graphical.target.
[ -f ~/.bashrc ] && . ~/.bashrc

if [ -z "${DISPLAY:-}" ] && [ "$(tty)" = "/dev/tty1" ]; then
    echo "WriteOnce (diagnostic mode): run 'startx' to launch the i3More desktop."
    # exec startx   # re-enable once Xorg/i915 is confirmed working
fi
