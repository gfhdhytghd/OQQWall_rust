#!/usr/bin/env python3
import argparse
import io
import json
import pathlib
import shutil
import struct
import sys

PNG_SIG = b"\x89PNG\r\n\x1a\n"


def read_u16(data, offset):
    return struct.unpack_from(">H", data, offset)[0]


def read_u32(data, offset):
    return struct.unpack_from(">I", data, offset)[0]


def read_i8(value):
    return value - 256 if value > 127 else value


def read_i16(data, offset):
    return struct.unpack_from(">h", data, offset)[0]


def parse_table_directory(font_bytes):
    num_tables = read_u16(font_bytes, 4)
    tables = {}
    offset = 12
    for _ in range(num_tables):
        tag = font_bytes[offset : offset + 4].decode("ascii")
        table_offset = read_u32(font_bytes, offset + 8)
        length = read_u32(font_bytes, offset + 12)
        tables[tag] = (table_offset, length)
        offset += 16
    return tables


def cmap_rank(format_id, platform_id, encoding_id):
    if format_id == 12 and platform_id in (0, 3) and encoding_id in (10,):
        return 0
    if format_id == 12 and platform_id in (0, 3):
        return 1
    if format_id == 4 and platform_id in (0, 3) and encoding_id in (1, 10):
        return 2
    if format_id == 4 and platform_id in (0, 3):
        return 3
    return 9


def parse_cmap_format_12(cmap_bytes, offset, glyph_ids):
    if offset + 16 > len(cmap_bytes):
        return {}
    num_groups = read_u32(cmap_bytes, offset + 12)
    groups_offset = offset + 16
    mapping = {}
    glyph_list = sorted(glyph_ids)
    for i in range(num_groups):
        entry_offset = groups_offset + i * 12
        if entry_offset + 12 > len(cmap_bytes):
            break
        start_char = read_u32(cmap_bytes, entry_offset)
        end_char = read_u32(cmap_bytes, entry_offset + 4)
        start_gid = read_u32(cmap_bytes, entry_offset + 8)
        end_gid = start_gid + (end_char - start_char)
        for gid in glyph_list:
            if gid < start_gid:
                continue
            if gid > end_gid:
                break
            codepoint = start_char + (gid - start_gid)
            mapping[codepoint] = gid
    return mapping


def parse_cmap_format_4(cmap_bytes, offset, glyph_ids):
    if offset + 14 > len(cmap_bytes):
        return {}
    seg_count = read_u16(cmap_bytes, offset + 6) // 2
    end_offset = offset + 14
    start_offset = end_offset + seg_count * 2 + 2
    delta_offset = start_offset + seg_count * 2
    range_offset = delta_offset + seg_count * 2
    mapping = {}
    for i in range(seg_count):
        end_code = read_u16(cmap_bytes, end_offset + i * 2)
        start_code = read_u16(cmap_bytes, start_offset + i * 2)
        id_delta = read_i16(cmap_bytes, delta_offset + i * 2)
        id_range_offset = read_u16(cmap_bytes, range_offset + i * 2)
        if start_code == 0xFFFF and end_code == 0xFFFF:
            break
        for codepoint in range(start_code, end_code + 1):
            if id_range_offset == 0:
                gid = (codepoint + id_delta) & 0xFFFF
            else:
                glyph_index_offset = (
                    range_offset + i * 2 + id_range_offset + 2 * (codepoint - start_code)
                )
                if glyph_index_offset + 2 > len(cmap_bytes):
                    continue
                gid = read_u16(cmap_bytes, glyph_index_offset)
                if gid != 0:
                    gid = (gid + id_delta) & 0xFFFF
            if gid in glyph_ids:
                mapping[codepoint] = gid
    return mapping


def parse_cmap_mapping(font_bytes, glyph_ids):
    tables = parse_table_directory(font_bytes)
    cmap = tables.get("cmap")
    if not cmap:
        return {}
    cmap_offset, cmap_len = cmap
    cmap_bytes = font_bytes[cmap_offset : cmap_offset + cmap_len]
    if len(cmap_bytes) < 4:
        return {}
    num_tables = read_u16(cmap_bytes, 2)
    best = None
    offset = 4
    for _ in range(num_tables):
        if offset + 8 > len(cmap_bytes):
            break
        platform_id = read_u16(cmap_bytes, offset)
        encoding_id = read_u16(cmap_bytes, offset + 2)
        sub_offset = read_u32(cmap_bytes, offset + 4)
        offset += 8
        if sub_offset + 2 > len(cmap_bytes):
            continue
        format_id = read_u16(cmap_bytes, sub_offset)
        rank = cmap_rank(format_id, platform_id, encoding_id)
        if best is None or rank < best[0]:
            best = (rank, format_id, sub_offset)
    if best is None:
        return {}
    _, format_id, sub_offset = best
    if format_id == 12:
        return parse_cmap_format_12(cmap_bytes, sub_offset, glyph_ids)
    if format_id == 4:
        return parse_cmap_format_4(cmap_bytes, sub_offset, glyph_ids)
    return {}


