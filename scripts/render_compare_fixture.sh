#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
old_root="${OLD_OQQWALL_ROOT:-"${repo_root}/../OQQWall"}"
fixture_name="${1:-all_content}"
out_dir="${2:-"${repo_root}/target/render-compare/${fixture_name}"}"
tag="fixture_${fixture_name}"

old_after_src="${repo_root}/showcase/render-fixtures/${fixture_name}.old-afterlm.json"
draft_src="${repo_root}/showcase/render-fixtures/${fixture_name}.draft.json"

require_bin() {
  command -v "$1" >/dev/null || {
    echo "missing dependency: $1" >&2
    exit 1
  }
}

replace_placeholders() {
  local src="$1"
  local dst="$2"
  python3 - "$src" "$dst" "$repo_root" "$old_run" <<'PY'
import pathlib
import sys

src, dst, repo_root, old_run = map(pathlib.Path, sys.argv[1:])
text = src.read_text(encoding="utf-8")
text = text.replace("__REPO_ROOT__", str(repo_root))
text = text.replace("__OLD_RUN_ROOT__", str(old_run))
pathlib.Path(dst).write_text(text, encoding="utf-8")
PY
}

container_path() {
  local path="$1"
  local data_root="${HOME}/data"
  case "$path" in
    "$data_root"/*)
      printf '/data/%s' "${path#"$data_root"/}"
      ;;
    *)
      echo "path is not under ${data_root}: ${path}" >&2
      exit 1
      ;;
  esac
}

for bin in python3 sqlite3 jq qrencode chromium magick compare docker; do
  require_bin "$bin"
done

if [[ ! -d "${old_root}/getmsgserv" ]]; then
  echo "old OQQWall getmsgserv not found: ${old_root}/getmsgserv" >&2
  exit 1
fi
if [[ ! -f "$old_after_src" || ! -f "$draft_src" ]]; then
  echo "fixture files not found for ${fixture_name}" >&2
  exit 1
fi

rm -rf "$out_dir"
mkdir -p "$out_dir"

old_run="${out_dir}/old-run"
mkdir -p "${old_run}/cache"
ln -s "${old_root}/getmsgserv" "${old_run}/getmsgserv"

old_after="${out_dir}/old.afterlm.json"
draft_json="${out_dir}/new.draft.json"
replace_placeholders "$old_after_src" "$old_after"
replace_placeholders "$draft_src" "$draft_json"

python3 - "$old_run" "$old_after" "$tag" <<'PY'
import pathlib
import sqlite3
import sys

old_run = pathlib.Path(sys.argv[1])
old_after = pathlib.Path(sys.argv[2]).read_text(encoding="utf-8")
tag = sys.argv[3]
db = old_run / "cache" / "OQQWall.db"
conn = sqlite3.connect(db)
conn.execute(
    "CREATE TABLE preprocess (tag TEXT, AfterLM TEXT, senderid TEXT, receiver TEXT, ACgroup TEXT, nickname TEXT)"
)
conn.execute(
    "INSERT INTO preprocess VALUES (?, ?, ?, ?, ?, ?)",
    (tag, old_after, "10000", "fixture-receiver", "fixture-group", "anonymous fixture"),
)
conn.commit()
conn.close()
PY

cat >"${old_run}/AcountGroupcfg.json" <<'JSON'
{
  "fixture-group": {
    "watermark_text": "fixture-watermark"
  },
  "MethGroup": {
    "watermark": "fixture-watermark"
  }
}
JSON

old_html="${out_dir}/old.html"
old_png="${out_dir}/old.png"
new_png="${out_dir}/new.png"
old_norm="${out_dir}/old.norm.png"
new_norm="${out_dir}/new.norm.png"
diff_png="${out_dir}/diff.png"
metric_file="${out_dir}/diff.metric.txt"
render_log="${out_dir}/new.render.log"

(
  cd "$old_run"
  bash "${old_root}/getmsgserv/HTMLwork/gotohtml.sh" "$tag" >"$old_html"
)

chromium \
  --headless \
  --no-sandbox \
  --disable-gpu \
  --hide-scrollbars \
  --run-all-compositor-stages-before-draw \
  --virtual-time-budget=1500 \
  --window-size=384,2304 \
  --screenshot="$old_png" \
  "file://${old_html}" >/dev/null 2>&1

container_fixture="$(container_path "$draft_json")"
container_output="$(container_path "$new_png")"
docker run --rm --network host \
  -v "$HOME/data:/data" \
  -w /data/OQQWall_rust \
  -v "$HOME/.cargo/registry:/root/.cargo/registry" \
  -v "$HOME/.cargo/git:/root/.cargo/git" \
  -v "$repo_root/.target:/work/target" \
  -e CARGO_HOME=/root/.cargo \
  -e CARGO_TARGET_DIR=/work/target \
  rust-glibc231:20.04-oqqwall \
  bash -c "cargo run -q -p oqqwall_rust_drivers --example render_fixture -- '${container_fixture}' '${container_output}'" \
  >"$render_log" 2>&1 || {
    echo "container render failed; falling back to host cargo with CARGO_TARGET_DIR=.target" | tee -a "$render_log" >&2
    CARGO_TARGET_DIR="$repo_root/.target" \
      cargo run -q -p oqqwall_rust_drivers --example render_fixture -- "$draft_json" "$new_png" \
      >>"$render_log" 2>&1
  }

magick "$old_png" -background '#f2f2f2' -gravity NorthWest -extent 384x2304 "$old_norm"
magick "$new_png" -resize 384x -background '#f2f2f2' -gravity NorthWest -extent 384x2304 "$new_norm"

set +e
compare -metric AE "$old_norm" "$new_norm" "$diff_png" 2>"$metric_file"
compare_status=$?
set -e
if [[ "$compare_status" -gt 1 ]]; then
  echo "image compare failed; see $metric_file" >&2
  exit "$compare_status"
fi

echo "fixture: $fixture_name"
echo "old segment types: $(jq -r '.. | objects | .type? // empty' "$old_after" | sort -u | paste -sd ',' -)"
echo "new draft blocks: $(jq -r '.draft.blocks[] | if type == "string" then . else keys[0] end' "$draft_json" | sort -u | paste -sd ',' -)"
magick identify -format 'old: %wx%h %b\n' "$old_png"
magick identify -format 'new: %wx%h %b\n' "$new_png"
magick identify -format 'diff: %wx%h %b\n' "$diff_png"
echo "AE difference pixels: $(cat "$metric_file")"
echo "outputs: $out_dir"
