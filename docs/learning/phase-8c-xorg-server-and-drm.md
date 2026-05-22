# Phase 8c — Xorg server, DRM/KMS, and the input chain

> Companion to [`../../build/10-xorg-server.sh`](../../build/10-xorg-server.sh).
> Explains what each of the ten packages contributes, the DRM/KMS
> picture, the input pipeline from the kernel to X, and why we
> deliberately disable Mesa's heavyweight options.

## What we add this round

```
                  ┌─────────────────────────────────────┐
                  │            X11 clients              │
                  │   (i3, i3More, terminals, GTK4)     │
                  └────────────────────┬────────────────┘
                                       │ X protocol over Unix-socket
                                       ▼
                  ┌─────────────────────────────────────┐
                  │           xorg-server               │
                  │  + glamor (GL-accelerated rendering)│
                  │  + xf86-input-libinput (input)      │
                  │  + modesetting driver (no -intel)   │
                  └──┬───────────────┬──────────────────┘
                     │               │
                     │ DRI3 via      │ via udev events
                     │ libGL/libgbm  │ from /dev/input/event*
                     ▼               ▼
                  ┌──────┐      ┌──────────────────┐
                  │ Mesa │      │  libinput        │
                  │ iris │      │  ↑               │
                  │ ── + │      │  libevdev + mtdev│
                  │ libdrm      │  ↑               │
                  └──┬───┘      │  kernel evdev    │
                     │          └────────┬─────────┘
                     │                   │
                     ▼                   ▼
                  ┌─────────────────────────────────────┐
                  │     Linux kernel (DRM + input)      │
                  │  i915 driver → /dev/dri/card0       │
                  │  input subsystem → /dev/input/event*│
                  └─────────────────────────────────────┘
```

After this round the sysroot has everything needed to start an X
session manually — `Xorg :0 vt7 -keeptty` works as long as the kernel
is up.

## DRM / KMS — what libdrm gives us

**DRM** (Direct Rendering Manager) is the kernel's umbrella for GPU
access. **KMS** (Kernel Mode-Setting) is the part of DRM responsible
for configuring display outputs (resolution, framebuffer, hot-plug
detection). On the T450 the i915 kernel driver provides both, exposing:

```
/dev/dri/card0           ← the GPU device node (KMS commands + render submit)
/dev/dri/renderD128      ← render-only node (no KMS; used by GBM in mesa)
/sys/class/drm/card0/*   ← connectors, modes, fb info
```

`libdrm` is the userspace wrapper around DRM ioctls. We disable every
vendor-specific subdir except `intel=enabled` to keep the binary small
and the surface focused.

X's **modesetting** driver (built into xorg-server as of 21.1.x — no
separate `xf86-video-intel` needed) talks to KMS through libdrm. It
asks libdrm for:

- Available CRTCs and connectors (what display outputs exist).
- Modes per connector (what resolutions are supported).
- Framebuffer allocation (memory the GPU scans out to the screen).
- Atomic mode-set submission (flicker-free resolution changes).

## Why we need Mesa at all for an X server

The X server's **glamor** acceleration backend uses **OpenGL** to draw.
That requires:

- **libGL** — the OpenGL API.
- **libEGL** — the platform-binding layer (X11 + DRI3 → GL context).
- **libgbm** — Generic Buffer Management; how to allocate GPU buffers
  the X server can then submit through libdrm.
- **The iris driver** — Mesa's modern driver for Intel gen8 (Broadwell)
  and newer. Translates GL calls into Intel hardware commands.

Without Mesa, the X server falls back to software rendering (slow).
With Mesa + glamor, even basic compositing (xdamage, redirected
windows) goes through the GPU.

## Mesa configuration choices

The `step_mesa` invocation in `10-xorg-server.sh` deliberately reduces
Mesa's surface area:

| Option | Reason |
| --- | --- |
| `-Dgallium-drivers=iris` | Only the Intel gen8+ driver. No AMD radeonsi, no nouveau, no software rasterizer. |
| `-Dvulkan-drivers=` (empty) | No Vulkan — no Vulkan workload on the T450. Saves ~30 min of compile time. |
| `-Dplatforms=x11` | No Wayland support — we don't ship Wayland. |
| `-Dllvm=disabled` | The iris driver doesn't need LLVM (the radeonsi/llvmpipe drivers do). Saves a huge LLVM build. |
| `-Dgallium-omx=disabled`, `-Dgallium-va=disabled`, `-Dgallium-xa=disabled` | Video decode/encode acceleration via OMX / VA-API / XA; none are needed for the i3 + i3More desktop. |
| `-Dosmesa=false` | Offscreen software rendering; not needed. |
| `-Dglvnd=enabled` | Use the GL Vendor Neutral Dispatch library — the modern way to resolve `libGL.so`. |
| `-Dgles1=disabled`, `-Dgles2=enabled` | GTK4 needs GLES2; nothing needs GLES1 anymore. |
| `-Dgbm=enabled`, `-Ddri3=enabled` | Required by Xorg-server's glamor. |

Resulting Mesa installs roughly: `libGL.so.1`, `libEGL.so.1`,
`libGLESv2.so.2`, `libgbm.so.1`, plus the `iris_dri.so` driver in
`$LFS/usr/lib/dri/`. About 50 MB on disk.

## The input chain — kernel → libinput → Xorg

