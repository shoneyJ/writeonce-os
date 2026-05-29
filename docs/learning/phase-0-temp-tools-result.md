# Phase 0 result — what `$LFS/` contains after Chapter 6 temp tools

> Companion to [`../../build/03-sysroot-temp-tools.sh`](../../build/03-sysroot-temp-tools.sh)
> and the earlier [`phase-0-lfs-tools-layout.md`](phase-0-lfs-tools-layout.md).
> Describes the end-state of Phase 0: a chroot-capable sysroot.

## The milestone

After `./03-sysroot-temp-tools.sh` completes, `$LFS/` is a **chroot-capable
userspace**. Drop in a kernel image and you have a usable Linux from the
keyboard up. Up to this point `$LFS/usr/bin/` was empty; now it carries the
minimum tooling needed to build the rest of the system from *inside* the
chroot rather than from the host.

## Concrete file additions

```
$LFS/usr/bin/        ~250 entries: bash, sh, ls/cp/mv/cat/… (coreutils, ~110 of them),
                                   awk, sed, grep, gzip, xz, tar, make, patch, file, find,
                                   diff/cmp, gcc, cc, g++, cpp, gcov, ar, as, ld, nm,
                                   objdump, readelf, strings, strip, addr2line, c++filt,
                                   m4, tic, tput, tset, infocmp, clear, …

$LFS/usr/lib/        adds: libncursesw.so.6, libmagic.so.1, libbfd-*.so, libopcodes-*.so,
                           libctf-*.so, liblzma.so.5, libgcc_s.so.1
                     (already had: libc.so.6, libm.so.6, libpthread.so.0, libstdc++.so.6,
                                   crt1.o, crti.o, crtn.o, ld-linux-x86-64.so.2)

$LFS/usr/libexec/gcc/x86_64-lfs-linux-gnu/14.2.0/
                     cc1, cc1plus, collect2 — the real GCC back-ends, now linked against
                     the target glibc (Pass 2), not the host's

$LFS/usr/include/    adds: curses.h, term.h, magic.h, bfd.h, libiberty.h, gawk headers
                     (already had: kernel UAPI + glibc headers from cross-toolchain step)

$LFS/usr/share/      terminfo database (huge — every terminal type), man pages, info pages,
                     locale stubs, gawk/m4/make docs
```

## The two compilers

After Pass 2 GCC, **two GCC chains** coexist on disk. Which one you invoke
depends on which `bin/` is on `PATH`:

| Path                                                     | What it is                                                        | Linked against                       |
| -------------------------------------------------------- | ----------------------------------------------------------------- | ------------------------------------ |
| `$LFS/tools/bin/x86_64-lfs-linux-gnu-gcc`                | The **cross-compiler**. Frozen since Phase 0 step 5. Host-side.  | Host's libc (used to build itself)  |
| `$LFS/usr/bin/gcc`                                       | The **target-native compiler**. Becomes the "real" gcc inside the chroot. | Target glibc (`$LFS/usr/lib/libc.so.6`) |

Same duplication exists for binutils: `tools/bin/x86_64-lfs-linux-gnu-{ar,as,ld,…}`
(cross) vs `usr/bin/{ar,as,ld,…}` (unprefixed, target-native). When Phase 7
wants to build a kernel and Rust modules, you'll be invoking the unprefixed
versions from inside the chroot — that compiler produces binaries linked
against the target glibc by default, so nothing on the host can leak in.

## What is still missing

Deliberately — these belong to later phases of the WriteOnce roadmap:

| Missing                                                        | Where it gets created                |
| -------------------------------------------------------------- | ------------------------------------ |
| Kernel image, `/boot/bzImage`                                  | Phase 2 (`02-cross-toolchain.sh` is for the **toolchain**; the kernel build itself is Phase 2's job) |
| `/init`, PID 1 binary                                          | Phase 2 (BusyBox transitional), Phase 3 (Rust replacement) |
| `/etc/passwd`, `/etc/fstab`, `/etc/group`, `/etc/hostname`    | Phase 2 sysroot author script        |
| `/var/log`, `/home/<user>`                                     | Phase 2                              |
| iwlwifi firmware blobs                                         | Phase 1 archive (already captured)   |
| D-Bus, X.Org, GTK4, i3                                         | Phase 8                              |
| PAM, PipeWire                                                  | Phase 8                              |
| i3More binaries                                                | Phase 9                              |
| Rust-built bootloader / initramfs / supervisor                 | Phases 3–6                           |
| Userspace daemons (`sshd`, `getty`, network manager)           | None — replaced by `wo-ctl` + targeted services |

## Disk usage at this point

| Phase complete                                  | Approx `du -sh $LFS/` |
| ----------------------------------------------- | --------------------- |
| Just `01-fetch.sh` (sources only)               | ~0.6 GB (in `sources/`, not `sysroot/`) |
| `02-cross-toolchain.sh` done                    | ~1.5 GB (mostly `$LFS/tools/`)         |
| `03-sysroot-temp-tools.sh` done                 | ~3.5 GB total                          |

Most of the chapter-6 growth comes from GCC Pass 2 (it ships large internal
libraries) and the ncurses terminfo database. Coreutils contributes another
~50 MB.

## What to do next

WriteOnce diverges from upstream LFS here. The book's Chapter 7 enters a
chroot and builds a few more temp packages (Gettext, Bison, Perl, Python,
Texinfo, Util-linux) before the final-system Chapter 8 install. WriteOnce
**skips Chapter 7 deliberately** — we don't need a fully general-purpose
userspace inside the chroot; we need just enough to:

1. Build the Linux 6.12 kernel for the T450 (Phase 2 next).
2. Produce a BusyBox-based transitional initramfs (Phase 2).
3. Hand off, eventually, to the Rust components built independently in
   Phases 3–6.

So the natural next step after `03-sysroot-temp-tools.sh` is **moving to
Phase 2** (kernel build + transitional initramfs + first T450 boot), not
LFS Chapter 7. See [`../../plan/done/phase-2-minimal-linux.md`](../../plan/done/phase-2-minimal-linux.md).
