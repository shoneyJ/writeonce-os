# Phase 9 — desktop bring-up

> Companion to:
> - [`../../build/16-i3-and-applets.sh`](../../build/16-i3-and-applets.sh)
> - [`../../build/17-stage-sysroot.sh`](../../build/17-stage-sysroot.sh)
> - [`../../build/18-make-artifacts.sh`](../../build/18-make-artifacts.sh)
> - [`../../build/skeleton/`](../../build/skeleton/)
> - [`../../crates/writeonce-svc/examples/services/`](../../crates/writeonce-svc/examples/services/)
>
> Explains the boot chain from PID 1 to a working i3 desktop, why
> we deliberately keep some daemons user-spawned via `.xinitrc` rather
> than supervisor-managed, the layout of `/etc/writeonce/`, and what
> the user actually sees the first time they boot.

## How i3 + i3More slot into Phase 9

**writeonce-os does NOT build i3 or i3More.** Both are external
artifacts produced by i3More's own build pipeline. Phase 9 in this
repo is just **stage + bundle** — it copies pre-built binaries into
the sysroot.

This is the correct design for two reasons:

1. **i3More already has a complete, working build pipeline.** The
   `Dockerfile.i3` builds i3 (the forked WM) with Ubuntu apt deps;
   `docker compose run dev` builds the i3More Rust binaries. The
   operator runs these on their workstation as part of normal i3More
   development. Re-implementing the same recipes inside writeonce-os
   would be code duplication.

2. **ABI forward-compatibility.** Binaries built against Ubuntu 24.04
   glibc 2.39 + GTK4 4.14 run cleanly against WriteOnce's glibc 2.40
   + GTK4 4.16 (newer runtime libs are backward-compatible with
   older-build-time symbol versions). So there's no need to rebuild
   against the LFS substrate.

The two external artifact source locations:

| Artifact | Source path | Built by |
| --- | --- | --- |
| i3 (forked WM) | `.agents/reference/i3More/vendor/i3/build/install-root/usr/local/` | `cd i3More && just i3-image && just i3-build && just i3-stage` |
| i3More binaries | `/opt/i3more/bin/` (override via `$I3MORE_BIN_DIR`) | `cd i3More && docker compose run dev cargo build --release …` followed by `sudo install -Dm755 dist/i3more* /opt/i3more/bin/` |

Both paths are overridable via env vars on `17-stage-sysroot.sh`:

```bash
I3_INSTALL_ROOT=/some/other/path \
I3MORE_BIN_DIR=/some/other/i3more/bin \
    ./build/17-stage-sysroot.sh
```

### What was DELETED from this Phase 9

The first draft of Phase 9 had `build/16-i3-and-applets.sh` rebuilding
i3 (plus libev, yajl, pcre2, xcb-util-xrm) inside wo-builder against
the LFS substrate. After auditing `.agents/reference/i3More`:

- **Deleted `build/16-i3-and-applets.sh`** — i3 build is in i3More's `Dockerfile.i3` + justfile
- **Deleted `build/prepare-vendor.sh`** — no source needs to be copied into the container; the build happens in i3More's own container
- **Removed Phase 9 entries** from `versions.env` + `01-fetch.sh` — no Phase 9 packages are fetched
- **Removed `build/skeleton/home/writeonce/.config/i3status/config`** — i3More replaces i3status

The remaining "Phase 9 work" in writeonce-os is purely staging — wiring
the external artifacts into the bundled sysroot.

