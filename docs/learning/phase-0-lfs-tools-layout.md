# `$LFS/tools/` — what's inside, where it lives

> Companion to [`../../build/02-cross-toolchain.sh`](../../build/02-cross-toolchain.sh)
> and [`phase-0-cross-toolchain.md`](phase-0-cross-toolchain.md).
> Captures the on-disk layout of the cross-toolchain *and* the surrounding
> sysroot, so a reader can navigate `$LFS` confidently.

## On-disk location

```
$LFS/tools/  → /home/shoney/projects/github/shoneyj/writeonce-os/build/cross-tools/
                                                                     ─────────────
            (symlink under sysroot; the real directory is a sibling at build/cross-tools)
```

The symlink is intentional. LFS book recipes use `--prefix=$LFS/tools` and reference `$LFS/tools/bin/...` verbatim, but the WriteOnce repo layout keeps `cross-tools/` (host-side) and `sysroot/` (target-side) at sibling top-level directories. The symlink lets every upstream command Just Work while keeping the repo organisation clean.

Size after binutils-1 + gcc-1 + headers + glibc + libstdc++: about **1.5 GB**.

## Layout of `$LFS/tools/`

```
tools/
├── bin/                                28 binaries, all prefixed x86_64-lfs-linux-gnu-*
│   ├── x86_64-lfs-linux-gnu-gcc        ← the cross-compiler driver
│   ├── x86_64-lfs-linux-gnu-g++
│   ├── x86_64-lfs-linux-gnu-cpp
│   ├── x86_64-lfs-linux-gnu-ld         ← cross-linker (BFD)
│   ├── x86_64-lfs-linux-gnu-as         ← cross-assembler
│   ├── x86_64-lfs-linux-gnu-{ar,nm,objcopy,objdump,readelf,strip,…}
│   └── (gcov-tool, lto-dump, gcc-14.2.0 versioned alias, etc.)
│
├── libexec/
│   └── gcc/x86_64-lfs-linux-gnu/14.2.0/
│       ├── cc1          ← C front-end (the actual C compiler back-end)
│       ├── cc1plus      ← C++ front-end
│       ├── collect2     ← linker wrapper that handles C++ ctor/dtor
│       └── plugin/, install-tools/
│
├── lib/
│   ├── libcc1.so*       ← GCC plugin library
│   ├── bfd-plugins/     ← linker plugins (LTO etc.)
│   └── gcc/             ← target-prefixed subdir with built-in includes & libgcc
│
├── x86_64-lfs-linux-gnu/   ← target-prefixed area used internally by binutils/gcc
│   ├── bin/             (target's "private" bins; mostly aliases)
│   ├── include/
│   └── lib/
│
├── include/                small, GCC-internal headers
└── share/                  locale catalogs, info pages
```

`tools/bin/` is the user-facing surface — the commands you actually invoke. Everything in `libexec/`, internal `lib/`, and `x86_64-lfs-linux-gnu/` is reached *via* those bin entry points: when `x86_64-lfs-linux-gnu-gcc` is called, it execs `cc1` from `libexec/gcc/x86_64-lfs-linux-gnu/14.2.0/`, links via `ld` in `tools/bin/`, and uses libraries from the target subtree.

## What is *not* in `$LFS/tools/` but exists nearby

Glibc and the kernel headers belong to the **target system**, not the toolchain. They land outside `tools/`:

```
$LFS/
├── etc/                    (minimal: /etc/ld.so.conf etc. produced by glibc)
├── lib64/
│   ├── ld-linux-x86-64.so.2 → ../lib/ld-linux-x86-64.so.2      ← target dynamic linker
│   └── ld-lsb-x86-64.so.3   → ../lib/ld-linux-x86-64.so.2
├── tools/                  → ../cross-tools (the toolchain — above)
├── usr/
│   ├── include/            (kernel UAPI headers from step linux-headers,
│   │                        plus glibc's own headers: aio.h, alloca.h, …)
│   └── lib/                (the target glibc:
│                            libc.so.6, libm.so.6, libpthread.so.0,
│                            crt1.o, crti.o, crtn.o,
│                            ld-linux-x86-64.so.2, … ~11.5 MB of glibc)
└── var/
```

## Mental model

| Directory tree                          | Side       | Lifecycle                                | What it produces                                              |
| --------------------------------------- | ---------- | ---------------------------------------- | ------------------------------------------------------------- |
| `build/cross-tools/` (aka `$LFS/tools/`) | **Host**   | Frozen after Phase 0 step 5 (libstdc++)  | The toolchain itself — runs on workstation, emits target binaries |
| `build/sysroot/` minus `tools/`         | **Target** | Grows continuously through Phase 2+      | The actual T450 rootfs being assembled                        |

The cross-compiler's `--sysroot=$LFS` argument is how the host-side compiler knows to look in `$LFS/usr/include/` for headers and `$LFS/usr/lib/` for libraries, instead of `/usr/include/` and `/usr/lib/` on the workstation. That single flag is the gate that keeps host bleed out.

## After `libstdcxx` step finishes

The last step of `02-cross-toolchain.sh` adds:

```
$LFS/usr/lib/
├── libstdc++.so.6        ← C++ runtime
├── libstdc++.so.6.0.33
└── libstdc++.so
```

…to the **sysroot side**, not into `tools/`. After that, `tools/` is fully complete and will not grow further from `02-cross-toolchain.sh`. Subsequent Phase 0 work (`03-sysroot-temp-tools.sh`) writes only into `$LFS/usr/`.
