# Wayland / Hyprland flavor — implementation plan

> Flavor: **`systemd-wayland-hyprland`** (branch `wayland-hyprland`, off `with-systmed`).
> Phase-by-phase plan for the Wayland desktop. Per-phase detail in `phase-w1-…` →
> `phase-w6-…`. Rationale + the architecture decision log live in
> [`../../docs/learning/phase-8-wayland-hyprland-quickshell.md`](../../docs/learning/phase-8-wayland-hyprland-quickshell.md).

## Context

This flavor replaces the `with-systmed` X11/i3 desktop with a **Wayland** desktop:
**Hyprland** compositor + **Quickshell** (Qt/QML) shell, with the desktop **delivered
via Nix** rather than source-built. Same boot path as `with-systmed` (systemd PID 1 +
logind + udev), same T450 target, kernel unchanged at **6.18.34**.

**Why Wayland-via-Nix.** Hyprland's source stack is high-risk to cross-compile for our
glibc-2.40 / GCC-14.2 sysroot (hyprcursor→librsvg (Rust); Hyprland wants C++26; plus Qt6
+ Quickshell). Per the project scope ("Nix for apps after login"), a self-contained Nix
Hyprland closure sidesteps all of it and only needs the kernel's DRM. The from-source
substrate was drafted then parked (`build/16a-wayland-substrate.sh`).

**Axes** (vs the other flavors): `INIT=systemd · DISPLAY=wayland · DE=hyprland · PKG=nix`.
Selected at build time via the flavor profile (`build/flavors/systemd-wayland-hyprland.conf`
+ `FLAVOR` in `setup-env.sh`) — see [build-time flavor profiles](../../docs/learning/phase-8-wayland-hyprland-quickshell.md).

**Desktop + `~/.config` are managed by Home Manager** (`/etc/writeonce/home`), which
supersedes the earlier desktop flake + raw skeleton configs; the terminal is **alacritty**
(was kitty); the dev toolchain (vscode/neovim/tmux) is a separate `nix develop` devShell
(`/etc/writeonce/devshell`). See [`home-manager-and-devshell.md`](../../docs/learning/home-manager-and-devshell.md).

## Phase map

| #  | Phase                                              | Status | File |
| -- | -------------------------------------------------- | ------ | ---- |
| W1 | Wayland substrate & delivery decision (Nix vs src) | **done** (decided: Nix; from-source parked) | [phase-w1-substrate-decision.md](phase-w1-substrate-decision.md) |
| W2 | Desktop config & session (Hyprland + Quickshell)   | **done** (committed) | [phase-w2-desktop-session.md](phase-w2-desktop-session.md) |
| W3 | Nix bootstrap — offline single-user `/nix`          | **build-time staging VERIFIED**; first-boot registration target-gated | [phase-w3-nix-bootstrap.md](phase-w3-nix-bootstrap.md) |
| W4 | First-boot networking (iwd/dhcpcd + install Wi-Fi)  | authored (units staged); runtime **UNVERIFIED** | [phase-w4-first-boot-network.md](phase-w4-first-boot-network.md) |
| W5 | Build → install → T450 first-boot bring-up          | **build half VERIFIED** (artifact built); install + boot pending | [phase-w5-build-install-bringup.md](phase-w5-build-install-bringup.md) |
| W6 | Hardening & expansion (XWayland, richer shell, …)   | future | [phase-w6-hardening-expansion.md](phase-w6-hardening-expansion.md) |

**Dependency order:** W1 → W2 → (W3, W4 in parallel) → **W5** → W6. The **workstation-build
half is verified** (2026-06-06): `01-fetch` → `17-stage` → `17a-install-nix` →
`18-make-artifacts` run clean and produce a flashable image with `/nix` baked in (W2 overlay
+ W3 staging proven). What remains **unrun** is the **target/first-boot half**: `wo-nix-init`
registering Nix (W3), the network bring-up (W4), and the Hyprland session (W5 steps 3–4) —
all need the T450 (the disk-wiping `install.sh` is not run from the authoring environment).

## How it relates to the roadmap

This flavor extends the `with-systmed` line (roadmap "Phase 11+"). It supersedes Phase 8/9
(X11/i3 desktop) with W1–W2/W6, and *realises* Phase 14 (Nix) as W3 + the runtime desktop
install. The build is profile-driven (one tree, `FLAVOR` selects the flavor); `master` is
the common base, the other flavors fold in later.

## Verification summary (per phase)

| Phase | How you know it landed |
| ----- | ---------------------- |
| W1 | Decision recorded; `16a-wayland-substrate.sh` parked + clearly marked; Nix path chosen |
| W2 | Skeleton config stages; `qmllint`/`qs -c wo --check` on `shell.qml`; default-flavor stage unchanged |
| W3 | First boot: `writeonce-nix-init` registers Nix → `nix --version` works; `/nix` single-user |
| W4 | Ethernet DHCP lease or `iwctl`/auto-connect to the provisioned SSID; default route before the Nix install |
| W5 | Autologin tty1 → `wo-session` installs + launches Hyprland; Quickshell bar; `Mod+Return` → alacritty |
| W6 | XWayland X-app runs; richer Quickshell modules load; (optional) pre-baked closure → no first-boot network |
