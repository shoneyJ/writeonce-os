# Phase 8 — userspace build strategy

> Companion to [`../../build/blfs-pkg.sh`](../../build/blfs-pkg.sh) and
> the per-stack `build/0N-*.sh` scripts.
> Captures the choices that turn "build ~80 upstream packages into the
> sysroot" from a vague task into a per-round playbook.

## The split: source-built substrate, Nix apps on top

The developer-workstation plan ([`plan/developer-workstation-implementation.md`](../../plan/developer-workstation-implementation.md))
draws the line:

| Layer                                       | Source                                                       |
| ------------------------------------------- | ------------------------------------------------------------ |
| Linux kernel, glibc, base utils             | Built in Phase 0/2 from kernel.org / gnu.org                 |
| **X.Org, GTK4, D-Bus, PAM, PipeWire, iwd**  | **Built from upstream source via `build/0N-*.sh` (Phase 8)** |
| Docker, Alacritty, Zen browser, IDEs        | Installed via Nix from nixpkgs (Phase 10)                    |

The reason for source-building the substrate is reproducibility +
ownership: the OS knows exactly what version of D-Bus it ships, with
which `./configure` flags, against which sysroot. The reason for
delegating user applications to Nix is "no reinvention" — we have
nothing to add to the Docker, Alacritty, or Zen browser builds; let
nixpkgs do the curation we'd otherwise have to maintain.

## The helper: `build/blfs-pkg.sh`

Sourceable. Two main entry points:

```bash
build_pkg   <name> <archive> [extra configure flags…]   # autoconf
build_meson <name> <archive> [extra meson options…]    # meson + ninja
```

Both:
- Skip when `logs/.done-blfs-<name>` exists. Delete the sentinel to redo.
- Extract into `work/<name>/` (wiped first — same idempotency rule as the LFS scripts).
- Stream stdout/stderr to `logs/blfs-<name>-{configure,make,install}.log`.
- Cross-compile against the Phase 0 toolchain (`--host=$LFS_TGT`).
- `DESTDIR=$LFS make install` so the package lands in the sysroot.

`PKG_CONFIG_PATH` + `PKG_CONFIG_SYSROOT_DIR` are set so packages find
each other through `pkg-config` even though they live in the sysroot.

The `build_meson` variant writes a small `cross-lfs.ini` meson
cross-file before running `meson setup`. The cross-file names the
toolchain binaries and the sysroot path; meson handles everything else.

## Package ordering

Each `0N-*.sh` script lists its packages in dependency order. The user
runs the whole script; the script invokes the steps in order. Sentinel
files prevent redoing completed work, so an interruption in the middle
just resumes where it left off.

The Phase 8 substrate splits across multiple rounds:

| Script                     | Round | Packages                                                          |
| -------------------------- | ----- | ----------------------------------------------------------------- |
| `08-base-substrate.sh`     | 8a    | zlib, brotli, expat, libffi, libxml2, util-macros, libpng, libjpeg-turbo, freetype, fontconfig, linux-pam, dbus |
| `09-x11-stack.sh`          | 8b    | xorgproto, xcb-proto, libXau, xtrans, libxcb, libX11, libX*, libxkbcommon, xcb-util-* |
| `10-xorg-server.sh`        | 8c    | xorg-server, xf86-input-libinput, xkeyboard-config                |
| `11-gtk-stack.sh`          | 8d    | glib, gobject-introspection, cairo, pango, harfbuzz, gdk-pixbuf, graphene, gtk4 |
| `12-audio-stack.sh`        | 8e    | alsa-lib, pipewire, wireplumber                                   |
| `13-network-stack.sh`      | 8f    | iproute2, iputils, iwd                                            |
| `14-i3-and-i3more.sh`      | 9     | libev, yajl, pcre2, i3, i3More (from your repo)                   |

Total: 7 scripts, ~50 source-built packages. Round 8a (this round) is
the foundation everything else needs.

## Trust model for new packages

Each new tarball goes through `01-fetch.sh`. For Round 8 packages we
**rely on SHA-256 only** — many of the freedesktop / sourceforge
upstreams don't publish detached GPG signatures consistently. The
trade:

- **Initial trust:** when populating `checksums.txt` for a new package,
  the human eyeballs the recorded SHA-256 against the upstream
  announcement (release note / GitHub release / mailing-list post).
- **Forever after:** the committed checksum in `checksums.txt` is the
  load-bearing anchor. Any future download must match it bit-for-bit.

For packages that DO ship GPG signatures (linux-pam, dbus, freetype),
Round 8b will optionally extend the `GPG_SIGNED=(…)` list in
`01-fetch.sh`. For now, SHA-256 covers the security need at the cost of
one extra eyeball-check per package addition.

