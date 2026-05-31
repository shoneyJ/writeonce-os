# WriteOnce OS — Phase 8 manual driver.
#
# Each phase step is sentinel-driven (build/logs/.done-blfs-<pkg>); reruns
# are cheap — already-built packages are skipped. Delete a sentinel to
# force a redo. Use `just progress` between steps to see what's done.

_default:
    @just --list --unsorted

# Fetch + verify all upstream tarballs (idempotent).
fetch:
    ./build/in-container.sh ./build/01-fetch.sh

# Audit the last meson configure failure for a package — surfaces THE
# blocker plus every optional dep that was probed NO, so you can fix
# everything in one pass instead of one-at-a-time trial-and-error.
# Example: `just audit xorg-server` after `just phase-8c` fails.
audit pkg:
    ./build/audit-deps.sh {{pkg}}

# Audit the most recently failed package (figured out from mtime of
# meson-log.txt under build/work/*/build/meson-logs/). Use after any
# `just phase-8X` failure when you don't want to scroll up looking
# for the "stopping at <pkg>" line.
audit-last:
    @last=$(ls -t build/work/*/build/meson-logs/meson-log.txt 2>/dev/null | head -1); \
     if [ -z "$last" ]; then echo "no meson-log.txt found under build/work/"; exit 1; fi; \
     pkg=$(echo "$last" | sed -E 's|build/work/([^/]+)/.*|\1|'); \
     printf 'most-recent meson failure: %s\n\n' "$pkg"; \
     ./build/audit-deps.sh "$pkg"

# Print sentinel count + most-recent packages built.
progress:
    @printf 'Built %s/75 Phase-8 packages\n' "$(ls build/logs/.done-blfs-* 2>/dev/null | wc -l)"
    @ls -t build/logs/.done-blfs-* 2>/dev/null | head -10 | sed 's|.*\.done-blfs-|  |'

# Assemble the final sysroot — runs on the host (not in the container)
# because it needs to read /opt/i3more/bin/ and the i3More symlink target.
# Output: build/staging/sysroot/
stage:
    ./build/17-stage-sysroot.sh

# Bundle the staged sysroot + kernel + initramfs + EFI bootloader into
# the installer artifact set under build/artifacts/. Runs in container.
artifacts:
    ./build/in-container.sh ./build/18-make-artifacts.sh

# Pre-flight checklist for the staged sysroot — verifies every file,
# user, library, and skeleton entry that boot has needed in past
# rounds. Runs on the workstation BEFORE USB flash, catching missing
# pieces in seconds instead of after a 5-minute USB+boot cycle on the
# T450. `just usb-install` runs this first and refuses to flash on
# any failure.
check-staging:
    ./build/check-staging.sh

# List USB block devices the installer can write to. Read-only, safe.
usb-list:
    sudo ./target/release/writeonce-installer list-usb

# Write the artifact set to a USB stick. DOUBLE-CHECK device path —
# the target device is wiped. Example: `just usb-install /dev/sdc`.
# Runs check-staging first; refuses to flash if the staged sysroot
# is missing anything boot has needed historically.
usb-install device: check-staging
    sudo ./build/install.sh {{device}}

# DIAGNOSTIC: rewrite cmdline.txt on the USB ESP to a verbose-debug
# variant — loglevel 7, no quiet, earlycon kept alive, panic=0 holds
# the panic message on screen instead of auto-rebooting. Use after
# `just usb-install` if you want to see kernel boot output on T450.
usb-cmdline-debug device:
    #!/usr/bin/env bash
    set -euo pipefail
    sudo mkdir -p /mnt/wo-esp
    sudo mount {{device}}1 /mnt/wo-esp
    ROOT_UUID="$(sudo blkid -s UUID -o value {{device}}2)"
    # rootwait — tells the kernel to wait for the root device to
    # appear (USB enumeration can race with root mount). Critical when
    # booting from removable media; without it the kernel gives up the
    # moment its first attempt to find root= fails.
    # writeonce.rootwait=30 = our initramfs polls /sys/class/block for
    # up to 30 s waiting for the root device to appear. Without it the
    # initramfs scans once and drops to the recovery shell before USB
    # enumeration completes on slow hardware. See plan/phase-5… and
    # crates/writeonce-initramfs/src/discover.rs.
    sudo tee /mnt/wo-esp/EFI/WriteOnce/cmdline.txt >/dev/null <<EOF
    console=tty0 earlycon=efifb,keep loglevel=7 ignore_loglevel panic=0 rootwait writeonce.rootwait=30 root=UUID=${ROOT_UUID} rw init=/usr/sbin/writeonce-pid1
    EOF
    echo "cmdline: $(sudo cat /mnt/wo-esp/EFI/WriteOnce/cmdline.txt)"
    sudo umount /mnt/wo-esp
    sudo eject {{device}}

# DIAGNOSTIC: restore the production (quiet) cmdline from manifest.toml.
usb-cmdline-prod device:
    #!/usr/bin/env bash
    set -euo pipefail
    sudo mkdir -p /mnt/wo-esp
    sudo mount {{device}}1 /mnt/wo-esp
    TPL="$(grep '^cmdline' build/artifacts/manifest.toml | sed -E 's/cmdline *= *"([^"]*)".*/\1/')"
    ROOT_UUID="$(sudo blkid -s UUID -o value {{device}}2)"
    echo "${TPL//__ROOT_UUID__/$ROOT_UUID}" | sudo tee /mnt/wo-esp/EFI/WriteOnce/cmdline.txt >/dev/null
    echo "cmdline: $(sudo cat /mnt/wo-esp/EFI/WriteOnce/cmdline.txt)"
    sudo umount /mnt/wo-esp
    sudo eject {{device}}

# DIAGNOSTIC: dump the bootloader's boot.log from the USB ESP. Our
# Rust bootloader writes a step-by-step trace to \EFI\WriteOnce\boot.log
# at handoff. Read this after a failed boot attempt to see how far
# the bootloader got and the exact UEFI status code on failure.
# Also lists the ESP contents so missing files are obvious.
usb-logs device:
    #!/usr/bin/env bash
    set -euo pipefail
    sudo mkdir -p /mnt/wo-esp
    sudo mount {{device}}1 /mnt/wo-esp
    echo "==== ESP contents ===="
    sudo ls -la /mnt/wo-esp/EFI/WriteOnce/ 2>/dev/null || echo "  /EFI/WriteOnce/ missing"
    echo
    echo "==== cmdline.txt ===="
    sudo cat /mnt/wo-esp/EFI/WriteOnce/cmdline.txt 2>/dev/null || echo "  (no cmdline.txt)"
    echo
    echo "==== boot.log ===="
    if sudo test -f /mnt/wo-esp/EFI/WriteOnce/boot.log; then
        sudo cat /mnt/wo-esp/EFI/WriteOnce/boot.log
    else
        echo "  (no boot.log — the bootloader didn't reach the FS open"
        echo "   step, or you're using GRUB which doesn't write boot.log)"
    fi
    echo
    sudo umount /mnt/wo-esp

# DIAGNOSTIC: replace our bzImage on the USB ESP with the host's
# Ubuntu kernel binary, and rewrite cmdline.txt to drop to a recovery
# shell. Used to bisect "is the failure in OUR kernel build OR in the
# bootloader→kernel handoff?". If Ubuntu's kernel boots from our USB,
# our build is suspect; if Ubuntu's also goes dark, the bootloader is.
# Backs up our bzImage as bzImage.wo so `just diag-restore-kernel`
# can swap it back. Default kernel = newest /boot/vmlinuz-*-generic;
# override with KVER=6.8.0-117-generic just diag-ubuntu-kernel /dev/sda.
diag-ubuntu-kernel device:
    #!/usr/bin/env bash
    set -euo pipefail
    KVER="${KVER:-$(ls -1 /boot/vmlinuz-*-generic 2>/dev/null | sort -V | tail -1 | sed 's|.*/vmlinuz-||')}"
    SRC="/boot/vmlinuz-${KVER}"
    [[ -f "$SRC" ]] || { echo "no kernel at $SRC — set KVER=… and try again"; exit 1; }
    echo "Swapping in $SRC onto ESP of {{device}}"
    sudo mkdir -p /mnt/wo-esp
    sudo mount {{device}}1 /mnt/wo-esp
    if [[ ! -f /mnt/wo-esp/EFI/WriteOnce/bzImage.wo ]]; then
        sudo cp /mnt/wo-esp/EFI/WriteOnce/bzImage /mnt/wo-esp/EFI/WriteOnce/bzImage.wo
        echo "  saved our kernel as bzImage.wo"
    else
        echo "  bzImage.wo already exists (keeping previous backup)"
    fi
    sudo cp "$SRC" /mnt/wo-esp/EFI/WriteOnce/bzImage
    sudo tee /mnt/wo-esp/EFI/WriteOnce/cmdline.txt >/dev/null <<'EOF'
    console=tty0 earlycon=efifb,keep loglevel=7 ignore_loglevel panic=0 init=/init
    EOF
    echo "  new cmdline: $(cat /mnt/wo-esp/EFI/WriteOnce/cmdline.txt)"
    sudo umount /mnt/wo-esp
    sudo eject {{device}}
    echo "Done. Boot the T450 from this USB."
    echo
    echo "If you see [ 0.000000] Linux version ... → bootloader/handoff is fine."
    echo "If still silent then reboot → bootloader/firmware handoff is suspect."
    echo "Restore ours with: just diag-restore-kernel {{device}}"

# DIAGNOSTIC: undo `diag-ubuntu-kernel` — restore our bzImage from the
# bzImage.wo backup, refresh cmdline.txt from manifest.toml.
diag-restore-kernel device:
    #!/usr/bin/env bash
    set -euo pipefail
    sudo mkdir -p /mnt/wo-esp
    sudo mount {{device}}1 /mnt/wo-esp
    if [[ -f /mnt/wo-esp/EFI/WriteOnce/bzImage.wo ]]; then
        sudo mv /mnt/wo-esp/EFI/WriteOnce/bzImage.wo /mnt/wo-esp/EFI/WriteOnce/bzImage
        echo "  restored our bzImage from .wo backup"
    else
        echo "  no bzImage.wo backup found — nothing to restore"
    fi
    # Reflash cmdline.txt from the manifest (re-substitutes UUID).
    CMDLINE_TEMPLATE="$(grep '^cmdline' build/artifacts/manifest.toml | sed -E 's/cmdline *= *"([^"]*)".*/\1/')"
    ROOT_UUID="$(sudo blkid -s UUID -o value {{device}}2)"
    REAL_CMDLINE="${CMDLINE_TEMPLATE//__ROOT_UUID__/$ROOT_UUID}"
    echo "$REAL_CMDLINE" | sudo tee /mnt/wo-esp/EFI/WriteOnce/cmdline.txt >/dev/null
    echo "  cmdline: $REAL_CMDLINE"
    sudo umount /mnt/wo-esp
    sudo eject {{device}}
    echo "Done. WriteOnce kernel restored on {{device}}."

# Boot the full bootloader→kernel→initramfs chain in QEMU under OVMF.
# Streams kernel + bootloader logs to the terminal via serial console.
# 30-sec iteration loop vs minutes-per-USB-flash on real hardware.
# Exit qemu with: Ctrl-] then x
qemu-full:
    ./build/qemu-full.sh

# Run the writeonce-kerngen hardware probe on THIS host. Dumps JSON to
# stdout by default; pass --output PATH to write a file:
#     just kerngen-probe                              # stdout
#     just kerngen-probe --output ~/t450-probe.json   # named file
# The probe walks /sys/bus/{pci,usb,acpi,virtio,platform}/devices,
# /proc/cpuinfo, /sys/class/dmi/id, /sys/firmware/efi and emits a
# flat schema (see crates/writeonce-kerngen/src/types.rs). Future
# `writeonce-kerngen resolve` (Phase 7b) will consume this output
# and a kernel source tree to derive a target-specific .config.
kerngen-probe *args:
    cargo run --release -p writeonce-kerngen -- probe {{args}}

# Kernel rebuild — drops config/build/modules-stage sentinels (extract is
# kept so the source tree isn't re-unpacked) and re-runs 04-kernel.sh +
# 05-initramfs.sh. Optionally pass a one-liner reason that gets logged
# to docs/kernel-build-history.md:
#     just kernel "added simpledrm + sysfb fallback for T450 i915 hang"
# When no reason is passed the entry says "(no reason supplied)".
kernel reason='':
    rm -f build/logs/.done-kernel-config build/logs/.done-kernel-build \
          build/logs/.done-kernel-modules-stage \
          build/logs/.done-initramfs-root build/logs/.done-initramfs-pack
    ./build/in-container.sh ./build/04-kernel.sh
    ./build/in-container.sh ./build/05-initramfs.sh
    KERNEL_REBUILD_REASON='{{reason}}' ./build/kernel-history-append.sh '{{reason}}'

# Initramfs-only rebuild — drops just the initramfs sentinels and
# re-runs 05-initramfs.sh inside wo-builder. Use this after editing
# the Rust /init crate (`crates/writeonce-initramfs/`) or the
# kernel-config-additions.fragment when only the initramfs payload
# needs to refresh. Skips the ~25 min kernel compile.
initramfs:
    rm -f build/logs/.done-initramfs-root build/logs/.done-initramfs-pack
    ./build/in-container.sh ./build/05-initramfs.sh

# Phase 8a — base substrate (zlib, libpng, freetype, fontconfig, pam, dbus, ...).
phase-8a:
    ./build/in-container.sh --no-network ./build/08-base-substrate.sh

# Phase 8b — X11 protocol headers + core libraries.
phase-8b:
    ./build/in-container.sh --no-network ./build/09-x11-stack.sh

# Phase 8c — Mesa (iris), libdrm, libinput, xorg-server.
phase-8c:
    ./build/in-container.sh --no-network ./build/10-xorg-server.sh

# Phase 8d — glib, harfbuzz, cairo, pango, gdk-pixbuf, gtk4.
phase-8d:
    ./build/in-container.sh --no-network ./build/11-gtk-stack.sh

# Phase 8e — alsa-lib, pipewire, wireplumber, lua.
phase-8e:
    ./build/in-container.sh --no-network ./build/12-audio-stack.sh

# Phase 8f — ell, iwd, iproute2, iputils, dhcpcd.
phase-8f:
    ./build/in-container.sh --no-network ./build/13-network-stack.sh

# Base userspace — the LFS Ch8 runtime essentials (kmod, util-linux, procps-ng,
# shadow, bzip2) into $LFS/usr. coreutils/sed/grep/gzip/tar come from
# 03-sysroot-temp-tools.sh; this completes the GNU base the boot chain + desktop
# need. Run a single package with e.g. `just base kmod`.
base *steps:
    ./build/in-container.sh --no-network ./build/14-base-userspace.sh {{steps}}

# systemd (with-systmed branch) — PID1 + service manager + logind + udev +
# journald, minimal cross build into $LFS/usr. Replaces the custom Rust init.
systemd:
    ./build/in-container.sh --no-network ./build/16-systemd.sh

# Install WriteOnce (systemd branch) to a block device — DESTROYS the target.
# Gates on check-staging first. Needs `just stage && just artifacts` done.
# Example: `just install /dev/sda`.
install device:
    ./build/check-staging.sh
    sudo ./build/install.sh {{device}}

# Copy the installed system's logs (systemd journal + Xorg) off a WriteOnce disk
# to /tmp/writeonce-logs/ so they can be read on the workstation after a boot —
# mounts read-only, needs sudo (the journal is root:systemd-journal), and chowns
# the copy to you so it's readable (e.g. by Claude). Defaults to the root
# partition by label; override the device if it differs:
#   just pull-logs                 # uses /dev/disk/by-label/writeonce-root
#   just pull-logs /dev/sdb2
# Then read it (works across systemd version gaps):
#   journalctl --directory=/tmp/writeonce-logs/journal --no-pager | tail -120
pull-logs device='/dev/disk/by-label/writeonce-root':
    #!/usr/bin/env bash
    set -euo pipefail
    dev="{{device}}"
    [ -b "$dev" ] || { echo "error: $dev is not a block device — plug in the USB and check 'lsblk -o NAME,SIZE,LABEL'"; exit 1; }
    me="$(id -un)"; grp="$(id -gn)"; out=/tmp/writeonce-logs
    mnt="$(mktemp -d)"
    echo ">> mounting $dev read-only (sudo may prompt for your password)..."
    sudo mount -o ro "$dev" "$mnt"
    rm -rf "$out"; mkdir -p "$out"
    if [ -d "$mnt/var/log/journal" ]; then
        sudo cp -a "$mnt/var/log/journal" "$out/journal"; echo "   journal: copied"
    else
        echo "   journal: NONE (/var/log/journal absent — journald was volatile)"
    fi
    sudo cp -a "$mnt"/var/log/Xorg.*.log "$out/" 2>/dev/null && echo "   Xorg (/var/log): copied" || true
    sudo cp -a "$mnt"/home/writeonce/.local/share/xorg/*.log "$out/" 2>/dev/null && echo "   Xorg (rootless): copied" || true
    sudo cp -a "$mnt"/home/writeonce/.cache/*.log "$out/" 2>/dev/null && echo "   ~/.cache logs (i3More/pipewire): copied" || true
    sudo chown -R "$me:$grp" "$out"
    sudo umount "$mnt"; rmdir "$mnt"
    echo ">> done — $out/:"
    ls -R "$out" 2>/dev/null | sed 's/^/   /' | head -40
    echo ">> read the boot journal with:"
    echo "   journalctl --directory=$out/journal --no-pager | tail -120"

# Run the whole Phase 8 chain (8a → 8f). Each step skips already-built packages.
phase-8: phase-8a phase-8b phase-8c phase-8d phase-8e phase-8f

# Resume Phase 8 from current sentinel state (alias for `phase-8`; the
# sentinels make it idempotent).
phase-8-resume: phase-8

# Force-redo a single package: delete its sentinel + workdir, then rerun
# the owning phase step. Example: `just redo-pkg mesa 10-xorg-server`
redo-pkg pkg phase:
    rm -f build/logs/.done-blfs-{{pkg}}
    rm -rf build/work/{{pkg}}
    ./build/in-container.sh --no-network ./build/{{phase}}.sh
