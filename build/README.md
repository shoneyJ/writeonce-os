# Phase 0 — Cross-toolchain build

This directory implements [Phase 0](../plan/phase-0-toolchain.md) of the
WriteOnce OS roadmap: a reproducible cross-toolchain that targets the T450
(`x86_64-lfs-linux-gnu`), built on this workstation from upstream sources.

## Layout

```
versions.env             Pinned upstream versions (single source of truth)
checksums.txt            SHA-256 lockfile (human-reviewed)
keys/                    GPG public keys for upstream signature verification
setup-env.sh             Sourced helper; exports $LFS, $LFS_TGT, $PATH
00-check-host.sh         Verify workstation prerequisites (LFS Ch.2 + WriteOnce)
01-fetch.sh              Download + GPG-verify + SHA-256-verify all sources
02-cross-toolchain.sh    LFS Ch.5: binutils-1 → gcc-1 → headers → glibc → libstdc++
03-sysroot-temp-tools.sh LFS Ch.6: temporary tools (skeleton; iterate package-by-package)
clean.sh                 Wipe in-progress state

(generated at runtime, all gitignored:)
sources/                 Downloaded tarballs + .sig files
cross-tools/             $LFS/tools — host-resident cross-toolchain
sysroot/                 $LFS — the target rootfs being built
                         (sysroot/tools is a symlink to ../cross-tools)
work/                    Per-package extracted source trees + build dirs
artifacts/               bzImage, initramfs, ESP image, ISO (Phase 2+)
logs/                    Per-step build logs and .done-* sentinels
gnupg/                   Project-local GPG keyring built from keys/
```

## Script naming

Executable steps carry a `NN-` numeric prefix indicating their run order
(`00-` before `01-` before `02-`). `setup-env.sh` and `clean.sh` are
unnumbered: the first is a sourced library, the second is an out-of-band
utility — neither is part of the sequence. See the
[`feedback-numbered-scripts`](../../../.claude/projects/-home-shoney-projects-github-shoneyj-writeonce-os/memory/feedback_numbered_scripts.md)
memory entry for the convention.

## End-to-end Phase 0 procedure

```bash
cd build/

# 1. Verify the workstation has the LFS host minimums + WriteOnce additions.
#    Prints a checklist; exit code 0 means proceed.
./00-check-host.sh

# 2. Import GPG keys for upstream verification (one-time).
#    See keys/README.md for the recv-keys + export commands.
ls keys/

# 3. Download all upstream tarballs and signatures.
#    First run will report missing SHA-256 entries — populate checksums.txt
#    from the .next-lock files, then rerun.
./01-fetch.sh
# (review sources/*.next-lock against upstream announcements, then:)
cat sources/*.next-lock >> checksums.txt && rm sources/*.next-lock
./01-fetch.sh   # second run: all hashes verified

# 4. Build the cross-toolchain.
#    Total wall-clock ~30 min on a recent workstation; per-step logs land in
#    logs/. Each step writes a .done-<step> sentinel and is skipped on rerun.
./02-cross-toolchain.sh

# 5. Verify the cross-toolchain.
cat > /tmp/hello.c <<'C'
#include <stdio.h>
int main(void){ puts("hello, T450"); return 0; }
C
./cross-tools/bin/x86_64-lfs-linux-gnu-gcc -static /tmp/hello.c -o /tmp/hello
file /tmp/hello   # expect: statically linked, x86-64

# 6. Build the temporary tools for the sysroot.
#    Currently a skeleton — open the file, enable one package at a time
#    against its LFS chapter 6 section in
#    ../.agents/reference/lfs-rendered.txt.
./03-sysroot-temp-tools.sh
```

## Idempotency

- `01-fetch.sh` skips any tarball whose SHA-256 already matches `checksums.txt`.
- `02-cross-toolchain.sh` skips any step whose `logs/.done-<step>` sentinel
  exists. Delete the sentinel to force a redo.
- `03-sysroot-temp-tools.sh` skips any package whose `logs/.done-temp-<pkg>`
  sentinel exists.

## Failure recovery

If a step fails partway, the corresponding `logs/.done-*` sentinel is *not*
written, so a rerun retries that step from scratch. The simplest recovery is:

```bash
# Re-run the failed step in isolation:
./02-cross-toolchain.sh gcc-1

# Or start fresh:
./clean.sh && ./01-fetch.sh && ./02-cross-toolchain.sh
```

To wipe and redownload everything (e.g. after a version bump):

```bash
./clean.sh --all
```

## Pinned versions

See [`versions.env`](versions.env). The current pins target the LFS 12.2-era
stable releases:

- Linux 6.12.10 (LTS)
- Binutils 2.43.1
- GCC 14.2.0
- Glibc 2.40

To bump: edit `versions.env`, blank the corresponding line in
`checksums.txt`, run `./fetch.sh` (records the new hash), spot-check
against the upstream announcement, copy from `*.next-lock` into
`checksums.txt`, then rebuild.

## Why this is Bash and not Rust

See the language-selection note in `../plan/phase-0-toolchain.md`. The short
version: Phase 0 is a one-shot orchestrator of upstream `configure && make
&& make install` invocations whose pedagogical content is the LFS sequence
itself; reading the scripts and the rendered LFS book side-by-side should
feel one-to-one. Rust takes over from Phase 3 (PID 1) where ownership and
type safety pay off.
