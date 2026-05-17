#!/usr/bin/env bash
# build/check-host.sh — verify the workstation meets both:
#   (a) the LFS Chapter 2 host prerequisite versions, and
#   (b) the WriteOnce-specific additions (Rust, QEMU, ISO tooling).
#
# Exits 0 on success, 1 on failure. Re-run after installing missing tools.

set -uo pipefail

cd "$( dirname "${BASH_SOURCE[0]}" )"

failed=0

# ---- helpers ----------------------------------------------------------------
ver_check() {
    local name="$1" cmd="$2" minimum="$3"
    local out version
    if ! out="$("$cmd" --version 2>/dev/null)"; then
        printf 'MISS  %-12s  (%s not found)\n' "$name" "$cmd"
        failed=1; return
    fi
    # Scan the first few lines; some tools (perl, bison) print a blank
    # line or banner before the version.
    version="$(printf '%s\n' "$out" | head -n5 | grep -oE '[0-9]+(\.[0-9]+)+' | head -n1)"
    if [[ -z "$version" ]]; then
        printf 'WARN  %-12s  cannot parse version from: %s\n' "$name" "${out%%$'\n'*}"
        return
    fi
    # version sort: minimum must be <= version
    if [[ "$(printf '%s\n%s\n' "$minimum" "$version" | sort -V | head -n1)" != "$minimum" ]]; then
        printf 'OLD   %-12s  %-10s (need >= %s)\n' "$name" "$version" "$minimum"
        failed=1
    else
        printf 'OK    %-12s  %-10s (>= %s)\n' "$name" "$version" "$minimum"
    fi
}

alias_check() {
    local cmd="$1" expected="$2"
    if ! command -v "$cmd" >/dev/null 2>&1; then
        printf 'MISS  alias %-7s  (not found)\n' "$cmd"; failed=1; return
    fi
    local target; target="$("$cmd" --version 2>/dev/null | head -n1)"
    if [[ "$target" == *"$expected"* ]]; then
        printf 'OK    alias %-7s  -> %s\n' "$cmd" "$expected"
    else
        printf 'WARN  alias %-7s  -> %s (expected %s)\n' "$cmd" "$target" "$expected"
    fi
}

have_cmd() {
    local name="$1" cmd="$2"
    if command -v "$cmd" >/dev/null 2>&1; then
        printf 'OK    %-12s  %s\n' "$name" "$(command -v "$cmd")"
    else
        printf 'MISS  %-12s  (%s not in PATH)\n' "$name" "$cmd"
        failed=1
    fi
}

# ---- LFS chapter 2 minimums -------------------------------------------------
echo
echo "== LFS Chapter 2 host prerequisites =="
ver_check Bash       bash      3.2
ver_check Binutils   ld        2.13.1
ver_check Bison      bison     2.7
ver_check Coreutils  sort      8.1
ver_check Diffutils  diff      2.8.1
ver_check Findutils  find      4.2.31
ver_check Gawk       gawk      4.0.1
ver_check GCC        gcc       5.4
ver_check "GCC C++"  g++       5.4
ver_check Grep       grep      2.5.1
ver_check Gzip       gzip      1.3.12
ver_check M4         m4        1.4.10
ver_check Make       make      4.0
ver_check Patch      patch     2.5.4
ver_check Perl       perl      5.8.8
ver_check Python     python3   3.4
ver_check Sed        sed       4.1.5
ver_check Tar        tar       1.22
ver_check Texinfo    texi2any  5.0
ver_check Xz         xz        5.0.0

echo
echo "== Required POSIX aliases =="
alias_check awk  GNU
alias_check yacc bison
alias_check sh   bash

# ---- WriteOnce-specific additions -------------------------------------------
echo
echo "== WriteOnce additions: source fetch + verify =="
have_cmd "wget"      wget
have_cmd "curl"      curl
have_cmd "gpg"       gpg
have_cmd "sha256sum" sha256sum
have_cmd "git"       git

echo
echo "== WriteOnce additions: kernel + initramfs build =="
have_cmd "bc"        bc
have_cmd "kmod"      kmod
have_cmd "flex"      flex
have_cmd "cpio"      cpio
have_cmd "zstd"      zstd
# Headers (libssl-dev, libelf-dev) checked indirectly by the kernel build.
[[ -e /usr/include/openssl/ssl.h ]] && printf 'OK    libssl-dev    /usr/include/openssl/ssl.h\n' \
                                    || { printf 'MISS  libssl-dev    (apt install libssl-dev)\n'; failed=1; }
[[ -e /usr/include/libelf.h ]]      && printf 'OK    libelf-dev    /usr/include/libelf.h\n' \
                                    || { printf 'MISS  libelf-dev    (apt install libelf-dev)\n'; failed=1; }

echo
echo "== WriteOnce additions: Rust toolchain (Phase 3+) =="
have_cmd "rustup"    rustup
have_cmd "cargo"     cargo
have_cmd "rustc"     rustc

echo
echo "== WriteOnce additions: QEMU + ISO tooling =="
have_cmd "qemu-system-x86_64" qemu-system-x86_64
[[ -e /usr/share/OVMF/OVMF_CODE.fd ]] && printf 'OK    OVMF          /usr/share/OVMF/OVMF_CODE.fd\n' \
                                      || { printf 'MISS  OVMF          (apt install ovmf)\n'; failed=1; }
have_cmd "xorriso"   xorriso
have_cmd "mkfs.vfat" mkfs.vfat
have_cmd "mtools"    mcopy
have_cmd "nc"        nc

# ---- summary ----------------------------------------------------------------
echo
if [[ $failed -eq 0 ]]; then
    echo "All host prerequisites satisfied. Phase 0 can proceed."
    exit 0
else
    echo "Host prerequisites incomplete. Install the missing tools above and re-run."
    echo
    echo "Ubuntu 24.04 one-shot installer (LFS basics + WriteOnce additions):"
    echo "  sudo apt-get install -y build-essential bison gawk texinfo xz-utils \\"
    echo "    bc kmod flex cpio libssl-dev libelf-dev zstd \\"
    echo "    wget curl gnupg git \\"
    echo "    qemu-system-x86 ovmf \\"
    echo "    xorriso mtools dosfstools \\"
    echo "    netcat-openbsd"
    echo
    echo "Rust toolchain (Ubuntu's apt rustc is too old):"
    echo "  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
    exit 1
fi
