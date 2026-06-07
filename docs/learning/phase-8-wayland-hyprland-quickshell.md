# Phase 8 (Wayland) — Hyprland + Quickshell desktop

Branch: `wayland-hyprland` (off `with-systmed`). Replaces the X11/i3 + i3More
desktop with a Wayland desktop: **Hyprland** compositor + **Quickshell** (Qt/QML)
shell. Scope: minimal-first (a working bar + terminal), kernel unchanged.

## Kernel compatibility verdict (the original question)

**The kernel is not the bottleneck — it is already over-spec for this DE.**
`with-systmed` pins Linux **6.18.34** and its `kernel-config-additions.fragment`
already has everything a Wayland compositor needs:

- `CONFIG_DRM=y`, `CONFIG_DRM_I915=y` — KMS + Intel Gen8 (HD 5500) driver.
- `CONFIG_DRM_FBDEV_EMULATION=y`, `CONFIG_DRM_SIMPLEDRM=y` — console + early FB.
- `CONFIG_INPUT_EVDEV=y` — libinput input path.
- `CONFIG_DEVTMPFS{,_MOUNT}=y` — `/dev/dri/{card0,renderD128}` auto-created.

Atomic modesetting is unconditional in modern DRM (there is **no** separate
`CONFIG_DRM_ATOMIC` to add). 6.18.34 is newer than what most distros ship
Hyprland on. **No kernel change was required.**

**Broadwell (Gen8) caveat:** the GPU is old. The kernel cmdline already disables
the buggy display features (`i915.enable_psr=0 enable_fbc=0 enable_dc=0`) to dodge
Gen8 hangs — important under Hyprland's heavy atomic-commit load. We complement
it at the compositor level: `cursor:no_hardware_cursors=true` + `WLR_NO_HARDWARE_CURSORS=1`,
blur disabled, `vfr=true`.

## Why the DE comes from Nix, not a source cross-build

The original plan was to cross-build the whole stack (substrate → Hyprland → Qt6
→ Quickshell) from source. Implementation surfaced that this is genuinely
high-risk on several independent axes for our `x86_64` glibc-2.40 / GCC-14.2
sysroot:

- **Hyprland 0.44.1 `REQUIRED`s `hyprcursor`**, which `REQUIRED`s **`librsvg-2.0`**
  (a Rust package) — not skippable for any aquamarine-based Hyprland. Cross-
  compiling librsvg against a hand-rolled sysroot is a multi-hour rabbit hole.
- **Hyprland 0.44.1 sets `CMAKE_CXX_STANDARD 26`.** GCC 14.2 implements only
  *partial* C++26 — the compiler pin itself was uncertain (may need GCC 15).
- **Qt6 + Quickshell** are each substantial, finicky cross-builds on top.

Per the project scope (`project-writeonce-scope`: *Rust + kernel own the boot
path; after login, upstream software via source build **or Nix***), the desktop
is post-login userspace and a legitimate **Nix** target. A Nix Hyprland closure
is self-contained (its own Mesa/Wayland/seatd/libinput from nixpkgs) and only
needs the kernel's DRM — so it sidesteps every cross-build risk above.

Decision (with the user): **deliver Hyprland + Qt6 + Quickshell + terminal via
Nix; keep only the kernel + boot substrate source-built.**

## Architecture (as implemented)

```
kernel 6.18.34 (DRM/i915/evdev — unchanged)
  └─ systemd PID1 → getty@tty1 autologin → login (PAM: pam_systemd → logind seat0/vt1)
       └─ ~/.bash_profile  (sources Nix profile; on tty1 runs wo-session)
            └─ /usr/local/bin/wo-session
                 ├─ exports Wayland/Qt env (+ Gen8 software-cursor workaround)
                 ├─ first boot: `nix profile install` the desktop set
                 └─ exec Hyprland
                      ├─ exec-once: dbus-update-activation-environment, pipewire, wireplumber
                      └─ exec-once: quickshell -c wo   (the bar)
```

Files added/changed under `build/skeleton/` (staged by `17-stage-sysroot.sh`'s
overlay step):

