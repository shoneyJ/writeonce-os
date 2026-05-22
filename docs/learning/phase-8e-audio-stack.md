# Phase 8e — the audio stack

> Companion to [`../../build/12-audio-stack.sh`](../../build/12-audio-stack.sh).
> Explains the four-package audio stack (Lua + ALSA + PipeWire + WirePlumber)
> and the kernel-side prerequisites already wired into the Phase 0/2 config.

## The audio system at a glance

```
                  ┌─────────────────────────────────────┐
                  │   Apps (i3more-audio, browsers,     │
                  │   `alacritty bell`, mpv, …)         │
                  └────────────────────┬────────────────┘
                                       │ PipeWire native API,
                                       │ or PulseAudio-compat, or ALSA-compat
                                       ▼
                  ┌─────────────────────────────────────┐
                  │           pipewire (daemon)         │
                  │  Audio + video graph, IPC over     │
                  │  /run/user/$UID/pipewire-0          │
                  └──┬────────────┬───────────────────┬─┘
                     │            │                   │
                     │            │   policy + routing│
                     │            │   via Lua scripts │
                     │            ▼                   │
                     │     ┌────────────────┐         │
                     │     │ wireplumber    │         │
                     │     │ (session-mgr)  │         │
                     │     └────────────────┘         │
                     │                                │
                     │ ALSA backend                   │ udev (device hotplug)
                     ▼                                ▼
                  ┌─────────────────────────────────────┐
                  │     Linux kernel (ALSA core)         │
                  │  - /dev/snd/pcmC0D0p (playback)      │
                  │  - /dev/snd/pcmC0D0c (capture)       │
                  │  - /dev/snd/controlC0 (mixer)        │
                  │  snd_hda_intel driver (CONFIG_SND_HDA_INTEL=y) │
                  └─────────────────────────────────────┘
```

i3More's audio applet (`i3more-audio`) talks to PipeWire directly via
the PipeWire client library, which is part of the `pipewire` package.
WirePlumber handles defaults — which sink is "the speakers", which
source is "the microphone", how to react when headphones plug in.

## ALSA — kernel side first, library second

ALSA is two distinct things:

1. **The kernel subsystem** (`sound/`, `CONFIG_SND_*`). Already enabled
   in `build/kernel-config-additions.fragment`: `CONFIG_SND=y`,
   `CONFIG_SND_HDA_INTEL=y` for the T450's HDA codec. The kernel
   creates `/dev/snd/*` device nodes on boot.

2. **The userspace library** (`alsa-lib`). What this round builds.
   Wraps the kernel device-node ioctls in a friendlier C API
   (`snd_pcm_open`, `snd_mixer_open`, etc.). PipeWire's `spa-alsa`
   plugin uses it to enumerate cards and stream audio.

`alsa-lib` is small (~3 MB installed). The build is plain autotools
with `--disable-static --disable-python`. Python bindings (pyalsa) and
the alsamixer / aplay CLI tools come from `alsa-utils` which we don't
build here — diagnostic tooling can come from `nixpkgs#alsa-utils` if
needed.

## Why PipeWire (not PulseAudio, not JACK)

Three reasons:

- **Single daemon for audio + video.** PipeWire handles both audio
  graphs and v4l2/screen capture. Future plans (screen sharing in
  browsers, video conferencing) won't need a separate stack.
- **Per-stream routing.** A single application can have multiple
  streams routed to different outputs (game audio to speakers, voice
  chat to headset) — PulseAudio struggled with this; PipeWire was
  designed for it.
- **Modern Wayland/Flatpak alignment.** Even though WriteOnce is X11,
  the broader Linux desktop trajectory uses PipeWire. Aligning means
  apps "just work" without per-app PulseAudio shims.

WirePlumber's relationship to PipeWire is similar to systemd's
`systemd-resolved` vs `systemd` — PipeWire is the kernel-of-userspace
audio routing engine; WirePlumber is the higher-level policy daemon
that tells PipeWire what to do when devices appear and disappear.
Separation of mechanism (PipeWire) and policy (WirePlumber) keeps
PipeWire small and stable while WirePlumber's Lua-based policy can
change without recompiling the engine.

## Lua — the policy engine

WirePlumber expresses its routing decisions as Lua 5.4 scripts:

```
/usr/share/wireplumber/main.lua.d/
├── 00-default-static-objects.lua     ← built-in nodes
├── 50-alsa-config.lua                ← "give every ALSA card a sink"
├── 60-default-device.lua             ← "prefer the most recently-active sink"
└── 90-enable-all.lua                 ← "expose everything to clients"
```

Hot-swap policy: change a Lua file, restart wireplumber service via
`wo-ctl restart wireplumber.service`, new policy is live.

We build Lua with the `linux` Makefile target (it has its own build
system, no autotools). Then we write a `pkg-config` file
(`lua5.4.pc`) by hand — Lua's upstream doesn't ship one, and
wireplumber's meson search probes via pkg-config.

## What the build deliberately skips

