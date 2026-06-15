#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

if [[ ! -d res ]]; then
  echo "error: missing ./res directory" >&2
  exit 1
fi

"$ROOT_DIR/scripts/cargo_docker.sh" build -p OQQWall_RUST

rm -rf target/debug/res
cp -a res target/debug/res

echo "debug binary: target/debug/OQQWall_RUST"
echo "debug resources: target/debug/res"
echo "run: OQQWALL_RES_DIR=$ROOT_DIR/res ./target/debug/OQQWall_RUST"
