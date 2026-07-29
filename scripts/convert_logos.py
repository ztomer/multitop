#!/usr/bin/env python3
"""Convert neofetch ASCII art JSON to compact binary + compress with zstd.

Usage:
    python3 scripts/convert_logos.py \
        ~/.local/share/opencode/tool-output/ascii_art_final.json \
        crates/multitop/data/logos.bin.zst
"""

import json
import re
import struct
import sys

import zstd


def parse_pattern(p: str) -> str:
    """Convert a neofetch shell case pattern to a Rust glob.

    Input:  ``"Ubuntu"*``, ``"Darwin"``, ``"i3buntu"*``
    Output: ``Ubuntu*``, ``Darwin``, ``i3buntu*``
    """
    p = p.strip()
    # Strip trailing ")" or "|"
    p = p.rstrip(")")
    parts = []
    for piece in p.split("|"):
        piece = piece.strip()
        # Remove surrounding quotes
        if piece.startswith('"') and piece.endswith('"'):
            piece = piece[1:-1]
        elif piece.startswith('"'):
            piece = piece[1:]
        # Convert bash glob * (which is * in our format too)
        piece = piece.replace("*", "*")
        parts.append(piece)
    return "|".join(parts)


NEONOTE_COLOR = re.compile(r"\$\{c\d+\}")


def strip_color_markers(line: str) -> str:
    """Remove ${c1}, ${c2}, etc. from a line of ASCII art."""
    return NEONOTE_COLOR.sub("", line)


def build_logo_db(json_path: str) -> bytes:
    with open(json_path) as f:
        entries = json.load(f)

    # Filter entries with at least one valid pattern
    valid = []
    for ent in entries:
        patterns_raw = ent["case_pattern"]
        pats = []
        for p in patterns_raw.rstrip(")").split("|"):
            p = p.strip().rstrip("*").strip('"').strip()
            if not p or p == "*":
                continue
            pats.append(p.lower())
        if pats:
            valid.append((ent, pats))

    buf = bytearray()
    # Header: magic "MTLG" (4), version u16le (2), count u16le (2)
    buf.extend(b"MTLG")
    buf.extend(struct.pack("<H", 1))  # version
    buf.extend(struct.pack("<H", len(valid)))  # count

    for ent, pattern_strs in valid:
        colors_str = ent.get("set_colors", "")
        if not colors_str:
            colors = [7]
        else:
            colors = [int(x) for x in colors_str.split() if x.isdigit()]

        art = ent["ascii_art"]
        lines_raw = art.split("\n")
        lines = []
        for ln in lines_raw:
            cleaned = strip_color_markers(ln)
            cleaned = cleaned.rstrip("\n")
            if cleaned:
                lines.append(cleaned)

        # --- pack this entry ---
        pattern_bytes = bytearray()
        for ps in pattern_strs:
            ps_bytes = ps.encode("ascii")
            if len(ps_bytes) > 255:
                ps_bytes = ps_bytes[:255]
            pattern_bytes.append(len(ps_bytes))
            pattern_bytes.extend(ps_bytes)
        # pattern total length (for skip) + num_patterns
        # Pre-compute entry size (for skip)
        line_data = bytearray()
        for ln in lines:
            ln_bytes = ln.encode("utf-8")
            if len(ln_bytes) > 255:
                ln_bytes = ln_bytes[:255]
            line_data.append(len(ln_bytes))
            line_data.extend(ln_bytes)

        entry_size = (
            1  # num_patterns
            + len(pattern_bytes)
            + 1  # num_colors
            + len(colors)
            + 2  # num_lines u16
            + len(line_data)
        )

        buf.extend(struct.pack("<H", entry_size))
        buf.append(len(pattern_strs))
        buf.extend(pattern_bytes)
        buf.append(len(colors))
        buf.extend(bytes(colors))
        buf.extend(struct.pack("<H", len(lines)))
        buf.extend(line_data)

    return bytes(buf)


def zstd_compress(data: bytes, level: int = 22) -> bytes:
    """Compress with zstd at maximum level for smallest binary size."""
    return zstd.compress(data, level)


def main():
    if len(sys.argv) != 3:
        print(f"Usage: {sys.argv[0]} <input.json> <output.bin.zst>", file=sys.stderr)
        sys.exit(1)

    inpath, outpath = sys.argv[1], sys.argv[2]
    raw = build_logo_db(inpath)
    compressed = zstd_compress(raw)
    with open(outpath, "wb") as f:
        f.write(compressed)

    print(
        f"logos: {len(json.load(open(inpath)))} entries, "
        f"{len(raw):,} raw bytes, "
        f"{len(compressed):,} zstd bytes → {outpath}"
    )


if __name__ == "__main__":
    main()
