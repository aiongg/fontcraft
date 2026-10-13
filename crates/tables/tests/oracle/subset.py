"""Compare `fontcraft-cli subset` with fontTools' pyftsubset on every compiled corpus font.

Usage (fontTools' Python, isolated mode):
    python -I subset.py FONTCRAFT_CLI CORPUS_DIR OUT_DIR [--match TEXT]

For each font and each of four selections it runs both subsetters with the same options,
then compares the old-to-new glyph index map and the TTX dump of every core table
(`cmap`, `glyf`, `CFF `, `hmtx`, `vmtx`, `post`, `name`, `OS/2`, `maxp`, `head`, `hhea`,
`vhea`, `gasp`, `cvt `, `fpgm`, `prep`, `VDMX`). The tables FontCraft does not subset yet
(layout, variation, colour and the other glyph-indexed tables) are dropped on both sides, so
the comparison covers the core subsetter only. Before dumping, CFF subroutines are inlined on
both sides (FontCraft keeps the subroutine INDEXes, pyftsubset prunes them), and
`head.checkSumAdjustment` and `head.indexToLocFormat` are ignored.

Prints one summary line per selection and writes every difference to OUT_DIR/report.json.
"""

import io
import json
import os
import subprocess
import sys
import traceback
from collections import Counter

from fontTools import subset
from fontTools.ttLib import TTFont

CORE = ["cmap", "glyf", "CFF ", "hmtx", "vmtx", "post", "name", "OS/2", "maxp", "head", "hhea", "vhea",
        "gasp", "cvt ", "fpgm", "prep", "VDMX"]
NOT_YET = ["GSUB", "GPOS", "GDEF", "fvar", "avar", "gvar", "cvar", "HVAR", "VVAR", "MVAR", "STAT", "CFF2", "VARC",
           "COLR", "CPAL", "SVG", "CBDT", "CBLC", "sbix", "MATH", "BASE", "kern", "EBLC", "EBDT", "VORG", "hdmx"]
ASCII = "".join(chr(c) for c in range(0x20, 0x7F))

SELECTIONS = {
    "ascii-text": {"text": ASCII, "cli": ["--text", ASCII]},
    "latin1-range": {"unicodes": list(range(0xA0, 0x100)), "cli": ["--unicodes", "A0-FF"]},
    "gid-list": {"gids": list(range(1, 21)) + [40, 60], "cli": ["--gids", "1-20,40,60", "--ignore-missing-glyphs"],
                 "opts": {"ignore_missing_glyphs": True}},
    "mixed-retain-gids": {"text": "Ab(1)", "unicodes": [0x2013, 0x2014], "gids": [5],
                          "cli": ["--text", "Ab(1)", "--unicodes", "2013,2014", "--gids", "5", "--retain-gids",
                                  "--no-hinting", "--ignore-missing-glyphs"],
                          "opts": {"retain_gids": True, "hinting": False, "ignore_missing_glyphs": True}},
}


def fonts(corpus):
    out = []
    for root, _, files in os.walk(corpus, followlinks=True):
        for f in files:
            if f.lower().endswith((".ttf", ".otf")):
                out.append(os.path.join(root, f))
    return sorted(out)


def oracle(path, sel, out_path):
    options = subset.Options()
    options.drop_tables += NOT_YET
    options.layout_closure = False
    for k, v in sel.get("opts", {}).items():
        setattr(options, k, v)
    font = subset.load_font(path, options)
    s = subset.Subsetter(options=options)
    s.populate(glyphs=[], gids=sel.get("gids", []), unicodes=sel.get("unicodes", []), text=sel.get("text", ""))
    s.subset(font)
    index_map = {int(k): int(v) for k, v in s.glyph_index_map.items()}
    subset.save_font(font, out_path, options)
    return index_map


def ours(cli, path, sel, out_path):
    r = subprocess.run([cli, "subset", path, *sel["cli"], "-o", out_path, "--json"], capture_output=True, text=True,
                       timeout=300)
    if r.returncode != 0:
        raise RuntimeError((r.stderr or r.stdout).strip().splitlines()[-1] if (r.stderr or r.stdout).strip() else
                           f"exit {r.returncode}")
    result = json.loads(r.stdout)
    return {int(o): int(n) for o, n in result["glyphMap"]}


