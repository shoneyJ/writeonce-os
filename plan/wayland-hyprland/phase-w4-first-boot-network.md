# Phase W4 — First-boot networking

**Status: authored, UNVERIFIED** (committed `7ab2314`). The desktop is fetched from Nix on
first boot, so the network must come up automatically — which it did **not** on this branch
before this phase.

## Goal

Bring up internet automatically on first boot (ethernet zero-config; Wi-Fi from
install-time credentials) so `wo-session`'s `nix profile install` can reach
`cache.nixos.org`. Realises roadmap Phase 12 for this flavor.

## Context / findings

iwd + dhcpcd were *built* (13-network-stack) but shipped **no systemd units and weren't
enabled**, and iwd's D-Bus policy was missing from the real sysroot — so nothing brought
the network up at boot.

## Subtasks

- [x] **`iwd.service` + `dhcpcd.service`** (skeleton) + `multi-user.target.wants/` symlinks.
  iwd does L2 association; dhcpcd does DHCP + writes `/etc/resolv.conf` (systemd-resolved is
  off). Ethernet (built-in e1000e) needs no config.
- [x] **`etc/dbus-1/system.d/iwd-dbus.conf`** — the missing bus policy so iwd can own
  `net.connman.iwd`.
- [x] **Install-time Wi-Fi** — `build/install.sh` prompts for SSID (default hint: the
  workstation's `HOME_SA`) + passphrase and writes `/var/lib/iwd/<SSID>.psk`
  (`AutoConnect=true`, chmod 600). Blank = skip (ethernet only).
  `etc/writeonce/wifi.psk.example` documents the format.
- [x] **`wo-session` network wait** — up to ~2 min for a default route before the Nix install
  (association + DHCP take a few seconds; `multi-user.target` ≠ network-online).

## Deliverable

A first boot that obtains an IP + DNS automatically (wired or the provisioned Wi-Fi) before
the desktop install runs.

## Acceptance / verification (on the target — W5)

- Ethernet: a DHCP lease + DNS resolve with no config. Wi-Fi: iwd auto-connects to the
  provisioned SSID; `iwctl station <dev> show` connected.
- `ip route` shows a default route before `wo-session` proceeds.

## Risks

- **iwd `Type=dbus` startup** depends on the staged D-Bus policy + system bus — first thing
  to check if iwd doesn't come up.
- **dhcpcd master mode** (`-B -q`) writing `/etc/resolv.conf` — verify DNS actually lands.
- No network on first boot ⇒ no desktop. Ethernet is the most reliable bring-up; Wi-Fi adds
  the SSID/passphrase + firmware (iwlwifi) variable.
