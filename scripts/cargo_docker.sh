#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
IMAGE="${OQQWALL_CARGO_DOCKER_IMAGE:-rust-glibc231:20.04-oqqwall}"
HOST_DATA_DIR="${OQQWALL_DOCKER_DATA_DIR:-$HOME/data}"
HOST_TARGET_DIR="${OQQWALL_CARGO_TARGET_DIR_HOST:-$ROOT_DIR/target}"
CONTAINER_WORKDIR="${OQQWALL_CARGO_DOCKER_WORKDIR:-/data/OQQWall_rust}"
HOST_UID="${OQQWALL_DOCKER_HOST_UID:-$(id -u)}"
HOST_GID="${OQQWALL_DOCKER_HOST_GID:-$(id -g)}"

if [[ $# -eq 0 ]]; then
  echo "usage: scripts/cargo_docker.sh <cargo args...>" >&2
  echo "example: scripts/cargo_docker.sh build -p OQQWall_RUST" >&2
  exit 2
fi

mkdir -p "$HOST_TARGET_DIR" "$HOME/.cargo/registry" "$HOME/.cargo/git"

docker run --rm --network host \
  -v "$HOST_DATA_DIR:/data" \
  -v "$HOME/.cargo/registry:/root/.cargo/registry" \
  -v "$HOME/.cargo/git:/root/.cargo/git" \
  -v "$HOST_TARGET_DIR:/work/target" \
  -e CARGO_TARGET_DIR=/work/target \
  -e CARGO_HOME=/root/.cargo \
  -e HOST_UID="$HOST_UID" \
  -e HOST_GID="$HOST_GID" \
  -w "$CONTAINER_WORKDIR" \
  "$IMAGE" \
  bash -c '
    set -euo pipefail

    chown_if_needed() {
      local path="$1"
      [[ -e "$path" ]] || return 0
      find "$path" \( ! -user "$HOST_UID" -o ! -group "$HOST_GID" \) -exec chown "$HOST_UID:$HOST_GID" {} + 2>/dev/null || true
    }

    cleanup() {
      local status=$?
      chown_if_needed /work/target
      chown_if_needed "$PWD/dist"
      exit "$status"
    }

    trap cleanup EXIT
    cargo "$@"
  ' cargo "$@"
