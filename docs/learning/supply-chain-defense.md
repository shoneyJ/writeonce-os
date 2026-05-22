# Supply-chain defense — what WriteOnce protects against, and what it doesn't

> Reference doc inspired by CVE-2024-3094 (the XZ backdoor). Audits
> WriteOnce's source-build and trust-anchor mechanisms against the
> known attack vectors and names the remaining gaps with concrete
> follow-up work.

## The threat model — what XZ actually did

| Stage | Mechanism (CVE-2024-3094) |
| --- | --- |
| 1. Maintainer takeover | "Jia Tan" spent ~2 years building trust, pressured an overworked solo maintainer (Lasse Collin) into granting co-maintainer status |
| 2. Code injection | Malicious code lived in **test-fixture binary blobs** (`tests/files/bad-3-corrupt_lzma2.xz`) that the build extracted at compile time |
| 3. Build-script tampering | An `m4/build-to-host.m4` macro **only present in the tarball, not in the git tree** unpacked the blob and patched the build |
| 4. Linkage path | sshd (Debian-patched) → `libsystemd` → `dlopen(liblzma)` → backdoor function `RSA_public_decrypt` IFunc-resolved at runtime |
| 5. Triggering | Inactive in dev/test environments — only fired when sshd was the parent and a specific RSA public key was supplied |
| 6. Detection | Microsoft engineer Andres Freund noticed sshd was ~500 ms slower than usual during PostgreSQL benchmarking; pulled the thread, found the backdoor |

The attack worked because nothing along the chain (`tarball → autoconf → libsystemd → liblzma`) raised an alarm. Defense is layered, not single-point.

## What WriteOnce already defends against

| Vector | Mitigation in WriteOnce |
| --- | --- |
| Tarball substitution (mirror compromise) | **SHA-256 lockfile** at `build/checksums.txt`, committed to git. Any byte change → mismatch → fetch refuses to proceed. |
| Tarball signed by attacker's key | **GPG signatures** verified against keys in `build/keys/*.asc`. As of this round, every Phase 0 + most of Phase 8 tarballs have signatures checked. |
| Network compromise during build | **Container-isolated builds** (`wo-builder` image); the `--no-network` flag (added this round) cuts external network for compile steps. |
| Host environment pollution | The build host never installs `<foo>-dev` packages — the image carries them. The host can't smuggle a tampered `libpam.so` into a clean build. |
| dlopen-chain abuse | **No `liblzma` in the auth path.** `writeonce-login` links libpam directly; doesn't load libsystemd; doesn't dlopen anything beyond what PAM itself does. |
| Backdoor in PID 1 / supervisor binary | **Static-musl** for PID 1, supervisor, initramfs, wo-ctl — no dynamic linkage surface. |
| Trust anchor erosion | `build/keys/*.asc` and `build/checksums.txt` are **committed to git**. Future bumps of either show up in PR diffs as auditable changes. |

## What WriteOnce still does NOT defend against

Honestly. Each is a real gap; the table includes the concrete remediation.

| Gap | Concrete fix |
| --- | --- |
| **Tarball ≠ git tag.** The exact XZ vector (an m4 macro present in the released tarball but not in the git repo) bypasses every check above. | Add a `build/audit-tarball-vs-git.sh` per package: clone the git tag, run `make dist` (or `meson dist`), diff the result against the upstream tarball. Auto-generated files differ; hand-edited files must match. **Pending.** |
| **No reproducible-build verification.** A subverted compiler could inject code into a build that passes all source-level checks. | A `build/verify-reproducible.sh` that runs the same build twice into two parameterised sysroots and diffs the resulting `*.so` SHA-256s. Requires parameterising `$LFS` per build. **Pending — bigger than one round.** |
| **Build-time network access.** The `wo-builder` image lets the compile step phone home if a malicious build script wanted to. | `./build/in-container.sh --no-network <cmd>` (landed this round). The user's discipline: always use `--no-network` for compile steps; only `01-fetch.sh` keeps the network. |
| **No supply-chain attestation.** SLSA / sigstore / in-toto attestations are ignored. | Future Phase 10 work. Pull attestations from GitHub releases / SLSA build-info where available; verify them in `01-fetch.sh`. |
| **Rust dependency surface unchecked.** `libc`, `serde`, `toml`, `uefi`, `log` are all top-100 crates — but we don't run `cargo vet` or `cargo audit`. | Add `./build/in-container.sh cargo audit` to a `make ci` target. **Pending.** |
| **No service-runtime sandboxing.** When sshd / dockerd / wireplumber get added, a compromised binary has full kernel reach. | Phase 4 supervisor's `clone3(CLONE_INTO_CGROUP)` already places services in cgroups. Round 2d should add seccomp BPF profiles per service. **Pending — Round 2d.** |
| **No build-artifact signing.** Anyone could publish a binary claiming to be a WriteOnce kernel. | Phase 10: sign release artifacts with `cosign` against a sigstore identity. Verifiers compare commit hash on releases. **Pending.** |
| **GPG verification still incomplete.** A handful of Phase 8 packages (zlib, brotli, libffi, util-macros) don't ship signatures upstream. | Accept SHA-256-only for those (the hash in checksums.txt is the load-bearing anchor). For libffi specifically, prefer GitHub-release tarballs over autogenerated mirror copies. |

