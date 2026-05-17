#!/usr/bin/env bash
# build/fetch.sh — download, GPG-verify, and SHA-256-verify all upstream sources.
#
# Idempotent: if a file is already present and its SHA-256 matches checksums.txt,
# it is skipped. If checksums.txt lacks an entry, the file is downloaded and
# its hash is written to sources/<basename>.next-lock; the script then refuses
# to proceed until you copy the hash into checksums.txt after spot-checking it.

set -euo pipefail

cd "$( dirname "${BASH_SOURCE[0]}" )"
# shellcheck disable=SC1091
source ./setup-env.sh

CHECKSUMS="$BUILD_ROOT/checksums.txt"
KEYRING="$BUILD_ROOT/gnupg"

# ---- per-package URL table --------------------------------------------------
# Each entry: <tarball-basename> <fetch-url>
#
# Signatures are inferred by appending ".sig" or ".sign" to the URL where the
# upstream provides one; absence is fine.

declare -A URLS=(
    [binutils-${BINUTILS_VERSION}.tar.xz]="https://ftpmirror.gnu.org/binutils/binutils-${BINUTILS_VERSION}.tar.xz"
    [gcc-${GCC_VERSION}.tar.xz]="https://ftpmirror.gnu.org/gcc/gcc-${GCC_VERSION}/gcc-${GCC_VERSION}.tar.xz"
    [glibc-${GLIBC_VERSION}.tar.xz]="https://ftpmirror.gnu.org/glibc/glibc-${GLIBC_VERSION}.tar.xz"
    [mpfr-${MPFR_VERSION}.tar.xz]="https://ftpmirror.gnu.org/mpfr/mpfr-${MPFR_VERSION}.tar.xz"
    [gmp-${GMP_VERSION}.tar.xz]="https://ftpmirror.gnu.org/gmp/gmp-${GMP_VERSION}.tar.xz"
    [mpc-${MPC_VERSION}.tar.gz]="https://ftpmirror.gnu.org/mpc/mpc-${MPC_VERSION}.tar.gz"
    [isl-${ISL_VERSION}.tar.xz]="https://libisl.sourceforge.io/isl-${ISL_VERSION}.tar.xz"

    [linux-${LINUX_VERSION}.tar.xz]="https://cdn.kernel.org/pub/linux/kernel/v${LINUX_MAJOR}/linux-${LINUX_VERSION}.tar.xz"

    [m4-${M4_VERSION}.tar.xz]="https://ftpmirror.gnu.org/m4/m4-${M4_VERSION}.tar.xz"
    [ncurses-${NCURSES_VERSION}.tar.gz]="https://invisible-mirror.net/archives/ncurses/ncurses-${NCURSES_VERSION}.tar.gz"
    [bash-${BASH_VERSION}.tar.gz]="https://ftpmirror.gnu.org/bash/bash-${BASH_VERSION}.tar.gz"
    [coreutils-${COREUTILS_VERSION}.tar.xz]="https://ftpmirror.gnu.org/coreutils/coreutils-${COREUTILS_VERSION}.tar.xz"
    [diffutils-${DIFFUTILS_VERSION}.tar.xz]="https://ftpmirror.gnu.org/diffutils/diffutils-${DIFFUTILS_VERSION}.tar.xz"
    [file-${FILE_VERSION}.tar.gz]="https://astron.com/pub/file/file-${FILE_VERSION}.tar.gz"
    [findutils-${FINDUTILS_VERSION}.tar.xz]="https://ftpmirror.gnu.org/findutils/findutils-${FINDUTILS_VERSION}.tar.xz"
    [gawk-${GAWK_VERSION}.tar.xz]="https://ftpmirror.gnu.org/gawk/gawk-${GAWK_VERSION}.tar.xz"
    [grep-${GREP_VERSION}.tar.xz]="https://ftpmirror.gnu.org/grep/grep-${GREP_VERSION}.tar.xz"
    [gzip-${GZIP_VERSION}.tar.xz]="https://ftpmirror.gnu.org/gzip/gzip-${GZIP_VERSION}.tar.xz"
    [make-${MAKE_VERSION}.tar.gz]="https://ftpmirror.gnu.org/make/make-${MAKE_VERSION}.tar.gz"
    [patch-${PATCH_VERSION}.tar.xz]="https://ftpmirror.gnu.org/patch/patch-${PATCH_VERSION}.tar.xz"
    [sed-${SED_VERSION}.tar.xz]="https://ftpmirror.gnu.org/sed/sed-${SED_VERSION}.tar.xz"
    [tar-${TAR_VERSION}.tar.xz]="https://ftpmirror.gnu.org/tar/tar-${TAR_VERSION}.tar.xz"
    [xz-${XZ_VERSION}.tar.xz]="https://github.com/tukaani-project/xz/releases/download/v${XZ_VERSION}/xz-${XZ_VERSION}.tar.xz"

    [busybox-${BUSYBOX_VERSION}.tar.bz2]="https://busybox.net/downloads/busybox-${BUSYBOX_VERSION}.tar.bz2"
)

