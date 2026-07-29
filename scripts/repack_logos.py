#!/usr/bin/env python3
"""Unpack HEAD logos.bin.zst, normalize pattern names for small/old logo variants,
and repack crates/multitop/data/logos.bin.zst.
"""

import subprocess
import struct
import sys
import zstd

def clean_patterns(patterns):
    new_pats = []
    for raw_p in patterns:
        p = raw_p.strip()
        # Handle mac case like mac"*"_small
        if 'mac' in p.lower():
            new_pats.extend(['mac', 'darwin', 'macos', 'apple'])
            continue
        p = p.replace('"', '').replace('*', '').strip().lower()
        for suffix in ('_small', '_old', 'old'):
            if p.endswith(suffix):
                p = p[:-len(suffix)].strip()
                break
        if p:
            new_pats.append(p)
            if p in ('mac', 'darwin'):
                new_pats.extend(['macos', 'apple'])
    return list(dict.fromkeys(new_pats))

def pack_db(entries):
    buf = bytearray()
    buf.extend(b"MTLG")
    buf.extend(struct.pack("<H", 1))  # version
    buf.extend(struct.pack("<H", len(entries)))  # count

    for ent in entries:
        patterns = ent['patterns']
        colors = ent['colors']
        lines = ent['lines']

        pattern_bytes = bytearray()
        for ps in patterns:
            ps_b = ps.encode("ascii")
            if len(ps_b) > 255:
                ps_b = ps_b[:255]
            pattern_bytes.append(len(ps_b))
            pattern_bytes.extend(ps_b)

        line_bytes = bytearray()
        for ln in lines:
            ln_b = ln.encode("utf-8")
            if len(ln_b) > 255:
                ln_b = ln_b[:255]
            line_bytes.append(len(ln_b))
            line_bytes.extend(ln_b)

        entry_payload = bytearray()
        entry_payload.append(len(patterns))
        entry_payload.extend(pattern_bytes)
        entry_payload.append(len(colors))
        entry_payload.extend(bytes(colors))
        entry_payload.extend(struct.pack("<H", len(lines)))
        entry_payload.extend(line_bytes)

        buf.extend(struct.pack("<H", len(entry_payload)))
        buf.extend(entry_payload)

    return bytes(buf)

def main():
    raw_bin = subprocess.check_output(['git', 'show', 'HEAD:crates/multitop/data/logos.bin.zst'])
    raw = zstd.decompress(raw_bin)
    count = struct.unpack_from('<H', raw, 6)[0]

    pos = 8
    entries = []
    for idx in range(count):
        entry_size = struct.unpack_from('<H', raw, pos)[0]
        entry_end = pos + 2 + entry_size
        pos += 2
        num_patterns = raw[pos]; pos += 1
        patterns = []
        for _ in range(num_patterns):
            plen = raw[pos]; pos += 1
            patterns.append(raw[pos:pos+plen].decode('ascii')); pos += plen
        num_colors = raw[pos]; pos += 1
        colors = raw[pos:pos+num_colors]; pos += num_colors
        num_lines = struct.unpack_from('<H', raw, pos)[0]; pos += 2
        lines = []
        for _ in range(num_lines):
            llen = raw[pos]; pos += 1
            line = raw[pos:pos+llen].decode('utf-8', errors='replace')
            lines.append(line)
            pos += llen

        cleaned_pats = clean_patterns(patterns)
        entries.append({
            'patterns': cleaned_pats,
            'colors': list(colors),
            'lines': lines,
        })
        pos = entry_end

    packed = pack_db(entries)
    compressed = zstd.compress(packed, 22)
    with open("crates/multitop/data/logos.bin.zst", "wb") as f:
        f.write(compressed)

    print(f"Repacked {len(entries)} logo entries -> crates/multitop/data/logos.bin.zst ({len(compressed):,} bytes)")

if __name__ == "__main__":
    main()