- `usr/local/bin/wo-session` — Wayland session launcher (env + Nix realize + exec).
- `home/writeonce/.config/hypr/hyprland.conf` — minimal Hyprland (kb_layout=de to
  match `/etc/vconsole.conf`; no plugins; Broadwell-conservative).
- `home/writeonce/.config/quickshell/wo/shell.qml` — thin bar (workspaces/clock/volume),
  only standard Quickshell+Qt modules (no Kirigami/QtPositioning/Qt5Compat).
- `etc/nix/nix.conf` — single-user Nix baseline (flakes + cache).
- `etc/writeonce/desktop/flake.nix` — the Tier-2 DE closure (`nixpkgs#hyprland`,
  `quickshell`, `alacritty`, `xdg-desktop-portal-hyprland`, `wl-clipboard`). Consumption
  only — no `*.nix` authored.
- `home/writeonce/.bash_profile` — sources Nix profile, launches `wo-session` on tty1.
- `etc/motd`, `getty@tty1.service.d/autologin.conf` — text updated for Hyprland.
- Removed `home/writeonce/.xinitrc` and `.config/i3/` (orphaned by the swap).
- `17-stage-sysroot.sh` — dropped the i3 + i3More staging blocks (DE is via Nix).

Also added (general infra): `build_cmake` in `blfs-pkg.sh` and host code-gen
deps (`libwayland-bin`, `libpugixml-dev`, …) in the `Containerfile`. And a
**parked** from-source fallback, `16a-wayland-substrate.sh` (wayland, mesa+wayland,
seatd, libdisplay-info, libliftoff) — correct but not wired into versions/fetch,
retained in case a source-built compositor is ever wanted.

## Phase 14 Nix bootstrap (authored — UNVERIFIED here)

The offline single-user Nix bootstrap is now wired (faithful to the official
single-user closure-install steps, but **not yet run** — there is no Nix on this
workstation and no target to test on):

- `build/versions.env` `NIX_VERSION` + `01-fetch.sh` URL (releases.nixos.org
  static tarball, SHA-only via `.next-lock`) + `checksums.txt` placeholder.
- `build/17a-install-nix.sh` — runs after `17-stage-sysroot.sh`, before
  `18-make-artifacts.sh`; unpacks the tarball into `$STAGING/nix` (store +
  `.reginfo` + single-user `var` skeleton). Store-DB load is deferred to boot
  because Nix store paths are absolute.
- `wo-nix-init` + `writeonce-nix-init.service` — first-boot oneshot (guarded by
  the absent store DB, ordered `Before=getty@tty1.service`): chown `/nix` to the
  user, `nix-store --load-db < /nix/.reginfo`, install the bundled nix into the
  default profile.
- `etc/profile.d/nix.sh` — puts the Nix profiles on PATH + sets
  `NIX_SSL_CERT_FILE` from the bundled `nss-cacert`. `etc/nix/nix.conf` — flakes,
  cache, single-user.
- `NIX_VERSION=2.34.7` (latest official, verified 2026-06-05); `checksums.txt`
  carries the upstream-published sha256, so `01-fetch.sh` verifies on download.

### First-boot networking (required — the desktop is fetched online)

The first-boot `nix profile install` needs internet, but networking was not
auto-started on this branch. Added:
- `iwd.service` + `dhcpcd.service` units + `multi-user.target.wants` symlinks
  (the binaries were built by 13-network-stack but shipped no units). iwd does
  L2 association; dhcpcd does DHCP + writes `/etc/resolv.conf` (resolved is off).
- `etc/dbus-1/system.d/iwd-dbus.conf` — iwd's bus policy was missing from the
  real sysroot, so iwd couldn't own `net.connman.iwd`; shipped here.