## What landed this round (concrete defenses)

### 1. Phase 8 packages now GPG-checked where signatures exist

`build/01-fetch.sh`'s `GPG_SIGNED=(...)` list now includes `expat`, `libpng`, `libjpeg-turbo`, `freetype`, `fontconfig`, `libxml2`, `dbus`, and `Linux` (matching `Linux-PAM-X.tar.xz`).

The fetch loop also now tries both `.sig` and `.asc` URL extensions before giving up — many freedesktop / GNOME upstreams use `.asc` (a-string-cipher armor) rather than `.sig`. Graceful fallback when neither exists.

### 2. `./build/in-container.sh --no-network <cmd>`

The first argument can be `--no-network`, which adds `--network=none` to `docker run`. Use it for every compile step:

```bash
# Fetch with network (downloads tarballs + signatures):
./build/in-container.sh ./build/01-fetch.sh

# Compile WITHOUT network — a tampered build script can't reach out:
./build/in-container.sh --no-network ./build/08-base-substrate.sh
./build/in-container.sh --no-network cargo build -p writeonce-login --release
```

If a build legitimately fails under `--no-network`, that's a flag: which network resource was it reaching for, and why?

### 3. This document

The audit + remediation catalogue lives here for future-you and any reviewers. Update the "still does NOT defend against" table when the corresponding fix lands.

## Operational procedure when adding a new package

Append this to the "how to add a package" checklist in
[`phase-8-userspace-build-strategy.md`](phase-8-userspace-build-strategy.md):

1. **Eyeball the upstream's release announcement page.** What hash does the maintainer publish on their own website? Cross-check against what `01-fetch.sh` recorded into the `*.next-lock` file. They must match.
2. **Check if the upstream ships a GPG signature.** Most do. Add the package's filename prefix to `GPG_SIGNED=(...)` in `01-fetch.sh`. Run `./import-keys.sh` to pick up the new signer key.
3. **Spot-check the tarball against the git tag.** For high-value packages (PAM, OpenSSH, sudo, OpenSSL once we add it): `git clone --branch v$VERSION` the upstream, then `diff -r` the extracted tarball against the git checkout. Auto-generated files (`Makefile.in`, `configure`, `aclocal.m4`) will differ; everything else must not. **This is the XZ-style detector.**
4. **Use `--no-network`** for the compile step.
5. **Don't enable unnecessary features.** Each `./configure --enable-X` is a new attack surface. WriteOnce's developer-workstation framing says no plug-ins, no NSS modules, no PAM modules beyond what login uses. Be conservative.

## Why we still expect to be vulnerable

Complete defense against a multi-year, well-resourced supply-chain attack is essentially impossible for a single-person project. Even Debian's CI didn't catch XZ. WriteOnce's structural defenses (small bespoke surface, pinned versions, static-musl supervisor, no liblzma in the auth path) make us **less attractive** as a target — but a determined attacker who compromised an upstream like `dbus` or `linux-pam` and slipped in a malicious release could still bite us.

What we CAN do is make the cost of a successful attack high enough that the attacker picks an easier target, and detect the attack reasonably fast when it happens. The combination of:

- Pinned versions + committed SHA-256 + GPG verification
- Static-musl boot path (no dlopen surface in PID 1 / supervisor / login)
- Container-isolated, network-cut compile steps
- Multi-component diff at version bumps (the `checksums.txt` + `keys/*.asc` show in PR review)
- Eventual sigstore attestation + reproducible-build verification (future)

…gets us to "the attacker has to compromise a major upstream maintainer AND get the bad commit past code review AND have it survive a tarball-vs-git audit AND have it not break reproducible build verification AND have the user not eyeball the SHA at version bump time." That's not zero attack surface, but it's significantly more than nothing.
