#!/usr/bin/env bash
set -euo pipefail

APP_NAME="OQQWall_RUST"
ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WEBVIEW_DIR="$ROOT_DIR/crates/app/webview-ui"
WEBVIEW_DIST_DIR="$WEBVIEW_DIR/dist"
BUILD_CACHE_DIR="$ROOT_DIR/.build-cache"
DIST_DIR="${DIST_DIR:-$ROOT_DIR/dist}"
CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$ROOT_DIR/target}"

INSTALL_SYSTEM_DEPS=1
INSTALL_RUSTUP=1
FORCE_WEBVIEW_BUILD=0
SKIP_CHECK=0
OFFLINE=0

usage() {
  cat <<'EOF'
Usage:
  bash scripts/package_ubuntu.sh [options]

Options:
  --skip-system-deps   Do not install Ubuntu build dependencies.
  --skip-rustup        Do not auto-install rustup / cargo when missing.
  --rebuild-webview    Force rebuild crates/app/webview-ui/dist with Bun.
  --skip-check         Skip cargo check before release build.
  --offline            Force cargo fetch/build to run in offline mode.
  -h, --help           Show this help message.

Environment overrides:
  DIST_DIR             Output directory. Default: <repo>/dist
  CARGO_TARGET_DIR     Cargo target directory. Default: <repo>/target
  SKIA_BINARY_FEATURE_SUFFIX
                       Override rust-skia binary cache key suffix.
                       Default: x86_64-unknown-linux-gnu-pdf-textlayout

What this script does:
  1. Install Ubuntu system dependencies when needed.
  2. Install Rust toolchain with rustup when needed.
  3. Reuse existing WebUI dist, or rebuild it when requested / missing.
  4. Pre-download a matching rust-skia prebuilt archive into .build-cache.
  5. Run cargo check and cargo build --release for OQQWall_RUST.
  6. Package a combined tar.gz and split bin/res tar.gz artifacts into dist/.
EOF
}

log() {
  printf '[package_ubuntu] %s\n' "$*"
}

warn() {
  printf '[package_ubuntu] warning: %s\n' "$*" >&2
}

die() {
  printf '[package_ubuntu] error: %s\n' "$*" >&2
  exit 1
}

as_root() {
  if [[ "${EUID}" -eq 0 ]]; then
    "$@"
  elif command -v sudo >/dev/null 2>&1; then
    sudo "$@"
  else
    die "need root privileges to run: $*"
  fi
}

parse_args() {
  while [[ $# -gt 0 ]]; do
    case "$1" in
      --skip-system-deps)
        INSTALL_SYSTEM_DEPS=0
        ;;
      --skip-rustup)
        INSTALL_RUSTUP=0
        ;;
      --rebuild-webview)
        FORCE_WEBVIEW_BUILD=1
        ;;
      --skip-check)
        SKIP_CHECK=1
        ;;
      --offline)
        OFFLINE=1
        ;;
      -h|--help)
        usage
        exit 0
        ;;
      *)
        die "unknown argument: $1"
        ;;
    esac
    shift
  done
}

ensure_linux() {
  if [[ "$(uname -s)" != "Linux" ]]; then
    die "this script must run inside Ubuntu / Linux"
  fi
}

source_cargo_env() {
  if [[ -f "${HOME}/.cargo/env" ]]; then
    # shellcheck source=/dev/null
    . "${HOME}/.cargo/env"
  fi
}

install_system_deps() {
  if ! command -v apt-get >/dev/null 2>&1; then
    die "apt-get not found; this Ubuntu packaging script only supports apt-based systems"
  fi

  local packages=(
    build-essential
    curl
    pkg-config
    python3
    libfreetype6-dev
    libfontconfig1-dev
    ca-certificates
    git
    perl
    file
    xz-utils
    tar
    gzip
  )

  if (( FORCE_WEBVIEW_BUILD )) || [[ ! -d "$WEBVIEW_DIST_DIR" ]]; then
    packages+=(unzip)
  fi

  log "installing Ubuntu build dependencies"
  as_root apt-get update
  as_root apt-get install -y --no-install-recommends "${packages[@]}"
}

ensure_rust_toolchain() {
  source_cargo_env

  if command -v cargo >/dev/null 2>&1 && command -v rustup >/dev/null 2>&1; then
    log "Rust toolchain already present: $(rustc -V)"
    return
  fi

  if (( ! INSTALL_RUSTUP )); then
    die "cargo / rustup not found and --skip-rustup was set"
  fi

  log "installing rustup toolchain"
  curl https://sh.rustup.rs -sSf | sh -s -- -y --profile minimal --component rustfmt --component clippy
  source_cargo_env
  command -v cargo >/dev/null 2>&1 || die "cargo still missing after rustup install"
}

