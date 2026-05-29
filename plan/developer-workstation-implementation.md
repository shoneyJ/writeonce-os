# WriteOnce OS — Developer-Workstation Implementation Plan

> Refines [`00-roadmap.md`](00-roadmap.md) with a sharpened scope:
> **WriteOnce OS is a developer-grade workstation.** Rust crates and the
> Linux kernel own the boot path through login. After login, the system
> runs unmodified upstream software (Docker, Zen browser, Alacritty,
> nix-managed packages, etc.). No reinvention of components that already
> exist and are well-maintained upstream.

---

## Context

**Target persona.** A working developer who wants to install and use
WriteOnce as their daily workstation. They expect:

- To boot a laptop (T450 today; commodity x86_64 UEFI machines in
  general) into a graphical session.
- A terminal emulator (Alacritty), a browser (Zen), an editor of their
  choice, network access, audio, suspend/resume, screen lock.
- Container workflows (Docker / Podman; OCI images, build/run, networking).
- A package manager that lets them install ~80k packages without us
  curating any of them.

**Where WriteOnce earns its keep.** Not in re-implementing applications.
The project's contribution is **the boot path itself**: a custom Rust
PID 1, a custom Rust service supervisor, a custom Rust initramfs, a
custom Rust UEFI bootloader, a Rust login binary. From login onward,
the system runs upstream code unmodified.

**The line.** Everything *up to and including* the first interactive
shell after PAM authentication is WriteOnce code (Rust binaries + a
configured Linux kernel). Everything *beyond* that — the X server, the
window manager, GTK, browsers, containers, package management — is
unmodified upstream.

## The Rust crate surface (bounded — 5 crates)

| Crate                     | Role                                                       | Status                  |
| ------------------------- | ---------------------------------------------------------- | ----------------------- |
| `writeonce-pid1`          | PID 1: reaping, mounts, signal dispatch, orderly reboot.    | **Prototype done** (990 KB, musl-static, tested). |
| `writeonce-svc`           | Supervisor: service descriptors, dependency graph, cgroup placement, logind D-Bus shim. | Designed; code pending. |
| `writeonce-initramfs`     | `/init` Rust binary: module load, root discovery, `pivot_root`, exec PID 1. | Designed; code pending. |
| `writeonce-bootloader`    | UEFI app via `uefi-rs`: read kernel+initramfs from ESP, build `boot_params`, EFI handover entry. | Designed; code pending. |
| `writeonce-login`         | Console PAM authentication; exec the user's session-starter (X+i3+i3More). | Designed; code pending. |

**Total expected scale:** ~5000 lines of Rust across the five crates.
That's the entire bespoke surface; the rest of the OS is configuration
+ upstream builds.

## The kernel surface

Linux 6.12 LTS, **unmodified**, with a WriteOnce-curated `.config`. The
config is the project's artefact, not patches. Two notable expansions
beyond the current `kernel-config-additions.fragment`:

1. **Container support** — namespaces (PID, mount, net, user, IPC, UTS,
   cgroup), overlayfs, bridge, veth, netfilter, seccomp BPF, MEMCG-swap.
   Required for Docker/Podman/youki.
2. **Developer ergonomics** — perf events, BPF + BTF, ftrace, kprobes,
   ptrace, KGDB. So `strace`, `perf`, `bcc`, `bpftrace` work.

Rust kernel modules (Phase 7 stretch goal) remain a pedagogical
exercise, not a load-bearing component of the workstation.

## The upstream userspace stack (build from source, LFS-style)

Curated, source-built from kernel.org/gnu.org/freedesktop.org. No
patches. No deviation from upstream defaults beyond what's strictly
needed for cross-build:

