# Phase 10 — runtime installer

> Companion to [`../../crates/writeonce-installer/`](../../crates/writeonce-installer/)
> and [`../../build/15-fake-artifacts.sh`](../../build/15-fake-artifacts.sh).
> Explains the artifacts bundle format, the install pipeline, and the
> safety hooks that prevent the installer from overwriting a system disk.

## What this round delivers

A single binary — `writeonce-installer` — that consumes the artifacts
produced by the Docker-driven build pipeline and writes them to a
connected USB stick as a bootable image. Two subcommands:

```
writeonce-installer list-usb
writeonce-installer install --from <artifacts-dir> --target /dev/sdX [--yes] [--dry-run]
```

The binary lives in the workspace alongside the other crates (pid1,
svc, login, logind, bootloader) and is built with `cargo build-installer`
from inside the wo-builder Docker container.

## The data flow

```
┌────────────────────────────────────────────────────────────────┐
│  Docker build (build/*.sh)                                      │
│    ./build/01-fetch.sh                                          │
│    ./build/02-host-toolchain.sh                                 │
│    …                                                             │
│    ./build/14-iso.sh    (or the proposed writeonce-builder)     │
│                                                                  │
│  produces:                                                       │
│    build/artifacts/                                              │
│      bzImage          — kernel                                   │
│      initramfs.img    — initramfs                                │
│      BOOTX64.EFI      — writeonce-bootloader (UEFI app)          │
│      sysroot.tar.zst  — root filesystem (tar + zstd)             │
│      manifest.toml    — versions, SHA-256s, cmdline template     │
└──────────────┬─────────────────────────────────────────────────┘
               │
               │  sudo writeonce-installer install \
               │      --from build/artifacts \
               │      --target /dev/sdX
               ▼
┌────────────────────────────────────────────────────────────────┐
│  writeonce-installer (runs as root on the workstation)          │
│                                                                  │
│  1. Load manifest.toml                                          │
│  2. Verify source-artifact SHA-256s against manifest             │
│  3. Detect + safety-check target device                          │
│  4. Confirm with operator (unless --yes)                         │
│  5. sgdisk: wipe + GPT (ESP 512 MiB + root)                      │
│  6. mkfs.vfat ESP + mkfs.ext4 root (captures root UUID)          │
│  7. Mount ext4 root + extract sysroot.tar.zst                    │
│  8. Mount ESP under root/boot/efi                                 │
│  9. Copy BOOTX64.EFI + kernel + initramfs to ESP                 │
│ 10. Write cmdline.txt with root=UUID=<actual UUID>               │
│ 11. sync + unmount                                                │
│ 12. Re-mount ESP read-only, SHA-256 reread, compare              │
└────────────────────────────────────────────────────────────────┘
```

## manifest.toml schema

```toml
schema_version = "0.1.0"

[image]
kernel     = "bzImage"
initramfs  = "initramfs.img"
bootloader = "BOOTX64.EFI"
sysroot    = "sysroot.tar.zst"
# Kernel command line template. The installer substitutes
# __ROOT_UUID__ with the actual UUID assigned by mkfs.ext4.
cmdline    = "console=tty0 root=UUID=__ROOT_UUID__ rw quiet"

[verification]
kernel_sha256     = "…"
initramfs_sha256  = "…"
bootloader_sha256 = "…"
sysroot_sha256    = "…"

[metadata]
build_key          = "fake-1700000000"      # hash for cache lookup
built_at           = "2026-05-22T19:00:00Z"
writeonce_git_sha  = "b93cd51"
```

The installer **refuses to run** if any `verification.*_sha256` field
disagrees with the actual file on disk. Catches: half-downloaded
artifacts, swapped files, tampered builds.

## Final ESP layout

```
/EFI/
├── BOOT/
│   └── BOOTX64.EFI              ← writeonce-bootloader (UEFI default boot path)
└── WriteOnce/
    ├── bzImage                  ← kernel
    ├── initramfs.img            ← initramfs
    └── cmdline.txt              ← "console=tty0 root=UUID=<…> rw quiet"
```

The UEFI firmware finds `/EFI/BOOT/BOOTX64.EFI` automatically (it's
the "removable media path" specified in the UEFI spec). Our bootloader
opens its sibling `/EFI/WriteOnce/` directory and reads the kernel +
initramfs + cmdline from there — no UEFI variables to set, no
`efibootmgr` invocation needed.

## Safety hooks

`writeonce-installer` is a destructive tool. Five layers of safety:

