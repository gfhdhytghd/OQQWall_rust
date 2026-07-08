#!/usr/bin/env python3
"""Build the thank-you sticker hash registry from URL seeds.

The output stores only hashes and source metadata; downloaded images are not
persisted. Requires Pillow.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import urllib.request
from io import BytesIO
from pathlib import Path

from PIL import Image


def average_hash(data: bytes) -> str:
    image = Image.open(BytesIO(data))
    try:
        image.seek(0)
    except EOFError:
        pass
    image = image.convert("L").resize((8, 8), Image.Resampling.LANCZOS)
    pixels = list(image.getdata())
    avg = sum(pixels) / len(pixels)
    bits = 0
    for idx, value in enumerate(pixels):
        if value >= avg:
            bits |= 1 << (63 - idx)
    return f"{bits:016x}"


def dimensions(data: bytes) -> list[int]:
    image = Image.open(BytesIO(data))
    return [image.width, image.height]


def download(url: str) -> bytes:
    request = urllib.request.Request(url, headers={"User-Agent": "Mozilla/5.0"})
    with urllib.request.urlopen(request, timeout=20) as response:
        return response.read()


def face_entry(path: Path, face_id: str, label: str) -> dict[str, str]:
    data = path.read_bytes()
    return {
        "id": face_id,
        "label": label,
        "source": "res/face/default_config.json",
        "sha256": hashlib.sha256(data).hexdigest(),
        "phash": average_hash(data),
    }


def image_entry(seed: dict[str, str]) -> dict[str, object]:
    data = download(seed["image_url"])
    return {
        "label": "thanks",
        "query": seed["query"],
        "title": seed["title"],
        "source_url": seed["source_url"],
        "image_url": seed["image_url"],
        "sha256": hashlib.sha256(data).hexdigest(),
        "phash": average_hash(data),
        "dimensions": dimensions(data),
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--seeds", required=True, help="JSON file with image seed objects")
    parser.add_argument("--out", required=True, help="registry JSON output path")
    args = parser.parse_args()

    seeds = json.loads(Path(args.seeds).read_text())
    registry = {
        "version": 1,
        "face_ids": [
            face_entry(Path("res/face/297.png"), "297", "拜谢"),
            face_entry(Path("res/face/118.png"), "118", "抱拳"),
            face_entry(Path("res/face/78.png"), "78", "握手"),
            face_entry(Path("res/face/76.png"), "76", "赞"),
            face_entry(Path("res/face/201.png"), "201", "点赞"),
        ],
        "mfaces": [],
        "file_uniques": [],
        "images": [image_entry(seed) for seed in seeds],
    }
    Path(args.out).write_text(json.dumps(registry, ensure_ascii=False, indent=2) + "\n")


if __name__ == "__main__":
    main()
