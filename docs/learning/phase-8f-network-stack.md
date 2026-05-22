# Phase 8f — the network stack

> Companion to [`../../build/13-network-stack.sh`](../../build/13-network-stack.sh).
> Explains the five-package network userspace (ell + iwd + iproute2 + iputils
> + dhcpcd), why we chose iwd over wpa_supplicant, and what the T450's
> two NICs (Intel I218-LM Ethernet, Intel Wireless 7265) need from the
> kernel side.

## The network stack at a glance

```
                 ┌────────────────────────────────────────┐
                 │   User-facing apps:                     │
                 │   - i3more-network applet               │
                 │   - browsers (wired/wifi unaware)       │
                 │   - alacritty + ssh / curl              │
                 └────────────────────────────────────────┘
                                  │
                                  │ socket() / connect() /
                                  │ getaddrinfo(...)
                                  ▼
                 ┌────────────────────────────────────────┐
                 │   Kernel: net + AF_INET/AF_INET6 stack  │
                 │   - routing table (per netns)           │
                 │   - netfilter / nftables                │
                 │   - tcp/udp/icmp protocols              │
                 └─────────┬──────────────────────┬──────┘
                           │                      │
              wired path   │           wireless   │ path
                           ▼                      ▼
                ┌──────────────────┐   ┌──────────────────────┐
                │ Intel I218-LM    │   │ Intel Wireless 7265   │
                │ kernel: e1000e    │   │ kernel: iwlwifi +     │
                │                   │   │ iwlmvm + cfg80211 +   │
                │                   │   │ mac80211 + nl80211    │
                └────────┬─────────┘   └──────────┬───────────┘
                         │                        │
              ┌──────────▼────────┐  ┌────────────▼──────────┐
              │  dhcpcd (DHCP)    │  │   iwd                  │
              │  - listens on enp │  │   - WPA / 802.11 assoc │
              │  - IPv4 + IPv6 SLAAC│ │   - built-in DHCP for  │
              │  - writes routes   │  │     wlan interface     │
              │  - writes resolv   │  │   - iwctl (CLI)        │
              └───────────────────┘  └────────────────────────┘
                         │                        │
                         └───────────┬────────────┘
                                     ▼
                         iproute2 (`ip`, `ss`)
                         lets you SEE what the above did.
```

`iputils` is just the diagnostic toolbox (`ping`, `traceroute`,
`arping`, `tracepath`) — not in the control flow.

## Per-package role

| Package | Role | Size on disk |
| --- | --- | --- |
| **ell** | Async event-loop + crypto + tiny D-Bus client library. Maintained by Intel alongside iwd. iwd's runtime is built on top of it. | ~1 MB |
| **iwd** | Wireless daemon: WPA-PSK / WPA2 / WPA3 / WEP authentication, 802.11 association, **built-in DHCPv4/v6 client for the wireless interface**, an EAP framework. Exposes a D-Bus API for control. Includes `iwctl` (CLI) and `iwmon` (sniffer). | ~3 MB |
| **iproute2** | The `ip` + `tc` + `ss` + `bridge` suite. Userspace control plane for the kernel's network stack. Replaces the deprecated net-tools (`ifconfig`, `route`, `netstat`). | ~5 MB |
| **iputils** | `ping`, `ping6`, `traceroute`, `arping`, `tracepath`, `clockdiff`, `ninfod`. Diagnostic tools. `ping` needs `CAP_NET_RAW` to send raw ICMP. | ~500 KB |
| **dhcpcd** | DHCPv4 + DHCPv6 client. We run it on the **wired** interface; iwd handles DHCP for the wireless one itself. | ~1 MB |
| **Total** | **~10 MB.** | |

The smallest substrate round.

## Why iwd, not wpa_supplicant?

`wpa_supplicant` is the legacy WPA daemon shipped by most distros for
the past two decades. It works. But:

| Aspect | wpa_supplicant | iwd |
| --- | --- | --- |
| **Size** | ~3 MB + ~5 MB OpenSSL | ~3 MB + ~1 MB ell, no OpenSSL |
| **Crypto** | Re-implements WPA in userspace (or uses OpenSSL) | Uses kernel crypto via CRYPTO_USER_API + nl80211 |
| **Config format** | Verbose `.conf` per network | Per-network `.psk` files in `/var/lib/iwd/` |
| **CLI** | `wpa_cli` (clunky) | `iwctl` (line-editing, tab completion) |
| **D-Bus** | Optional, second-class | Primary control interface |
| **DHCP** | None — needs separate client | Built-in v4 + v6 |
| **Maintainer** | hostap project (Jouni Malinen at Intel) | Intel BlueZ + iwd team |
| **Code lineage** | 2003 (hostapd was wifi AP, supplicant added) | 2016 ground-up rewrite |

