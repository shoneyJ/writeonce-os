#!/usr/bin/env bash
# build/audit-deps.sh — report every meson dependency probe that failed,
# for a package that has already been *attempted* via blfs-pkg.sh.
#
# Reads the package's `meson-logs/meson-log.txt` (written by `meson setup`,
# even on failure) and prints:
#   - Required dep that aborted configure          (THE blocker)
#   - All optional deps that were probed and NO'd  (informational; build
#     proceeds without them but loses features)
#
# Use after any `just phase-8<x>` failure to plan the full fix in one
# pass rather than fix → re-run → fix the next blocker → re-run.
#
# Usage:
#     ./build/audit-deps.sh <package-name>
#     e.g.  ./build/audit-deps.sh xorg-server
#           ./build/audit-deps.sh gtk
#           ./build/audit-deps.sh pipewire

set -uo pipefail

pkg="${1:-}"
[[ -n "$pkg" ]] || { echo "usage: $0 <package-name>" >&2; exit 1; }

log="build/work/$pkg/build/meson-logs/meson-log.txt"
if [[ ! -f "$log" ]]; then
    log="$(find build/work -path "*/$pkg/*/meson-logs/meson-log.txt" 2>/dev/null | head -1)"
fi
[[ -f "$log" ]] || {
    echo "no meson-log.txt for '$pkg' — run the build at least once first" >&2
    exit 2
}

printf '=== %s ===\n\n' "$log"

printf '— Fatal:\n'
grep -E "^meson\.build:[0-9]+:.*ERROR:" "$log" | head -3
echo

printf '— Required deps that failed (will block again unless fixed):\n'
# The fatal line names the missing dep; surface it for clarity.
grep -E "^meson\.build:[0-9]+:.*ERROR: Dependency" "$log" \
  | sed -E 's/.*Dependency "([^"]+)".*/  \1/'
echo

printf '— Optional deps probed NO (informational; feature loss only):\n'
grep -E "Run-time dependency .* found: NO" "$log" \
  | sed -E 's/Run-time dependency (.*) found: NO.*/  \1/'
echo

printf '— Linker probes NO (only flags blocking link-time misses):\n'
grep -E "^Library .* found: NO" "$log" \
  | sed -E 's/Library (.*) found: NO.*/  \1/'
echo

printf '— C header probes that failed:\n'
# Match both styles: `Has header "x" : NO` and `C header 'x' not usable`.
grep -E "Header [\"'][^\"']+[\"'] (not usable|: NO)|C header [\"'][^\"']+[\"'] not usable" "$log" \
  | sed -E "s/.*[Hh]eader [\"']([^\"']+)[\"'].*/  \\1/"
echo

printf '— Programs not found (build scripts, codegen tools):\n'
grep -E "Program .* found: NO" "$log" \
  | sed -E 's/Program (.*) found: NO.*/  \1/'
echo

# Always succeed — the audit is informational; never block a recipe.
exit 0
