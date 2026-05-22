# Multi-GPU portability — what mainstream distros do, what WriteOnce does, what to change

> Companion to [`../../build/10-xorg-server.sh`](../../build/10-xorg-server.sh)
> and the developer-workstation plan. Documents the trade WriteOnce
> currently makes (compile-time-targeted Mesa for the T450's Intel HD
> 5500) and what would need to change for it to install cleanly on a
> machine with a different GPU.

## The problem

A GPU driver in Linux is two layers:

1. **Kernel-side DRM driver** (`i915.ko`, `amdgpu.ko`, `nouveau.ko`, …)
   matched to the PCI device by the kernel's PCI ID table.
2. **Userspace driver** (`*_dri.so` in Mesa, `*_icd.so` for Vulkan,
   proprietary `libGLX_nvidia.so`, …) loaded at runtime by Mesa or the
   Vulkan loader.

Different GPUs need different drivers at both layers. A binary that
"just works" across an Intel iGPU, an AMD discrete card, and an NVIDIA
discrete card must carry all three.

## What mainstream distros do

Every modern distro (Ubuntu, Debian, Arch, Fedora, NixOS, Alpine) ships
a **universal Mesa**: one binary build with **every gallium and Vulkan
driver** compiled in.

Disk layout:

```
/usr/lib/dri/
├── i965_dri.so          (~30 MB) Intel gen4–gen7
├── iris_dri.so          (~30 MB) Intel gen8+
├── crocus_dri.so        (~25 MB) Intel legacy bridge
├── radeonsi_dri.so      (~40 MB) AMD GCN+ (needs LLVM at runtime)
├── r600_dri.so          (~25 MB) AMD pre-GCN
├── nouveau_dri.so       (~35 MB) NVIDIA open-source
├── llvmpipe_dri.so      (~25 MB) software fallback (needs LLVM)
├── zink_dri.so          (~30 MB) OpenGL-on-Vulkan
└── …
```

Total: ~400–500 MB of `*_dri.so` of which **exactly one is used on any
given boot**.

The detection mechanism:

```
1. Kernel auto-probes PCI; loads i915 / amdgpu / nouveau / nvidia.
2. Creates /dev/dri/card0 (KMS) and /dev/dri/renderD128 (render).
3. Userspace process opens /dev/dri/card0.
4. Mesa reads /sys/class/drm/card0/device/{vendor,device} (PCI IDs).
5. Mesa's drm.c maps PCI ID → driver name (e.g. "iris" for 8086:1616).
6. Mesa dlopen()s /usr/lib/dri/iris_dri.so.
7. App talks to that driver as libGL.
```

For Vulkan: a similar table lives in
`/usr/share/vulkan/icd.d/*_icd.x86_64.json` (pointer files), processed
by the Vulkan loader.

For **proprietary drivers** (NVIDIA / AMDGPU-PRO): shipped as separate
packages that swap in their own libGL via either `update-alternatives`
(Debian/Ubuntu/Fedora) or **glvnd** (the GL Vendor Neutral Dispatch
library). With glvnd, `libGL.so` is a thin dispatcher that picks
between `libGLX_mesa.so.0` and `libGLX_nvidia.so.0` at runtime based
on the GPU vendor.

**Kernel**: distros enable every common driver:

```
CONFIG_DRM_I915=y         CONFIG_DRM_AMDGPU=m       CONFIG_DRM_NOUVEAU=m
CONFIG_DRM_RADEON=m       CONFIG_DRM_GMA500=m       CONFIG_DRM_VC4=m
```

Plus `linux-firmware`: a 400+ MB tarball of vendor blobs (`amdgpu/*`,
`nvidia/*`, `nouveau/*`) that the kernel modules request at probe time.

The combined effect: any x86_64 distro's installer image contains
~1 GB of driver-related code that's almost all unused on any given
boot — but the user never has to think about it.

## What WriteOnce currently does