def parse_coverage(gsub_bytes, offset):
    if offset + 4 > len(gsub_bytes):
        return []
    fmt = read_u16(gsub_bytes, offset)
    if fmt == 1:
        count = read_u16(gsub_bytes, offset + 2)
        glyphs = []
        base = offset + 4
        for i in range(count):
            if base + 2 * (i + 1) > len(gsub_bytes):
                break
            glyphs.append(read_u16(gsub_bytes, base + 2 * i))
        return glyphs
    if fmt == 2:
        range_count = read_u16(gsub_bytes, offset + 2)
        glyphs = []
        base = offset + 4
        for i in range(range_count):
            entry = base + 6 * i
            if entry + 6 > len(gsub_bytes):
                break
            start_gid = read_u16(gsub_bytes, entry)
            end_gid = read_u16(gsub_bytes, entry + 2)
            glyphs.extend(range(start_gid, end_gid + 1))
        return glyphs
    return []


def parse_gsub_ligatures(font_bytes):
    tables = parse_table_directory(font_bytes)
    gsub = tables.get("GSUB")
    if not gsub:
        return {}
    gsub_offset, gsub_len = gsub
    gsub_bytes = font_bytes[gsub_offset : gsub_offset + gsub_len]
    if len(gsub_bytes) < 10:
        return {}
    lookup_list_offset = read_u16(gsub_bytes, 8)
    if lookup_list_offset <= 0 or lookup_list_offset >= len(gsub_bytes):
        return {}
    lookup_count = read_u16(gsub_bytes, lookup_list_offset)
    ligatures = {}
    for i in range(lookup_count):
        entry_offset = lookup_list_offset + 2 + 2 * i
        if entry_offset + 2 > len(gsub_bytes):
            break
        lookup_offset = lookup_list_offset + read_u16(gsub_bytes, entry_offset)
        if lookup_offset + 6 > len(gsub_bytes):
            continue
        lookup_type = read_u16(gsub_bytes, lookup_offset)
        subtable_count = read_u16(gsub_bytes, lookup_offset + 4)
        if lookup_type != 4:
            continue
        for j in range(subtable_count):
            sub_off = lookup_offset + 6 + 2 * j
            if sub_off + 2 > len(gsub_bytes):
                break
            subtable_offset = lookup_offset + read_u16(gsub_bytes, sub_off)
            if subtable_offset + 6 > len(gsub_bytes):
                continue
            subst_format = read_u16(gsub_bytes, subtable_offset)
            if subst_format != 1:
                continue
            coverage_offset = read_u16(gsub_bytes, subtable_offset + 2)
            lig_set_count = read_u16(gsub_bytes, subtable_offset + 4)
            coverage = parse_coverage(gsub_bytes, subtable_offset + coverage_offset)
            limit = min(lig_set_count, len(coverage))
            for k in range(limit):
                lig_set_off = subtable_offset + 6 + 2 * k
                if lig_set_off + 2 > len(gsub_bytes):
                    break
                lig_set_offset = subtable_offset + read_u16(gsub_bytes, lig_set_off)
                if lig_set_offset + 2 > len(gsub_bytes):
                    continue
                lig_count = read_u16(gsub_bytes, lig_set_offset)
                for l in range(lig_count):
                    lig_off = lig_set_offset + 2 + 2 * l
                    if lig_off + 2 > len(gsub_bytes):
                        break
                    lig_offset = lig_set_offset + read_u16(gsub_bytes, lig_off)
                    if lig_offset + 4 > len(gsub_bytes):
                        continue
                    lig_glyph = read_u16(gsub_bytes, lig_offset)
                    comp_count = read_u16(gsub_bytes, lig_offset + 2)
                    comp_base = lig_offset + 4
                    if comp_count < 1:
                        continue
                    if comp_base + 2 * (comp_count - 1) > len(gsub_bytes):
                        continue
                    components = []
                    for c in range(comp_count - 1):
                        components.append(read_u16(gsub_bytes, comp_base + 2 * c))
                    sequence = tuple([coverage[k]] + components)
                    ligatures[sequence] = lig_glyph
    return ligatures


def is_regional_indicator(codepoint):
    return 0x1F1E6 <= codepoint <= 0x1F1FF


