#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT_DIR"

if [[ ! -d res ]]; then
  echo "error: missing ./res directory" >&2
  exit 1
fi

cargo build -r -p OQQWall_RUST

BIN_PATH="target/release/OQQWall_RUST"
if [[ ! -x "$BIN_PATH" ]]; then
  echo "error: binary not found: $BIN_PATH" >&2
  exit 1
fi

DIST_DIR="dist"
mkdir -p "$DIST_DIR"
STAMP="$(date +%Y%m%d_%H%M%S)"
BIN_OUT="$DIST_DIR/OQQWall_RUST"
RES_PKG_META="$DIST_DIR/.res_pkg_meta"
RES_PKG=""
TMP_RES_TAR="$DIST_DIR/.res-build-${STAMP}.tar"
TMP_RES_GZ="${TMP_RES_TAR}.gz"

cp "$BIN_PATH" "$BIN_OUT"
tar --sort=name --mtime='UTC 1970-01-01' --owner=0 --group=0 --numeric-owner -C "$ROOT_DIR" -cf "$TMP_RES_TAR" "res"
gzip -n -f "$TMP_RES_TAR"

NEW_RES_SHA="$(sha256sum "$TMP_RES_GZ" | awk '{print $1}')"
if [[ -f "$RES_PKG_META" ]]; then
  LAST_RES_SHA="$(awk 'NR==1{print $1}' "$RES_PKG_META")"
  LAST_RES_PKG="$(awk 'NR==1{print $2}' "$RES_PKG_META")"
  if [[ "$NEW_RES_SHA" == "$LAST_RES_SHA" && -n "$LAST_RES_PKG" && -f "$LAST_RES_PKG" ]]; then
    RES_PKG="$LAST_RES_PKG"
    rm -f "$TMP_RES_GZ"
    echo "res unchanged, reuse: $RES_PKG"
  fi
fi

if [[ -z "$RES_PKG" ]]; then
  RES_PKG="$DIST_DIR/OQQWall_RUST-res-${STAMP}.tar.gz"
  mv "$TMP_RES_GZ" "$RES_PKG"
  printf '%s %s\n' "$NEW_RES_SHA" "$RES_PKG" > "$RES_PKG_META"
fi

echo "created: $BIN_OUT"
echo "created: $RES_PKG"
echo "sha256: $NEW_RES_SHA"
echo "deploy: unpack both packages into the same directory"