`build/10-xorg-server.sh`'s `step_mesa` deliberately strips everything
that isn't relevant to the T450's Intel HD 5500:

```
-Dgallium-drivers=iris      # Intel gen8+ only
-Dvulkan-drivers=            # no Vulkan
-Dllvm=disabled              # no LLVM
-Dplatforms=x11              # no Wayland
-Dgles1=disabled             # no GLES1
-Dgallium-omx=disabled, etc. # no video accel
```

Resulting Mesa install: ~50 MB, compiles in ~45 minutes. Plus:

```
build/kernel-config-additions.fragment:
  CONFIG_DRM_I915=y           # Intel-only
  # (no DRM_AMDGPU / DRM_NOUVEAU / DRM_RADEON)
```

WriteOnce is, by construction, an **Intel-iGPU laptop OS**. Install it
on a different machine and:

- A machine with an AMD GPU: kernel has no `amdgpu` driver → no
  `/dev/dri/card0` → Xorg fails to start.
- A machine with an NVIDIA GPU: same, plus no nouveau / nvidia in
  Mesa → no GL backend even if KMS came up via VESA.
- A machine with a different Intel iGPU (gen9 / gen10 / Xe): kernel's
  `i915` covers it; Mesa's `iris` covers gen8 and later, so this
  actually works. The only Intel GPUs WriteOnce supports today are
  Broadwell-era and newer.

## Three paths to portability

### Path 1 — Stay per-machine

Keep the philosophy: WriteOnce is built for the exact target. For each
new machine:

```
1. Run scripts/survey-target-machine.sh on the new hardware.
2. Edit build/kernel-config-additions.fragment to enable the relevant
   CONFIG_DRM_<x>=y/m for the new GPU.
3. Edit step_mesa in build/10-xorg-server.sh: change gallium-drivers=
   to the matching driver (radeonsi for AMD, nouveau for NVIDIA open).
4. If the GPU needs firmware blobs, capture them from the current
   distro and add to build/firmware/.
5. Re-run Phases 0 → 8 → 9 → 10.
```

Honest. Tedious for more than one machine. Matches LFS spirit.

### Path 2 — Build universal Mesa

Make WriteOnce a binary that fits any GPU. Mostly a Mesa flag flip:

```
step_mesa() {
    build_meson mesa "mesa-${MESA_VERSION}.tar.xz" \
        -Dgallium-drivers=iris,radeonsi,nouveau,crocus,llvmpipe \
        -Dvulkan-drivers=intel,amd                              \
        -Dllvm=enabled                                          \
        -Dplatforms=x11                                          \
        -Degl=enabled -Dgbm=enabled -Ddri3=enabled              \
        -Dgles1=disabled                                          \
        -Dglx=dri
}
```

Plus a new prerequisite step `step_llvm` (LLVM 17 or 18 needed by
radeonsi + llvmpipe — adds 30 min compile + 600 MB on disk).

Kernel side:

```
# Add to build/kernel-config-additions.fragment:
CONFIG_DRM_AMDGPU=m
CONFIG_DRM_NOUVEAU=m
CONFIG_DRM_RADEON=m
```

Firmware: WriteOnce currently captures only iwlwifi firmware. For a
portable install, capture `linux-firmware`'s full
`amdgpu/`, `nouveau/`, `radeon/` subtrees too (~200 MB extra). Add a
fetch step.

Resulting Mesa: ~450 MB, compile time ~2 hours. WriteOnce's total
install size grows from ~3.5 GB to ~5 GB.

### Path 3 — Nix for Mesa specifically (recommended)

The developer-workstation plan already says: source-build the
substrate, install user-facing apps via Nix. Mesa is technically
*system-level* (Xorg links against libGL), but it has every property of
a "use Nix here" case:

- Massive surface area (every GPU vendor)
- Heavy compile (Mesa + LLVM = ~3 hours)
- Already perfectly maintained upstream by Nixpkgs
- No WriteOnce-specific tuning is meaningful — we'd just be replicating
  what every distro packages