```
Hardware                             /dev/input/event0 (keyboard)
  ↓                                  /dev/input/event1 (touchpad)
Linux input subsystem                /dev/input/event2 (lid switch)
  ↓                                  …
        (kernel emits canonical events; /dev/input/eventN per device)
  ↓
        ┌─────────────────────────────────────────┐
        │            libevdev                      │
        │ Translates the raw evdev ioctl interface │
        │ into a friendlier C API                  │
        └────────────┬─────────────────────────────┘
                     │
        ┌────────────▼─────────────┐
        │       mtdev               │
        │ Translates "MT protocol A" │
        │ to "MT protocol B" for     │
        │ legacy multitouch devices  │
        └────────────┬───────────────┘
                     │
        ┌────────────▼─────────────────────────────┐
        │              libinput                      │
        │ - acceleration profile (flat/adaptive)     │
        │ - palm rejection                            │
        │ - tap-to-click, two-finger scroll, etc.    │
        │ - gesture detection (3-finger swipe, …)    │
        └────────────┬─────────────────────────────┘
                     │ events
        ┌────────────▼─────────────────────────────┐
        │     xf86-input-libinput                   │
        │ Xorg input driver — bridges libinput's    │
        │ events into the X11 core protocol         │
        │ (XInput2 events, button mapping, …)       │
        └────────────┬─────────────────────────────┘
                     │
                  X clients (i3, i3More, GTK4 apps)
```

Each link in the chain is replaceable in theory (libinput could be
replaced with synaptics for touchpads, libevdev with raw ioctls, …) but
the listed stack is the modern norm and what i3More expects.

The T450's touchpad is detected by the kernel as `psmouse:synaptics`,
exposed at `/dev/input/event2` or similar; libinput probes it via udev,
applies tap-to-click + two-finger scroll, hands events to
xf86-input-libinput, which sends XInput2 events to Xorg, which routes
them to focused i3 workspace.

## Why no xf86-video-intel

The xorg-server's modern modesetting driver supersedes the older
`xf86-video-intel`. modesetting:

- Uses KMS + libdrm directly (no Intel-specific code in the X server).
- Uses glamor for acceleration (calls into Mesa's iris driver via
  OpenGL / EGL).
- Hot-plugs cleanly on display connector changes.

`xf86-video-intel` is still maintained but is becoming a legacy code
path; new desktops should use modesetting. WriteOnce skips it
deliberately — fewer moving parts.

## Why no systemd-logind support in Xorg

We pass `-Dsystemd_logind=false` to xorg-server. The reasoning:

- WriteOnce doesn't ship systemd-logind (Round 2d is on the roadmap for
  a minimal logind shim built into the Rust supervisor, but the
  upstream `xorg-server`'s `systemd_logind=true` would expect to talk
  to *real* logind via D-Bus, not our shim).
- Without logind, Xorg uses the **setuid wrapper** (`/usr/libexec/Xorg.wrap`)
  to drop privileges after grabbing /dev/dri and /dev/input. Standard,
  well-trodden path.
- The `wo-login` PAM stack (Phase 9) handles seat assignment manually
  via PAM `pam_loginuid` if needed.

If we later add the Round 2d D-Bus logind shim and want xorg-server to
talk to it, we'd recompile with `-Dsystemd_logind=true` — small change,
not a structural shift.

## Estimated compile time

Single-thread per package on a modern workstation in `wo-builder`:

| Package | Time |
| --- | --- |
| libdrm           | ~30 s    |
| libpciaccess     | ~10 s    |
| libXfont2        | ~30 s    |
| libepoxy         | ~30 s    |
| libevdev         | ~20 s    |
| mtdev            | ~5 s     |
| libinput         | ~1 min   |
| **mesa**         | **~45–60 min** |
| **xorg-server**  | **~15–25 min** |
| xf86-input-libinput | ~10 s |
| **Total**        | **~60–90 min** |

Use `./build/in-container.sh --no-network ./build/10-xorg-server.sh` so
nothing in the build can phone home.

For iterative debugging (the most likely round to need debugging given
Mesa's complexity):

```bash
# Run one step at a time:
./build/in-container.sh --no-network ./build/10-xorg-server.sh libdrm
./build/in-container.sh --no-network ./build/10-xorg-server.sh libpciaccess
…
./build/in-container.sh --no-network ./build/10-xorg-server.sh mesa
```

Each step's logs land in `build/logs/blfs-<name>-{setup,compile,install}.log`.

## What's in $LFS/usr after this round

Approximately:

```
/usr/bin/
  Xorg                              the X server binary (~5 MB)
  xkbcomp                           keymap compiler used by the server
  glxinfo, eglinfo                  diagnostic tools

/usr/lib/
  libdrm.so.2, libdrm_intel.so.1
  libpciaccess.so.0
  libGL.so.1                        Mesa OpenGL (~10 MB)
  libEGL.so.1
  libGLESv2.so.2
  libgbm.so.1
  libepoxy.so.0
  libevdev.so.2
  libmtdev.so.1
  libinput.so.10
  libXfont2.so.2

/usr/lib/dri/
  iris_dri.so                       Intel gen8+ driver (~30 MB)

/usr/lib/xorg/modules/
  libglamoregl.so
  libfb.so, libshadow.so, libwfb.so
  drivers/modesetting_drv.so        ← what makes the screen light up
  input/libinput_drv.so             ← xf86-input-libinput

/usr/share/X11/xkb/                 keymaps (already from Phase 8b)
```

Add ~80 MB to the sysroot total. Combined with Phase 8a (~30 MB) and
8b (~10 MB), we're at ~120 MB of native userspace so far. GTK4 in
Round 8d will roughly double that.

## After this round you can — manually — start X

In theory:

```bash
# from a tty inside the running WriteOnce system:
sudo Xorg :0 vt7 -keeptty &
sleep 2
DISPLAY=:0 xterm    # if/when xterm is built; not in scope here
```

In practice Round 8c gives you the *foundation* for an X session;
Round 8d (GTK4) lets you build apps that draw via GTK, and Round 9
brings up i3 + i3More to actually use the session. Without GTK4 there
are no clients to run; without a window manager there's nothing to
arrange them.
