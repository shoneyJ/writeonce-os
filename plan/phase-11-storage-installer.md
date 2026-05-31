# Phase 11 — Live installer: partitions, swap, dynamic fstab

> systemd branch (`with-systmed`), post-desktop. Supersedes the workstation-side
> `build/install.sh /dev/sdX` model with an **on-target, interactive live installer**.

**Goal.** Stick the WriteOnce USB into the target, boot it, and get an interactive
installer that collects the hostname + user + passwords, auto-detects the machine's
internal disk, proposes a **recommended partition layout the user can resize** (ESP +
swap + root, optional separate `/home`), then installs WriteOnce to that disk with a
correctly generated `/etc/fstab` and working swap.

## Context

Today (`build/install.sh`) the operator runs the installer **from the workstation**
against a device node, and it makes only **ESP + ext4 root** — no swap, no `/home`
split, and a **static** `/etc/fstab` (`build/17-stage-sysroot.sh:254`). Root is found
purely via the fixed PARTUUID `b0007001-…-002` baked into `CONFIG_CMDLINE`
(`build/kernel-config-additions.fragment:229`). This phase moves that logic onto the
target as a guided installer and adds swap + multi-partition support.

## Subtasks

1. **Live-installer boot mode.** When the running root is on **removable media**, boot
   into an installer instead of the desktop. Implement as a dedicated
   `installer.target` (or a tty1 unit that launches `writeonce-install`) selected when
   `/sys/block/<root-dev>/removable == 1`. The *installed* disk boots normally to
   `multi-user`/`graphical.target`. Keep the same single USB image for both roles.
2. **Target-disk detection.** Enumerate fixed disks (`lsblk -dno NAME,SIZE,MODEL,TRAN`,
   `removable==0`), **exclude the boot USB** (resolve the device backing `/`), and
   present the candidate(s) for the user to confirm. Refuse to proceed if the only disk
   is the USB itself.
3. **Interactive identity prompts.** Hostname, username, user password, root password.
   Reuse the host-side hashing approach already proven in `build/install.sh:75`
   (`openssl passwd -6` → direct `/etc/shadow` edit) — no chroot/PAM round-trip.
4. **Recommended layout, user-editable sizes.** Default scheme on the detected disk:
   `ESP 512 MiB (EF00)` + `swap` + `root (ext4)`, with an optional separate `/home`.
   Show recommended sizes (see §5 for swap, e.g. root = remainder, optional `/home`
   takes the bulk on larger disks) and let the user override each before committing.
   Confirm with a destructive-action prompt showing the final table.
5. **Swap mechanism — weigh and choose.** Decision deferred to implementation; the
   trade-offs:

   | Option | Pros | Cons |
   |---|---|---|
   | **Swapfile** on root (`/swapfile`) | simplest, resizable later, no partition | hibernate needs `resume=` + `resume_offset` (fiddly); ext4 only |
   | **Swap partition** (GPT 8200) | clean, enables hibernate-to-disk via `resume=PARTUUID` | fixed size; one more partition |
   | **zram** (compressed RAM) | fast, SSD-friendly, no disk wear | no hibernate; capped by RAM; needs `zram` kmod (currently not built) |

   **Recommendation:** swapfile by default (T450 has 16 GB RAM; swap is mostly a safety
   margin), with a documented switch to a swap partition for users who want hibernate.
   zram is a later nicety (needs the `zram` module added to the kernel).
6. **PARTUUID strategy under EFI-stub.** The kernel cmdline is **baked**
   (`CONFIG_CMDLINE`), so `root=` can't be chosen at install time. Keep assigning the
   **fixed root PARTUUID** to the root partition (as `build/install.sh:50` already does
   via `sgdisk --partition-guid`); generate everything else (swap, `/home`, ESP)
   dynamically into `/etc/fstab`. Document the alternative (a real bootloader / UEFI
   load-options to make `root=` dynamic) as out-of-scope for this branch.
7. **Dynamic `/etc/fstab` generation.** Replace the static heredoc
   (`build/17-stage-sysroot.sh:254`) with installer-generated entries: root
   (`PARTUUID=`), ESP (`/boot/efi`, vfat), swap (`/swapfile` or `UUID=` partition),
   optional `/home` (`UUID=`), plus the existing pseudo-fs lines. Use `UUID=`/`PARTUUID=`
   (never `/dev/sdX`).
8. **Reuse the existing install steps.** `sgdisk` partitioning, `mkfs.vfat`/`mkfs.ext4`,
   `zstd -d | tar -x` of `sysroot.tar.zst`, `BOOTX64.EFI` → ESP `\EFI\BOOT\`, and empty
   `/etc/machine-id` are all in `build/install.sh` — refactor them into a
   `writeonce-install` shell tool driven by the prompts above. `mkswap`/`swapon` (or
   `fallocate`+`mkswap` for the swapfile) is the only new mkfs step.
9. **Post-install.** Write `/etc/hostname` from the prompt; `chown -R` the new user's
   home; sync + unmount; offer reboot. First boot of the installed disk regenerates
   `machine-id` and brings up swap from fstab.

## Deliverable

A single USB image that, when booted on the target, runs `writeonce-install`
interactively and produces a fully partitioned internal disk (ESP + swap + root [+
`/home`]) with a generated `/etc/fstab`, set passwords, and the EFI-stub kernel on the
ESP. The workstation-side `build/install.sh` remains as the "flash the USB" tool.

## Acceptance criteria

- Boot the USB on the T450 → installer prompts for hostname/user/passwords, lists the
  Samsung SSD (not the USB), shows a recommended layout, accepts size edits.
- After install + reboot **without the USB**: system boots from the internal SSD;
  `swapon --show` lists active swap; `findmnt /home` shows the separate partition if
  chosen; `/etc/fstab` uses `UUID=`/`PARTUUID=` only.
- The installer never touches the USB it booted from.

## References

- `build/install.sh` — current non-interactive installer to refactor.
- `build/17-stage-sysroot.sh:254` — static fstab heredoc to replace.
- `build/kernel-config-additions.fragment:229` — baked `CONFIG_CMDLINE` / fixed root PARTUUID.
- `crates/writeonce-installer/target-os.json` — prior declarative-layout shape (reference only; this branch is interactive shell, no Rust).

## Risks

- **Wiping the wrong disk.** Hard-exclude the boot USB; require an explicit typed
  confirmation showing model + size before `sgdisk --zap-all`.
- **Baked cmdline vs dynamic root.** As long as the installer assigns the fixed root
  PARTUUID, the baked `root=` resolves; a future bootloader is the only way to relax this.
- **Hibernate expectations.** Swapfile hibernate needs `resume_offset`; if the user
  wants reliable hibernate, steer them to the swap-partition option.
- **`partprobe` races.** Settle (`udevadm settle` / `partprobe` + retry) before `mkfs`.
