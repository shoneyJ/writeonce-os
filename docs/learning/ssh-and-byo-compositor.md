# SSH-accessible base + bring-your-own compositor

WriteOnce no longer bakes in a desktop. The image boots to an **autologin
console** with the network up, **mDNS** advertising `writeonce.local`, and Nix
ready. From there you:

1. enable SSH once (`sudo wo-sshd-setup`),
2. drive the box from your workstation (`ssh writeonce@writeonce.local`),
3. install the Wayland compositor *you* want via Nix (sway / hyprland / wayfire)
   and launch it, bringing your own config.

This note explains the pieces and the order to use them.

## Why this shape

The earlier flavor force-launched Hyprland + Quickshell on tty1. That coupled the
only console to a specific compositor and a specific Nix closure — a GPU or
compositor problem could swallow the login, and "the desktop" was a decision
baked into the image. Decoupling it:

- **The base is a substrate, not a desktop.** It provides kernel + systemd +
  network + Nix. The desktop is a runtime choice, installed per-machine.
- **You drive it over SSH.** Typing long `nix` commands at a TTY (with no desktop
  to copy-paste from) is painful; SSH from the workstation is how you actually
  iterate. So the base has to be reachable *before* any desktop exists.

## Finding the box: mDNS (`writeonce.local`)

`avahi-daemon` (built daemon-only — see below) advertises this host as
`<hostname>.local` over multicast DNS. `/etc/hostname` is `writeonce`, so the box
answers to **`writeonce.local`** on the LAN — no IP hunting, no DHCP-lease
lookups.

- **On the target:** `avahi-daemon.service` is enabled at boot
  (`multi-user.target.wants/`). It only *publishes* — it needs no dbus and no
  glib, so it's built `--disable-dbus --disable-glib` (`build/13a-mdns.sh`,
  packages `libdaemon` + `avahi`). The `avahi:avahi` user it drops to ships in
  the skeleton `passwd`/`group`.
- **On the workstation (the resolver side):** Ubuntu/Fedora already resolve
  `.local` (they run avahi + `nss-mdns`). If `ping writeonce.local` fails on
  Ubuntu: `sudo apt install avahi-daemon libnss-mdns`. Verify with
  `avahi-resolve -n writeonce.local` or `ping writeonce.local`.

If mDNS is unavailable, fall back to the IP: on the target `ip -4 addr`, or from
the workstation `sudo arp-scan --localnet` / `nmap -sn <subnet>`.

## Enabling SSH: `sudo wo-sshd-setup` (OpenSSH via Nix)

WriteOnce builds **no OpenSSH/OpenSSL** in the substrate (deliberately — it's a
large surface we don't want in the from-source base). Instead sshd is delivered
on demand from the single-user Nix store. Run once, as root:

```sh
sudo wo-sshd-setup
```

It (idempotently): installs `github:NixOS/nixpkgs/nixos-unstable#openssh` into the
`writeonce` profile (the **full flakeref** — the bare `nixpkgs` alias does not
resolve out of the box), ensures the privsep prerequisites (the `sshd` user +
`/var/empty`), runs `ssh-keygen -A` for host keys, finalizes
`/etc/ssh/sshd_config`, and enables + starts `sshd.service`. The unit's
`ExecStart` is `/home/writeonce/.nix-profile/bin/sshd` — a profile symlink that
stays valid across `nix profile upgrade`.

**Keys vs passwords.** Authorize your workstation key at install time (the
installer's `[5c/6]` prompt writes `~writeonce/.ssh/authorized_keys`). When a key
is present, `wo-sshd-setup` sets `PasswordAuthentication no` (key-only). With no
key, password login (the install-time password) stays on so you're never locked
out. `PermitRootLogin no`, `AllowUsers writeonce`, `UsePAM no` (auth checks
`/etc/shadow` directly — this base ships no PAM stack).

## Installing a compositor (your choice)

Once SSH'd in:

```sh
nix profile add github:NixOS/nixpkgs/nixos-unstable#sway   # or #hyprland / #wayfire
exec sway                                                  # launch from tty1
```

Bring your own config (e.g. from your dotfiles repo) into `~/.config`. WriteOnce
no longer ships `~/.config/hypr` or `~/.config/quickshell`, `wo-session`, or the
`/etc/writeonce/desktop` flake — that's all yours now.

> Pin it for reproducibility: `nix registry pin nixpkgs github:NixOS/nixpkgs/<rev>`
> then use `nixpkgs#sway`, or write a small flake and `nix profile add path:...`.

## The immediate path (no reflash, on the box you have today)

The current image predates this change, but the same Nix path works by hand:

```sh
# on the T450 console
nix profile add github:NixOS/nixpkgs/nixos-unstable#openssh
sudo ssh-keygen -A
mkdir -p ~/.ssh && chmod 700 ~/.ssh
printf '%s\n' 'ssh-ed25519 AAAA... you@workstation' >> ~/.ssh/authorized_keys
chmod 600 ~/.ssh/authorized_keys
sudo ~/.nix-profile/bin/sshd        # or: sudo ~/.nix-profile/bin/sshd -D -e
# find the IP from the workstation if mDNS isn't up yet:
#   sudo arp-scan --localnet   (or)   nmap -sn <subnet>
ssh writeonce@<ip>
```

The next flash bakes all of this in (mDNS + the `wo-sshd-setup` one-liner + the
install-time key prompt), so it's `ssh writeonce@writeonce.local` out of the box.

## Where it lives

| Piece | Path |
| ----- | ---- |
| avahi/libdaemon build | `build/13a-mdns.sh`; pins in `build/versions.env`; URLs in `build/01-fetch.sh` (`just phase-8g`) |
| mDNS config + unit | `…/etc/avahi/avahi-daemon.conf`, `…/etc/systemd/system/avahi-daemon.service` (+ `.wants` link) |
| `sshd` / `avahi` users | `build/skeleton/common/etc/{passwd,group}` |
| SSH helper + templates | `…/usr/local/sbin/wo-sshd-setup`, `…/etc/ssh/sshd_config`, `…/etc/systemd/system/sshd.service` |
| Install-time key prompt | `build/install.sh` (`[5c/6]`) |
| Boot-to-console | `…/home/writeonce/.bash_profile` (no auto-launch) |

(`…` = `build/skeleton/systemd-wayland-hyprland/`.)

## Verification

- **Build (container):** `just phase-8g` → `avahi-daemon` + libs land in `$LFS`.
  (avahi's autotools build may want `intltool`/`gettext`; if configure errors on
  them, add to `build/Containerfile` and rerun — host-vs-target rule.)
- **Stage:** after `17-stage-sysroot.sh`, the skeleton overlay places the avahi
  conf/unit, the sshd templates, `wo-sshd-setup`, and the `avahi`/`sshd` users.
- **Target (after reflash):** `ping writeonce.local` resolves from the
  workstation; `sudo wo-sshd-setup` → `systemctl status sshd` active →
  `ssh writeonce@writeonce.local` logs in; boot lands at a console (no
  auto-compositor); `nix profile add …#sway` + `exec sway` brings up a desktop.
