# Container support — what the kernel needs

> Companion to [`../../build/kernel-config-additions.fragment`](../../build/kernel-config-additions.fragment).
> Explains the kernel options WriteOnce adds so Docker, Podman, youki,
> and nerdctl can run without any patching. Also covers the
> developer-tooling toggles (BPF/BTF/ftrace/perf) that come along for
> the ride.

## The model

A "container" is not a single kernel feature. It is a layered cake of
six independent kernel primitives that, used together, give a process
its own apparent view of the system. A container runtime
(Docker/Podman/youki) is mostly the userspace code that sets up these
primitives correctly when spawning a process.

```
┌────────────────────────────────────────────────────────────┐
│  Container runtime (Docker / Podman / youki)               │
│    - sets up:                                              │
└──┬───────────────────────────────────────────────────────┬─┘
   │                                                       │
   ▼                                                       ▼
┌─────────────────┐  ┌──────────┐  ┌──────────┐  ┌───────────────┐
│ Namespaces      │  │ Cgroup v2│  │ Overlay  │  │ Bridge + veth │
│ (PID/mnt/net/   │  │ (cpu/mem/│  │ FS       │  │ + iptables/   │
│  user/IPC/UTS/  │  │  pids/   │  │ (image   │  │  nftables NAT │
│  cgroup/time)   │  │  io/...) │  │  layers) │  │               │
└─────────────────┘  └──────────┘  └──────────┘  └───────────────┘
   │                       │              │             │
   └───────────────────────┴──────────────┴─────────────┘
                           │
                           ▼
                  Plus seccomp BPF (syscall filtering)
                  and capabilities (privilege drop)
```

Each of these is a separate `CONFIG_*` switch in the kernel. WriteOnce
turns them all on.

## What each kernel option enables

### Namespaces — the isolation primitive

| Option              | Namespace type | What it isolates                                            |
| ------------------- | -------------- | ----------------------------------------------------------- |
| `CONFIG_NAMESPACES` | umbrella       | enables all namespace machinery                              |
| `CONFIG_PID_NS`     | PID            | the container sees its own PID 1, can't see host PIDs       |
| `CONFIG_NET_NS`     | network        | container has its own network interfaces, routes, ports     |
| `CONFIG_USER_NS`    | user           | container's UID 0 ≠ host UID 0 (rootless containers)        |
| `CONFIG_UTS_NS`     | UTS            | container's `hostname` is independent                       |
| `CONFIG_IPC_NS`     | IPC            | container's SysV IPC + POSIX message queues are separate    |
| `CONFIG_CGROUP_NS`  | cgroup         | container sees only its own cgroup subtree                  |
| `CONFIG_TIME_NS`    | time           | container can have a different system clock offset (newer)  |

A "container" in Docker terms is a process running with **all seven**
namespaces unshared from the host. `unshare(2)` and `clone3(2)` are the
syscalls the runtime invokes.

WriteOnce's `writeonce-svc` doesn't run containers directly — Docker
does. But the namespace machinery is needed for Docker to function, so
the kernel must have it compiled in.

### OverlayFS — the image-layer primitive

`CONFIG_OVERLAY_FS` enables a union filesystem where a writable upper
layer is stacked over one or more read-only lower layers. Docker images
are exactly that: each `FROM` / `RUN` line in a Dockerfile produces a
read-only layer; the running container has a writable layer on top.

Without overlayfs, Docker falls back to copy-up semantics (vfs driver),
which is slow and disk-expensive. With it, containers start in
milliseconds and share image storage.

- `CONFIG_OVERLAY_FS_REDIRECT_DIR` — needed for renames across layers.
- `CONFIG_OVERLAY_FS_INDEX` — speeds up the lookup of duplicated inodes.

### Bridge + veth — the network plumbing

A container's network namespace has no physical interface. Docker
solves this by creating a **veth pair**: two virtual network interfaces
connected like a cable. One end (eth0) lives in the container's
namespace; the other end (vethXXXX) lives in the host's namespace and
attaches to a bridge (docker0). The bridge then forwards packets out
through the host's real interface.

| Option            | Role                                                                  |
| ----------------- | --------------------------------------------------------------------- |
| `CONFIG_BRIDGE`   | enables the Linux software bridge (docker0, podman0, …)               |
| `CONFIG_VETH`     | the virtual ethernet pair driver                                      |
| `CONFIG_MACVLAN`  | alternative — give the container its own MAC on the host's NIC        |
| `CONFIG_IPVLAN`   | similar, sharing the host's MAC                                       |
| `CONFIG_VLAN_8021Q` | 802.1Q VLAN tagging, for tagged bridge networks                    |

### Netfilter — NAT, port forwarding, isolation

Containers expect to:
- Reach the internet (SNAT through the host).
- Be reachable on host-mapped ports (DNAT: `-p 8080:80`).
- Be isolated from each other (per-network ACL).

All of that is netfilter rules. Docker programs them via `iptables-nft`
(modern Docker) or `iptables-legacy` (older). The kernel options enable
both backends:

| Option               | What it provides                                |
| -------------------- | ----------------------------------------------- |
| `CONFIG_NETFILTER`   | umbrella                                        |
| `CONFIG_NF_TABLES`   | modern nftables backend                         |
| `CONFIG_IP_NF_*`     | classic iptables backend (kept for compat)      |
| `CONFIG_NF_NAT`      | NAT machinery (SNAT, DNAT, masquerade)          |
| `CONFIG_NF_CONNTRACK` | connection tracking — required for stateful NAT |

### Seccomp BPF — syscall filtering