| Layer        | Packages                                                                       |
| ------------ | ------------------------------------------------------------------------------ |
| C library    | glibc (already built in Phase 0)                                               |
| Toolchain    | binutils, gcc (already built in Phase 0)                                       |
| Base utils   | bash, coreutils, util-linux, sed, awk, grep, tar, gzip, xz (already)           |
| Graphics     | mesa-libs, libdrm, libX11+xcb suite, xorg-server (modesetting), xkbcommon       |
| Toolkits     | glib, gobject-introspection, cairo, pango, harfbuzz, gdk-pixbuf, gtk4, graphene |
| WM           | i3 (upstream)                                                                  |
| IPC          | dbus + libdbus                                                                 |
| Auth         | linux-pam                                                                      |
| Audio        | alsa-lib, pipewire, wireplumber                                                |
| Fonts        | freetype, fontconfig, dejavu-fonts                                             |
| Net basics   | iproute2, iputils, dhcpcd (or systemd-networkd alternative)                    |
| Net mgmt     | iwd (Intel wireless daemon — lighter than NetworkManager, Rust-friendly D-Bus surface) |
| Container reqs | iptables-nft, bridge-utils (kernel features do the heavy lifting)            |
| Developer    | git, openssh, gnupg (most installed via nix; only sshd in the base)             |

Estimated source-build count: ~80 packages on top of LFS Ch. 6. Big but
finite. The `build/` script directory grows with `08-x11-stack.sh`,
`09-gtk4-stack.sh`, `10-audio-stack.sh`, `11-network-stack.sh`,
`12-i3-and-i3more.sh`, plus a `BLFS-pkg.sh` helper that takes a package
name + configure flags and does the boring `configure && make &&
make install` dance.

## Package management — adopt Nix (don't reinvent)

The most defensible "no reinvention" choice. Nix gives WriteOnce:

- **Atomic, transactional installs** — interruptible without breaking the system.
- **Multiple versions of one package** without conflicts.
- **Rollback** — every profile generation is preserved until garbage-collected.
- **Reproducible per-user environments** via flakes + `flake.lock`.
- **~80,000 packages** in nixpkgs, including Docker, Zen browser,
  Alacritty, every IDE, every language toolchain.

Integration approach: **Nix in single-user mode**, store at `/nix/store`,
profile at `/home/<user>/.nix-profile`, no nix-daemon at first. The
later upgrade to nix-daemon (multi-user) is a one-command swap once the
base system is stable.

WriteOnce-specific Nix wiring:

1. `build/13-install-nix.sh` — bootstraps Nix into the sysroot from the
   upstream Nix binary release tarball. (Nix itself is C++; we don't
   build it from source — accepting that as part of "no reinvention.")
2. `/etc/nix/nix.conf` — pre-configured with `experimental-features =
   nix-command flakes`, `auto-optimise-store = true`.
3. A default-profile flake at `/etc/writeonce/profile.nix` that ships
   docker, alacritty, zen-browser, git, neovim, ripgrep, bat, fd, fzf,
   tmux, htop. Users opt out or extend via their own flake.

What about Docker daemon: installed via Nix as `pkgs.docker`. The
WriteOnce supervisor manages `docker.service` (a unit file in
`/etc/writeonce/services/`) that starts `dockerd` after `network.target`.

## Applications the workstation must run, and where each comes from

| Application       | Source                                       | Service unit?                          |
| ----------------- | -------------------------------------------- | -------------------------------------- |
| **Docker**        | `nixpkgs#docker`                             | Yes — `docker.service` in `/etc/writeonce/services/` |
| **Alacritty**     | `nixpkgs#alacritty`                          | No (launched by i3 keybinding)         |
| **Zen browser**   | `nixpkgs#zen-browser`                        | No (launched by user)                  |
| **i3**            | Built from upstream source (Phase 8)         | No (launched by user session)          |
| **i3More**        | Built from your `i3More/` repo (Phase 9)     | No (autostart from i3 config)          |
| **OpenSSH server**| Built from upstream OR `nixpkgs#openssh`     | Yes — `sshd.service`                   |
| **PipeWire**      | Built from upstream (Phase 8)                | Yes — `pipewire.service`               |
| **D-Bus**         | Built from upstream (Phase 8)                | Yes — `dbus.service`                   |
| **Xorg**          | Built from upstream (Phase 8)                | Yes — `xorg.service` per session       |
| **iwd**           | Built from upstream OR `nixpkgs#iwd`         | Yes — `iwd.service` (Wi-Fi)            |

Direction of travel: anything **system-level** is built from source and
managed by `writeonce-svc`. Anything **user-level** (browser, editor,
terminal) comes from Nix and is installed per-user. The cleanest
separation between "what WriteOnce maintains" and "what's just there."