ensure_webview_dist() {
  if (( ! FORCE_WEBVIEW_BUILD )) && [[ -d "$WEBVIEW_DIST_DIR" ]]; then
    log "using existing WebUI dist: $WEBVIEW_DIST_DIR"
    return
  fi

  if ! command -v bun >/dev/null 2>&1; then
    log "installing Bun"
    curl -fsSL https://bun.sh/install | bash
    export PATH="$HOME/.bun/bin:$PATH"
  fi
  command -v bun >/dev/null 2>&1 || die "bun not found; install Bun first"

  log "building embedded WebUI dist"
  pushd "$WEBVIEW_DIR" >/dev/null
  bun install --frozen-lockfile
  bun run typecheck
  bun run build
  popd >/dev/null
}

run_cargo_fetch() {
  local fetch_cmd=(cargo fetch --locked)

  if (( OFFLINE )); then
    fetch_cmd+=(--offline)
    log "running cargo fetch in offline mode"
    "${fetch_cmd[@]}"
    return
  fi

  log "running cargo fetch"
  if ! "${fetch_cmd[@]}"; then
    warn "online cargo fetch failed, retrying with --offline"
    cargo fetch --locked --offline
  fi
}

extract_skia_hash_from_crate() {
  local crate_file="$1"
  local member
  member="$(tar -tzf "$crate_file" | grep '/.cargo_vcs_info.json$' | head -n1 || true)"
  [[ -n "$member" ]] || return 1
  tar -xOf "$crate_file" "$member" | python3 -c 'import json, sys; print(json.load(sys.stdin)["git"]["sha1"][:20])'
}

prepare_skia_binary_cache() {
  mkdir -p "$BUILD_CACHE_DIR"

  local cache_root="${CARGO_HOME:-$HOME/.cargo}/registry/cache"
  local crate_file
  crate_file="$(find "$cache_root" -type f -name 'skia-bindings-*.crate' | sort | tail -n1 || true)"
  if [[ -z "$crate_file" ]]; then
    warn "skia-bindings crate archive not found in cargo cache; skip local Skia binary cache setup"
    return
  fi

  local crate_name
  crate_name="$(basename "$crate_file")"
  local skia_version="${crate_name#skia-bindings-}"
  skia_version="${skia_version%.crate}"

  local short_hash
  short_hash="$(extract_skia_hash_from_crate "$crate_file" || true)"
  if [[ -z "$short_hash" ]]; then
    warn "failed to read skia-bindings VCS hash from $crate_name; skip local Skia binary cache setup"
    return
  fi

  local feature_suffix="${SKIA_BINARY_FEATURE_SUFFIX:-x86_64-unknown-linux-gnu-pdf-textlayout}"
  local key="${short_hash}-${feature_suffix}"
  local archive_path="$BUILD_CACHE_DIR/skia-binaries-${key}.tar.gz"

  if [[ ! -f "$archive_path" ]]; then
    local archive_url="https://github.com/rust-skia/skia-binaries/releases/download/${skia_version}/skia-binaries-${key}.tar.gz"
    log "downloading rust-skia prebuilt archive: $archive_url"
    if ! curl -fL --retry 5 --retry-delay 2 --connect-timeout 20 --max-time 900 "$archive_url" -o "$archive_path"; then
      warn "failed to pre-download rust-skia archive, cargo will fall back to rust-skia default behavior"
      rm -f "$archive_path"
      return
    fi
  else
    log "reusing rust-skia prebuilt archive: $archive_path"
  fi

  local build_cache_uri
  build_cache_uri="$(python3 -c 'from pathlib import Path; import sys; print(Path(sys.argv[1]).resolve().as_uri())' "$BUILD_CACHE_DIR")"
  export SKIA_BINARIES_URL="${build_cache_uri}/skia-binaries-{key}.tar.gz"
  log "exported SKIA_BINARIES_URL=$SKIA_BINARIES_URL"
}

run_cargo_builds() {
  export CARGO_HTTP_TIMEOUT="${CARGO_HTTP_TIMEOUT:-600}"
  export CARGO_NET_RETRY="${CARGO_NET_RETRY:-10}"
  export CARGO_REGISTRIES_CRATES_IO_PROTOCOL="${CARGO_REGISTRIES_CRATES_IO_PROTOCOL:-sparse}"
  export CARGO_TARGET_DIR

  if (( OFFLINE )); then
    export CARGO_NET_OFFLINE=true
  fi

  run_cargo_fetch
  prepare_skia_binary_cache

  if (( ! SKIP_CHECK )); then
    log "running cargo check"
    cargo check -p "$APP_NAME"
  fi

  log "building release binary"
  cargo build --release -p "$APP_NAME"
}