`seccomp` lets a process declare "I will only call these syscalls; kill
me if I try anything else." Docker ships a default seccomp profile that
blocks ~50 syscalls deemed dangerous (`reboot`, `mount`, `swapon`,
`pivot_root`, the legacy `kexec` calls, …).

`CONFIG_SECCOMP_FILTER` is the BPF-program-based form, the one Docker
actually uses. The plain `CONFIG_SECCOMP` is the older 1990s mode and
not what's relevant today.

### Extra cgroup controllers

We already enable `CONFIG_CGROUPS`, `CONFIG_MEMCG`, `CONFIG_CGROUP_PIDS`,
`CONFIG_CGROUP_BPF`, `CONFIG_CGROUP_FREEZER` for the WriteOnce
supervisor's own service isolation. Docker uses a wider set:

| Controller             | What Docker uses it for                                       |
| ---------------------- | ------------------------------------------------------------- |
| `CONFIG_CPUSETS`       | `--cpuset-cpus`, pinning containers to CPU sets               |
| `CONFIG_CGROUP_CPUACCT`| per-container CPU accounting (`docker stats`)                  |
| `CONFIG_CGROUP_DEVICE` | `--device` / device cgroup whitelist                          |
| `CONFIG_CGROUP_HUGETLB`| huge-page accounting per container                             |
| `CONFIG_CGROUP_PERF`   | per-cgroup perf events                                         |
| `CONFIG_FAIR_GROUP_SCHED` | weighted CPU scheduling between cgroups                    |
| `CONFIG_BLK_CGROUP`    | per-container block I/O accounting + `--blkio-weight`          |
| `CONFIG_MEMCG_SWAP`    | per-container swap accounting (`--memory-swap`)                |

## Developer tooling — the BPF / BTF / ftrace / perf stack

A developer-grade workstation expects `strace`, `perf`, `bcc`,
`bpftrace`, `ftrace`, `kprobes` to all work out of the box. They share
a common kernel-feature surface:

| Option                       | Without it, what breaks                                |
| ---------------------------- | ------------------------------------------------------ |
| `CONFIG_BPF` + `CONFIG_BPF_SYSCALL` | no BPF programs at all (bpftrace, bcc, cilium fail) |
| `CONFIG_BPF_JIT`             | BPF programs interpret-only — much slower              |
| `CONFIG_DEBUG_INFO_BTF`      | bcc/bpftrace can't resolve kernel structures without external `kernel-debuginfo` |
| `CONFIG_DEBUG_INFO_BTF_MODULES` | same, for out-of-tree modules                      |
| `CONFIG_PERF_EVENTS`         | `perf` doesn't function at all                          |
| `CONFIG_HW_PERF_EVENTS`      | `perf stat -e cycles,instructions` doesn't see hardware counters |
| `CONFIG_FUNCTION_TRACER` + `CONFIG_DYNAMIC_FTRACE` | `ftrace`, function-graph tracing, `trace-cmd` non-functional |
| `CONFIG_FTRACE_SYSCALLS`     | `trace-cmd record -e syscalls:*` doesn't work          |
| `CONFIG_KPROBES`             | bcc tools that attach to kernel functions fail         |
| `CONFIG_UPROBES`             | `bpftrace 'uprobe:...'` fails                          |

The BTF info in particular is a big quality-of-life win: without
`CONFIG_DEBUG_INFO_BTF=y`, every bcc/bpftrace invocation either fails
or requires a ~500 MB kernel-debuginfo download to work.

## What WriteOnce does *not* enable (deliberately)

| Option                    | Why we skip                                                |
| ------------------------- | ---------------------------------------------------------- |
| `CONFIG_SELINUX`          | i3More / typical workstation use doesn't need it; SELinux policies are an ongoing maintenance cost |
| `CONFIG_APPARMOR`         | same                                                       |
| `CONFIG_DEFAULT_SECURITY_SMACK` | same                                                |
| `CONFIG_AUDIT`            | adds noise to a single-user laptop; can be enabled later if needed |
| `CONFIG_DEBUG_KERNEL`     | adds kernel size; toggle on for debug builds only          |

If a future workload demands one of these (e.g. running a hardened
container with SELinux confinement), turn it on then. The fragment is
the canonical place to record the decision with a comment.

## How to verify after a rebuild

After re-running `./04-kernel.sh` with the expanded fragment, boot the
kernel (in QEMU or on the T450) and run:

```bash
# Confirm namespaces:
zgrep -E 'CONFIG_(USER|PID|NET|MNT|UTS|IPC|CGROUP|TIME)_NS' /proc/config.gz
# Confirm overlayfs:
zgrep 'CONFIG_OVERLAY_FS' /proc/config.gz
# Confirm container cgroups:
ls /sys/fs/cgroup/   # expect: cgroup.controllers, cpu.stat, memory.stat, …
cat /sys/fs/cgroup/cgroup.controllers   # expect: cpuset cpu io memory hugetlb pids …
# Confirm seccomp:
zgrep 'CONFIG_SECCOMP_FILTER' /proc/config.gz
# Confirm BPF + BTF:
ls /sys/kernel/btf/vmlinux   # exists when CONFIG_DEBUG_INFO_BTF=y was set
```

When all those pass, Docker installed via Nix will work without further
kernel work.

## Triggering a rebuild

The kernel-config step has a sentinel at `build/logs/.done-kernel-config`.
After editing the fragment:

```bash
rm build/logs/.done-kernel-config build/logs/.done-kernel-build
cd build && ./04-kernel.sh
```

`merge_config.sh` will pick up the new fragment lines and
`olddefconfig` will resolve dependencies. Then the kernel is rebuilt
(~5–15 min on the workstation).