## Phase-by-phase plan (refined)

The original 10-phase roadmap stays, with edits:

| #  | Phase                                                        | Status                                  | Refinement                |
| -- | ------------------------------------------------------------ | --------------------------------------- | ------------------------- |
| 0  | Workstation cross-compile environment                        | ✓ done                                  | unchanged                 |
| 1  | T450 prep (rescue USB, netconsole, firmware archive)         | partial — script exists, exec pending   | unchanged                 |
| 2  | LFS-style minimal Linux + transitional BusyBox initramfs      | partial — kernel built, initramfs+QEMU smoke pending | unchanged       |
| 3  | Rust PID 1                                                   | ✓ prototype done                        | Phase 4 expansion needed  |
| 4  | Rust supervisor + cgroup v2 + logind shim                    | designed, code pending                  | **expand kernel config for containers; add `docker.service` template** |
| 5  | Rust initramfs                                               | designed                                | unchanged                 |
| 6  | Rust UEFI bootloader                                         | designed                                | unchanged                 |
| 7  | Kernel customization + Rust kernel module experiment         | designed                                | **expand to include perf/bpf/ftrace developer toggles** |
| 8  | X11 substrate (Xorg + i3 + GTK4 + D-Bus + PAM + PipeWire)     | designed                                | unchanged                 |
| 9  | i3More integration + login flow                              | designed                                | `writeonce-login` crate here |
| 10 | Packaging + Nix integration + install ISO                    | designed                                | **adopt Nix as the package manager; default profile flake** |

Two phases are folded in:

- **iwd** (Intel wireless daemon) replaces NetworkManager in Phase 8.
  Smaller, written in C against the kernel's ioctl-based wireless stack,
  has a clean D-Bus surface for status reporting.
- **Nix bootstrap** is the Phase 10 deliverable — not a custom package
  manager.

## Detailed Phase 4 expansion (next-up, was pending)

Now that the scope is "developer workstation", Phase 4 grows in two
ways:

1. **Container kernel features.** Extend
   `build/kernel-config-additions.fragment` with namespace, overlayfs,
   netfilter, bridge, seccomp, veth options. (Concrete patch listed
   below.)
2. **Pre-canned service units.** The supervisor's
   `/etc/writeonce/services/` ships with descriptors for the daemons
   the workstation expects: `dbus.service`, `pipewire.service`,
   `wireplumber.service`, `iwd.service`, `sshd.service`,
   `docker.service`, plus targets `multi-user.target` and
   `graphical.target`.

### Kernel config additions (concrete patch to `kernel-config-additions.fragment`)

```
# --- Namespaces (Docker/Podman/youki) ---
CONFIG_NAMESPACES=y
CONFIG_UTS_NS=y
CONFIG_IPC_NS=y
CONFIG_USER_NS=y
CONFIG_PID_NS=y
CONFIG_NET_NS=y
CONFIG_CGROUP_NS=y
CONFIG_TIME_NS=y

# --- Overlayfs (container layered images) ---
CONFIG_OVERLAY_FS=y
CONFIG_OVERLAY_FS_REDIRECT_DIR=y
CONFIG_OVERLAY_FS_INDEX=y

# --- Bridge + veth (container networking) ---
CONFIG_BRIDGE=y
CONFIG_BRIDGE_NETFILTER=m
CONFIG_VLAN_8021Q=m
CONFIG_VETH=m
CONFIG_MACVLAN=m
CONFIG_IPVLAN=m

# --- Netfilter / iptables / nftables ---
CONFIG_NETFILTER=y
CONFIG_NETFILTER_XTABLES=y
CONFIG_NF_TABLES=m
CONFIG_NF_TABLES_INET=y
CONFIG_NF_TABLES_NETDEV=y
CONFIG_NFT_NAT=m
CONFIG_NFT_MASQ=m
CONFIG_NF_NAT=m
CONFIG_NF_CONNTRACK=m
CONFIG_IP_NF_IPTABLES=m
CONFIG_IP_NF_FILTER=m
CONFIG_IP_NF_NAT=m

# --- Seccomp BPF (container sandboxing) ---
CONFIG_SECCOMP=y
CONFIG_SECCOMP_FILTER=y

# --- BPF + BTF (perf, bcc, bpftrace) ---
CONFIG_BPF=y
CONFIG_BPF_SYSCALL=y
CONFIG_BPF_JIT=y
CONFIG_BPF_JIT_ALWAYS_ON=y
CONFIG_DEBUG_INFO_BTF=y
CONFIG_DEBUG_INFO_BTF_MODULES=y

# --- Tracing / perf ---
CONFIG_FUNCTION_TRACER=y
CONFIG_DYNAMIC_FTRACE=y
CONFIG_FTRACE_SYSCALLS=y
CONFIG_KPROBES=y
CONFIG_UPROBES=y
CONFIG_PERF_EVENTS=y
CONFIG_HW_PERF_EVENTS=y

# --- Container-needed cgroup controllers ---
CONFIG_CPUSETS=y
CONFIG_CGROUP_CPUACCT=y
CONFIG_CGROUP_DEVICE=y
CONFIG_CGROUP_HUGETLB=y
CONFIG_CGROUP_PERF=y
CONFIG_CGROUP_SCHED=y
CONFIG_FAIR_GROUP_SCHED=y
CONFIG_BLK_CGROUP=y
CONFIG_MEMCG_SWAP=y

# --- Capabilities, swap, hugetlb ---
CONFIG_HUGETLBFS=y
CONFIG_SWAP=y
```

