# Phase 0 — Workstation cross-compile environment

**Goal.** A reproducible cross-toolchain on this workstation that targets `x86_64-lfs-linux-gnu` (T450). Bricked T450 must never block workstation work.

## Subtasks

1. **Pick a workspace layout in the repo.**
   - `build/host/` — host-only scripts, never installed to target
   - `build/cross-tools/` — `$LFS/tools` equivalent (binutils-pass-1 + GCC-pass-1 + glibc-headers + libstdc++)
   - `build/sysroot/` — `$LFS` root that will become the T450 rootfs
   - `build/sources/` — verified upstream tarballs (kernel, glibc, binutils, gcc, busybox, etc.)
   - `build/artifacts/` — `bzImage`, initramfs, ESP image, ISO

2. **Define exact upstream versions** in `build/versions.env`. Initial choices:
   - Linux **6.12 LTS** (kernel.org)
   - binutils 2.43, gcc 14.x, glibc 2.40, busybox 1.37 (or current LFS stable)
   - Pin every version with SHA-256 + GPG signature checks.

3. **Write `build/fetch.sh`** — downloads tarballs to `build/sources/`, verifies SHA-256 from `build/checksums.txt`, verifies GPG signatures from `build/keys/`. Idempotent.

4. **Write `build/cross-toolchain.sh`** — runs LFS chapter 5 sequence (binutils-pass-1 → GCC-pass-1 → linux-headers → glibc → libstdc++). Logs each step. Stops on failure with a clear error.

5. **Run the toolchain build end-to-end** on the workstation. Verify with a hello-world: `$LFS/tools/bin/x86_64-lfs-linux-gnu-gcc -static hello.c -o hello`, then `file hello` (must report `statically linked, x86-64`).

6. **Write `build/sysroot-temp-tools.sh`** — LFS chapter 6 (~20 temp packages built against the cross-toolchain, installed into `$LFS/usr`). Output: chroot-capable sysroot.

7. **Decide on a containerised vs bare-host build.** Recommendation: bare host (the user's `~/projects/linux/` setup suggests they already build kernels); add a `Containerfile` only if reproducibility issues surface.

8. **Document the bootstrap** in this file — exact commands, expected runtime per step (i5-5300U would take hours; the workstation should be 10–30 min).

## Deliverable

`$LFS/tools` with a working cross-gcc; `$LFS/usr` populated with temporary tools; scripts that re-run cleanly from zero.

## Acceptance criteria

- `make -C linux O=build/kernel ARCH=x86_64 CROSS_COMPILE=$LFS/tools/bin/x86_64-lfs-linux-gnu- defconfig` succeeds.
- `./build/fetch.sh && ./build/cross-toolchain.sh && ./build/sysroot-temp-tools.sh` succeeds on a clean workstation in one shot.

## References

- `../.agents/reference/linux/Documentation/kbuild/` — kernel cross-compile flags.
- LFS book chapters 5–6 (use upstream version matching `versions.env`).

## Risks

- Host toolchain version drift breaks reproducibility. Mitigation: record host gcc/binutils versions in build logs; pin LFS-recommended host minimums.
- Tarball checksums change if upstream re-rolls. Mitigation: cache verified tarballs in `build/sources/`.