- **Ethernet needs no config** (built-in e1000e + dhcpcd). **Wi-Fi** is
  provisioned at install: `build/install.sh` prompts for SSID (default hint:
  the workstation's `HOME_SA`) + passphrase and writes `/var/lib/iwd/<SSID>.psk`
  (`AutoConnect=true`). `wo-session` waits up to ~2 min for a default route
  before the Nix install.

### No Nix account / registration

Nix is **not** account-based — there is nothing to sign up for. `nix profile
install nixpkgs#<pkg>` pulls prebuilt binaries from the public binary cache
`https://cache.nixos.org` (set as a substituter + trusted key in `nix.conf`); no
login, token, or registration. The only "registration" in play is internal: the
store-path DB load (`nix-store --load-db`, done by `writeonce-nix-init`) and the
optional nixpkgs **flake-registry pin** (a config pin for reproducibility, not
an account). Just network access to the cache is required.

### What remains (cannot be done in this environment)

1. **Realize the desktop closure** — first graphical boot with network
   (`wo-session` runs `nix profile install` from `desktop/flake.nix`), or, once
   Nix is on the workstation, pre-bake the closure into `/nix/store` at
   image-build (most robust; no first-boot network for the compositor). Pin the
   nixpkgs registry rev for reproducibility.
3. **Verify on the T450**: first boot runs `writeonce-nix-init` → `nix --version`
   works; autologin tty1 → `wo-session` installs + launches Hyprland;
   `$WAYLAND_DISPLAY` set, `/run/user/1000/wayland-1` exists; Quickshell bar
   shows (clock + ≥1 workspace pill); `Mod+Return` opens alacritty; `Mod+Shift+E`
   exits to a shell. QEMU (virtio-gpu) can confirm Hyprland reaches DRM + opens a
   Wayland socket, but the iris-only Mesa won't render the bar reliably there —
   the real GPU is authoritative.
4. **Footprint:** staging the base Nix closure adds ~hundreds of MB to the image
   (`du -sh $STAGING/nix`); the desktop closure (installed at runtime) is larger.

## Build-time flavor profiles

To stop the flavors diverging across branches (a shared fix had to be applied to
each), the build is now **profile-driven** — one tree, `FLAVOR` selects the flavor:

- `build/flavors/<name>.conf` declares the axes: `FLAVOR_INIT` (systemd|rust),
  `FLAVOR_DISPLAY` (wayland|x11), `FLAVOR_DE` (hyprland|i3more), `FLAVOR_PKG`
  (nix|source), `FLAVOR_KERNEL` (kernel pin).
- `build/setup-env.sh` sources `flavors/$FLAVOR.conf` after `versions.env`
  (default `systemd-wayland-hyprland`), exports the axis vars, and overrides
  `LINUX_VERSION` from `FLAVOR_KERNEL`. `build/in-container.sh` forwards `-e FLAVOR`.
- `build/skeleton/` → `common/` (shared) + `<flavor>/` (init/display/DE/pkg files);
  `17-stage-sysroot.sh` overlays `common/` then `$FLAVOR` (flavor wins).
- Divergent steps are axis-gated: `16-systemd.sh` (FLAVOR_INIT=systemd),
  `17a-install-nix.sh` (FLAVOR_PKG=nix). Numbered scripts `00–14` stay shared.

**Done now:** framework + the `systemd-wayland-hyprland` flavor (default build is
byte-for-byte unchanged — verified: `skeleton/common/ ∪ skeleton/systemd-wayland-hyprland/`
== the pre-split skeleton). `systemd-x11-i3` and `rust-x11-i3` are declared configs only.

**Migrating another flavor (the follow-on recipe), e.g. `systemd-x11-i3` from `with-systmed`:**
1. `git merge`/cherry-pick that branch's build-script diffs into this tree, putting the
   flavor-conditional parts behind the axis vars (e.g. the X11 desktop scripts `09–11`
   gated on `FLAVOR_DISPLAY=x11`/`FLAVOR_DE=i3more`; its install/kernel-config differences
   gated likewise). Numbered scripts shared by both flavors stay shared.
2. Populate `build/skeleton/systemd-x11-i3/` from that branch's skeleton (its
   `.xinitrc`, i3 config, startx `.bash_profile`, X11 pam.d/login, …). Re-run the
   union check against that branch's staged skeleton.
3. Build with `FLAVOR=systemd-x11-i3` and verify on the target.
The `rust-x11-i3` flavor is the largest (Rust-PID1 boot path + `writeonce-installer` +
kernel 6.12 without the Ubuntu base config) and should be migrated last.