That's ~40 added lines. After landing it, re-run `./04-kernel.sh` (the
sentinel system will detect the fragment change and request rebuild) and
the resulting kernel is Docker-ready.

### Service unit catalogue (`/etc/writeonce/services/`)

| Unit                  | `[Unit]` deps               | `[Service]` summary                                          |
| --------------------- | --------------------------- | ------------------------------------------------------------ |
| `dbus.service`        | —                           | `exec-start = "/usr/bin/dbus-daemon --system --nofork"`     |
| `iwd.service`         | `after = ["dbus.service"]`  | `exec-start = "/usr/libexec/iwd"`                            |
| `network-online.target` | `wants = ["iwd.service"]` | virtual; reached when iwd reports a routable IP             |
| `sshd.service`        | `after = ["network-online.target"]` | `exec-start = "/usr/sbin/sshd -D"`                  |
| `docker.service`      | `after = ["network-online.target"]` | `exec-start = "/run/current-system/sw/bin/dockerd"` |
| `pipewire.service`    | (user-scoped, Phase 9)      | `exec-start = "/usr/bin/pipewire"`                           |
| `wireplumber.service` | `after = ["pipewire.service"]` | `exec-start = "/usr/bin/wireplumber"`                    |
| `xorg.service`        | `after = ["dbus.service"]`  | `exec-start = "/usr/bin/Xorg :0 vt7 -keeptty"`               |
| `multi-user.target`   | `requires = ["basic.target"]` | target                                                    |
| `graphical.target`    | `requires = ["multi-user.target"]` | target                                                |

These ship in the WriteOnce installer; `writeonce-svc` loads them at boot.

## Phase 9 expansion — login flow

`writeonce-login` is a small Rust binary, ~250 lines, that:

1. Renders a prompt on tty1: hostname, username field, password field.
2. Calls PAM: `pam_start("login", username)`, `pam_authenticate`,
   `pam_acct_mgmt`, `pam_setcred(PAM_ESTABLISH_CRED)`, `pam_open_session`.
3. On success: `setresgid`/`setresuid` to the authenticated user,
   `chdir(/home/user)`, sets `XDG_RUNTIME_DIR=/run/user/<uid>`,
   `execve("/etc/writeonce/session-start.sh")`.
4. The session-start script invokes `dbus-launch --exit-with-session
   startx /etc/writeonce/xinitrc`.
5. xinitrc runs i3 + i3More autostarts.

Crate dependencies: `libc`, `pam-sys` (FFI to libpam), `serde` + `toml`
for `/etc/writeonce/login.toml`.

No graphical display manager initially; `writeonce-login` is the console
DM. A graphical DM (`writeonce-greeter`, GTK4) is a follow-up.

## Phase 10 expansion — Nix integration + ISO

### Nix bootstrap (`build/13-install-nix.sh`)

