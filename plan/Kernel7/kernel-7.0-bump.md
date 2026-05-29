# Plan: Install Linux 7.0 in WriteOnce OS (LFS-style)

## Context

The kernel tree at `.agents/reference/linux/` is now checked out to tag `v7.0` (commit `028ef9c96e96`), but the active build still pins `LINUX_VERSION=6.12.10` (`build/versions.env:17`). The goal is to move the boot kernel forward to 7.0 using the existing WriteOnce cross-build pipeline, which is already an LFS-chapter-10.3-shaped recipe (`mrproper` → config → `make` → `modules_install` → install) executed in the `wo-builder` container rather than via chroot.

Two risks drive the plan:

1. **CONFIG drift.** `build/kernel-config-additions.fragment` was authored against 6.12. Between 6.12 LTS and 7.0 some CONFIG symbols may have been renamed, split, or removed. `make olddefconfig` will auto-resolve most, but renamed hardware-critical options (I218 ethernet, Intel 7265 wifi, HD 5500 graphics, HDA audio — all required for T450 functionality) need a visual diff pass.
2. **Firmware compatibility.** `LINUX_FIRMWARE_COMMIT=adb6dceb45b98c4149e1ce68fc1a5f394fd67695` (`build/versions.env:24`) was pinned for the 6.12 kernel. 7.0 may request newer firmware blobs (especially `iwlwifi-7265D-*.ucode` API revisions). The kernel will boot either way, but missing-firmware warnings should be checked against the dmesg smoke output.

Phase 7 `writeonce-kerngen` is **not** in scope for this change. The fragment-merge approach stays; kerngen's `resolve` step is future work tracked in `plan/done/phase-7-kerngen.md`.

## Approach

Layer 7.0 into the existing cross-build by editing version pins, busting sentinels, re-running, and validating. No script changes required — `04-kernel.sh` is version-agnostic and reads everything from `versions.env`.

### Step 1 → verify: tarball lands with correct hash

Edit `build/versions.env`:
- `LINUX_VERSION=6.12.10` → `LINUX_VERSION=7.0`
- `LINUX_MAJOR=6.x` → `LINUX_MAJOR=7.x`
- Leave `LINUX_FIRMWARE_COMMIT` alone for now; revisit in Step 5 if dmesg shows missing-firmware warnings.

Edit `build/checksums.txt:58`:
- Replace the `linux-6.12.10.tar.xz` line with the SHA-256 of `linux-7.0.tar.xz` from `https://cdn.kernel.org/pub/linux/kernel/v7.x/sha256sums.asc` (verified against `.agents/reference/linux/` tag `v7.0` if a local archive is producible, or by GPG-checking the signed sums file).

Run `./build/in-container.sh ./build/01-fetch.sh`. Verify `sources/linux-7.0.tar.xz` exists and matches the pinned SHA. The fetch script auto-constructs the URL from `LINUX_VERSION` + `LINUX_MAJOR` (`build/01-fetch.sh:33`), so no URL edit needed.

### Step 2 → verify: olddefconfig diff is reviewed

Delete the kernel sentinel: `rm -f build/logs/.done-04-kernel`. Run `./build/in-container.sh ./build/04-kernel.sh`. The script will:
- Extract `linux-7.0.tar.xz` into `$BUILD_ROOT/work/linux-7.0`
- Run `make defconfig` → `merge_config.sh` with `kernel-config-additions.fragment` → `make olddefconfig`
- Capture the merged `.config` at `build/artifacts/kernel.config`

**Critical manual gate**: before the `make` step proceeds, capture `merge_config.sh` stderr output (lines starting with `Value of CONFIG_X is redefined...` or `Symbol: X has no prompt`). Diff against the fragment. Any CONFIG that the merger silently dropped is a hardware risk — explicitly verify by name:
- `CONFIG_E1000E` (I218 ethernet)
- `CONFIG_IWLMVM` / `CONFIG_IWLWIFI` (7265 wifi)
- `CONFIG_DRM_I915` (HD 5500 graphics)
- `CONFIG_SND_HDA_INTEL` + `CONFIG_SND_HDA_CODEC_REALTEK` (audio)
- `CONFIG_INTEL_PMC_CORE` (Broadwell PMC)

If any are missing post-`olddefconfig`, edit `build/kernel-config-additions.fragment` to use the new symbol name and rerun. Persist the rename mapping to `docs/learning/kernel-7.0-config-migration.md` (per `feedback_persist_explanations`).