def is_emoji_sequence(codepoints):
    if len(codepoints) < 2:
        return False
    if len(codepoints) == 2 and all(is_regional_indicator(cp) for cp in codepoints):
        return True
    if codepoints[-1] == 0x20E3 and chr(codepoints[0]) in "0123456789#*":
        return True
    if 0x200D in codepoints:
        return True
    if any(0x1F3FB <= cp <= 0x1F3FF for cp in codepoints):
        return True
    if any(cp in (0xFE0E, 0xFE0F) for cp in codepoints):
        return True
    return False


def parse_cblc_sizes(cblc_bytes):
    if len(cblc_bytes) < 8:
        return []
    num_sizes = read_u32(cblc_bytes, 4)
    sizes = []
    for i in range(num_sizes):
        base = 8 + i * 48
        if base + 48 > len(cblc_bytes):
            break
        sizes.append(
            {
                "index_subtable_array_offset": read_u32(cblc_bytes, base),
                "index_subtable_array_size": read_u32(cblc_bytes, base + 4),
                "num_subtables": read_u32(cblc_bytes, base + 8),
                "start_glyph": read_u16(cblc_bytes, base + 40),
                "end_glyph": read_u16(cblc_bytes, base + 42),
                "ppem_x": cblc_bytes[base + 44],
                "ppem_y": cblc_bytes[base + 45],
                "bit_depth": cblc_bytes[base + 46],
                "flags": struct.unpack_from(">b", cblc_bytes, base + 47)[0],
            }
        )
    return sizes


def resolve_subtable_base(cblc_bytes, entries, array_start):
    for base in (array_start, 0):
        for entry in entries[: min(3, len(entries))]:
            sub_offset = base + entry["additional_offset"]
            if sub_offset + 8 > len(cblc_bytes):
                continue
            index_format = read_u16(cblc_bytes, sub_offset)
            image_format = read_u16(cblc_bytes, sub_offset + 2)
            if index_format in (1, 2, 3, 4, 5) and image_format in (17, 18):
                return base
    return array_start


def parse_png_payload(data, offset):
    if offset + 4 <= len(data):
        png_len = read_u32(data, offset)
        png_start = offset + 4
        png_end = png_start + png_len
        if png_end <= len(data):
            payload = data[png_start:png_end]
            if payload.startswith(PNG_SIG):
                return payload
    sig_index = data.find(PNG_SIG, offset)
    if sig_index != -1:
        return data[sig_index:]
    return None


def parse_format_17(data):
    if len(data) < 9:
        return None
    height = data[0]
    width = data[1]
    bearing_x = read_i8(data[2])
    bearing_y = read_i8(data[3])
    png = parse_png_payload(data, 5)
    if png is None:
        return None
    return width, height, bearing_x, bearing_y, png


def parse_format_18(data):
    if len(data) < 12:
        return None
    height = data[0]
    width = data[1]
    bearing_x = read_i8(data[2])
    bearing_y = read_i8(data[3])
    png = parse_png_payload(data, 8)
    if png is None:
        return None
    return width, height, bearing_x, bearing_y, png


def load_resample_filter(name):
    try:
        from PIL import Image
    except ImportError as exc:
        raise RuntimeError(
            "Pillow is required for --output-ppem scaling; install with: pip install Pillow"
        ) from exc
    resampling = getattr(Image, "Resampling", Image)
    mapping = {
        "nearest": resampling.NEAREST,
        "bilinear": resampling.BILINEAR,
        "bicubic": resampling.BICUBIC,
        "lanczos": resampling.LANCZOS,
    }
    return Image, mapping[name]


def resize_png(png_bytes, target_width, target_height, resample_filter, image_module):
    with image_module.open(io.BytesIO(png_bytes)) as image:
        image = image.convert("RGBA")
        image = image.resize((target_width, target_height), resample=resample_filter)
        buf = io.BytesIO()
        image.save(buf, format="PNG")
        return buf.getvalue()


def scale_dim(value, scale, label):
    if value == 0:
        return 0
    scaled = int(round(value * scale))
    if scaled < 1:
        scaled = 1
    if scaled > 255:
        raise ValueError(f"{label} scaled out of range: {scaled}")
    return scaled


def scale_bearing(value, scale, label):
    scaled = int(round(value * scale))
    if scaled < -128 or scaled > 127:
        raise ValueError(f"{label} scaled out of range: {scaled}")
    return scaled


