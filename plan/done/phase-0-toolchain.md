# Phase 0 — Workstation cross-compile environment

**Goal.** A reproducible cross-toolchain on this workstation that targets `x86_64-lfs-linux-gnu` (T450). Bricked T450 must never block workstation work.

**Status (as of 2026-05-17).** Build harness staged at `../build/`. Scripts are syntax-clean and `check-host.sh` runs. Outstanding: install ~9 missing apt packages, populate GPG keys, run `fetch.sh` + `cross-toolchain.sh`, and iterate `sysroot-temp-tools.sh` (currently skeleton) package by package.

## Language choice — Bash, not Rust

Phase 0 is orchestration of upstream `configure && make && make install` recipes. The pedagogical content is **the LFS sequence itself** — the scripts should read one-for-one against LFS chapter 5 of `../.agents/reference/lfs/`. Rust would obscure rather than illuminate. Rust takes over from Phase 3 (PID 1) where ownership and type safety pay off. See [`../build/README.md`](../../build/README.md) for the longer rationale.

## Subtasks

1. **Pick a workspace layout in the repo.** ☑ Done — see `../build/`:
   - `build/cross-tools/` — `$LFS/tools` equivalent (binutils-pass-1 + GCC-pass-1 + glibc-headers + libstdc++), exposed at `$LFS/tools` via symlink
   - `build/sysroot/` — `$LFS` root that will become the T450 rootfs
   - `build/sources/` — verified upstream tarballs (gitignored)
   - `build/artifacts/` — `bzImage`, initramfs, ESP image, ISO (gitignored)
   - `build/logs/` — per-step build logs + `.done-*` sentinels (gitignored)

2. **Define exact upstream versions** in `build/versions.env`. ☑ Done. Pinned:
   - Linux **6.12.10** (LTS)
   - Binutils 2.43.1, GCC 14.2.0, Glibc 2.40
   - BusyBox 1.37.0 (transitional, Phase 2 only)
   - 16 chapter-6 temp-tool packages (M4, ncurses, bash, coreutils, …)
   - Pin every version with SHA-256 + GPG signature checks.

3. **Write `build/00-check-host.sh`** ☑ Done — verifies LFS chapter 2 minimums (Bash, Binutils, Bison, Coreutils, …) plus WriteOnce additions (Rust, QEMU, ISO tooling, kernel build deps). Prints the apt install command for missing items.

4. **Write `build/01-fetch.sh`** ☑ Done — downloads tarballs to `build/sources/`, verifies GPG signatures via `build/keys/` (project-local keyring, no global pollution), enforces SHA-256 hashes from `build/checksums.txt`. First-run flow: hashes recorded to `*.next-lock`, user spot-checks against upstream announcements, then merges into `checksums.txt`.

5. **Write `build/02-cross-toolchain.sh`** ☑ Done — implements LFS chapter 5 verbatim: binutils-pass-1 → GCC-pass-1 → linux-headers → glibc → libstdc++. Per-step idempotency via `logs/.done-<step>` sentinels. Each step's full configure/make/install log lands under `build/logs/`. Includes a cross-glibc smoke test that verifies the dynamic linker path.

6. **Run the toolchain build end-to-end** ☐ Pending — user action. Total wall-clock ~30 min on a recent workstation. Verify with a static hello-world: `build/cross-tools/bin/x86_64-lfs-linux-gnu-gcc -static hello.c -o hello && file hello` (expect `statically linked, x86-64`).

7. **Write `build/03-sysroot-temp-tools.sh`** ☐ Skeleton only — LFS chapter 6 (~16 temp packages built against the cross-toolchain, installed into `$LFS/usr`). The script has a generic `build_pkg` helper and per-package commented stubs; uncomment each as you read its LFS-chapter-6 section. Two non-trivial cases (ncurses, file) need host-side prebuild steps not yet captured.

8. **Decide on a containerised vs bare-host build.** ☑ Done — bare host. Documented in `../build/README.md` and this file.

9. **Document the bootstrap** ☑ Done — see `../build/README.md` for the end-to-end procedure, idempotency model, failure recovery, and version-bump workflow.

## What you (the operator) need to do now

```bash
# 1. Install the missing host deps (apt prompts for sudo password).
sudo apt-get install -y \
  bison m4 flex libelf-dev \
  qemu-system-x86 ovmf xorriso mtools dosfstools

# 2. Import GPG keys for upstream verification (one-time).
#    See build/keys/README.md for the recv-keys + export commands.

# 3. Run the harness (scripts are numbered NN- in run order).
cd build/
./00-check-host.sh         # expect all OK
./01-fetch.sh              # first run records hashes; review *.next-lock
cat sources/*.next-lock >> checksums.txt && rm sources/*.next-lock
./01-fetch.sh              # second run verifies all hashes
./02-cross-toolchain.sh    # ~30 min wall-clock; per-step logs in logs/
```

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