# Packages whose upstreams publish a detached GPG signature alongside the tarball.
# (For the others, only the SHA-256 is verified; the hash is the bridge to
#  trust, established once by a human against the upstream announcement.)
GPG_SIGNED=(
    binutils gcc glibc mpfr gmp mpc
    linux
    m4 bash coreutils diffutils findutils gawk grep gzip make sed tar xz
)

# ---- import keys from build/keys/ into project-local keyring ----------------
mkdir -p "$KEYRING"; chmod 700 "$KEYRING"
if compgen -G "$BUILD_ROOT/keys/*.asc" >/dev/null; then
    for k in "$BUILD_ROOT/keys/"*.asc; do
        GNUPGHOME="$KEYRING" gpg --quiet --import "$k" 2>/dev/null || true
    done
fi

# ---- per-file flow ----------------------------------------------------------
need_review=0
gpg_failures=()
sha_failures=()

for base in "${!URLS[@]}"; do
    url="${URLS[$base]}"
    out="$SOURCES/$base"

    # 1. download (if missing)
    if [[ ! -f "$out" ]]; then
        echo ">>> fetching $base"
        wget --quiet --show-progress -O "$out.part" "$url"
        mv "$out.part" "$out"
    fi

    # 2. optional GPG signature
    pkg="${base%%-*}"
    if [[ " ${GPG_SIGNED[*]} " == *" $pkg "* ]]; then
        sig_url=""
        case "$pkg" in
            linux)              sig_url="${url%.xz}.sign" ;;
            busybox)            sig_url="$url.sig" ;;
            *)                  sig_url="$url.sig" ;;
        esac
        sig="$SOURCES/$base.sig"
        if [[ ! -f "$sig" ]]; then
            wget --quiet -O "$sig.part" "$sig_url" 2>/dev/null \
                && mv "$sig.part" "$sig" \
                || { echo "    (no signature published for $pkg, skipping GPG)"; sig=""; }
        fi
        if [[ -n "$sig" && -f "$sig" ]]; then
            # The kernel's .sign is over the *decompressed* tarball; everyone
            # else signs the compressed file. Handle both.
            verified=0
            if [[ "$pkg" == "linux" ]]; then
                xz -dc "$out" | GNUPGHOME="$KEYRING" gpg --verify "$sig" - 2>/dev/null \
                    && verified=1
            else
                GNUPGHOME="$KEYRING" gpg --verify "$sig" "$out" 2>/dev/null && verified=1
            fi
            if [[ $verified -eq 1 ]]; then
                echo "    gpg: OK"
            else
                echo "    gpg: FAILED — refusing to trust $base (key probably missing)"
                gpg_failures+=("$base")
                # Do NOT proceed to SHA-256: an unverified file's hash must not
                # be recorded into checksums.txt. Skip to next package.
                continue
            fi
        fi
    fi

    # 3. SHA-256
    actual="$(sha256sum "$out" | awk '{print $1}')"
    # Skip comment lines (which would otherwise return "#" as the hash).
    expected="$(awk -v f="$base" '$1!~/^#/ && $1!="" && $2==f {print $1}' "$CHECKSUMS")"

    if [[ -z "$expected" ]]; then
        echo "    sha256: NO ENTRY in checksums.txt — review required"
        echo "$actual  $base" > "$out.next-lock"
        need_review=1
    elif [[ "$actual" != "$expected" ]]; then
        echo "    sha256: MISMATCH"
        echo "      got      $actual"
        echo "      expected $expected"
        sha_failures+=("$base")
    else
        echo "    sha256: OK"
    fi
done

# --- summary ----------------------------------------------------------------
if [[ ${#gpg_failures[@]} -gt 0 ]]; then
    echo
    echo "GPG verification failed for ${#gpg_failures[@]} file(s):"
    for f in "${gpg_failures[@]}"; do echo "  $f"; done
    echo
    echo "Likely cause: the matching public keys aren't in build/keys/ yet."
    echo "Bulk-import every key referenced by sources/*.sig and re-run:"
    echo "    ./import-keys.sh && ./01-fetch.sh"
fi

if [[ ${#sha_failures[@]} -gt 0 ]]; then
    echo
    echo "SHA-256 mismatch for ${#sha_failures[@]} file(s):"
    for f in "${sha_failures[@]}"; do echo "  $f"; done
    echo "Either the tarball was tampered with, or the entry in checksums.txt is wrong."
    echo "Investigate before proceeding. Do NOT silently update checksums.txt."
fi

if [[ $need_review -eq 1 ]]; then
    echo
    echo "Some files lack a SHA-256 entry in checksums.txt."
    echo "Review each sources/<file>.next-lock against the upstream announcement,"
    echo "then merge into checksums.txt:"
    echo
    echo "    cat sources/*.next-lock >> checksums.txt && rm sources/*.next-lock"
fi

# Non-zero exit if anything is unresolved.
if [[ ${#gpg_failures[@]} -gt 0 || ${#sha_failures[@]} -gt 0 ]]; then
    exit 2
elif [[ $need_review -eq 1 ]]; then
    exit 4
fi

echo
echo "All sources present and verified."