def extract_glyphs(cblc_bytes, cbdt_bytes, size_table, limit=None):
    array_start = size_table["index_subtable_array_offset"]
    array_size = size_table["index_subtable_array_size"]
    num_subtables = size_table["num_subtables"]
    if array_start + array_size > len(cblc_bytes):
        raise ValueError("index subtable array out of range")
    entries = []
    for i in range(num_subtables):
        entry_offset = array_start + i * 8
        if entry_offset + 8 > len(cblc_bytes):
            break
        entries.append(
            {
                "first_glyph": read_u16(cblc_bytes, entry_offset),
                "last_glyph": read_u16(cblc_bytes, entry_offset + 2),
                "additional_offset": read_u32(cblc_bytes, entry_offset + 4),
            }
        )
    base = resolve_subtable_base(cblc_bytes, entries, array_start)
    glyphs = {}
    for entry in entries:
        sub_offset = base + entry["additional_offset"]
        if sub_offset + 8 > len(cblc_bytes):
            continue
        index_format = read_u16(cblc_bytes, sub_offset)
        image_format = read_u16(cblc_bytes, sub_offset + 2)
        image_data_offset = read_u32(cblc_bytes, sub_offset + 4)
        if index_format != 1 or image_format not in (17, 18):
            continue
        num_glyphs = entry["last_glyph"] - entry["first_glyph"] + 1
        offset_array_offset = sub_offset + 8
        offsets_end = offset_array_offset + 4 * (num_glyphs + 1)
        if offsets_end > len(cblc_bytes):
            continue
        offsets = struct.unpack_from(
            ">" + "I" * (num_glyphs + 1), cblc_bytes, offset_array_offset
        )
        for i in range(num_glyphs):
            if limit is not None and len(glyphs) >= limit:
                return glyphs
            start = image_data_offset + offsets[i]
            end = image_data_offset + offsets[i + 1]
            if start >= end or end > len(cbdt_bytes):
                continue
            data = cbdt_bytes[start:end]
            if image_format == 17:
                parsed = parse_format_17(data)
            else:
                parsed = parse_format_18(data)
            if parsed is None:
                continue
            width, height, bearing_x, bearing_y, png = parsed
            glyph_id = entry["first_glyph"] + i
            glyphs[glyph_id] = {
                "width": width,
                "height": height,
                "bearing_x": bearing_x,
                "bearing_y": bearing_y,
                "png": png,
            }
    return glyphs