Intel hardware on the T450 is the iwd team's primary target. For an
Intel-only laptop the choice is unambiguous. iwd is also why we don't
need to build OpenSSL in this phase — the network stack has no TLS
client (yet); curl + browsers from nixpkgs will bring their own.

## Why dhcpcd and not just iwd for everything

iwd's built-in DHCP only handles **the wireless interface it's managing**.
Wired Ethernet (the I218-LM enpXsY device on T450) needs its own DHCP
client. Options:

- **dhcpcd** — modern, small, supports both v4 and v6, integrates with
  hostname / NTP / resolv.conf write-back. Default choice on many
  distros. ✓
- **dhclient** (ISC DHCP) — venerable, larger, ISC stopped maintaining
  it ~2022; replacement is Kea but Kea is server-only.
- **udhcpc** (BusyBox) — tiny but minimal, IPv4 only, no v6.
- **systemd-networkd** — has built-in DHCP, but we don't ship systemd.

dhcpcd it is. Single daemon, both v4 and v6, written by someone who
also writes BSD code so the codebase tends toward POSIX cleanliness.

We disable two stock dhcpcd hook scripts:
- `10-wpa_supplicant` — we don't have wpa_supplicant
- `15-timezone` — we set timezone in `/etc/localtime` once at install,
  no need to update it from DHCP options

## Kernel side — required configs

These should already be in `build/kernel-config-additions.fragment`
from Phase 2. Verifying here for the audit trail.

### Wired (Intel I218-LM)

```
CONFIG_E1000E=y                  # the I218-LM driver
CONFIG_NET_VENDOR_INTEL=y
CONFIG_INET=y                    # IPv4
CONFIG_IPV6=y                    # IPv6
CONFIG_PACKET=y                  # AF_PACKET (needed by dhcpcd)
CONFIG_UNIX=y                    # AF_UNIX
CONFIG_NETLINK=y                 # AF_NETLINK (iproute2, iwd)
```

### Wireless (Intel Wireless 7265)

```
CONFIG_CFG80211=y                # the 802.11 framework
CONFIG_MAC80211=y                # soft-MAC layer
CONFIG_IWLWIFI=m                 # base iwlwifi driver
CONFIG_IWLMVM=m                  # MVM firmware op-mode (7265 uses MVM)
CONFIG_IWLWIFI_OPMODE_MODULAR=y  # load op-mode modules on demand
CONFIG_CRYPTO_USER_API=y         # iwd uses this
CONFIG_CRYPTO_USER_API_HASH=y
CONFIG_CRYPTO_USER_API_SKCIPHER=y
CONFIG_KEYS=y                    # for kernel-side WPA key storage
CONFIG_CFG80211_DEFAULT_PS=y     # power saving default-on
# Deliberately NOT set:
# CONFIG_CFG80211_WEXT — old API, iwd uses nl80211 only
```

### Firmware blobs

The 7265 needs `iwlwifi-7265-ucode-*.ucode` from
`linux-firmware`. These are non-redistributable as kernel code but the
linux-firmware project ships them under a permissive license.

We stage them into the initramfs in **Phase 5** (initramfs build):

```
$INITRAMFS_ROOT/usr/lib/firmware/
├── iwlwifi-7265-9.ucode    (or -10.ucode, -16.ucode — kernel picks the
│                             highest-version blob it finds)
├── intel-ucode/             (CPU microcode — different topic but lives nearby)
└── i915/                    (Intel iGPU firmware for GuC/HuC)
```

The kernel loads them via the firmware loader (`/lib/firmware` search path).
On `iwlwifi` module load, the kernel walks `/lib/firmware/iwlwifi-7265-*.ucode`
and uses the newest version it can find. Boot logs will show
`iwlwifi 0000:03:00.0: loaded firmware version <ver>`.

## Service-unit shapes for Phase 9

```toml
# /etc/writeonce/services/network-wired.service.toml
[unit]
description = "Wired Ethernet DHCP (dhcpcd)"
after       = ["sys-subsystem-net-devices-enp0s31f6.device"]
# (the actual device name depends on PCI position — derived at install)

[service]
type        = "simple"
exec-start  = "/usr/sbin/dhcpcd -B -q enp0s31f6"  # -B = no-fork, -q = quiet
restart     = "on-failure"
restart-sec = "10s"
user        = "root"
group       = "root"

[install]
wanted-by   = ["network.target"]
```

```toml
# /etc/writeonce/services/iwd.service.toml
[unit]
description = "Wireless daemon (iwd)"
after       = ["dbus.service"]
requires    = ["dbus.service"]

[service]
type        = "simple"
exec-start  = "/usr/libexec/iwd"
restart     = "on-failure"
restart-sec = "5s"
user        = "root"   # needs CAP_NET_ADMIN; runs as root for simplicity
group       = "root"

[install]
wanted-by   = ["network.target"]
```

`iwctl` is the user-facing CLI; runs in the terminal:

```
$ iwctl
[iwd]# device list
[iwd]# station wlan0 scan
[iwd]# station wlan0 get-networks
[iwd]# station wlan0 connect "MyHomeWifi"
Type the network passphrase for MyHomeWifi
[iwd]#
```

iwd persists `/var/lib/iwd/MyHomeWifi.psk` after first connect; future
connects are auto.

## iproute2's quirks

iproute2 doesn't use autotools. It has a hand-rolled Makefile +
`./configure` that's really a feature-probe script. We cross-compile
by passing `CC=$LFS_TGT-gcc` to both `configure` and `make`, plus
`HOSTCC=cc` so any host tools built during make use the workstation's
toolchain (not the cross one).

`tc` (traffic control) is heavy and we never use it on a laptop. The
build always includes it; we can strip the `/usr/sbin/tc` binary in
Phase 10 if size matters.

## What the build deliberately skips

| Skipped | Why |
| --- | --- |
| **wpa_supplicant** | iwd is the choice. |
| **NetworkManager** | Adds D-Bus surface + GUI assumptions. Writeonce-svc orchestrates iwd + dhcpcd directly. |
| **systemd-networkd** | We don't ship systemd. |
| **ifupdown / netplan** | Debian / Ubuntu config models. Not relevant. |
| **net-tools** (`ifconfig`, `route`, `arp`) | Deprecated; iproute2 replaces. |
| **iptables (legacy)** | Replaced by nftables in-kernel. If a firewall is needed, nftables userspace can come in a future round. |
| **openvpn / strongSwan** | Application-level VPN. Defer; user-installable from nixpkgs. |
| **WireGuard userspace** (`wg`, `wg-quick`) | Kernel WireGuard is in-tree; userspace `wg-tools` is small — could add later. Skip for v1. |
| **bluez / Bluetooth networking** | Separate stack; future round. |
| **avahi (mDNS)** | "Find my printer" via Zeroconf. Not relevant for headless workstation. |
| **ntp / chrony** | Time sync. Should be added in Phase 9 — but a `systemd-timesyncd` equivalent in Rust would be Round 2e territory. For v1, install `chrony` from nixpkgs later. |

## Build times

| Package | Time |
| --- | --- |
| ell      | ~1 min  |
| iwd      | ~4 min  |
| iproute2 | ~3 min  |
| iputils  | ~1 min  |
| dhcpcd   | ~2 min  |
| **Total** | **~11 min** |

Tiny round. Use `--no-network`.

## What's in $LFS/usr after this round

```
/usr/sbin/
  ip               tc              ss              bridge      # iproute2
  dhcpcd
  iwd                                                          # via libexec? check
/usr/libexec/
  iwd                                                          # actual daemon
/usr/bin/
  iwctl            iwmon                                       # iwd CLI + sniffer
  ping             ping6           traceroute      arping      # iputils
  tracepath        clockdiff
/usr/lib/
  libell.so.0                                                  # ell shared object
/etc/iwd/                                                      # iwd config dir
/etc/iproute2/
  rt_protos        rt_tables       rt_scopes                   # iproute2 lookup tables
/var/lib/iwd/                                                  # iwd PSK store (created on first use)
/var/lib/dhcpcd/                                               # dhcpcd lease store
```

About **10 MB** added, bringing the running total to ~390 MB of native
userspace.

## Verifying it works (after Phase 9 boot)

```bash
# Wired
ip link show enp0s31f6                     # should show UP, has IPv4
ip route                                    # should show default via gateway
ping -c 1 8.8.8.8                           # should succeed

# Wireless
iwctl device list                           # should show wlan0
iwctl station wlan0 scan
iwctl station wlan0 get-networks            # should list visible APs
iwctl station wlan0 connect "MyHomeWifi"
ip addr show wlan0                          # should show IPv4 from iwd's DHCP
ping -c 1 1.1.1.1
```

The i3More network applet renders the same info — talks to iwd via D-Bus
for wireless, watches `ip monitor` (netlink) for link state.

## Future hardening passes

Things deliberately deferred from this round:

1. **Bluetooth** — bluez + bluez-alsa for headset audio. Separate stack
   with its own D-Bus services.
2. **NTP / time sync** — Round 2e (Rust port of `systemd-timesyncd`) or
   chrony from nixpkgs.
3. **VPN** — OpenVPN / WireGuard userspace, application-installed from
   nixpkgs.
4. **DNS resolver beyond glibc's `getaddrinfo`** — systemd-resolved
   adds local caching + DNSSEC. We rely on glibc + `/etc/resolv.conf`
   (written by dhcpcd) for v1. Future Round 2f could ship a Rust
   resolver shim.
5. **Firewall** — nftables userspace + a default ruleset. Add when
   needed.
6. **mDNS / Zeroconf** — avahi. Skip until use case appears.

For v1 — wired DHCP + WPA2 wireless + ping + ssh — these five
packages are the complete set.