```
1. Download nix-2.x-x86_64-linux.tar.xz from upstream + verify GPG.
2. Extract into $LFS/tmp/nix-staging/.
3. Run the upstream nix install in single-user mode targeting $LFS:
       NIX_INSTALLER_NO_CHANNEL_ADD=1 \
       NIX_INSTALLER_NO_MODIFY_PROFILE=1 \
       sh nix-staging/install --no-daemon --tarball-url-prefix \
           "file://$LFS/tmp/nix-staging"
4. Stage /etc/nix/nix.conf with WriteOnce-recommended settings.
5. Stage /etc/writeonce/default-profile.nix with the developer baseline
   (docker, alacritty, zen-browser, git, openssh, neovim, …).
6. Add a writeonce-svc service unit `nix-bootstrap.service` that on
   first boot runs `nix profile install $(cat /etc/writeonce/default-profile.nix)`
   as the user, then disables itself.
```

### Installer ISO (`build/14-iso.sh`)

Hybrid UEFI ISO via `xorriso`. Contents:

- `/EFI/BOOT/BOOTX64.EFI` — the WriteOnce bootloader (`writeonce-bootloader`).
- `/bzImage`, `/initramfs.img` — live kernel + initramfs.
- `/sysroot.tar.xz` — the entire installable sysroot.
- `/install/` — `writeonce-install` Rust binary that:
  - Prompts for target disk.
  - Partitions GPT (ESP, /boot, /, /home).
  - mkfs + mounts target.
  - Extracts `sysroot.tar.xz` to target.
  - Runs `efibootmgr` to register the bootloader.
  - Configures `iwd` for the user's chosen Wi-Fi network (optional).
  - Pre-stages the Nix store for the first boot.

## Verification — what "done" means for the developer workstation

1. **Boot to login.** Power-on T450 → WriteOnce bootloader → kernel →
   Rust initramfs → Rust PID 1 → Rust supervisor brings up `dbus`,
   `iwd`, network → Rust login prompt on tty1. Time: < 15 seconds cold
   boot.
2. **Login starts the desktop.** Type creds → PAM ok → X.Org + i3 + i3More.
3. **Network works.** `ping 1.1.1.1` over Wi-Fi.
4. **SSH in.** From the workstation: `ssh writeonce@<t450-ip>` succeeds.
5. **Nix works.** `nix --version` shows expected version; `nix profile
   list` shows the default-profile contents.
6. **Docker runs.** `docker pull alpine && docker run --rm alpine echo
   ok` prints `ok`.
7. **Alacritty runs.** `Mod+Enter` in i3 spawns Alacritty.
8. **Zen browser runs.** Launch from i3More; opens a window, loads
   `https://example.com`.
9. **`i3more-lock` locks the screen.** `Mod+Shift+L` blanks the display;
   PAM auth unlocks it. `writeonce-svc`'s minimal logind shim satisfies
   the `Inhibit()` call.
10. **`perf`, `strace`, `bpftrace` all work.** Developer tooling
    feeds off the kernel BPF/perf/ftrace toggles.

When all 10 pass, the developer workstation is real.

## What's explicitly NOT in scope

| Reinvented? | What                                                                                  |
| ----------- | ------------------------------------------------------------------------------------- |
| No          | Xorg, i3, GTK4, D-Bus, PAM, PipeWire, iwd, Nix — built/installed from upstream.        |
| No          | The Docker daemon, the runc/containerd stack — installed from Nix.                     |
| No          | The browser, terminal emulator, editors — installed from Nix.                          |
| Yes         | PID 1 (`writeonce-pid1`).                                                              |
| Yes         | Service supervisor (`writeonce-svc`).                                                  |
| Yes         | initramfs `/init` (`writeonce-initramfs`).                                             |
| Yes         | UEFI bootloader (`writeonce-bootloader`).                                              |
| Yes         | Console login (`writeonce-login`).                                                     |
| Yes (small) | A `writeonce-install` Rust binary used by the ISO. Phase 10.                           |

That's the entire bespoke surface. **Five Rust crates plus the installer
glue. ~5–6 KLOC total.** The rest is configuration of upstream software.

## Critical files (paths) — refined

**Already exist:**