### Step 3 → verify: artifacts produced

Wait for `04-kernel.sh` to complete. Confirm:
- `build/artifacts/bzImage` exists, ELF/bzImage header present
- `build/artifacts/modules-stage/lib/modules/7.0/` populated (no stray `6.12.10` dir)
- `build/artifacts/kernel.config` shows `# Linux/x86 7.0 Kernel Configuration`
- `docs/kernel-build-history.md` gained a new entry (auto-appended by `build/kernel-history-append.sh`)

### Step 4 → verify: QEMU smoke-boot reaches userspace

Rebuild initramfs and re-smoke:
- `rm -f build/logs/.done-05-initramfs` → `./build/in-container.sh ./build/05-initramfs.sh` (picks up new `modules-stage`)
- `./build/in-container.sh ./build/06-qemu-smoke.sh`

Success criteria:
- Boot reaches `writeonce-init` PID 1 banner
- No `Kernel panic` in serial log
- `modprobe` of each driver from the critical-CONFIG list above succeeds (visible in smoke-test stage modprobe output, or check `dmesg` for `e1000e`, `iwlwifi`, `i915`, `snd_hda_intel` initialization lines)

### Step 5 → verify: firmware warnings catalogued

Scan QEMU `dmesg` for `firmware: failed to load` lines. If `iwlwifi`-related entries appear that didn't appear under 6.12.10:
- Bump `LINUX_FIRMWARE_COMMIT` in `build/versions.env:24` to a commit dated near 7.0's release (`git -C .agents/reference/linux-firmware log --until=2026-04-01 --format='%H %s' | head` — adjust date once 7.0 release date is confirmed)
- Re-run `01-fetch.sh` then `04-kernel.sh` (modules step only) — sentinel logic only requires kernel rebuild if `LINUX_VERSION` changed.

If no new warnings, leave firmware pin alone and document the decision in the learning doc from Step 2.

## Critical files

| File | Change |
|------|--------|
| `build/versions.env:17,18` | `LINUX_VERSION=7.0`, `LINUX_MAJOR=7.x` |
| `build/checksums.txt:58` | New SHA-256 for `linux-7.0.tar.xz` |
| `build/kernel-config-additions.fragment` | Symbol renames as surfaced by `olddefconfig` (only if needed) |
| `build/logs/.done-04-kernel` | Delete to force rebuild |
| `build/logs/.done-05-initramfs` | Delete to pick up new modules tree |
| `docs/learning/kernel-7.0-config-migration.md` | New — capture rename mappings + firmware decision rationale |

No edits required to: `build/04-kernel.sh`, `build/01-fetch.sh`, `build/kernel-base.config`, `build/05-initramfs.sh`, `build/06-qemu-smoke.sh`. They're already version-parameterized.

## Reused components

- `build/04-kernel.sh:27-131` — orchestrator, version-agnostic
- `build/01-fetch.sh:33` — URL templating from `LINUX_VERSION`/`LINUX_MAJOR`
- `build/kernel-history-append.sh` — auto-logs each build to `docs/kernel-build-history.md`
- `.agents/reference/lfs/chapter10/kernel.xml` — LFS-canonical recipe; nothing to copy, but the merged-config approach is consistent with LFS 10.3's `make defconfig` + manual selections (just expressed as a fragment instead of a menuconfig session)
- `.agents/reference/linux/` (tag `v7.0`) — read-only reference for cross-checking symbol availability with `grep -r CONFIG_X arch/x86/configs/ Kconfig`

## Verification

End-to-end success means all of:

1. `./build/in-container.sh ./build/01-fetch.sh` → tarball SHA verified
2. `./build/in-container.sh ./build/04-kernel.sh` → `build/artifacts/bzImage` produced; `kernel.config` reports `7.0`
3. `./build/in-container.sh ./build/05-initramfs.sh` → modules from 7.0 in initramfs
4. `./build/in-container.sh ./build/06-qemu-smoke.sh` → reaches `writeonce-init` banner, no panic, critical hardware drivers (`e1000e`, `iwlwifi`, `i915`, `snd_hda_intel`) load
5. `docs/kernel-build-history.md` shows new 7.0 entry
6. `docs/learning/kernel-7.0-config-migration.md` captures any CONFIG renames + firmware decision

If the QEMU smoke passes but a T450-on-bare-metal boot is desired afterward, that's a separate step using the existing UEFI bootloader artifact (`crates/writeonce-bootloader/`); not in scope for this plan.
