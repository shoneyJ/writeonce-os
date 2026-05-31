# Phase 12 — Automatic network bring-up (wired + wifi + DNS)

> systemd branch (`with-systmed`), post-desktop.

**Goal.** Wired ethernet comes up automatically at boot via DHCP, wifi is connectable
with one command, and DNS resolves — using the `iwd` + `dhcpcd` daemons already built in
Phase 8f, wired to systemd units. No manual `ip`/`udhcpc` dance.

## Context

`iwd` (3.3), `dhcpcd` (10.1.0), `iproute2`, `iputils`, and `ell` are **built and staged**
(`build/13-network-stack.sh`, `build/versions.env:192`) but wired to **no init**:
`systemd-networkd` and `systemd-resolved` are disabled in the systemd build
(`build/16-systemd.sh:57` — `-Dnetworkd=false -Dresolve=false`), and no `*.service`
symlinks exist under `build/skeleton/etc/systemd/system/*.wants/`. So a freshly installed
system has zero network at boot. This phase wires it up.

## Subtasks

1. **Control-plane decision (weigh).**
   - **(A) iwd + dhcpcd directly** (recommended): both are already built and ship
     upstream systemd units; iwd has a built-in DHCP+DNS client for wifi, dhcpcd handles
     wired. No systemd rebuild.
   - **(B) systemd-networkd + resolved**: cohesive, but requires rebuilding systemd with
     `-Dnetworkd=true -Dresolve=true` (`build/16-systemd.sh`) and then *not* using
     dhcpcd. More moving parts; defer unless (A) proves insufficient.
   Recommendation: **(A)**.
2. **Enable wired DHCP.** Stage `dhcpcd.service` and enable it (symlink into
   `multi-user.target.wants/` in `build/skeleton`). Confirm it brings up the I218-LM
   (`e1000e`, built-in) at boot. Prefer the global `dhcpcd.service` (all interfaces) for
   simplicity over per-interface `dhcpcd@.service`.
3. **Enable wifi daemon.** Stage `iwd.service`, enable it. Add `/etc/iwd/main.conf` with
   `[General] EnableNetworkConfiguration=true` so iwd performs DHCP **and** writes
   resolv.conf for wifi links itself (avoids double-DHCP races with dhcpcd on wlan).
   Scope dhcpcd to wired only (`denyinterfaces wlan*` in `/etc/dhcpcd.conf`).
4. **DNS / resolv.conf.** With (A): dhcpcd manages `/etc/resolv.conf` for wired, iwd for
   wifi. Ship a sane fallback (e.g. a commented `/etc/resolv.conf` with a public resolver)
   and ensure `/etc/nsswitch.conf` has `hosts: files dns`. Document the ordering so the two
   daemons don't fight over the file (dhcpcd's hook vs iwd's resolvconf).
5. **firmware check.** iwlwifi (Intel 7265) needs its firmware blob in `/lib/firmware`
   (staged by `build/17-stage-sysroot.sh` firmware step) and the `iwlwifi`/`iwlmvm`
   modules (built `=m`, now staged via the Phase-fix modules step). Verify wifi actually
   probes (`iwctl device list`).
6. **Connect UX.** Document `iwctl station <dev> get-networks` / `connect <ssid>` for
   wifi. Note a future i3More network applet as the GUI path (out of scope here).
7. **machine-id + transient hostname.** Ensure `/etc/hostname` (set by the Phase 11
   installer) is applied; `systemd-hostnamed` is already enabled in the systemd build.

## Deliverable

A booted, installed system that has a wired DHCP lease within seconds of boot, resolves
DNS, and can join a wifi network with `iwctl` — all via enabled systemd units shipped in
the skeleton, no rebuild of systemd.

## Acceptance criteria

- After boot on wired ethernet: `ip addr` shows a lease on the `e1000e` NIC;
  `ping -c1 1.1.1.1` and `ping -c1 example.com` (DNS) both succeed.
- `systemctl status dhcpcd iwd` → both active/enabled.
- `iwctl station wlan0 connect <ssid>` associates and gets an address; DNS still resolves.
- `/etc/resolv.conf` is populated and stable (not flapping between the two daemons).

## References

- `build/13-network-stack.sh`, `build/versions.env:192` — the built network stack.
- `build/16-systemd.sh:57` — networkd/resolved disabled (the reason we use iwd+dhcpcd).
- `build/17-stage-sysroot.sh` — firmware + modules staging (iwlwifi).
- upstream `iwd` `main.conf` + `dhcpcd.conf` man pages.

## Risks

- **Two DHCP clients on wlan.** Keep dhcpcd off wlan (`denyinterfaces wlan*`); let iwd own
  wifi end-to-end.
- **resolv.conf ownership.** Decide one writer per interface class; document to avoid the
  classic resolvconf tug-of-war.
- **No resolved.** Apps expecting the `systemd-resolved` stub (`127.0.0.53`) won't find it;
  fine for a files+dns nsswitch, but note it for anything that hard-codes resolved.