```
build/00-check-host.sh        plan/00-roadmap.md          docs/learning/00-concepts-coverage.md
build/01-fetch.sh             plan/phase-0..10-*.md       docs/learning/phase-0-*.md
build/02-cross-toolchain.sh                               docs/learning/phase-2-*.md
build/03-sysroot-temp-tools.sh                            docs/learning/phase-4-*.md
build/04-kernel.sh                                        docs/learning/systemd-feature-survey.md
build/05-initramfs.sh
build/06-qemu-smoke.sh
build/07-bootable-usb.sh
build/kernel-config-additions.fragment
crates/writeonce-pid1/        (5 src/*.rs files + Cargo.toml + tests)
Cargo.toml + rust-toolchain.toml + .cargo/config.toml
```

**To create as this plan executes:**

```
build/08-x11-stack.sh             — Xorg + i3 + xkb + libinput
build/09-gtk4-stack.sh            — glib + cairo + pango + gtk4
build/10-audio-stack.sh           — alsa-lib + pipewire + wireplumber
build/11-network-stack.sh         — iproute2 + iwd + iputils + dhcpcd-ish
build/12-i3-and-i3more.sh         — i3 (upstream) + i3More (your repo)
build/13-install-nix.sh           — Nix bootstrap, default profile
build/14-iso.sh                   — Hybrid UEFI installer ISO
crates/writeonce-svc/             — Phase 4 supervisor + cgroup placement + logind shim
crates/writeonce-initramfs/       — Phase 5 Rust /init
crates/writeonce-bootloader/      — Phase 6 UEFI app (uefi-rs)
crates/writeonce-login/           — Phase 9 PAM-based console login
crates/writeonce-install/         — Phase 10 ISO installer
plan/done/phase-4-supervisor.md        — already exists; gets updated with container kernel features
plan/phase-10-packaging.md        — already exists; gets updated with Nix adoption
```

## Execution order (one phase per round)

Each step below is an "implement this in a focused round" milestone. Not
attempted in a single turn.

1. **Expand kernel config for containers.** Add the ~40 lines to
   `kernel-config-additions.fragment`. Re-run `./04-kernel.sh`. Verify
   `zcat /proc/config.gz | grep CONFIG_USER_NS=y` succeeds when booted.
2. **Build and run the initramfs** (`./05-initramfs.sh && ./06-qemu-smoke.sh`).
   First end-to-end QEMU boot. **(this is your immediate next action.)**
3. **Bootable USB on the T450** (`./07-bootable-usb.sh /dev/sdX`). First
   real-hardware boot.
4. **Scaffold `writeonce-svc`** — Cargo crate with the unit-file loader,
   topological sort, `clone3(CLONE_INTO_CGROUP)` spawner. Implement one
   trivial service (`hello.service`) end-to-end. ~1500 LOC.
5. **Scaffold `writeonce-login`** — PAM auth, exec session-starter. ~250 LOC.
6. **Phase 8 userspace builds** — `build/08-x11-stack.sh`, `09-gtk4-stack.sh`,
   `10-audio-stack.sh`, `11-network-stack.sh`. Largest single chunk of work
   (~30 packages). Source the build commands from BLFS.
7. **Phase 9 i3More integration** — `12-i3-and-i3more.sh`. Cross-compile
   i3More from your repo against the Phase 8 substrate. Author service
   units + xinitrc.
8. **Phase 5 Rust initramfs** — replace BusyBox initramfs.
9. **Phase 6 Rust UEFI bootloader** — replace GRUB.
10. **Phase 10 Nix bootstrap + installer ISO** — `13-install-nix.sh`,
    `14-iso.sh`, `writeonce-install` crate.

Each round is bounded (1–3 days of focused work for me to draft + you
to review + iterate). Total to first usable workstation:
8–10 rounds.

## What I'm doing in *this* turn

Just writing this plan. Not implementing the next phase yet — that's the
next turn.

When you say go, the next round delivers:

1. The kernel-config expansion for containers (concrete patch listed above).
2. A learning doc `docs/learning/containers-kernel-requirements.md` so the
   additions are explainable.
3. A scaffold of `crates/writeonce-svc/` matching the writeonce-pid1
   pattern — Cargo.toml, src/*.rs stubs that compile, a unit test that
   exercises the service.toml parser.

That's the immediate Phase 4 step. Subsequent phases follow in the order
above.
