# Future scope — remote-build installer

> Status: **design sketch, not committed to a phase.** Captures the
> "compile-as-a-service" installer model so the vision survives and can
> be implemented when the rest of the boot path is solid.
> Companion to [`multi-gpu-portability.md`](multi-gpu-portability.md) —
> this is the alternative to universal binaries that preserves
> WriteOnce's "build for this exact machine" philosophy.

## The vision

User inserts a small (≤ 100 MB) WriteOnce install USB into any
x86_64 machine. They get, in this order:

1. A **minimal TUI** to bring the machine onto the network.
2. **Hardware probing** of the target machine.
3. **Submission** of the hardware survey to a remote WriteOnce build
   server.
4. **Server-side custom build** of WriteOnce parameterised by the
   survey (kernel config, Mesa flags, firmware blobs).
5. **Signed delivery** of the resulting sysroot image back to the
   installer.
6. **Local install** to the target's disk.
7. Reboot.

The user never sees a kernel-config file, never edits a Mesa flag,
never knows about LLVM. The build server does the per-machine
specialisation that Path 1 ("stay per-machine") today requires the user
to do by hand.

The trade vs. mainstream universal-binary distros is the same as
Path 1 vs Path 2 in the multi-GPU doc — small final install, exactly
the right code for the hardware. The difference is *who pays the
compile cost*: not the user (mainstream Path 2/3 with universal Mesa),
not the target machine (Gentoo's model), but a build server somewhere
in the network.

## Architecture

```
                    ┌─────────────────────────────┐
                    │  user-facing install USB     │
                    │  ~100 MB:                    │
                    │  - WriteOnce bootloader      │
                    │  - kernel (T450-compatible)  │
                    │  - tiny initramfs with TUI   │
                    │  - iwlwifi/intel-ucode blobs │
                    └────────┬────────────────────┘
                             │
                             │  Boot.  TUI walks user through Wi-Fi /
                             │  ethernet setup.
                             ▼
                    ┌─────────────────────────────┐
                    │ writeonce-installer-tui      │
                    │ (Rust, ncurses-style)        │
                    │ - lists ip links            │
                    │ - iwd for Wi-Fi             │
                    │ - dhcpcd / udhcpc for IPv4  │
                    │ - tests connectivity to     │
                    │   build.writeonce.os        │
                    └────────┬────────────────────┘
                             │
                             │  Once online, probe the hardware.
                             ▼
                    ┌─────────────────────────────┐
                    │ writeonce-installer-probe    │
                    │ - shells survey-target-      │
                    │   machine.sh (already in     │
                    │   scripts/)                  │
                    │ - serialises survey to TOML  │
                    │ - asks user to confirm scope:│
                    │   "wipe /dev/sda?"          │
                    └────────┬────────────────────┘
                             │
                             │  Authenticated POST to build server.
                             │  HTTPS + per-installer JWT (in USB image)
                             │  or sigstore identity (longer-term).
                             ▼
                ╔═════════════════════════════════════╗
                ║  Remote WriteOnce build server      ║
                ║  ─────────────────────────────────  ║
                ║  1. Validate survey (schema check). ║
                ║  2. Compute build-key hash:         ║
                ║     hash(kernel-config-derived-     ║
                ║          -from-survey + mesa-flags  ║
                ║          + WriteOnce git SHA).      ║
                ║  3. If cached: serve immediately.   ║
                ║     Else: enqueue full build job.   ║
                ║  4. Build phases 0 → 9 + Phase 10   ║
                ║     bootable ISO, parameterised by  ║
                ║     the survey.                     ║
                ║  5. Sign artefact via sigstore /    ║
                ║     in-toto attestation.            ║
                ║  6. Return download URL + signature.║
                ╚═════════════════════════════════════╝
                             │
                             │  ~3 GB sysroot.tar.xz over HTTPS.
                             │  Resumable. SHA-256 + signature checked
                             │  before any disk write.
                             ▼
                    ┌─────────────────────────────┐
                    │ writeonce-installer-apply   │
                    │ - parted /dev/sda           │
                    │ - mkfs.vfat ESP + mkfs.ext4 │
                    │ - extract sysroot.tar.xz    │
                    │ - efibootmgr register       │
                    │ - copy BOOTX64.EFI + cmdline│
                    └────────┬────────────────────┘
                             │
                             │  Reboot into a fully bespoke
                             │  WriteOnce build.
                             ▼
                          🎉
```

## What's on the USB

Tight install image:

```
ESP volume (FAT32, ~100 MB)
├── EFI/BOOT/BOOTX64.EFI           ← writeonce-bootloader (Phase 6)
├── EFI/WriteOnce/
│   ├── bzImage                     ← lightweight kernel
│   ├── initramfs.img               ← contains installer-TUI binaries
│   └── cmdline.txt                 ← console=tty0 wo.installer
└── server.pubkey                   ← build server's signing key (pinned)
```

The "lightweight kernel" needs:

- All common network drivers built-in or as modules: e1000e, r8169, igb,
  iwlwifi (firmware loaded from initramfs), ath9k, brcmfmac, mt76 —
  cover ~95% of laptop network controllers.
- Common storage: ahci, nvme, sd_mod, usb-storage.
- KMS for a console with a readable font: vesafb / efifb / drm_simple
  / i915 / amdgpu / nouveau (modular; user might be on any GPU).
- `CONFIG_EFI_STUB=y` so the WriteOnce bootloader hands off cleanly.

Roughly the **mainstream-distro kernel config**, in fact — at install
time we don't yet know what hardware we're on, so we accept a fatter
kernel for the duration of the install.

## What the build server is

A single **stateless Rust service**, plus a build runner backed by the
existing `build/0N-*.sh` scripts.

```rust
struct BuildRequest {
    survey:        TargetMachineSurvey,   // exact schema from scripts/survey-target-machine.sh
    writeonce_git: String,                 // commit SHA the installer was built against
    requestor:    InstallerIdentity,       // JWT / sigstore identity / per-org token
}

struct BuildResponse {
    sysroot_url:   String,
    sysroot_sha256: String,
    signature:     SigstoreSignature,
    build_log_url: String,                 // user can inspect
}
```

The server:

1. **Hashes the request** — `hash(survey, writeonce_git_sha, build_flags_derived_from_survey)`.
2. **Cache lookup**: if a build with that hash already exists, return it instantly. **Most installs hit the cache** — the T450 family of "ThinkPad x10s with Intel CPU + Intel iGPU + iwlwifi" produces the same hash for thousands of machines.
3. **Cache miss**: enqueue a job.
   - Derive the kernel-config-additions.fragment from the survey (which DRM driver, network driver, firmware blobs).
   - Derive the Mesa flags (`gallium-drivers=`).
   - Run the WriteOnce build pipeline: Phase 0 → 9 → 10 ISO assembly.
   - Sign the resulting sysroot.tar.xz with the server's sigstore identity.
   - Store under hash-keyed path.
4. **Return URL + signature.**

Stateless: no per-user data persisted beyond the build cache itself.
Restartable: a crashed build job is re-enqueued via the request hash.

Implementation language: Rust, naturally. Build runner is the
existing shell scripts inside the wo-builder Docker image.

## Open design questions

| Question | Note |
| --- | --- |
| **Authentication** | Open WriteOnce installs accept any installer that ships with a baked-in JWT. Per-organisation deployments could pin specific identities. Self-hosted build servers default to "open within LAN." |
| **Trust transfer** | How does the user know the binary they got is the binary the server intended? Sigstore + a pinned `server.pubkey` on the USB. Transparency log entry per build is verifiable post-hoc. |
| **Privacy** | The hardware survey reveals serials, MAC addresses, dmidecode strings. Default: scrub identifying fields before submission (same scrubber the `.agents/target-machine.md` gitignore conversation flagged). Only hardware *classes* (CPU family, GPU PCI ID, NIC PCI ID) need to reach the server. |
| **Bandwidth** | A WriteOnce sysroot.tar.xz is ~1 GB compressed. 8 minutes on a 20 Mbit link. Acceptable for an install; not for an upgrade — upgrades should be differential. |
| **Build server availability** | A single point of failure. Mitigations: multiple geographic replicas + Anycast; allow users to point at any compatible server; for the most security-conscious, encourage self-hosting. |
| **Cache key stability** | A survey field that doesn't affect the build (e.g. a serial number) must not appear in the hash, or every install becomes a cache miss. The derivation `survey → build flags` is the part the server commits to, and *that* is what gets hashed. |
| **Diff updates** | Phase 10 is essentially "install ISO." A separate Phase 11 (future) could be "delta updates via differential patches against an installed generation." Out of scope here. |
| **Air-gapped installs** | If the target has no network — fallback to a "no remote build" mode that ships a universal binary on the USB. Practically that means having both modes; the USB image grows but stays usable offline. |

## Why this is worth building eventually

It is the only model that reconciles three otherwise-conflicting WriteOnce values:

1. **No reinvention.** Build server doesn't write new code — it runs the existing `build/0N-*.sh` scripts.
2. **Per-machine specialisation.** Each install is tuned. No carrying ~450 MB of unused Mesa drivers.
3. **One-click install UX.** User runs an installer; nothing in front of them suggests a kernel-config file exists.

It also makes WriteOnce **demonstrably reproducible**: the build server's signed attestation says "this sysroot is what you get from this exact survey + this exact WriteOnce git SHA + this exact build pipeline." A second build server independently fed the same input must produce the same output.

## What's needed in the codebase first (prerequisites)

Before this becomes implementable:

1. **Phase 0 → 10 must complete** — there's no build pipeline to invoke remotely if the pipeline isn't done.
2. **Phase 10 ISO step must be parameterisable** — accept a JSON config and produce the matching ISO. Currently `build/14-iso.sh` (not yet written) is sketched as a fixed pipeline.
3. **Hardware survey schema must be stable** — `scripts/survey-target-machine.sh` is shell-only today and emits free-form Markdown. We need a stable TOML/JSON schema that the build server validates against.
4. **Build-server-side sandboxing** — running arbitrary user-submitted surveys through `bash`/`make`/`cargo` is a privilege boundary. Each build runs in a fresh ephemeral container (Firecracker-style microVM or gVisor-isolated container). Inside-Docker-running-Docker patterns work.
5. **WriteOnce installer Rust crate** — `crates/writeonce-installer/` containing the TUI + probe + apply binaries.

Roughly an additional 2–3 months of focused work on top of completing Phase 10. Not Phase 11; more like Phase 12.

## Comparison to existing systems

| System | Closest analog to WriteOnce remote-build |
| --- | --- |
| **Gentoo** | Per-machine builds, but on the target itself. Slow but no remote dep. |
| **NixOS + cache.nixos.org** | Hash-keyed binary cache. We'd be its per-machine specialisation. |
| **Yocto / OpenEmbedded** | Per-board images built on a build server, but boards are fixed configurations, not arbitrary surveys. |
| **Ubuntu MAAS** | Per-machine provisioning, but selects from pre-built generic images. |
| **Chromium OS auto-update** | Signed differential updates over the network, not per-machine builds. |
| **Tailscale's installer** | Stateless one-liner; sets up a service but doesn't build the OS. |
| **System76's Pop!_OS installer** | Universal install; per-machine tuning happens post-install via drivers package. |

None of them combine "remote build server + survey-driven custom image + signed delivery + Rust-native installer" in the way this sketch proposes. Closest in spirit is probably **NixOS + a per-deployment flake** built on a remote builder, but Nix's hash model already exposes the missing parts the WriteOnce vision adds.

## When this becomes a real plan

When the substrate is solid:

- Phases 0–9 complete and self-test (Phase 9 boots i3More on the T450).
- Phase 10 ships a working install ISO that's parameterised, even if the parameters are constants today.
- We have at least *two* target machines to install on (one Intel, one AMD or NVIDIA) — proving the per-machine build path is real, not theoretical.

Then turn this sketch into `plan/phase-12-remote-build-installer.md` with concrete tasks, route the relevant code into `crates/writeonce-installer/`, and stand up a build server (`build.writeonce.os` or a self-hosted equivalent).

Until then, the per-machine reconfigure procedure from `multi-gpu-portability.md` Path 1 is the operational answer for installing on a non-T450 machine.