def dump(path, tag):
    font = TTFont(path, lazy=False)
    if tag not in font:
        return None
    if tag == "CFF ":
        font["CFF "].cff.desubroutinize()
    buf = io.StringIO()
    font.saveXML(buf, tables=[tag])
    lines = []
    for line in buf.getvalue().splitlines():
        s = line.strip()
        if s.startswith(("<?xml", "<ttFont", "</ttFont", "<checkSumAdjustment", "<indexToLocFormat")):
            continue
        lines.append(s)
    return "\n".join(lines)


def main():
    cli, corpus, out_dir = sys.argv[1:4]
    match = sys.argv[sys.argv.index("--match") + 1] if "--match" in sys.argv else None
    os.makedirs(out_dir, exist_ok=True)
    report = {}
    paths = [p for p in fonts(corpus) if match is None or match in p]
    for name, sel in SELECTIONS.items():
        counts = Counter()
        table_diffs = Counter()
        causes = Counter()
        details = []
        for path in paths:
            a = os.path.join(out_dir, "oracle.bin")
            b = os.path.join(out_dir, "ours.bin")
            for f in (a, b):
                if os.path.exists(f):
                    os.replace(f, f + ".old")
            try:
                theirs_map = oracle(path, sel, a)
            except Exception as e:  # noqa: BLE001 - any oracle failure skips the font
                counts["oracle failed"] += 1
                causes["oracle: " + type(e).__name__] += 1
                continue
            try:
                our_map = ours(cli, path, sel, b)
            except Exception as e:  # noqa: BLE001
                counts["ours failed"] += 1
                msg = str(e)
                causes["ours: " + msg[:90]] += 1
                details.append({"font": path, "failed": msg})
                continue
            entry = {"font": path}
            if our_map == theirs_map:
                counts["map equal"] += 1
            else:
                counts["map different"] += 1
                only_ours = sorted(set(our_map) - set(theirs_map))[:10]
                only_theirs = sorted(set(theirs_map) - set(our_map))[:10]
                entry["map"] = {"onlyOurs": only_ours, "onlyTheirs": only_theirs, "ours": len(our_map),
                                "theirs": len(theirs_map)}
            different = []
            for tag in CORE:
                try:
                    x, y = dump(a, tag), dump(b, tag)
                except Exception as e:  # noqa: BLE001
                    different.append(tag)
                    entry.setdefault("dumpErrors", []).append(f"{tag}: {type(e).__name__}: {e}"[:200])
                    continue
                if x != y:
                    different.append(tag)
                    if x is not None and y is not None:
                        xl, yl = x.splitlines(), y.splitlines()
                        first = next((i for i, (p, q) in enumerate(zip(xl, yl)) if p != q), min(len(xl), len(yl)))
                        entry.setdefault("firstDiff", {})[tag] = {
                            "theirs": xl[first] if first < len(xl) else "<end>",
                            "ours": yl[first] if first < len(yl) else "<end>"}
                    else:
                        entry.setdefault("presence", {})[tag] = {"theirs": x is not None, "ours": y is not None}
            if different:
                counts["tables different"] += 1
                table_diffs.update(different)
                entry["tables"] = different
            else:
                counts["tables equal"] += 1
            if len(entry) > 1:
                details.append(entry)
        report[name] = {"counts": dict(counts), "tables": dict(table_diffs), "causes": dict(causes), "details": details}
        print(f"{name}: {dict(counts)} tables differing: {dict(table_diffs)}")
        for cause, n in causes.most_common(8):
            print(f"    {n:4d}  {cause}")
    with open(os.path.join(out_dir, "report.json"), "w") as f:
        json.dump(report, f, indent=1)


if __name__ == "__main__":
    try:
        main()
    except Exception:
        traceback.print_exc()
        sys.exit(1)