| Disabled | Cost saved | Replacement plan |
| --- | --- | --- |
| **bluez5** (Bluetooth audio) | ~20 min compile, +30 MB deps | Phase 8f or later — bluez is its own can of worms (D-Bus services, bluetoothd, pairing). Skip until needed. |
| **gstreamer** | ~1 hour compile, hundreds of MB | GTK4 also has `-Dmedia-gstreamer=disabled`; matched here. Use `mpv` (from nixpkgs) or browser media for video playback. |
| **avahi** | ~30 min compile | mDNS/Zeroconf audio (AirPlay-style network speakers). Edge case. |
| **JACK API compat** | small | JACK is for pro-audio low-latency workflows. Developer-workstation rarely needs it. |
| **Vulkan in pipewire** | -- | pipewire's video acceleration. We don't compose video on the GPU here. |
| **libcamera** | medium | Webcams. Add when video conferencing becomes a requirement. |
| **systemd / elogind integration** | small | wireplumber's autostart via systemd-user services. We use `wo-ctl` user-mode services instead (Phase 9 territory). |
| **echo-cancel-webrtc** | medium | WebRTC echo cancellation for voice calls. Browser-side WebRTC has its own. |

## Service-unit shapes for Phase 9

When Phase 9 wires the userspace into `writeonce-svc`, the units look
roughly like this:

```toml
# /etc/writeonce/services/pipewire.service.toml
[unit]
description = "PipeWire audio + video server"
after       = ["dbus.service"]

[service]
type        = "simple"
exec-start  = "/usr/bin/pipewire"
restart     = "on-failure"
restart-sec = "5s"
user        = "writeonce"   # per-user, not system-wide
group       = "writeonce"
slice       = "user.slice"

[install]
wanted-by   = ["graphical.target"]
```

```toml
# /etc/writeonce/services/wireplumber.service.toml
[unit]
description = "PipeWire session manager"
after       = ["pipewire.service"]
requires    = ["pipewire.service"]

[service]
type        = "simple"
exec-start  = "/usr/bin/wireplumber"
restart     = "on-failure"
restart-sec = "5s"
user        = "writeonce"
group       = "writeonce"
slice       = "user.slice"

[install]
wanted-by   = ["graphical.target"]
```

PipeWire is **per-user**, not system-wide — each logged-in user gets
their own daemon under `/run/user/$UID/pipewire-0`. Mirrors how modern
Linux desktops handle audio (systemd user manager spawns it per
session). Our supervisor will need a "user-slice" mechanism in
Round 2d to spawn services as the logged-in user; for v1 we can use a
hack: the `wo-login` session-start script invokes `pipewire &
wireplumber &` directly before launching i3.

## Kernel prerequisites (already in place)

```
CONFIG_SND=y                        ← sound core
CONFIG_SND_HDA_INTEL=y              ← Broadwell HDA codec (T450)
CONFIG_SND_USB_AUDIO=m              ← USB headsets / DACs (loaded on demand)
CONFIG_SND_SEQUENCER=y              ← MIDI sequencer (used by some apps)
```

These are already in `build/kernel-config-additions.fragment` — confirmed by
the Phase 2 builds.

For Bluetooth audio (future Phase 8e-bis): `CONFIG_BT_BNEP=m`,
`CONFIG_BT_HCIBTUSB=m`, plus `bluez` userspace and `bluez-alsa` bridge.

## Build times

| Package | Time |
| --- | --- |
| lua          | ~30 s   |
| alsa-lib     | ~2 min  |
| pipewire     | ~10 min |
| wireplumber  | ~5 min  |
| **Total**    | **~20 min** |

The smallest substrate round so far. Use `--no-network`.

## What's in $LFS/usr after this round

```
/usr/bin/
  pipewire           pw-cli           pw-cat           pw-link
  wireplumber        wpctl            wpexec
  lua               luac

/usr/lib/
  liblua.a, liblua.so → lua-5.4
  libasound.so.2     libatopology.so.2  libsndinfo.so.0
  libpipewire-0.3.so.0   ← ~3 MB
  libspa-0.2/                          ← plugins (alsa, audio, jack, …)
  libwireplumber-0.5.so.0
  pipewire-0.3/                        ← pipewire SPA + modules
  wireplumber-0.5/                     ← Lua scripts loaded at startup

/usr/share/
  pipewire/pipewire.conf               ← daemon config (overridable in /etc)
  wireplumber/                         ← default Lua policy
  alsa/                                ← alsa-lib defaults
```

About **20–30 MB** added on top of Phase 8d, bringing the running total
to ~380 MB of native userspace.

## Verifying it works

After all phases complete and the system is booted (Phase 9 + later),
the audio path is testable end-to-end:

```bash
wpctl status                          # show pipewire graph + default sink
wpctl set-default <sink-id>           # change default output
pw-cat --playback /usr/share/sounds/alsa/Front_Center.wav
                                      # plays test sound through pipewire → alsa-lib → kernel → speakers
```

i3More's audio applet calls PipeWire's D-Bus interface to set volume
and mute. That works once both pipewire + wireplumber + dbus services
are up under writeonce-svc.

## Future hardening passes

These are deliberate Phase-8e omissions worth queuing as separate rounds:

1. **bluez + bluez-alsa** — Bluetooth headset audio. Adds bluez daemon
   + D-Bus interfaces + udev rules. Medium-sized round on its own.
2. **alsa-utils** — `alsamixer`, `aplay`, `amixer`. Diagnostic tools.
   Small.
3. **libcamera + pipewire-libcamera** — when the webcam-on-the-T450
   needs to deliver frames to apps. Future.
4. **MIDI** (`alsa-utils` + maybe `fluidsynth`) — for users doing
   audio production. Out of scope for v1.

For v1 — speaker output, microphone capture, USB headset — the four
packages here are enough.