def main():
    repo_root = pathlib.Path(__file__).resolve().parents[1]
    default_font = repo_root / "res/fonts/AppleColorEmoji.ttf"
    default_out = repo_root / "res/emoji_png/apple_color_emoji"

    parser = argparse.ArgumentParser(
        description="Extract AppleColorEmoji CBDT PNGs into res/emoji_png/apple_color_emoji."
    )
    parser.add_argument("--input", default=str(default_font), help="Path to AppleColorEmoji.ttf.")
    parser.add_argument(
        "--output-dir", default=str(default_out), help="Output directory for PNGs."
    )
    parser.add_argument(
        "--ppem",
        type=int,
        default=None,
        help="Override source strike size (ppem) to extract.",
    )
    parser.add_argument(
        "--output-ppem",
        type=int,
        default=None,
        help="Scale PNGs/metrics to this ppem after extraction.",
    )
    parser.add_argument(
        "--resample",
        choices=("nearest", "bilinear", "bicubic", "lanczos"),
        default="lanczos",
        help="Resample filter for --output-ppem scaling.",
    )
    parser.add_argument(
        "--limit",
        type=int,
        default=None,
        help="Only extract the first N glyphs (debug).",
    )
    parser.add_argument(
        "--clean", action="store_true", help="Delete the output directory before writing."
    )
    args = parser.parse_args()

    font_path = pathlib.Path(args.input)
    if not font_path.exists():
        print(f"font not found: {font_path}", file=sys.stderr)
        return 1

    font_bytes = font_path.read_bytes()
    tables = parse_table_directory(font_bytes)
    if "CBLC" not in tables or "CBDT" not in tables:
        print("missing CBLC/CBDT tables in font", file=sys.stderr)
        return 1

    cblc_offset, cblc_length = tables["CBLC"]
    cbdt_offset, cbdt_length = tables["CBDT"]
    cblc_bytes = font_bytes[cblc_offset : cblc_offset + cblc_length]
    cbdt_bytes = font_bytes[cbdt_offset : cbdt_offset + cbdt_length]

    size_tables = parse_cblc_sizes(cblc_bytes)
    if not size_tables:
        print("no CBLC size tables found", file=sys.stderr)
        return 1

    if args.ppem is None:
        if args.output_ppem is None:
            size_table = max(size_tables, key=lambda item: item["ppem_x"])
        else:
            size_table = min(
                size_tables, key=lambda item: abs(item["ppem_x"] - args.output_ppem)
            )
    else:
        matches = [item for item in size_tables if item["ppem_x"] == args.ppem]
        if not matches:
            available = ", ".join(str(item["ppem_x"]) for item in size_tables)
            print(f"ppem {args.ppem} not found; available: {available}", file=sys.stderr)
            return 1
        size_table = matches[0]

    source_ppem = size_table["ppem_x"]
    if source_ppem <= 0:
        print("source ppem is zero; font strike invalid", file=sys.stderr)
        return 1
    output_ppem = args.output_ppem if args.output_ppem is not None else source_ppem
    if output_ppem <= 0 or output_ppem > 255:
        print(f"output ppem {output_ppem} out of range (1-255)", file=sys.stderr)
        return 1
    scale = output_ppem / source_ppem

    output_dir = pathlib.Path(args.output_dir)
    if args.clean and output_dir.exists():
        shutil.rmtree(output_dir)
    output_dir.mkdir(parents=True, exist_ok=True)

    glyphs = extract_glyphs(
        cblc_bytes, cbdt_bytes, size_table, limit=args.limit
    )
    if not glyphs:
        print("no glyphs extracted", file=sys.stderr)
        return 1

    image_module = None
    resample_filter = None
    if scale != 1.0:
        try:
            image_module, resample_filter = load_resample_filter(args.resample)
        except RuntimeError as exc:
            print(str(exc), file=sys.stderr)
            return 1

    records = []
    try:
        for glyph_id in sorted(glyphs.keys()):
            info = glyphs[glyph_id]
            width = info["width"]
            height = info["height"]
            bearing_x = info["bearing_x"]
            bearing_y = info["bearing_y"]
            png_bytes = info["png"]
            if scale != 1.0:
                width = scale_dim(width, scale, f"width for gid {glyph_id:04x}")
                height = scale_dim(height, scale, f"height for gid {glyph_id:04x}")
                bearing_x = scale_bearing(
                    bearing_x, scale, f"bearing_x for gid {glyph_id:04x}"
                )
                bearing_y = scale_bearing(
                    bearing_y, scale, f"bearing_y for gid {glyph_id:04x}"
                )
                png_bytes = resize_png(
                    png_bytes, width, height, resample_filter, image_module
                )
            png_path = output_dir / f"gid_{glyph_id:04x}.png"
            png_path.write_bytes(png_bytes)
            records.append(
                {
                    "glyph_id": glyph_id,
                    "width": width,
                    "height": height,
                    "bearing_x": bearing_x,
                    "bearing_y": bearing_y,
                }
            )
    except (RuntimeError, ValueError) as exc:
        print(str(exc), file=sys.stderr)
        return 1

    ligatures = parse_gsub_ligatures(font_bytes)
    component_gids = set()
    for sequence in ligatures:
        component_gids.update(sequence)

    cmap_glyph_ids = set(glyphs.keys())
    cmap_glyph_ids.update(component_gids)
    codepoints = parse_cmap_mapping(font_bytes, cmap_glyph_ids)
    codepoint_map = {}
    gid_to_codepoint = {}
    for codepoint, glyph_id in codepoints.items():
        if glyph_id == 0:
            continue
        if glyph_id not in gid_to_codepoint or codepoint < gid_to_codepoint[glyph_id]:
            gid_to_codepoint[glyph_id] = codepoint
        if glyph_id in glyphs:
            try:
                codepoint_map[chr(codepoint)] = glyph_id
            except ValueError:
                continue

    sequences = {}
    for sequence, lig_glyph in ligatures.items():
        if lig_glyph not in glyphs:
            continue
        codepoints = []
        missing = False
        for gid in sequence:
            cp = gid_to_codepoint.get(gid)
            if cp is None:
                missing = True
                break
            codepoints.append(cp)
        if missing or not is_emoji_sequence(codepoints):
            continue
        seq_str = "".join(chr(cp) for cp in codepoints)
        sequences[seq_str] = lig_glyph
    metadata = {
        "strike_ppem": output_ppem,
        "glyphs": records,
        "codepoints": codepoint_map,
        "sequences": sequences,
    }
    metadata_path = output_dir / "metadata.json"
    metadata_path.write_text(json.dumps(metadata, ensure_ascii=True, separators=(",", ":")))

    if output_ppem == source_ppem:
        print(f"extracted {len(records)} glyphs at ppem={source_ppem} -> {output_dir}")
    else:
        print(
            "extracted {} glyphs at ppem={} scaled to ppem={} -> {}".format(
                len(records), source_ppem, output_ppem, output_dir
            )
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
