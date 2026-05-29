# writeonce-installer

Runtime installer for WriteOnce OS. It reads the artifact bundle produced by the
Docker-driven build and writes a bootable image to a connected USB stick (GPT +
ESP + ext4 root), customising the staged sysroot for the operator (user account,
keyboard, optional network) along the way.

It runs on the **host** (typically the workstation where the build ran), **as
root**, and is a separate concern from the on-target boot path (`writeonce-pid1`,
`writeonce-svc`, `writeonce-login`). It never runs on the target machine.

## Requirements

- **Root.** The installer talks to `/dev/sdX`, mounts filesystems, and runs mkfs.
  It refuses to start otherwise (`must run as root (try sudo)`).
- **Host tools** (any modern Linux distro has these):
  - `sgdisk`            — from `gptfdisk`
  - `mkfs.vfat`         — from `dosfstools`
  - `mkfs.ext4`         — from `e2fsprogs`
  - `mount` / `umount`  — from `util-linux`
- Built **dynamic-glibc**, not the musl static target the on-target crates use.

## Build

```bash
cargo build-installer            # alias for: build -p writeonce-installer --release
# → target/release/writeonce-installer
```

## Quick start

```bash
# 1. See which removable devices the installer would consider.
sudo ./target/release/writeonce-installer list-usb

# 2. Install a build onto a USB. With a TTY and no other flags this opens
#    an interactive TUI for picking the device and filling in the plan.
sudo ./target/release/writeonce-installer install --from /path/to/artifacts

# Fully scripted (non-interactive): supply a complete spec + device, skip
# the confirmation. The password is still prompted (see below).
sudo ./target/release/writeonce-installer install \
    --from /path/to/artifacts \
    --target /dev/sdb \
    --spec examples/target-os.json \
    --yes
```

## CLI reference

```
writeonce-installer [--verbose] <command>

  list-usb
      List removable block devices (reads /sys/block; non-removable and
      loop/ram/dm/optical devices are filtered out).

  install --from <dir> [options]
      --from <dir>             Artifacts directory (manifest.toml + images). Required.
      --target /dev/sdX        Device to install onto. Omit to pick interactively.
      --spec <target-os.json>  Pre-fill the plan. Any omitted/null field is prompted.
      --yes                    Skip the type-"yes" confirmation (required for non-TTY).
      --dry-run                Run every non-destructive step, then stop before sgdisk.
      --force-non-removable    Allow a non-removable target. Almost never what you want.
      --no-tui                 Force the line-by-line CLI prompt flow instead of the TUI.
```

### Input modes

The installer chooses how to gather the plan:

- **TUI (default):** a TTY is present and none of `--no-tui`, `--target`, or `--yes`
  was given → a ratatui screen picks the device and fills in the plan, confirming
  via a summary screen.
- **CLI prompts:** `--no-tui`, or `--target` was supplied, or stdin/stdout is not a
  TTY → line-by-line prompts for whatever the spec leaves unset, then a type-`yes`
  wipe confirmation (unless `--yes`).
- **Non-interactive:** `--spec` complete + `--target` + `--yes`. Note the user
  **password is always prompted** regardless — it is never read from the spec.

## Artifacts bundle (`--from <dir>`)

The build pipeline emits a directory containing the images plus a `manifest.toml`
that names them and pins their SHA-256s:

```
manifest.toml
bzImage            # kernel
initramfs.img      # initramfs
BOOTX64.EFI        # writeonce-bootloader
sysroot.tar.zst    # root filesystem (zstd-compressed tar)
```

`manifest.toml`:

```toml
schema_version = "0.1.0"

[image]
kernel     = "bzImage"
initramfs  = "initramfs.img"
bootloader = "BOOTX64.EFI"
sysroot    = "sysroot.tar.zst"
# Kernel command line template. __ROOT_UUID__ is substituted with the
# root partition's UUID after mkfs.
cmdline    = "root=UUID=__ROOT_UUID__ rootfstype=ext4 ro quiet"

[verification]
kernel_sha256     = "…"
initramfs_sha256  = "…"
bootloader_sha256 = "…"
sysroot_sha256    = "…"

[metadata]            # optional, for human inspection
build_key         = "…"
built_at          = "…"
writeonce_git_sha = "…"
```

Every artifact's SHA-256 is checked against `[verification]` **before any
destructive operation**; a mismatch aborts before the USB is touched.

## Install plan (`target-os.json`)

