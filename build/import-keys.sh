#!/usr/bin/env bash
# build/import-keys.sh — bulk-import every upstream GPG key referenced by
# any sources/*.sig file we've already downloaded.
#
# Workflow:
#   1. ./01-fetch.sh         (downloads tarballs + signatures; reports missing keys)
#   2. ./import-keys.sh      (this script — fetches and locks every referenced key)
#   3. ./01-fetch.sh         (re-run — every GPG should now pass)
#
# Idempotent: re-running is safe. Already-imported keys are noops.
#
# This is a *utility* (not a sequenced step), so it lives unnumbered alongside
# clean.sh per the project's NN- naming convention.

# NOTE: deliberately not using `set -e`. The script does explicit error
# accounting via the `failed` counter and the per-keyserver retry loop;
# letting individual gpg/grep failures bubble up through -e would mask
# the partial-success reporting we want.
set -uo pipefail

cd "$( dirname "${BASH_SOURCE[0]}" )"

KEYRING="$PWD/gnupg"
mkdir -p "$KEYRING" keys
chmod 700 "$KEYRING"

# Pick a keyserver pool. keys.openpgp.org is the most reliable nowadays;
# keyserver.ubuntu.com is a good fallback. You can override with:
#     KEYSERVER=hkps://keyring.kernel.org ./import-keys.sh
KEYSERVERS=( "${KEYSERVER:-hkps://keys.openpgp.org}" "hkps://keyserver.ubuntu.com" "hkps://pgp.mit.edu" )

# Map .sig basenames to friendly key file names (so we get keys/gcc.asc, not
# keys/gcc-14.2.0.tar.xz.sig.asc).
friendly_name() {
    local sig_path="$1"
    local base
    base="$(basename "$sig_path")"
    base="${base%.tar*}"        # drop .tar.xz.sig / .tar.gz.sig / .tar.bz2.sig
    base="${base%-*}"           # drop -X.Y.Z version
    echo "$base"
}

shopt -s nullglob
sig_files=( sources/*.sig )
if [[ ${#sig_files[@]} -eq 0 ]]; then
    echo "No sources/*.sig files found. Run ./01-fetch.sh first to download them."
    exit 1
fi

declare -A SEEN
ok=0; failed=0

for sig in "${sig_files[@]}"; do
    name="$(friendly_name "$sig")"

    # Extract the long fingerprint if present, otherwise the short key ID.
    # gpg prints the long form by default in modern releases; older versions
    # may print only the 16-char short form.
    keyid="$(GNUPGHOME="$KEYRING" gpg --verify "$sig" 2>&1 \
              | grep -oE '[A-F0-9]{40}|[A-F0-9]{16}' \
              | sort -u | tail -n1)"   # prefer the longer fingerprint

    if [[ -z "$keyid" ]]; then
        # If the kernel sig is over the decompressed file, plain --verify
        # won't reveal the key id. Try the linux-only form:
        if [[ "$sig" == *linux-*.sign ]]; then
            tarball="${sig%.sign}"
            [[ -f "$tarball" ]] || tarball="${tarball%.xz}.xz"
            keyid="$(xz -dc "$tarball" 2>/dev/null \
                      | GNUPGHOME="$KEYRING" gpg --verify "$sig" - 2>&1 \
                      | grep -oE '[A-F0-9]{40}|[A-F0-9]{16}' \
                      | sort -u | tail -n1)"
        fi
    fi

    if [[ -z "$keyid" ]]; then
        echo "??? $sig — could not extract a key id"
        failed=$((failed+1))
        continue
    fi

    if [[ -n "${SEEN[$keyid]:-}" ]]; then
        echo "    $name -> $keyid (already imported via ${SEEN[$keyid]})"
        continue
    fi
    SEEN[$keyid]="$name"

    # Try each keyserver in order until one *actually* hands us the key.
    # keys.openpgp.org serves only identity-verified keys; recv-keys exits
    # success with "0 imported" when the key isn't there. So we confirm the
    # key landed by checking the keyring afterwards.
    got=0
    for ks in "${KEYSERVERS[@]}"; do
        echo ">>> $name: recv $keyid from $ks"
        GNUPGHOME="$KEYRING" gpg --quiet \
              --keyserver "$ks" --recv-keys "$keyid" 2>/dev/null || true
        if GNUPGHOME="$KEYRING" gpg --list-keys "$keyid" >/dev/null 2>&1; then
            got=1; break
        fi
        echo "    (keyserver returned nothing for $keyid, trying next)"
    done

    if [[ $got -eq 0 ]]; then
        echo "    FAIL: no keyserver returned $keyid"
        failed=$((failed+1))
        continue
    fi

    # Export ASCII-armored to keys/ so the build doesn't depend on a network
    # keyserver from this point on.
    GNUPGHOME="$KEYRING" gpg --export --armor "$keyid" > "keys/${name}.asc"
    echo "    OK: keys/${name}.asc written"
    ok=$((ok+1))
done

echo
echo "imported: $ok | failed: $failed | distinct keys: ${#SEEN[@]}"
echo "Now re-run ./01-fetch.sh to GPG-verify every tarball."

[[ $failed -eq 0 ]]