So: **don't source-build Mesa.** Delete `step_mesa` from
`10-xorg-server.sh`. In Phase 10, after Nix is bootstrapped:

```bash
nix profile install nixpkgs#mesa nixpkgs#libdrm
```

This installs the universal Mesa + libdrm at `/nix/store/<hash>-mesa-*/`
and exposes its libraries under
`/home/<user>/.nix-profile/lib/{libGL.so.1,libEGL.so.1,libgbm.so.1,…}`.
Set `LD_LIBRARY_PATH` (or the WriteOnce environment script) to find it.

xorg-server in Phase 8c then links against the Nix-provided Mesa
during its build — we point its `pkg-config` at the Nix profile's
`lib/pkgconfig`.

Trade:

| Dimension | Source-built Mesa (per-machine) | Nix Mesa |
| --- | --- | --- |
| WriteOnce compile time | +45 min for Mesa | 0 (Nix downloads prebuilt) |
| Disk footprint of WriteOnce sysroot | +50 MB Mesa | 0 (Mesa in /nix/store) |
| Disk footprint of Nix store | n/a | +500 MB universal Mesa |
| GPU coverage | one family | every family Mesa supports |
| Reproducibility | committed to checksums.txt | committed via flake.lock |
| Per-machine reconfigure for new GPU | yes | no |
| Adds /nix dep to system | no | yes (already planned for Phase 10) |

The reproducibility note matters: Nix is **more** reproducible than our
source build, not less — `flake.lock` pins exact derivation hashes
including all transitive deps.

## What changes if we go with Path 3

A focused round (call it 8c-bis or part of Phase 10):

1. Bootstrap Nix into `$LFS` (Phase 10 work; ~50 LOC of shell).
2. Add a Phase-10 service that installs `nixpkgs#mesa` and
   `nixpkgs#libdrm` to the system profile at first boot.
3. Delete `step_mesa` (and the corresponding flags) from
   `build/10-xorg-server.sh`.
4. Adjust `step_xorg-server`'s pkg-config path so it finds the Nix
   profile's libGL/libEGL/libgbm.
5. Update `build/kernel-config-additions.fragment` to enable the
   common DRM drivers (`amdgpu=m`, `nouveau=m`, etc.) — this is the
   kernel-side portability that Mesa-from-Nix complements.

Net effect: WriteOnce becomes installable on any x86_64 machine with a
DRM-capable GPU, while keeping the rest of the substrate
source-built and the "developer workstation, no reinvention" framing
intact.

## What WriteOnce will not do

Even with Path 3, some GPUs need proprietary blobs we won't ship:

- **NVIDIA proprietary** (`nvidia-driver`) — kernel module + closed-source
  userspace. Users wanting it install it themselves via Nix
  (`nixpkgs#linuxPackages.nvidia_x11`) or by manually downloading
  NVIDIA's installer. WriteOnce won't add NVIDIA-specific build paths.
- **AMDGPU-PRO** (closed-source AMD userspace, also tied to specific
  kernel versions). Same answer: not WriteOnce's problem.
- **Intel "non-mesa"** drivers — there aren't really any for Linux
  these days; iris in Mesa is the canonical Intel userspace.

The open-source path (Intel iris + Mesa, AMD radeonsi + Mesa, NVIDIA
nouveau + Mesa) covers ~95% of common laptop / workstation
configurations. Users with edge cases install proprietary blobs
themselves; WriteOnce neither stops them nor helps them.

## Decision

Open question for the user: do we keep WriteOnce as a T450-specific
build (Path 1), refactor for universal GPU support source-built
(Path 2), or adopt Path 3 (Nix for Mesa)?

The plan as written (developer-workstation, Nix for apps in Phase 10)
points naturally at Path 3. Path 1 is what we have today. Path 2 is a
bigger source-build commitment with comparable cost to Nix.