Every field is optional — anything omitted or `null` is prompted for. Pass a file
with `--spec`; without `--spec`, every choice is prompted. A template lives at
[`examples/target-os.json`](examples/target-os.json).

| Section | Field | Default | Notes |
| --- | --- | --- | --- |
| `partitions` | `esp_mib` | `512` | EFI System Partition size (MiB). |
| `partitions` | `root_gib` | `null` | Root size (GiB); `null` = rest of disk. |
| `user` | `name` | *(prompt)* | Login name. Not `root`; lowercase alphanumeric + `_`. UID/GID are always `1000`. |
| `user` | `real_name` | *(prompt)* | GECOS / display name. |
| `user` | `shell` | `/bin/bash` | Login shell. |
| `user` | `groups` | `[wheel, video, audio, input, plugdev]` | Supplementary groups. |
| `user` | `password_hash` | *(ignored)* | Present only so old files still parse — the password is **always prompted**. |
| `keyboard` | `layout` | *(prompt)* | Console/X11 keymap, e.g. `us`, `de`, `uk`. |
| `keyboard` | `variant` | `null` | Optional variant, e.g. `dvorak`, `nodeadkey`. |
| `network` | `enabled_at_boot` | `false` | When `true`, pre-enable `iwd` + `dhcpcd` + `writeonce-modules-load` (enabled.d stubs) and point `default.target` at `multi-user.target` — for headless/SSH-only installs. Desktop installs leave network opt-in (`wo-ctl enable iwd dhcpcd` after first login). |

## What it does (`install`)

1. Load `manifest.toml` from `--from`.
2. **Verify** each source artifact's SHA-256 against the manifest (before anything destructive).
3. Load the spec (`--spec`) and build the install plan (TUI or prompts).
4. Select the target device and **safety-check** it.
5. Confirm the wipe (CLI: type `yes`; TUI: the summary screen — skipped by `--yes`).
6. Partition GPT: ESP (`esp_mib` MiB) + root (`root_gib` GiB, or the rest).
7. `mkfs.vfat` the ESP, `mkfs.ext4` the root (captures the root UUID).
8. Mount root and extract `sysroot.tar.zst`.
9. **Customise** the staged sysroot (see below).
10. Populate the ESP: bootloader + kernel + initramfs, with `__ROOT_UUID__` substituted into the cmdline.
11. `sync`, unmount, then **re-read and re-verify** SHA-256 to catch silent USB corruption.

`--dry-run` performs steps 1–4 and then stops before step 6 (no disk writes).

## Safety

- Refuses non-removable devices unless `--force-non-removable`.
- Refuses a device (or any of its partitions) that is currently mounted.
- SHA-256 verification of every artifact *before* the first destructive step.
- A type-`yes` confirmation (CLI) or summary confirmation (TUI), bypassable only with `--yes`.
- Post-write re-read verification catches blocks the USB silently corrupted.

## Sysroot customisation

Applied to the extracted root *before* the ESP is populated
([`customize.rs`](src/customize.rs)):

- Rewrites `/etc/passwd`, `/etc/shadow`, `/etc/group` for the chosen user (replacing the skeleton's placeholder `writeonce` account).
- Renames `/home/writeonce` → `/home/<user>`.
- Patches `~/.xinitrc` and writes `/etc/vconsole.conf` for the keyboard layout/variant.
- If `network.enabled_at_boot`, writes the `enabled.d` stubs and switches `default.target`.

`/etc/machine-id` is **not** written here — it is generated fresh on first boot by
`writeonce-bootstrap`, so a single image flashed to many sticks doesn't share an ID.

## Layout

| File | Responsibility |
| --- | --- |
| `main.rs` | CLI, the 11-step `install` flow, `list-usb`. |
| `manifest.rs` | `manifest.toml` parsing + artifact SHA-256 verification. |
| `spec.rs` | `target-os.json` schema and the resolved `InstallationPlan`. |
| `detect.rs` | Removable-device enumeration + `safety_check`. |
| `tui.rs` / `prompt.rs` | The TUI and the line-by-line CLI plan-gathering flows. |
| `partition.rs` / `mkfs.rs` / `mount_.rs` | GPT, filesystems, mounting. |
| `extract.rs` | `sysroot.tar.zst` extraction. |
| `customize.rs` | Post-extract sysroot mutations. |
| `bootloader.rs` | ESP population + cmdline templating. |
| `verify.rs` | Post-write re-read verification. |
| `confirm.rs` | The type-`yes` wipe confirmation. |
