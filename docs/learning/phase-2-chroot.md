# What "chroot into `$LFS`" means

> Companion to [`phase-0-temp-tools-result.md`](phase-0-temp-tools-result.md)
> and [`../../plan/done/phase-2-minimal-linux.md`](../../plan/done/phase-2-minimal-linux.md).
> Clarifies a colloquialism — and explains why WriteOnce's Phase 2 actually
> *skips* the chroot step that LFS conventionally takes.

## The mechanism

`chroot` (change root) is a Linux syscall — and the matching userspace command — that **swaps a process's view of `/`** to a different directory tree. It is a filesystem-namespace boundary, not a security one (root can break out trivially), but it is a clean way to "step into" a sysroot and run commands as if that sysroot were the live system.

### Before vs after

| Operation                 | Before `chroot $LFS`                  | After `chroot $LFS`                              |
| ------------------------- | ------------------------------------- | ------------------------------------------------ |
| `cd /`                    | Goes to host's `/`                    | Goes to `$LFS/`                                  |
| `ls /usr/bin`             | Host's `/usr/bin` (~3 000 entries)    | `$LFS/usr/bin` (~250 entries after LFS Ch. 6)    |
| `cat /etc/os-release`     | Host's release file                   | Whatever's in `$LFS/etc/` (probably empty)       |
| `gcc hello.c`             | Host's GCC, host glibc                | `$LFS/usr/bin/gcc`, target glibc at `$LFS/usr/lib/libc.so.6` |
| Kernel running underneath | Same kernel                            | Same kernel — `chroot` does not swap that         |
| PID 1                     | Host's init                            | Host's init — `chroot` is not a container        |

`chroot` is a userspace illusion: only file paths re-anchor. Hardware, kernel, PID 1, and the process's PIDs are all the host's.

## Prerequisites before chrooting

A bare `chroot $LFS /bin/bash` fails because bash and most utilities want `/dev/null`, `/proc/self/...`, devices, etc. Those don't yet exist inside the new root. The standard pre-chroot dance is:

```bash
sudo mount --bind /dev $LFS/dev
sudo mount -t devpts devpts -o gid=5,mode=0620 $LFS/dev/pts
sudo mount -t proc   proc   $LFS/proc
sudo mount -t sysfs  sysfs  $LFS/sys
sudo mount -t tmpfs  tmpfs  $LFS/run
```

These are bind / pseudo-mounts of the host kernel's runtime filesystems into the sysroot, so anything inside that reads `/proc/self/...` or opens `/dev/null` Just Works.

Then enter the chroot with a scrubbed environment so host `PATH`, `LD_*`, etc. don't leak:

```bash
sudo chroot "$LFS" /usr/bin/env -i \
    HOME=/root  TERM="$TERM"  PS1='(lfs chroot) \u:\w\$ '  \
    PATH=/usr/bin:/usr/sbin \
    /bin/bash --login
```

On exit, unmount in reverse — the `-R` is important because `devpts` is nested under `/dev`:

```bash
sudo umount -R $LFS/{dev,proc,sys,run}
```

## Why LFS chapter 7 uses chroot

After LFS Chapter 6, `$LFS/usr/` carries bash + gcc + binutils + coreutils + … linked against the *target* glibc. Chapter 7 enters the chroot so the remaining packages (Gettext, Bison, Perl, Python, Texinfo, Util-linux, etc.) can be built as a **native compile** instead of a cross-compile. From inside the chroot, `gcc` is just `gcc`; the cross-prefixed `x86_64-lfs-linux-gnu-gcc` becomes unnecessary because the system *is* the target.

That eliminates a class of cross-build edge cases — `autoconf` getting confused by the host/target/build triplets, configure scripts that insist on running test binaries, packages with custom build logic that doesn't grasp `--host=$LFS_TGT`, and so on.

## Why WriteOnce Phase 2 does not chroot

Phase 2 of WriteOnce builds exactly two artifacts, and neither needs a chroot:

| Phase 2 deliverable        | How it's built                                                                            | Chroot needed? |
| -------------------------- | ----------------------------------------------------------------------------------------- | -------------- |
| Linux 6.12 kernel for T450 | `make ARCH=x86_64 CROSS_COMPILE=$LFS/tools/bin/x86_64-lfs-linux-gnu- bzImage` on workstation | no            |
| BusyBox static `/init`     | Cross-built statically against the cross-toolchain                                       | no            |

Both are leaf artifacts copied into `build/artifacts/`. Their build does not need a native-looking environment; the cross-toolchain from Phase 0 is exactly the tool for the job.

Therefore WriteOnce **skips LFS chapter 7 entirely** (the extended chroot-only packages) and proceeds from "Chapter 6 temp tools complete" straight to "build kernel + initramfs". An earlier note in `build/README.md` that read "you can now chroot into `$LFS` for Phase 2" was a colloquial flourish — practically, we never enter one.

## When you might still chroot

Even though Phase 2 does not require it, one good use remains:

**Sanity-check the sysroot is internally consistent.** Drop in interactively, list `/usr/bin/`, run `bash --version`, run `gcc --version`, compile a hello-world. If the binaries cannot find each other, that surfaces a real Phase 0 bug before it bites Phase 2.

```bash
sudo mount --bind /dev $LFS/dev
sudo mount -t devpts devpts -o gid=5,mode=0620 $LFS/dev/pts
sudo mount -t proc proc $LFS/proc
sudo mount -t sysfs sysfs $LFS/sys
sudo mount -t tmpfs tmpfs $LFS/run

sudo chroot "$LFS" /usr/bin/env -i HOME=/root TERM=$TERM \
    PATH=/usr/bin:/usr/sbin /bin/bash --login

# inside the chroot:
(chroot) # gcc --version
(chroot) # echo 'int main(){return 42;}' | gcc -x c - -o /tmp/t
(chroot) # /tmp/t; echo $?     # expect 42

(chroot) # exit

sudo umount -R $LFS/{dev,proc,sys,run}
```

That is the practical sum of what "chroot into `$LFS`" means in this project.
