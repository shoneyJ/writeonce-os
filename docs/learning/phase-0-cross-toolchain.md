# Phase 0 — The cross-toolchain, in short

> Reference companion to [`../../build/02-cross-toolchain.sh`](../../build/02-cross-toolchain.sh)
> and [`../../plan/phase-0-toolchain.md`](../../plan/phase-0-toolchain.md).
> Source-of-truth in [LFS book Chapter 5](../../.agents/reference/lfs-rendered.txt) (sections 5.1–5.6).

## Role

The cross-toolchain is a compiler/linker/library set that **runs on the workstation but produces binaries for the target system's libc + ABI**, with **zero contamination from the host's libc, headers, or library paths**.

Even when host and target are both x86_64, the host's glibc is a *different* glibc from the one the T450 will ship with. Without a cross-toolchain, every binary built would silently link against host headers and host glibc symbol versions, defeating reproducibility and bricking the target the moment its glibc drifts from the host's.

The toolchain lives in `$LFS/tools/` (host-side, never installed to the target). From Chapter 6 onward it is the **only** compiler used to build anything that ends up on the T450.

## LFS Chapter 5 — what each step builds and why

| LFS §   | What                              | Why it exists                                                                                            |
| ------- | --------------------------------- | -------------------------------------------------------------------------------------------------------- |
| **5.2** | **Binutils — Pass 1**             | Cross-assembler (`x86_64-lfs-linux-gnu-as`) and cross-linker (`-ld`). GCC and Glibc both probe the linker during their own configure to decide which features to enable, so the linker must come first. |
| **5.3** | **GCC — Pass 1**                  | Minimal C/C++ compiler targeted at `x86_64-lfs-linux-gnu`. Built with `--without-headers --with-newlib --disable-shared` because the target Glibc doesn't exist yet — this is a bootstrap compiler whose only job is to compile Glibc. |
| **5.4** | **Linux API headers**             | The kernel's UAPI headers (`<linux/*.h>`, `<asm/*.h>`). Glibc wraps Linux syscalls, so it needs to know what syscalls and structures the target kernel exposes. |
| **5.5** | **Glibc**                          | The target C library. Cross-built using Pass-1 GCC + the kernel headers; installs into `$LFS/usr/lib`. Once this exists, the toolchain can produce real binaries that run on the target. |
| **5.6** | **Libstdc++ (from GCC)**          | C++ runtime library, built **separately** from Pass-1 GCC, against the newly-available target Glibc. Pass-1 GCC was `--disable-libstdcxx` because libstdc++ needs glibc; now it has glibc, so it builds. |

The **"Pass 1"** labels are not stylistic. Pass 1 GCC's binaries cannot themselves be the final compiler because they were built without target headers; a **Pass 2** rebuild happens in Chapter 6 once the temporary toolchain in `$LFS/usr` is in place. By that point you will have built the same GCC twice — once to bootstrap glibc, once to use it.

## How this maps to the scripts

```
02-cross-toolchain.sh step    LFS §       Output lands in
─────────────────────────     ─────       ───────────────
binutils-1                    5.2         $LFS/tools/bin/x86_64-lfs-linux-gnu-{ld,as,...}
gcc-1                         5.3         $LFS/tools/bin/x86_64-lfs-linux-gnu-{gcc,g++,...}
linux-headers                 5.4         $LFS/usr/include/{linux,asm,...}
glibc                         5.5         $LFS/usr/lib/{libc.so.6,ld-linux-x86-64.so.2,...}
libstdcxx                     5.6         $LFS/usr/lib/libstdc++.so.6
```

## Acceptance: how to know it worked

After all five steps complete:

- `$LFS/tools/` is **complete and frozen** until Phase 7 of WriteOnce (kernel/module work) — nothing else writes here.
- `$LFS/usr/` contains the **target glibc + libstdc++** — the foundation that every Chapter 6 temp tool, and every final Chapter 8 system program, will link against.
- The cross-compiler smoke test in `02-cross-toolchain.sh` (`step_glibc`) builds a hello-world and inspects its ELF interpreter:

  ```bash
  $LFS/tools/bin/x86_64-lfs-linux-gnu-gcc /tmp/dummy.c -o /tmp/dummy
  readelf -l /tmp/dummy | grep 'program interpreter'
  ```

  The interpreter must be `/lib64/ld-linux-x86-64.so.2` (the **target** dynamic linker, which will resolve to `$LFS/lib64/...` when the binary runs in the target rootfs). It must **not** be the host's `/lib/x86_64-linux-gnu/ld-linux-x86-64.so.2`. The mismatch is exactly how you confirm host bleed didn't happen.

## References while running `./02-cross-toolchain.sh`

- `.agents/reference/lfs-rendered.txt` — search for `5.2.1`, `5.3.1`, `5.4`, `5.5.1`, `5.6.1` for the per-step LFS prose.
- `.agents/reference/lfs/chapter05/` — raw DocBook XML if you want the un-rendered source.
- `.agents/reference/linux/Documentation/kbuild/headers_install.rst` — for the `make headers` step in 5.4.