| Layer | What it catches | Where it fires |
| --- | --- | --- |
| **`require_root()`** | Non-root invocation | First syscall before anything else |
| **SHA-256 verification of source artifacts** | Corrupted / tampered bundle | After loading manifest, before touching disk |
| **`detect::safety_check`** | Non-removable device (system disk!) + currently-mounted partitions | After --target resolution |
| **Type-"yes" confirmation** | Operator confusion / wrong device | Before sgdisk |
| **`--dry-run` flag** | Sanity check the full pipeline without writes | Stops at step 4 |
| **Post-write SHA reread** | USB stick that silently corrupted blocks during write | Final step |

The non-removable refusal is the load-bearing one: if you accidentally
type `--target /dev/nvme0n1` (your laptop's internal SSD), the
installer refuses unless you also pass `--force-non-removable`.

## CLI surface

```
$ writeonce-installer list-usb
DEVICE      VENDOR             MODEL                    SIZE
/dev/sda    USB                SanDisk 3.2Gen1            61.5 GB

$ sudo writeonce-installer install \
      --from build/artifacts \
      --target /dev/sda \
      --spec crates/writeonce-installer/examples/target-os.json   # optional

[1/11] Loading manifest from build/artifacts
[2/11] Verifying source artifacts (SHA-256 vs manifest.toml) ...
[3/11] Selecting target device
       /dev/sda — USB SanDisk 3.2Gen1 — 61.50 GB (removable)
[4/11] Gathering installation plan

============================================================
 Partition layout for /dev/sda (61.50 GB)
============================================================
 ESP size in MiB [512]: 
 Root partition size in GiB (0 = use rest, max 60) [60]: 
 → ESP:  512 MiB
 → root: rest of disk (~60 GiB)

============================================================
 Primary user account
============================================================
 Username (not root): shoney
 Real name (optional, Enter to skip): Shoney Arickathil
 Password for shoney: ●●●●●●●●
 Confirm password: ●●●●●●●●
 → user:  shoney (uid 1000)
 → shell: /bin/bash
 → groups: wheel,video,audio,input,plugdev

============================================================
 Keyboard layout
============================================================
 Common layouts: us, uk, de, fr, es, it, ru, jp, cn
 Layout [us]: us
 Variant (Enter for none): 
 → layout: us

============================================================
 Installation summary
============================================================
 Target device : /dev/sda (61.50 GB)
                 USB SanDisk 3.2Gen1
 ESP size      : 512 MiB
 Root size     : rest of disk
 Username      : shoney (uid 1000, real-name "Shoney Arickathil")
 Shell         : /bin/bash
 Groups        : wheel,video,audio,input,plugdev
 Keyboard      : us

============================================================
 CONFIRM DESTRUCTIVE OPERATION
============================================================
 ...
 The ENTIRE contents of /dev/sda will be erased.
 Type "yes" exactly (no quotes) to continue, anything else aborts.
 > yes

[5/11] Partitioning GPT (ESP 512 MiB + root) ...
[6/11] Formatting ESP + root
[7/11] Mounting target + extracting sysroot
[8/11] Customising sysroot (user account + keyboard layout)
[9/11] Installing bootloader + kernel + initramfs to ESP
[10/11] sync + unmount
[11/11] Verifying re-read (SHA-256)
✓ Install complete.
  Eject /dev/sda and boot it on the target machine.
  Cmdline: console=tty0 root=UUID=8a3f4c1e-… rw quiet …
  Login as: shoney
```

## target-os.json schema

Optional. Passing `--spec target-os.json` makes any field that's set in
the file skip its interactive prompt. Any field omitted or set to
`null` still prompts.

```json
{
  "schema_version": "0.1.0",

  "partitions": {
    "esp_mib": 512,            // MiB; default 512 if absent
    "root_gib": null           // null = use rest; integer = fixed GiB
  },

  "user": {
    "name": "shoney",          // username (must not be "root")
    "real_name": "Shoney A.",  // GECOS / display name
    "password_hash": null,     // null = prompt; if set, must be $6$… SHA-512 crypt
    "shell": "/bin/bash",      // default /bin/bash if absent
    "groups": ["wheel", "video", "audio", "input", "plugdev"]
  },

  "keyboard": {
    "layout": "us",            // X11 / console keymap layout
    "variant": null            // optional variant (dvorak, intl, …)
  }
}
```

Field semantics:

| Field | Required | If null / absent |
| --- | --- | --- |
| `partitions.esp_mib` | No | Prompts; default 512 MiB |
| `partitions.root_gib` | No | Prompts; default = rest of disk |
| `user.name` | No, prompts | — |
| `user.real_name` | No, prompts | Empty string allowed |
| `user.password_hash` | No | Prompts twice (no echo), hashes via `openssl passwd -6` |
| `user.shell` | No | `/bin/bash` |
| `user.groups` | No | `["wheel","video","audio","input","plugdev"]` |
| `keyboard.layout` | No, prompts | Default `us` |
| `keyboard.variant` | No, prompts | None |

## Customisation step (`src/customize.rs`)

After the sysroot tar is extracted but BEFORE the ESP is mounted, the
installer runs **customize::apply()** on the staged tree:

1. **`/etc/passwd`** — replaces the skeleton's `writeonce:x:1000:1000:…`
   line with `<chosen-name>:x:1000:1000:<real-name>:/home/<chosen-name>:<shell>`.
2. **`/etc/shadow`** — replaces the skeleton's locked entry with
   `<chosen-name>:$6$<salt>$<hash>:<days-since-epoch>:0:99999:7:::`.
   The hash is produced by shelling out to `openssl passwd -6 -stdin`.
3. **`/etc/group`** — renames the `writeonce` group at gid 1000 to the
   chosen username; updates supplementary-group memberships
   (`wheel`, `video`, `audio`, `input`, `plugdev` by default).
4. **`/home/writeonce/` → `/home/<chosen-name>/`** rename; recursive
   `chown 1000:1000`.
5. **`/home/<chosen-name>/.xinitrc`** — patches the `setxkbmap us` line
   with the chosen layout (and `-variant <v>` if set).
6. **`/etc/vconsole.conf`** — writes `KEYMAP=<layout>` for the
   non-X console (boot-time tty1 prompt).

Idempotent against re-runs (each `rewrite_*` reads + reconstructs;
re-running over a customised tree changes nothing).

## Runtime host-tool dependencies

The installer **shells out** to existing battle-tested userspace tools
rather than reimplementing GPT, FAT32, ext4, or crypt in Rust:

| Tool | Package | Used by |
| --- | --- | --- |
| `sgdisk` | gptfdisk | partition.rs |
| `partprobe` | parted | partition.rs (re-read after sgdisk) |
| `mkfs.vfat` | dosfstools | mkfs.rs |
| `mkfs.ext4` | e2fsprogs | mkfs.rs |
| `blkid` | util-linux | mkfs.rs (capture root UUID) |
| `mount`, `umount` | util-linux | mount_.rs |
| `sync` | coreutils | main.rs |
| **`openssl`** | openssl | prompt.rs (`passwd -6 -stdin` for SHA-512 crypt) |

All shipped by default on every modern Linux distro (Ubuntu/Debian/Fedora/Arch).

## Module layout

```
crates/writeonce-installer/
├── Cargo.toml                  ~25 LOC, deps via workspace
├── src/
│   ├── main.rs                 ~200 LOC, CLI + orchestration
│   ├── detect.rs               ~115 LOC, /sys/block enumeration + safety
│   ├── confirm.rs              ~30 LOC, type-yes prompt
│   ├── partition.rs            ~55 LOC, sgdisk wrapper
│   ├── mkfs.rs                 ~60 LOC, mkfs.vfat + mkfs.ext4 + blkid UUID
│   ├── mount_.rs               ~60 LOC, mount + umount + RAII guard
│   ├── extract.rs              ~75 LOC, tar+zstd with progress bar
│   ├── bootloader.rs           ~60 LOC, ESP populate + cmdline format
│   ├── verify.rs               ~50 LOC, post-write SHA reread
│   └── manifest.rs             ~120 LOC, manifest.toml + SHA helpers
└── (no examples/ — fake artifacts live at build/artifacts/ instead)
```

Roughly **850 LOC of Rust + 25 LOC of TOML**. Binary 4.5 MB unstripped.

## Why tokio here, not async-io

The earlier writeonce-logind round deliberately avoided tokio (single-
threaded daemon, no need). For the installer the calculus flips:

- **Many concurrent I/O streams.** Extracting `sysroot.tar.zst` is a
  pipeline: file read → zstd decompress → tar parse → write to ext4 →
  SHA accumulate → progress bar redraw. `tokio::io::copy`-shaped
  pipelines compose these naturally.
- **CPU-bound SHA + ext4 writes.** `spawn_blocking` for the tar+zstd
  step puts sync code on a tokio worker thread without blocking the
  reactor — clean separation of concerns.
- **Process spawning + waiting.** `tokio::process::Command` is the
  natural way to invoke sgdisk, mkfs, mount, blkid, sync without
  blocking the runtime on each.

The runtime is `rt-multi-thread` with `enable_all()` — the SHA reread
benefits from a parallel decoder, and the extract pipeline is
naturally async-friendly.

## Runtime host-tool dependencies

The installer **shells out** to existing battle-tested userspace tools
rather than reimplementing GPT, FAT32, and ext4 in Rust:

| Tool | Package | Used by |
| --- | --- | --- |
| `sgdisk` | gptfdisk | partition.rs |
| `partprobe` | parted | partition.rs (re-read after sgdisk) |
| `mkfs.vfat` | dosfstools | mkfs.rs |
| `mkfs.ext4` | e2fsprogs | mkfs.rs |
| `blkid` | util-linux | mkfs.rs (capture root UUID) |
| `mount`, `umount` | util-linux | mount_.rs |
| `sync` | coreutils | main.rs |

All of these are on every modern Linux distro. The installer does NOT
require these tools to be in the WriteOnce target sysroot — they only
need to be on the **workstation running the installer**.

## Smoke testing without a real build

`build/15-fake-artifacts.sh` produces a synthetic bundle so the
installer can be exercised end-to-end without running the multi-hour
Phase 0–9 build:

```
$ ./build/15-fake-artifacts.sh
=== fake-artifacts: writing to build/artifacts/
    used real writeonce-bootloader.efi
total 1.3M
-rwxr-xr-x  45K BOOTX64.EFI
-rw-r--r-- 1.0M bzImage              ← random bytes, NOT a real kernel
-rw-rw-r--  246 initramfs.img        ← minimal cpio with /init=/bin/sh
-rw-rw-r--  695 manifest.toml
-rw-rw-r--  372 sysroot.tar.zst      ← /etc/{hostname,os-release,writeonce-release}
```

What you can validate with fake artifacts:
- ✓ list-usb prints connected devices
- ✓ Manifest loads + SHA verification passes
- ✓ Safety check refuses non-removable / mounted devices
- ✓ Dry-run prints would-do without writing
- ✓ Full install + verify completes (the disk **boots into nothing**
  because bzImage is random, but the install pipeline is exercised)

What needs a real Phase 0–9 build:
- ✗ Actual boot of the resulting USB on a target machine

## Smoke-test commands

```bash
# In wo-builder Docker container:
./build/in-container.sh cargo build-installer

# On the host:
./target/release/writeonce-installer list-usb
./build/15-fake-artifacts.sh
sudo ./target/release/writeonce-installer install \
    --from build/artifacts \
    --target /dev/sda \
    --dry-run

# To actually write (still safe; fake artifacts will produce
# unbootable USB, but the partition tables + filesystems + ESP layout
# will all be real):
sudo ./target/release/writeonce-installer install \
    --from build/artifacts \
    --target /dev/sda
```

## Open work — future rounds

1. **ratatui TUI** — the current CLI is plain prompts. A ratatui-based
   UI would show device list + progress bars + log tail in one
   coordinated view. ~1 day.

2. **`writeonce build` subcommand** — the installer doesn't currently
   know how to invoke the build pipeline. A `writeonce` umbrella
   binary with `build` + `install` + `list-usb` would make the UX
   single-binary. ~1 day.

3. **Live-USB mode** — the "Model A" path in
   [`docs/learning/future-installer-remote-build.md`](future-installer-remote-build.md):
   the installer runs ON the target from a small live USB. Needs:
   - In-place ext4 install (the live USB is one device; the target is another)
   - Network bootstrap (iwd config)
   - Bootloader install to target's actual disk
   ~3 days.

4. **Resume / partial install** — if the install is interrupted (power
   loss, Ctrl-C), restart from the last sentinel. ~1 day.

5. **Multi-target** — install the same artifacts to N USBs in
   parallel. Useful for batch deployment. ~2 days, mostly tokio task
   plumbing.

6. **Sigstore / Cosign verification of manifest.toml** — sign the
   manifest at build time, verify at install time. Closes the trust
   loop for remote-build scenarios. ~1 day.

## Cross-references

- [`docs/learning/phase-6-bootloader-efi-stub-delegation.md`](phase-6-bootloader-efi-stub-delegation.md) — what BOOTX64.EFI does at boot.
- [`docs/learning/future-installer-remote-build.md`](future-installer-remote-build.md) — the remote-build-server vision this installer is a piece of.
- [`docs/learning/supply-chain-defense.md`](supply-chain-defense.md) — why we SHA-verify every artifact.
- [`crates/writeonce-bootloader/`](../../crates/writeonce-bootloader/) — produces BOOTX64.EFI.
- [`build/15-fake-artifacts.sh`](../../build/15-fake-artifacts.sh) — synthetic test bundle.