write_bundle_readme() {
  local file_path="$1"
  local glibc_version="$2"

  cat >"$file_path" <<EOF
OQQWall Ubuntu package
======================

Binary:
  ./${APP_NAME}

Runtime resources:
  ./res

Built on:
  $(date -u '+%Y-%m-%d %H:%M:%S UTC')

glibc on build host:
  ${glibc_version}

Quick start:
  1. cd into this directory
  2. chmod +x ./${APP_NAME}
  3. ./${APP_NAME}
  4. if config.json is missing or invalid, open the printed Web OOBE URL
  5. fill the form and save; the current process will continue startup automatically

Runtime shared library expectations:
  libfontconfig.so.1
  libfreetype.so.6

If you want to start OOBE manually later:
  ./${APP_NAME} oobe --config config.json

If you want to rebuild the embedded WebUI later:
  cd crates/app/webview-ui
  npm install --no-package-lock
  ./node_modules/.bin/vite build
EOF
}

package_release() {
  local bin_path="$CARGO_TARGET_DIR/release/$APP_NAME"
  [[ -x "$bin_path" ]] || die "release binary not found: $bin_path"
  [[ -d "$ROOT_DIR/res" ]] || die "missing runtime resource directory: $ROOT_DIR/res"

  mkdir -p "$DIST_DIR"

  local app_version
  app_version="$(awk -F'"' '/^version = / { print $2; exit }' "$ROOT_DIR/crates/app/Cargo.toml")"
  [[ -n "$app_version" ]] || app_version="unknown"

  local glibc_version
  glibc_version="$(
    ldd --version 2>&1 | awk '
      NR == 1 {
        for (i = 1; i <= NF; i++) {
          if ($i ~ /^[0-9]+\.[0-9]+$/) {
            version = $i
          }
        }
      }
      END {
        if (version != "") {
          print version
        }
      }
    '
  )"
  [[ -n "$glibc_version" ]] || glibc_version="unknown"

  local stamp
  stamp="$(date +%Y%m%d_%H%M%S)"
  local bundle_name="${APP_NAME}-v${app_version}-linux-x86_64-glibc${glibc_version}-${stamp}"
  local stage_dir="$DIST_DIR/$bundle_name"
  local bundle_tar="$DIST_DIR/${bundle_name}.tar.gz"
  local bin_tar="$DIST_DIR/${APP_NAME}-bin-v${app_version}-linux-x86_64-glibc${glibc_version}-${stamp}.tar.gz"
  local res_tar="$DIST_DIR/${APP_NAME}-res-v${app_version}-${stamp}.tar.gz"

  rm -rf "$stage_dir"
  mkdir -p "$stage_dir"

  cp "$bin_path" "$stage_dir/$APP_NAME"
  cp -a "$ROOT_DIR/res" "$stage_dir/res"
  write_bundle_readme "$stage_dir/README.package.txt" "$glibc_version"

  tar --sort=name --mtime='UTC 1970-01-01' --owner=0 --group=0 --numeric-owner \
    -C "$DIST_DIR" -czf "$bundle_tar" "$bundle_name"
  sha256sum "$bundle_tar" > "${bundle_tar}.sha256"

  tar --sort=name --mtime='UTC 1970-01-01' --owner=0 --group=0 --numeric-owner \
    -C "$CARGO_TARGET_DIR/release" -czf "$bin_tar" "$APP_NAME"
  sha256sum "$bin_tar" > "${bin_tar}.sha256"

  tar --sort=name --mtime='UTC 1970-01-01' --owner=0 --group=0 --numeric-owner \
    -C "$ROOT_DIR" -czf "$res_tar" "res"
  sha256sum "$res_tar" > "${res_tar}.sha256"

  log "package ready"
  log "combined: $bundle_tar"
  log "combined sha256: ${bundle_tar}.sha256"
  log "split binary: $bin_tar"
  log "split binary sha256: ${bin_tar}.sha256"
  log "split resources: $res_tar"
  log "split resources sha256: ${res_tar}.sha256"
}

main() {
  parse_args "$@"
  ensure_linux
  cd "$ROOT_DIR"

  if (( INSTALL_SYSTEM_DEPS )); then
    install_system_deps
  fi

  ensure_rust_toolchain
  ensure_webview_dist
  run_cargo_builds
  package_release
}

main "$@"
