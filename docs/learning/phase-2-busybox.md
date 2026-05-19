# BusyBox — what it is, why Phase 2 uses it

> Companion to [`../../build/05-initramfs.sh`](../../build/05-initramfs.sh)
> and [`../../plan/phase-2-minimal-linux.md`](../../plan/phase-2-minimal-linux.md).

## In one sentence

A **multi-call binary**: one statically-linked executable, about 1–2 MB, that contains the code for ~400 standard Unix utilities (`ls`, `cp`, `mv`, `sh`, `mount`, `ps`, `wget`, `vi`, …). When invoked, it looks at `argv[0]` — the name it was called as — and dispatches to the matching applet inside itself.

```
/bin/busybox  ──┐ (the real ELF binary, ~1.5 MB)
                │
/bin/ls       ──┤  all symlinks pointing
/bin/cat      ──┤  to /bin/busybox
/bin/mount    ──┤
/bin/sh       ──┤
/bin/vi       ──┤
… ~400 more ──┘
```

Invoke as `ls`: BusyBox sees `argv[0] = "ls"` and runs its internal `ls_main()`. Invoke as `cat`: it runs `cat_main()`. Same binary, different identities. (Git's subcommand model is the same idea, just centralized under one entry point.)

## Why it exists

Three problems it solves:

1. **Size.** A traditional Linux userspace (GNU coreutils + bash + util-linux + iproute2 + …) is dozens of binaries, each dynamically linked against glibc, each with its own copies of common code. Total: ~30 MB plus glibc. BusyBox bundles equivalent functionality into ~1.5 MB, statically linked, no glibc at runtime.

2. **Self-containment.** A statically-linked single binary has zero shared-library dependencies. You can drop `busybox` into an empty filesystem and run it. That makes it the natural choice for **initramfs**, **rescue media**, **embedded firmware**, **busybox-based container images** (Alpine, distroless minimal), and anywhere else a libc + dynamic linker isn't available yet.

3. **Bootstrap.** A real `/init` (in Rust, eventually — WriteOnce's Phase 5) is a non-trivial piece of code to author before you've confirmed the kernel even boots. BusyBox gives you a usable shell + tools immediately, so Phase 2 can verify "kernel comes up, mounts root, networking works" before investing in the Rust replacement.

## What's inside

```
Shell:           ash (bash-like but smaller), hush
Coreutils:       ls cat cp mv rm chmod chown ln mkdir rmdir pwd echo head tail
                 sort uniq wc cut tr expr basename dirname stat dd df du free
                 (~100 of them)
Util-linux:      mount umount mknod losetup swapon switch_root
Networking:      ip ifconfig route ping nslookup wget udhcpc httpd telnet
System:          ps top dmesg lsmod insmod rmmod modprobe init halt reboot
                 hostname uptime uname
Editors:         vi (a tiny vi clone), ed, sed, awk
Misc:            tar gzip gunzip bzip2 cpio find xargs grep
```

Each applet is a **subset implementation** — `busybox ls` knows fewer flags than GNU `ls`, `busybox awk` doesn't match GNU gawk feature-for-feature. For an initramfs's needs, the subsets are more than enough.

## How WriteOnce uses it (Phase 2)

In `build/05-initramfs.sh`, the initramfs root is constructed like this:

```bash
cp "$BUILD_ROOT/artifacts/busybox" "$INITRAMFS_ROOT/bin/busybox"

# For every applet name busybox knows, create a symlink to busybox:
for cmd in $(busybox --list); do
    ln -sf busybox bin/"$cmd"
done
```

Then the transitional `/init` script invokes BusyBox by applet name:

```sh
/bin/busybox mount -t proc proc /proc
/bin/busybox mount -t sysfs sysfs /sys
/bin/busybox mount -t devtmpfs devtmpfs /dev
exec /bin/busybox sh
```

When the kernel hands control to `/init`, the initramfs has exactly two binary contents that matter: `/bin/busybox` itself, and the `/init` script. Everything else (`/bin/ls`, `/bin/sh`, `/bin/mount`, etc.) is a symlink to the same 1.5 MB blob.

## When WriteOnce stops using it

**Phase 5.** The roadmap replaces this BusyBox-based initramfs wholesale with a Rust `/init` binary that does the same mount-and-handoff dance but with type-safe error handling. After that, BusyBox is no longer part of the WriteOnce stack at all — the chapter-6 sysroot has full GNU coreutils, bash, etc., for the running system, and the initramfs is bespoke Rust.

So in the WriteOnce lifecycle BusyBox is **transitional scaffolding**: critical to Phase 2's first boot, gone by Phase 5.

## History + alternatives

- Created in 1995 by Bruce Perens for the Debian rescue floppy. Now maintained by Denys Vlasenko (the GPG key you imported in Phase 0).
- **Alpine Linux** uses BusyBox + musl as its standard userspace (~5 MB base install).
- **OpenWrt**, **Buildroot**, **Yocto** all use BusyBox by default.
- **Toybox** is a permissively-licensed alternative (BSD vs BusyBox's GPLv2) used by Android.