**2. i3More replaces three of the "standard" i3 helpers.** The
[i3More README](https://github.com/shoneyJ/i3More) is explicit: it's
"a self-contained standalone program that communicates with i3 via
IPC." It REPLACES (not augments):

| i3More binary | Replaces upstream | Why |
| --- | --- | --- |
| `i3more`          | i3status (+ i3bar's "the bar" role)  | i3More is a freestanding GTK4 program that positions itself on-screen — not an i3bar status_command. The default i3bar isn't used at all. |
| `i3more-lock`     | i3lock                               | PAM-backed, built with `--features lock`. Subscribes to `Session.Lock` signals on writeonce-logind. |
| `i3more-launcher` | dmenu / rofi                         | GTK4 app search + launch. |

Phase 9 therefore **deliberately does not build** i3status, i3lock, or
dmenu. The build script (`16-i3-and-applets.sh`) has 9 step functions,
not 11.

## The boot chain from kernel to login prompt

```
UEFI firmware finds /EFI/BOOT/BOOTX64.EFI
                            │
                            ▼
writeonce-bootloader (Rust no_std UEFI app, 46 KB)
  - reads /EFI/WriteOnce/{bzImage, initramfs.img, cmdline.txt}
  - LoadImage(FromBuffer) + StartImage
  - delegates to the kernel's EFI stub
                            │
                            ▼
Linux kernel 6.12.10 (LTS)
  - EFI stub initializes itself
  - mounts the embedded initramfs at /
  - executes /init
                            │
                            ▼
writeonce-initramfs (Rust static-musl, 658 KB)
  - parses /proc/cmdline for root=UUID=…
  - mounts /sys, /proc, /dev
  - loads required kernel modules (filesystem driver, USB controllers)
  - waits for the root device to appear
  - mounts it at /sysroot
  - switch_root(/sysroot)
  - execve("/usr/sbin/writeonce-pid1", ["writeonce-pid1"], env)
                            │
                            ▼
writeonce-pid1 (Rust static-musl, 990 KB) ← THIS PROCESS IS PID 1
  - mounts /proc /sys /dev /dev/pts /run /sys/fs/cgroup
  - blocks signals, opens signalfd, opens epoll
  - parses /etc/writeonce/writeonce.toml
  - reaps zombies in a loop
  - fork+execs /usr/sbin/writeonce-svc as child
                            │
                            ▼
writeonce-svc (Rust static-musl, 1.25 MB)
  - reads /etc/writeonce/services/*.toml
  - builds the dependency graph
  - topologically sorts; starts in waves
  - opens /run/writeonce-svc.sock for wo-ctl
                            │
        Wave 1 (no deps):   ▼
        sysinit.target  (synthetic, no-op)
                            │
        Wave 2:             ▼
        dbus.service                ← system bus
                            │
        Wave 3:             ▼
        logind.service              ← writeonce-logind, claims org.freedesktop.login1
        iwd.service                 ← wireless daemon
        dhcpcd@enp0s31f6.service    ← wired DHCP
                            │
        Wave 4:             ▼
        multi-user.target           ← synthetic milestone
                            │
        Wave 5:             ▼
        writeonce-login.service     ← spawns login on /dev/tty1
        graphical.target            ← synthetic graphical milestone
                            │
                            ▼
    On /dev/tty1:
    ┌─────────────────────────────────────────────┐
    │                                              │
    │              WriteOnce OS                    │
    │     Rust + kernel boot path                  │
    │                                              │
    │    login: ▮                                  │
    │                                              │
    └─────────────────────────────────────────────┘
```

## From login prompt to running i3

```
writeonce-login (on /dev/tty1)
  - reads username + password
  - PAM auth chain via /etc/pam.d/writeonce-login:
       auth      required  pam_unix.so      nullok_secure
       auth      required  pam_nologin.so
       account   required  pam_unix.so
       session   required  pam_unix.so
       session   required  pam_env.so       envfile=/etc/environment
       session   optional  pam_motd.so      motd=/etc/motd
       password  required  pam_unix.so      sha512 shadow
  - on PAM_SUCCESS:
       - fork() — parent waits for the child to exit (logout)
       - child (still root): execve /usr/sbin/writeonce-session-create
         with --user / --uid / --gid / --home / --shell / --tty /
         --vtnr / --session-script args
       - writeonce-session-create (still root):
         * D-Bus: Manager.CreateSession(uid, pid, ...)
         * receive: session_id, runtime_path, fifo_fd (lifecycle FD)
         * fcntl FD_CLOEXEC off (so FD survives execve)
         * mkdir /run/user/$UID + chown + chmod 0700
         * initgroups + setresgid + setresuid (drop to user)
         * chdir $HOME
         * build env: USER, HOME, SHELL, PATH, XDG_SESSION_ID,
           XDG_RUNTIME_DIR, XDG_SESSION_CLASS=user, XDG_SESSION_TYPE=tty
         * execve(/usr/bin/startx, [...], env)
                            │
                            ▼
startx (shell wrapper around xinit)
  - generates a fresh X authority cookie
  - launches Xorg :0 -auth <cookie> on a free VT
  - sources ~/.xinitrc as the X session
                            │
                            ▼
~/.xinitrc
  - pipewire    >/tmp/pipewire.log    2>&1 &
  - wireplumber >/tmp/wireplumber.log 2>&1 &
  - eval $(dbus-launch --sh-syntax --exit-with-session)   ← per-user dbus bus
  - setxkbmap us; xset r rate 200 30
  - exec i3                                                ← the WM takes over
                            │
                            ▼
i3 (the window manager)
  - loads ~/.config/i3/config
  - spawns i3status on the i3bar
  - listens on $XDG_RUNTIME_DIR/i3/ipc-socket.<PID>
  - waits for the user's first Mod+Return (alacritty) or Mod+d (dmenu)
```

## Why some daemons are user-spawned, not service-managed

A clean systemd-style design would make `pipewire.service`,
`wireplumber.service`, `Xorg.service`, `i3.service` all user-mode
units managed by a per-user systemd instance. That's what GNOME / KDE
do today.

We don't take that path for Phase 9 because:

1. **writeonce-svc is system-only.** Adding per-user-instance support
   is a substantial feature — separate cgroup slices, per-user socket
   activation, lifecycle tied to login/logout. That's a Round 2h+ scope.
2. **`.xinitrc` is the LFS / Gentoo idiom.** Simple, well-understood,
   no new infrastructure.
3. **One less thing to debug.** When pipewire fails to start, the user
   sees the log right at `/tmp/pipewire.log` in the same session,
   not in a per-user journal hidden behind `wo-ctl --user`.

The cost: when the user logs out (i3 exits), pipewire + wireplumber
get SIGHUP'd by the X session teardown and exit. That's fine — they
restart fresh on next login.

**Future Round 2h** will add per-user supervision if it becomes
necessary (e.g. apps that expect a per-user `XDG_RUNTIME_DIR/systemd/`
socket layout).

## /etc/writeonce/ layout

```
/etc/writeonce/
├── writeonce.toml                       — top-level config (pid1, svc, login, logind)
└── services/                            — system-managed service units
    ├── sysinit.target.toml              — synthetic
    ├── multi-user.target.toml           — synthetic
    ├── graphical.target.toml            — synthetic
    ├── dbus.service.toml
    ├── logind.service.toml
    ├── iwd.service.toml
    ├── dhcpcd.service.toml
    └── writeonce-login.service.toml
```

That's the **entire** system-service set Phase 9 needs. Compare with a
typical systemd install: ~150 units enabled by default. We get away
with 5 + 3 targets because:

- No `systemd-tmpfiles` (our `/tmp` is just `mode=1777` from /etc/fstab)
- No `systemd-resolved` (glibc + /etc/resolv.conf via dhcpcd)
- No `systemd-timesyncd` (will come from nixpkgs chrony in Phase 10)
- No `systemd-udevd` (eudev manages itself; not a service)
- No `systemd-journald` (logs to /var/log/* via stderr→file)
- No `systemd-cryptsetup` (no LUKS in v1)
- No `getty@.service` for tty2-6 (only tty1 has a login)

The fewer services, the fewer race conditions and the smaller the
boot-time window.

## i3More — separate repo, mandatory for the desktop UX

i3More is a sibling Rust workspace at
[`git@github.com:shoneyj/i3More.git`](https://github.com/shoneyj/i3More).
It is NOT optional — without it, there's no launcher, no lock screen,
and no status bar in the resulting desktop. The full binary list (from
its Cargo.toml):

| Binary | Role |
| --- | --- |
| `i3more`                | Main bar — workspace navigator, system tray, notifications, system info, control panel |
| `i3more-translate`      | Standalone translation popup |
| `i3more-audio`          | Volume control + audio device switching |
| `i3more-launcher`       | App search + launch (replaces dmenu) |
| `i3more-workspace`      | Visual workspace navigator |
| `i3more-lock`           | Lock screen (PAM-backed; `--features lock`) |
| `i3more-popup-translate`| Translate-from-selection popup |
| `i3more-power`          | Power inhibitors + suspend / hibernate UI |
| `i3more-power-profile`  | TLP-style power-profile switching |
| `i3more-keyhint`        | Floating keybinding help overlay |
| `i3more-window`         | Window manipulation utility |
| `i3more-layout`         | Layout templates |
| `i3more-speech-text`    | Whisper.cpp German→English STT (CUDA, `--features speech-text`) |
| `i3more-speech-text-ui` | UI for the above |

i3More links against (all from our Phase 8 substrate):

- `libgtk-4.so.1` — Phase 8d GTK4 stack
- `libgdk-4.so.1` with X11 backend — same
- `libpam.so.0` — Phase 8a (for `i3more-lock`)
- `libpipewire-0.3.so.0` — Phase 8e (for `i3more-audio`)
- `libdbus-1.so.3` — Phase 8a (for D-Bus calls into writeonce-logind)
- `libX11.so.6` + libxcb — Phase 8b

It uses **zbus 5** + **async-io** + **x11rb** (Rust X11 client) per
its Cargo.toml — same async runtime story as our writeonce-logind.

i3More is built **out-of-tree** on the workstation — it's not part of
the writeonce-os build pipeline. The operator's existing build
installs binaries to `/opt/i3more/bin/`:

```
/opt/i3more/bin/
  i3more                  i3more-back            i3more-keyhint
  i3more-audio            i3more-launcher        i3more-layout
  i3more-lock             i3more-popup-translate i3more-power
  i3more-power-profile    i3more-speech-text-ui  i3more-window
```

The `17-stage-sysroot.sh` script picks these up automatically:

```bash
# In 17-stage-sysroot.sh, step [3b/7]:
I3MORE_BIN_DIR="${I3MORE_BIN_DIR:-/opt/i3more/bin}"
for bin in "$I3MORE_BIN_DIR"/i3more*; do
    install -Dm755 "$bin" "$STAGING/usr/bin/$(basename "$bin")"
done
```

**No cargo build needed during writeonce-os builds.** The operator
maintains i3More separately; staging just copies whatever is
currently installed at /opt/i3more/bin/. If the directory is empty
or absent, staging warns but continues — the resulting WriteOnce
boots to bare i3 (no launcher/lock/audio applet).

**Overriding the location:**

```bash
I3MORE_BIN_DIR=/some/other/path/i3more-binaries \
    ./build/in-container.sh ./build/17-stage-sysroot.sh
```

Useful for CI scenarios that cache i3More artifacts under a known path.

### ABI compatibility notes

The /opt/i3more/bin binaries were built against the workstation's
system libraries — Ubuntu's glibc, GTK4, PipeWire, PAM, etc. They run
on the WriteOnce target against **WriteOnce's** versions of those
libraries. In practice this works because:

| Library | Build-time (Ubuntu 24.04) | Runtime (WriteOnce sysroot) | Compatibility |
| --- | --- | --- | --- |
| glibc           | 2.39    | 2.40    | ✓ glibc is backward-compatible: a 2.39-linked binary uses ELF symbol versions like `glibc_2.4`, `glibc_2.17` etc.; 2.40 provides all of those plus newer. |
| GTK4            | 4.14.x  | 4.16.7  | ✓ GTK4 only adds symbols across minor versions. |
| libpipewire-0.3 | 1.0.x   | 1.2.7   | ✓ PipeWire's `0.3` SONAME is stable across the 1.x release line. |
| libpam.so.0     | 1.5.x   | 1.6.1   | ✓ PAM ABI hasn't broken since 2007. |
| libdbus-1.so.3  | 1.14.x  | 1.16.0  | ✓ SONAME stable. |

If the workstation's GTK4 version were NEWER than the WriteOnce sysroot
(e.g. workstation has 4.20, target has 4.16), some symbols might be
missing. In that case rebuild i3More against WriteOnce's exact
substrate inside `wo-builder` — but for the current target hardware
(Ubuntu 24.04 workstation → WriteOnce v0.1) the version ordering goes
the right way (newer at runtime), so this isn't a concern.

### Force-rebuild path (when /opt/i3more is stale)

If the operator wants to refresh /opt/i3more/bin before staging:

```bash
cd ~/projects/github/shoneyj/i3More
docker compose run --rm dev bash -c '
    cargo build --release \
        --bin i3more --bin i3more-audio --bin i3more-launcher \
        --bin i3more-workspace --bin i3more-power --bin i3more-back \
        --bin i3more-power-profile --bin i3more-keyhint \
        --bin i3more-window --bin i3more-layout --bin i3more-popup-translate
    cargo build --release --features lock --bin i3more-lock
'
sudo install -Dm755 target/release/i3more*    /opt/i3more/bin/
sudo install -Dm755 target/release/i3more-lock /opt/i3more/bin/

# Then re-run staging:
cd ~/projects/github/shoneyj/writeonce-os
./build/in-container.sh ./build/17-stage-sysroot.sh
```

## What the user sees on first boot

```
GRUB / EFI bootloader:        none — writeonce-bootloader auto-loads
Plymouth / splash:            none — text console only

Boot output on /dev/tty0 (very fast on T450 SSD):

[    0.000000] Linux version 6.12.10 …
[    0.123456] writeonce-initramfs: parsing /proc/cmdline
[    0.234567] writeonce-initramfs: root=UUID=… found at /dev/sda2
[    0.345678] writeonce-initramfs: pivoting → /sysroot
[    0.456789] writeonce-pid1: hello, PID 1
[    0.567890] writeonce-pid1: mounted /proc /sys /dev /run /sys/fs/cgroup
[    0.678901] writeonce-svc: loaded 8 service units
[    0.789012] writeonce-svc: starting sysinit.target (wave 1)
[    0.890123] writeonce-svc: starting dbus.service (wave 2)
[    1.012345] writeonce-svc: starting logind.service, iwd.service, dhcpcd@…
[    1.234567] writeonce-svc: starting writeonce-login.service (wave 5)

On /dev/tty1 (after ~2 seconds):

                 _ _                                
 __      ___ __(_) |_ ___  ___  _ __   ___ ___    
 \ \ /\ / / '__| | __/ _ \/ _ \| '_ \ / __/ _ \   
  \ V  V /| |  | | ||  __/ (_) | | | | (_|  __/   
   \_/\_/ |_|  |_|\__\___|\___/|_| |_|\___\___|   

  Rust + kernel boot path. No reinvention beyond that.

login: writeonce
password: ●●●●●●●●

[ X starts, i3 takes over the display ]
```

The user is now in i3. `Mod+Return` opens alacritty (once Phase 10
installs it via Nix); `Mod+d` opens dmenu.

## What's deferred to later rounds

| Item | Round | Reason |
| --- | --- | --- |
| ~~writeonce-login → CreateSession D-Bus call~~ | **✓ 2g — done** | Helper crate `writeonce-session-create` does the D-Bus dance + privilege drop + execve. See `docs/learning/phase-4d-logind-shim.md`. |
| Per-user writeonce-svc instances | **2h** | Pipewire-as-service-unit pattern; XDG_RUNTIME_DIR/systemd compat |
| i3More cross-compile inside Phase 9 build | — | Separate repo by design; documented above |
| Phase 10 — Nix bootstrap | **10** | Adds alacritty, browsers, editors, dev toolchains |
| Suspend / hibernate wiring | **2f** | Kernel s2idle hooks via writeonce-logind |
| `getty@tty[2-6].service` for additional ttys | — | YAGNI; tty1 is enough |
| Display manager (lightdm/gdm equivalent) | — | Out of scope; writeonce-login + startx is the design |

## Build sequence summary

After the operator has run Phase 0-8 (the source-build of toolchain +
kernel + substrate) and the Rust crates have been built, Phase 9 is:

```bash
# === 1. External: build i3 + i3More on the workstation ====================
# Done in the i3More repo, not in writeonce-os. Each command takes ~5-10 min.
cd ~/projects/github/shoneyj/i3More

# 1a. i3 (forked WM) — produces vendor/i3/build/install-root/
just i3-image           # one-time: build the i3-build Docker image
just i3-build           # meson + ninja
just i3-stage           # meson install --destdir vendor/i3/build/install-root

# 1b. i3More binaries — produces dist/i3more*
docker compose run --rm dev cargo build --release \
    --bin i3more --bin i3more-audio --bin i3more-launcher \
    --bin i3more-workspace --bin i3more-power --bin i3more-back \
    --bin i3more-power-profile --bin i3more-keyhint \
    --bin i3more-window --bin i3more-layout --bin i3more-popup-translate
docker compose run --rm dev cargo build --release --features lock --bin i3more-lock

# 1c. Install i3More to /opt/i3more/bin (the path 17-stage-sysroot.sh reads).
sudo install -dDm755 /opt/i3more/bin
sudo install -m755 target/release/i3more* /opt/i3more/bin/

# === 2. writeonce-os stage + bundle + install =============================
cd ~/projects/github/shoneyj/writeonce-os

# 2a. Stage. Runs ON THE HOST (not in wo-builder) — reads /opt/i3more/bin
#     and the i3More symlink target, both outside the container mount.
./build/17-stage-sysroot.sh

# 2b. Bundle artifacts for the installer
./build/in-container.sh ./build/18-make-artifacts.sh

# 2c. Write to USB
sudo ./target/release/writeonce-installer install \
    --from build/artifacts \
    --target /dev/sda
```

After these four commands run successfully, the operator has a
**bootable USB containing a complete WriteOnce desktop**: kernel +
initramfs + bootloader + sysroot with i3 + all our Rust crates +
service units pre-installed. Plug it into the T450 and reboot.

## Final package tally across all phases

```
Phase 0 toolchain         : 9 packages (gcc + binutils + glibc + math libs + isl + linux headers)
Phase 1+2 temporary tools : 27 packages
Phase 5 kernel + initramfs: 1 (kernel itself)
Phase 8a base substrate   : 12
Phase 8b X11 stack        : 27
Phase 8c xorg + drivers   : 11
Phase 8d GTK4 stack       : 11
Phase 8e audio            : 4
Phase 8f network          : 5
Phase 9 i3 + i3More       : 0   (external artifacts, not built in writeonce-os)
                            ───
Total source-built        : 107 packages (in writeonce-os)
                          + i3 + 12 i3More binaries (in i3More repo)

Rust crates (workspace)   : 7
                            ───
Final image (T450, full)  : ~3.5 GB sysroot, ~1 GB zstd-compressed artifact
```

That's the entire WriteOnce v0.1 surface from `/init` to a logged-in
i3 desktop, built from source.
