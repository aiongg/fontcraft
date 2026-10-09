"""Cross-check fontcraft-container against fontTools' WOFF and WOFF2 writers and readers. Run
locally; not part of the gate.

For every TTF and OTF under corpus/ (or the fonts named), the Rust example `wrap` writes a WOFF,
an untransformed WOFF2, a WOFF2 with the default options (`glyf` and `loca` transformed) and a
WOFF2 with the `hmtx` transform too. fontTools writes the same four files from the same font:

    WOFF:  TTFont(path, recalcTimestamp=False).flavor = "woff", then save()
    WOFF2: fontTools.ttLib.woff2.compress(path, out, transform_tables=T), with T the empty set,
           None (fontTools' default: glyf and loca) and {"glyf", "loca", "hmtx"}

The script reports, per kind, how many files are byte-identical; how many differ only in the
compressed streams (same header fields, same directory, same tables once decompressed: zlib and
Brotli implementations may choose different encodings of the same data, and for WOFF a table one
zlib shrinks by a byte another may leave raw); and how many of our files fontTools reads back to
the same tables as its own. For the transformed kinds it also unwraps fontTools' files with the
Rust example `unwrap` and compares the tables with what fontTools reads from them, and does the
same for every WOFF2 file in the corpus. `head` is compared without `checkSumAdjustment` there,
since the reader lays the tables out again.

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


def corpus_fonts(suffixes=(".ttf", ".otf")):
    out = []
    for p in sorted((REPO / "corpus").rglob("*")):
        if ".git" in p.parts or not p.is_file():
            continue
        if p.suffix.lower() in suffixes:
            out.append(p)
    return out


def reference_woff(path):
    font = TTFont(str(path), recalcTimestamp=False, recalcBBoxes=False)
    font.flavor = "woff"
    buf = io.BytesIO()
    font.save(buf)
    return buf.getvalue()


def reference_woff2(path, transform):
    buf = io.BytesIO()
    woff2.compress(str(path), buf, transform_tables=transform)
    return buf.getvalue()


# (name, our file's extension, fontTools' transform_tables)
KINDS = [
    ("woff", "woff", None),
    ("woff2", "woff2", set()),
    ("woff2 transformed", "t.woff2", None),
    ("woff2 with hmtx", "h.woff2", {"glyf", "loca", "hmtx"}),
]


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


def unwrapped_tables(data):
    """The tables of an sfnt, `head` without checkSumAdjustment."""
    out = tables_of(data)
    if "head" in out:
        out["head"] = out["head"][:8] + bytes(4) + out["head"][12:]
    return out


def run_example(name, out, files):
    run = subprocess.run(
        ["cargo", "run", "-q", "-p", "fontcraft-container", "--example", name, "--", str(out)] + [str(p) for p in files],
        cwd=REPO,
        capture_output=True,
        text=True,
    )
    return {int(line.split("\t")[0]): line for line in run.stderr.splitlines() if "\t" in line}


def decoded_like_fonttools(files, out, label, counts, problems):
    """Unwraps WOFF2 `files` with our reader and compares with what fontTools reads."""
    out.mkdir(parents=True, exist_ok=True)
    failed = run_example("unwrap", out, files)
    for i, path in enumerate(files):
        if i in failed:
            problems.append(f"{label} {path}: ours refused: {failed[i]}")
            continue
        try:
            theirs = unwrapped_tables(Path(path).read_bytes())
        except Exception as e:
            problems.append(f"{label} {path}: fontTools refused: {e}")
            continue
        ours = unwrapped_tables((out / f"{i}.sfnt").read_bytes())
        if ours == theirs:
            counts[label] = counts.get(label, 0) + 1
        else:
            diff = sorted(t for t in set(ours) | set(theirs) if ours.get(t) != theirs.get(t))
            problems.append(f"{label} {path}: tables differ: {', '.join(diff)}")


def main():
    out = Path(sys.argv[1]).resolve()
    out.mkdir(parents=True, exist_ok=True)
    fonts = [Path(p) for p in sys.argv[2:]] or corpus_fonts()
    ours_failed = run_example("wrap", out, fonts)
    counts = {f"{k} {m}": 0 for k, _, _ in KINDS for m in ("same", "streams", "read")}
    counts["timestamps"] = counts["raw"] = 0
    problems = []
    reference_files = {k: [] for k, _, _ in KINDS[2:]}
    (out / "ref").mkdir(exist_ok=True)
    for i, path in enumerate(fonts):
        name = path.relative_to(REPO) if path.is_relative_to(REPO) else path
        if i in ours_failed:
            problems.append(f"{name}: ours refused: {ours_failed[i]}")
            continue
        source = TTFont(str(path))
        source_head = source.reader["head"] if "head" in source else b""
        for kind, ext, transform in KINDS:
            try:
                theirs = reference_woff(path) if kind == "woff" else reference_woff2(path, transform)
            except Exception as e:  # the reference refuses some damaged test fonts
                problems.append(f"{name} ({kind}): reference refused: {e}")
                continue
            if kind in reference_files:
                ref = out / "ref" / f"{i}.{ext}"
                ref.write_bytes(theirs)
                reference_files[kind].append(ref)
            ours = (out / f"{i}.{ext}").read_bytes()
            mask = False
            if kind != "woff":
                mask = tables_of(theirs).get("head", b"")[20:36] != source_head[20:36]
                counts["timestamps"] += mask and kind == "woff2"
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
    decoded = {}
    for kind, files in reference_files.items():
        decoded_like_fonttools(files, out / "unwrapped" / kind.replace(" ", "-"), kind, decoded, problems)
    corpus_woff2 = corpus_fonts((".woff2",))
    decoded_like_fonttools(corpus_woff2, out / "unwrapped" / "corpus", "corpus woff2", decoded, problems)
    print(f"{len(fonts)} fonts")
    for kind, _, _ in KINDS:
        print(
            f"  {kind.upper()}: {counts[kind + ' same']} byte-identical, "
            f"{counts[kind + ' streams']} identical apart from the compressed streams, "
            f"{counts[kind + ' read']} read back by fontTools to its own tables"
        )
    for kind, files in reference_files.items():
        print(f"  our reader on fontTools' {kind.upper()} files: {decoded.get(kind, 0)} of {len(files)} give fontTools' tables")
    print(f"  our reader on the corpus WOFF2 files: {decoded.get('corpus woff2', 0)} of {len(corpus_woff2)} give fontTools' tables")
    print(f"  WOFF files where the zlib implementations disagree on which tables to store raw: {counts['raw']}")
    print(f"  WOFF2 files compared with head timestamps masked (fontTools moved them): {counts['timestamps']}")
    for p in problems:
        print("  " + p)


if __name__ == "__main__":
    main()