## meson vs autoconf

Most BLFS packages still use autoconf — `./configure --prefix=/usr
--host=$LFS_TGT && make && make install`. The newer freedesktop /
Wayland ecosystem (libxkbcommon, GTK4, PipeWire) uses meson.

The `build_meson` helper writes the cross-file at
`work/<name>/cross-lfs.ini` and runs `meson setup --cross-file=...`.
Adding a meson-based package is identical to adding an autoconf-based
one — just call `build_meson` instead of `build_pkg`.

## Three packages that don't fit the helpers

A small minority of packages use neither autoconf nor meson cleanly:

- **zlib** ships a hand-written shell `configure` that doesn't accept
  `--host=`. We cross-compile via `CC=$LFS/tools/bin/$LFS_TGT-gcc` env
  override (see `step_zlib` in `08-base-substrate.sh`).
- **libjpeg-turbo** is CMake-only. We invoke `cmake` directly with the
  cross-compiler set explicitly (`step_libjpeg-turbo`). Round 8a's
  Containerfile may need `cmake` added.
- **brotli** has both autotools and CMake; the autotools is increasingly
  stale. Round 8a uses autotools (`build_pkg`) for simplicity; switch
  to CMake if it breaks on libstd-c++ linking.

These are documented inline in the scripts as exceptions.

## Adding a new package: the standard procedure

```bash
# 1. Pick a version + tarball URL. Update versions.env:
echo 'FOO_VERSION=1.2.3' >> build/versions.env

# 2. Add a fetch URL to 01-fetch.sh's URLS table:
[foo-${FOO_VERSION}.tar.xz]="https://example.org/foo-${FOO_VERSION}.tar.xz"

# 3. Run fetch — it records the SHA-256 in *.next-lock:
./build/01-fetch.sh

# 4. Eyeball the hash against the upstream's announcement page; copy
#    into checksums.txt:
cat build/sources/foo-1.2.3.tar.xz.next-lock >> build/checksums.txt
rm  build/sources/foo-*.next-lock

# 5. Add a step function to the relevant 0N-*.sh script:
#       step_foo() { build_pkg foo "foo-${FOO_VERSION}.tar.xz" --some-flag; }
#    and append `foo` to the STEPS=(…) list.

# 6. Run the script. Sentinel files prevent re-doing earlier packages.
./build/in-container.sh ./build/08-base-substrate.sh   # if inside the container
# or, if all build-deps are on host:
./build/08-base-substrate.sh
```

## Why not just install everything via Nix?

Tempting, and the developer-workstation plan partially adopts that for
user-facing apps. The reasons we still source-build the substrate:

1. **Boot path independence.** PID 1, the supervisor, and the login
   prompt all run *before* Nix is loaded. Their substrate (libpam,
   libdbus, libsystemd-equivalent) has to be already present on the
   root filesystem.
2. **Footprint.** A full Nix profile pulling Xorg + GTK4 +
   `glibcLocales` and friends easily reaches 4–6 GB. A curated source
   build is closer to 800 MB.
3. **Predictability.** Each source build is one tarball + a known
   `./configure` invocation. Nix's evaluation model adds layers of
   indirection that don't matter for the small substrate but obscure
   the cross-compile.
4. **Pedagogy.** Reading the build/0N-*.sh scripts top-to-bottom is
   exactly the BLFS recipe in shell form. Nix hides that.

For everything *above* the substrate — IDEs, browsers, language
toolchains, containers — Nix is the right answer. The two approaches
coexist; the cut-line is "what does the supervisor need to bring up
a graphical session?"

## Time budget

Rough estimate from the wo-builder container, single-thread make,
modern workstation CPU:

| Round | Wall-clock | Notes                                                |
| ----- | ---------- | ---------------------------------------------------- |
| 8a (this round) | 20–40 min | Most packages are small; freetype + libxml2 are the long pole |
| 8b      | 30–50 min  | Many small X11 libs; xkbcommon's meson takes 5 min on its own |
| 8c      | 1–2 hr     | xorg-server is the heaviest single package           |
| 8d      | 1–2 hr     | gtk4 + harfbuzz                                      |
| 8e      | 30 min     | pipewire is meson; wireplumber is small              |
| 8f      | 10–20 min  | iproute2 + iwd                                       |
| 9       | 30 min     | i3 + i3More (Rust)                                   |

Total for the full substrate: **~5–8 hours of compile time** if every
package builds clean first try. Each round adds 1–3 sleep-while-cargo-builds
windows the user can spread across days.
