"""Cross-check fontcraft-container against fontTools' WOFF and WOFF2 writers and readers. Run
locally; not part of the gate.

For every TTF and OTF under corpus/ (or the fonts named), the Rust example `wrap` writes a WOFF
and an untransformed WOFF2. fontTools writes the same two files from the same font:

    WOFF:  TTFont(path, recalcTimestamp=False).flavor = "woff", then save()
    WOFF2: fontTools.ttLib.woff2.compress(path, out, transform_tables=set())

The script reports, per container, how many files are byte-identical; how many differ only in
the compressed streams (same header fields, same directory, same tables once decompressed:
zlib and Brotli implementations may choose different encodings of the same data, and for WOFF a
table one zlib shrinks by a byte another may leave raw); and how many of our files fontTools
reads back to the same tables as its own.

One fontTools behaviour is not a container choice and is set aside: its WOFF2 writer decompiles
and recompiles `head` (to set bit 11 of `flags`), and decompiling moves a `created` or
`modified` timestamp below 0x7C259DC0 by that amount, reading it as Unix time. Fonts where that
happened are compared with `head`'s timestamps and checksum adjustment masked, and counted.

It needs fontTools and the Python Brotli module (the fontmake tool environment has no Brotli):

    uv venv VENV && uv pip install --python VENV/bin/python fonttools==4.66.1 brotli
    VENV/bin/python -I crates/container/crosscheck/crosscheck.py OUT_DIR [FONT ...]
"""

import io
import struct
import subprocess
import sys
import zlib
from pathlib import Path

import brotli
from fontTools.ttLib import TTFont, woff2

REPO = Path(__file__).resolve().parents[3]


def corpus_fonts():
    out = []
    for p in sorted((REPO / "corpus").rglob("*")):
        if ".git" in p.parts or not p.is_file():
            continue
        if p.suffix.lower() in (".ttf", ".otf"):
            out.append(p)
    return out


def reference_woff(path):
    font = TTFont(str(path), recalcTimestamp=False, recalcBBoxes=False)
    font.flavor = "woff"
    buf = io.BytesIO()
    font.save(buf)
    return buf.getvalue()


def reference_woff2(path):
    buf = io.BytesIO()
    woff2.compress(str(path), buf, transform_tables=set())
    return buf.getvalue()


def woff_parts(data):
    """Header fields that do not depend on the compressor, and the directory with the tables
    inflated."""
    (sig, flavor, length, num, res, total, major, minor, mo, ml, mol, po, pl) = struct.unpack(
        ">4sLLHHLHHLLLLL", data[:44]
    )
    tables = []
    for i in range(num):
        tag, off, comp, orig, cs = struct.unpack(">4sLLLL", data[44 + 20 * i : 64 + 20 * i])
        raw = data[off : off + comp]
        tables.append((tag, orig, cs, raw if comp == orig else zlib.decompress(raw)))
    meta = zlib.decompress(data[mo : mo + ml]) if ml else b""
    return (sig, flavor, num, res, total, major, minor, mol, pl, tables, meta, data[po : po + pl])


def raw_choices(data):
    """How many tables a WOFF file stores raw."""
    num = struct.unpack(">H", data[12:14])[0]
    return sum(
        1 for i in range(num) if data[52 + 20 * i : 56 + 20 * i] == data[56 + 20 * i : 60 + 20 * i]
    )


def masked_head(head):
    return head[:8] + bytes(4) + head[12:20] + bytes(16) + head[36:]


def woff2_parts(data, mask_head=False):
    (sig, flavor, length, num, res, total, comp, major, minor, mo, ml, mol, po, pl) = struct.unpack(
        ">4sLLHHLLHHLLLLL", data[:48]
    )
    reader = woff2.WOFF2Reader(io.BytesIO(data))
    start = 48 + sum(len(e.toString()) for e in reader.tables.values())
    stream = brotli.decompress(data[start : start + comp])
    tables = []
    for tag, e in reader.tables.items():
        t = stream[e.offset : e.offset + e.length]
        tables.append((tag, masked_head(t) if mask_head and tag == "head" else t))
    meta = brotli.decompress(data[mo : mo + ml]) if ml else b""
    return (sig, flavor, num, res, total, major, minor, mol, pl, data[48:start], tables, meta, data[po : po + pl])


def tables_of(data, check=0, mask_head=False):
    font = TTFont(io.BytesIO(data), checkChecksums=check)
    out = {tag: font.reader[tag] for tag in font.reader.keys()}
    if mask_head and "head" in out:
        out["head"] = masked_head(out["head"])
    return out


def main():
    out = Path(sys.argv[1]).resolve()
    out.mkdir(parents=True, exist_ok=True)
    fonts = [Path(p) for p in sys.argv[2:]] or corpus_fonts()
    run = subprocess.run(
        ["cargo", "run", "-q", "-p", "fontcraft-container", "--example", "wrap", "--", str(out)]
        + [str(p) for p in fonts],
        cwd=REPO,
        capture_output=True,
        text=True,
    )
    ours_failed = {int(line.split("\t")[0]): line for line in run.stderr.splitlines() if "\t" in line}
    counts = {
        k: 0
        for k in ("woff same", "woff streams", "woff read", "woff2 same", "woff2 streams", "woff2 read", "timestamps", "raw")
    }
    problems = []
    for i, path in enumerate(fonts):
        name = path.relative_to(REPO) if path.is_relative_to(REPO) else path
        if i in ours_failed:
            problems.append(f"{name}: ours refused: {ours_failed[i]}")
            continue
        for kind in ("woff", "woff2"):
            try:
                theirs = reference_woff(path) if kind == "woff" else reference_woff2(path)
            except Exception as e:  # the reference refuses some damaged test fonts
                problems.append(f"{name} ({kind}): reference refused: {e}")
                continue
            ours = (out / f"{i}.{kind}").read_bytes()
            mask = False
            if kind == "woff2":
                source_head = TTFont(str(path)).reader["head"] if "head" in TTFont(str(path)) else b""
                mask = tables_of(theirs).get("head", b"")[20:36] != source_head[20:36]
                counts["timestamps"] += mask
                parts = lambda d: woff2_parts(d, mask)
            else:
                parts = woff_parts
                counts["raw"] += raw_choices(ours) != raw_choices(theirs)
            if ours == theirs:
                counts[f"{kind} same"] += 1
            elif parts(ours) == parts(theirs):
                counts[f"{kind} streams"] += 1
            else:
                problems.append(f"{name} ({kind}): differs beyond the compressed streams")
            try:
                if tables_of(ours, 2 if kind == "woff" else 0, mask) == tables_of(theirs, 0, mask):
                    counts[f"{kind} read"] += 1
                else:
                    problems.append(f"{name} ({kind}): fontTools reads other tables from ours")
            except Exception as e:
                problems.append(f"{name} ({kind}): fontTools cannot read ours: {e}")
    print(f"{len(fonts)} fonts")
    for kind in ("woff", "woff2"):
        print(
            f"  {kind.upper()}: {counts[kind + ' same']} byte-identical, "
            f"{counts[kind + ' streams']} identical apart from the compressed streams, "
            f"{counts[kind + ' read']} read back by fontTools to its own tables"
        )
    print(f"  WOFF files where the zlib implementations disagree on which tables to store raw: {counts['raw']}")
    print(f"  WOFF2 files compared with head timestamps masked (fontTools moved them): {counts['timestamps']}")
    for p in problems:
        print("  " + p)


if __name__ == "__main__":
    main()
