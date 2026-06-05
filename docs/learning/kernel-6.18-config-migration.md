# Kernel 6.12.10 → 6.18.34 LTS migration

**Date:** 2026-06-04
**Why:** Move the boot kernel from the 6.12 LTS series to the newest longterm
series. Per kernel.org's `releases.json` on this date, the longterm kernels are
6.18 / 6.12 / 6.6 / 6.1 / 5.15 / 5.10; 7.0.11 is the latest *stable* but is **not**
longterm (7.1 was already at rc6, so 7.0 loses backports within weeks). 6.18.34 is
therefore the "latest LTS stable version".

The bump is the mechanical pin-and-rebuild flow the build was designed for — only
`build/versions.env` and `build/checksums.txt` change; `04-kernel.sh` is
version-agnostic. Trust anchor for the new tarball is the GPG signature
(`build/keys/linux.asc` = Greg KH's stable-release key) over the decompressed
tarball; the SHA-256 (`640c4732…d760`) was locked into `checksums.txt` only after
`gpg: OK`.

## Config drift (the part that needed judgment)

The base config (`build/kernel-base.config`) is an Ubuntu **6.8.12** defconfig, so
`make olddefconfig` reconciles a 6.8 base against 6.18 source (10 minor versions),
then the T450 fragment (`build/kernel-config-additions.fragment`) is re-asserted on
top via `merge_config.sh -m`.

**Every T450-critical driver survived** the reconciliation (verified in the final
`build/artifacts/kernel.config`):

| Subsystem | Symbol | Final |
|---|---|---|
| Ethernet (I218) | `CONFIG_E1000E` | `=y` |
| WiFi (Intel 7265) | `CONFIG_IWLWIFI` / `CONFIG_IWLMVM` | `=m` |
| Graphics (HD 5500) | `CONFIG_DRM_I915` | `=y` |
| Audio | `CONFIG_SND_HDA_INTEL` / `..._CODEC_REALTEK` | `=m` |
| Broadwell PMC | `CONFIG_INTEL_PMC_CORE` | `=m` |
| Storage | `CONFIG_SATA_AHCI` / `CONFIG_BLK_DEV_NVME` | `=y` / `=m` |
| Root FS | `CONFIG_EXT4_FS` | `=y` |
| UEFI boot | `CONFIG_EFI_STUB` / `CONFIG_EFI_PARTITION` / `CONFIG_FB_EFI` | `=y` |

The "Value of CONFIG_X is redefined by fragment" lines in
`build/logs/kernel-mergeconfig.log` are expected and benign — that is the fragment
overriding the Ubuntu base, which is its entire job.

### 8 fragment symbols dropped by olddefconfig — all benign, none new to 6.18

`merge_config.sh -m` does not run the final verification pass, so a fragment symbol
that 6.18 renamed/removed is silently dropped by the subsequent `olddefconfig`.
Eight were dropped; each was cross-checked against the v6.18 Kconfig and its parent
symbol in the final config. **None cost the T450 any capability, and all stem from
upstream changes in 6.1–6.6 — i.e. they were already being dropped under the old
6.12 pin, so the bump introduces no new regression.**

| Dropped | Status in 6.18 | Why benign |
|---|---|---|
| `CONFIG_CGROUP_NS` | removed | cgroup namespaces always-on with `CONFIG_NAMESPACES=y` ✓ |
| `CONFIG_MEMCG_SWAP` | removed (6.1) | swap accounting folded into `CONFIG_MEMCG=y` (+`SWAP=y`), runtime-controlled ✓ |
| `CONFIG_MICROCODE_INTEL` | removed (6.6) | Intel µcode included whenever `CONFIG_MICROCODE=y` + `CONFIG_CPU_SUP_INTEL=y` ✓ — **Broadwell microcode updates intact** |
| `CONFIG_HW_PERF_EVENTS` | ARM-only symbol | x86 hardware perf comes via `CONFIG_PERF_EVENTS=y` ✓; the fragment line was always a no-op on x86 |
| `CONFIG_DEBUG_INFO_BTF` | exists, dep unmet | BTF needs `pahole`(dwarves) at build time; the `wo-builder` image lacks it. Non-critical (BPF works; only CO-RE tooling wants `/sys/kernel/btf/vmlinux`) |
| `CONFIG_DEBUG_INFO_BTF_MODULES` | exists, dep unmet | same as above (depends on BTF) |
| `CONFIG_IP_NF_FILTER` | exists, dep unmet | legacy xtables split behind `CONFIG_IP_NF_IPTABLES_LEGACY` (~6.4), which is off. NAT/filter available via `CONFIG_NF_TABLES=m` + `CONFIG_NF_NAT=m` ✓ |
| `CONFIG_IP_NF_NAT` | exists, dep unmet | same legacy-split reason |

### Optional follow-ups (not done here — pre-existing, not blocking the boot)

These are stale/no-op fragment entries that predate this bump. Left untouched to keep
the diff surgical; listed so a future maintainer can decide:

1. **Prune the 4 confirmed-dead entries** from `kernel-config-additions.fragment`:
   `CGROUP_NS`, `MEMCG_SWAP`, `MICROCODE_INTEL` (removed upstream) and
   `HW_PERF_EVENTS` (ARM-only). They no longer name real x86/6.18 symbols.
2. **Restore BTF** (if eBPF CO-RE tooling is ever wanted): add `dwarves` to
   `build/Containerfile` (host-side fix per the host-vs-target rule), then
   `DEBUG_INFO_BTF` will satisfy its dependency on rebuild.
3. **Restore legacy iptables modules** (only if something needs the `iptable_nat`
   kernel module rather than nftables): add `CONFIG_IP_NF_IPTABLES_LEGACY=y` to the
   fragment.

## Build defects found and fixed during the bump

The version bump itself was clean, but staging the modules surfaced three real
defects — one of which had been **latent since the 6.12 build** and silently
shipped an empty module set.

### 1. Module auto-signing broke `modules_install` (config)

The Ubuntu base config carries `CONFIG_MODULE_SIG=y` + `CONFIG_MODULE_SIG_ALL=y`,
so `make modules_install` runs `scripts/sign-file` on every `.ko`. In the
cross-build that aborts on the **first** module:

```
SIGN  …/amd-uncore.ko
- SSL error:1E08010C:DECODER routines::unsupported   (sign-file can't load a key)
make[1]: *** modules_install Error 2
```

We don't enforce signatures (`MODULE_SIG_FORCE` is off, Secure Boot is off), so
the signing is pure dead weight. **Fix:** `# CONFIG_MODULE_SIG_ALL is not set`
in `kernel-config-additions.fragment`. That symbol gates only the install
Makefile (no compiled code), so it applies with **no recompile** — for the
already-built tree it was enough to flip the line in `.config`, run
`make syncconfig` to regenerate `include/config/auto.conf`, and re-stage.

### 2. A failed `modules_install` was silently sentinel'd (build script)

`step_kernel-modules-stage` in `build/04-kernel.sh` piped `make … | tee` with no
`PIPESTATUS` check, so the signing failure above exited 0, the step was marked
done, and a tree with **zero `.ko` files** (only `modules.builtin*` metadata) was
staged. The `kernel-build` step already guards this exact trap; the modules-stage
step did not. **Fix:** capture `rc=${PIPESTATUS[0]}` and `return 1` on non-zero.

This is why the defect went unnoticed under 6.12: the T450's essential drivers
(`e1000e`, `i915`, AHCI, ext4) are `=y` builtin, so a console smoke-boot succeeds
with no modules at all — only the `=m` set (wifi, audio) was missing, and the
smoke test never exercised it.

### 3. Build image had no `depmod` (Containerfile)

`make modules_install` warned `requires depmod … probably in the kmod package`,
so `modules.dep` / `modules.alias` were never generated — without them udev can't
autoload `=m` drivers by modalias on the real T450. Per the host-vs-target rule,
**fix the host:** added `kmod` to `build/Containerfile`'s apt list, rebuilt the
image, and re-staged so `depmod` runs as part of the build.

## Module-tree reorganization in 6.18 (not a regression)

The HDA codec subsystem was refactored from monolithic modules into
`sound/hda/codecs/<vendor>/` with per-codec-family modules. The old
`snd-hda-codec-realtek.ko` is now `snd-hda-codec-realtek-lib.ko` + per-ALC
modules; the T450's ALC292 is covered by `snd-hda-codec-alc269.ko`. udev loads
the right one by modalias (hence defect #3 mattering). All `=m` drivers
(6350 `.ko`, ~487 MB staged) are present.

## Firmware

`LINUX_FIRMWARE_COMMIT` was left unchanged. The QEMU smoke boot can't exercise it
— there is no Intel wifi in the VM, so `iwlwifi` never probes and no firmware is
requested. The iwlwifi-7265 family firmware API is stable and 6.18's `iwlwifi`
supports a range of ucode API revisions, so the pinned `iwlwifi-7265-17.ucode` /
`iwlwifi-7265D-29.ucode` blobs are expected to work. **This can only be confirmed
on the real T450**: if its boot shows `iwlwifi … failed to load … iwlwifi-7265D-NN`
for an `NN` higher than 29, bump `LINUX_FIRMWARE_COMMIT` in `versions.env` to a
commit carrying the newer ucode and re-run `01-fetch.sh`.

## QEMU smoke boot result (6.18.34)

`./build/06-qemu-smoke.sh`, KVM, `-kernel`+`-initrd` (no disk attached):

- `Linux version 6.18.34 … x86_64-lfs-linux-gnu-gcc 14.2.0` — version confirmed.
- No `Kernel panic` / `Oops` / `BUG` / `Call Trace` through hardware init.
- Builtin drivers came up: `e1000e`, `ata`/AHCI.
- `Run /init` → `writeonce-initramfs: starting (pid=1)`; the Rust init read
  `root=PARTUUID=b0007001-…`, found no matching block device (none attached in
  the smoke VM), and **gracefully entered its recovery shell** — the correct
  no-root behavior, not a kernel fault.

Bare-metal / disk-backed boot on the T450 is the separate next validation.

## Deployment artifacts (17 + 18)

The deployable image must be regenerated after a kernel bump — `BOOTX64.EFI`
(the EFI-stub kernel = the bootloader on this branch) was still the **old
6.12.10** until rebuilt:

- `17-stage-sysroot.sh` → staged the 6.18.34 modules + depmod metadata + iwlwifi
  firmware into `build/staging/sysroot` (2.4 GB).
- `18-make-artifacts.sh` → `BOOTX64.EFI`/`bzImage` now report **6.18.34**
  (`kernel_sha256 == bootloader_sha256`), `sysroot.tar.zst` = 554 MB.
  (Pass `KERNEL_BZIMAGE=build/work/linux-<ver>/arch/x86/boot/bzImage` — pointing
  it at `build/artifacts/bzImage` makes `install` copy a file onto itself.)

### qemu-full ESP had to grow (initramfs-bloat fallout)

`qemu-full.sh` hardcoded a **128 MiB** virtual ESP, but `bzImage`(17M) +
`BOOTX64.EFI`(17M) + the now-113 MiB `initramfs.img` overflow it (mtools
"Disk full"). Bumped to **512 MiB**. With that, OVMF loads and **starts the
6.18.34 EFI-stub kernel** from the ESP — confirming UEFI-loadability of the
regenerated image.

Note `qemu-full.sh` predates the EFI-stub/systemd model: its `cmdline.txt`
(`console=ttyS0`) was read by the retired Rust bootloader, but OVMF now runs
`BOOTX64.EFI` directly with no load options, so the kernel uses its baked
`CONFIG_CMDLINE` (`console=tty0` → no serial after EFI handoff; `rootwait` → waits
for the absent root device, no panic). So qemu-full validates UEFI hand-off but
can't show the Linux boot. **Full boot (root mount → systemd → desktop) needs the
bare-metal T450 or a disk-backed image** (`install.sh` to a loopback/qcow2).

### Note: transitional initramfs size jumped to ~118 MB

Fixing the empty-module-set bug (#2) has a side effect: `05-initramfs.sh` copies
`modules-stage/lib/modules` wholesale, so the initramfs went from a (wrongly) tiny
1.79 MB to **118 MB** (493 MB of modules, gzipped). It boots fine under QEMU
`-m 2G`, but bundling every module into the initramfs is wasteful — a future
improvement is selective inclusion of only boot-essential modules (the rest live
on the root fs). Out of scope for the version bump.
