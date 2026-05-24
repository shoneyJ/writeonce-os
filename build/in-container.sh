#!/usr/bin/env bash
# build/in-container.sh — run a command inside the wo-builder image.
#
# The image carries dev libraries (libpam0g-dev today; libdbus, libgtk-4,
# libpipewire later) that we deliberately do NOT install on the host.
# Bind-mounts the repo at /work and runs as the caller's UID/GID so
# files written to target/ stay host-owned.
#
# Usage:
#     ./build/in-container.sh cargo build -p writeonce-login --release
#     ./build/in-container.sh cargo test --workspace
#     ./build/in-container.sh                                # interactive shell
#
# The first invocation builds the wo-builder image (~700 MB, ~3 min).
# Subsequent invocations reuse it unless build/Containerfile has changed
# — a sha label tracks freshness.

set -uo pipefail

REPO="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
IMAGE="wo-builder:latest"
CF="$REPO/build/Containerfile"

want_sha="$(sha256sum "$CF" | awk '{print $1}')"
have_sha="$(docker image inspect --format '{{ index .Config.Labels "wo.containerfile.sha" }}' \
            "$IMAGE" 2>/dev/null || true)"

if [[ "$want_sha" != "$have_sha" ]]; then
    echo ">>> Rebuilding $IMAGE (Containerfile sha: $want_sha)"
    docker build --label "wo.containerfile.sha=$want_sha" \
                 -t "$IMAGE" \
                 -f "$CF" "$REPO/build"
fi

# Optional --no-network: cut network access to the container. Use for
# *compile* steps (sources are already fetched by 01-fetch.sh). This is
# a supply-chain defense — a tampered build script can't phone home.
NET_ARG=""
if [[ "${1:-}" == "--no-network" ]]; then
    NET_ARG="--network=none"
    shift
fi

# RUSTUP_HOME points at the image-baked toolchain install (made
# world-readable by build/Containerfile). CARGO_HOME goes on the bind
# mount so the registry cache + target stamps stay host-owned and
# persist across container invocations.
COMMON=(
    -v "$REPO":/work
    -u "$(id -u):$(id -g)"
    -e HOME=/work/.container-home
    -e RUSTUP_HOME=/root/.rustup
    -e CARGO_HOME=/work/.cargo-cache
)

# Interactive shell if no args, otherwise run the command.
if [[ $# -eq 0 ]]; then
    exec docker run --rm -it $NET_ARG "${COMMON[@]}" "$IMAGE" bash
else
    exec docker run --rm    $NET_ARG "${COMMON[@]}" "$IMAGE" "$@"
fi
