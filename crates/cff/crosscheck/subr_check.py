"""The subroutiniser's measure against cffsubr (M12.25). Run locally; not part of the gate.

For every OpenType font with a CFF or CFF2 table under the given paths (default: corpus/):

1. cffsubr (AFDKO's tx subroutiniser through its Python wrapper) subroutinises a copy;
2. the Rust example `subr_measure` subroutinises it with crates/cff/src/subr, checks that the
   result expands to the input and that read-fonts draws the same outlines, and compares the
   charstring and subroutine bytes with cffsubr's;
3. fontTools reads every font we wrote, decodes and draws every charstring with logging
   captured, and compares the drawing with the original font's.

The measure passes when every font has equal outlines, no decoding warning, and our bytes are
at most 2% above cffsubr's.

Usage (with the fontmake tool environment's Python, which has fontTools and cffsubr; tx needs a
writable temporary folder):

    python -I crates/cff/crosscheck/subr_check.py OUT_DIR [FONT_OR_DIR ...]
"""

import io
import logging
import subprocess
import sys
from pathlib import Path

import cffsubr
from fontTools.pens.recordingPen import DecomposingRecordingPen
from fontTools.ttLib import TTFont

ROOT = Path(__file__).resolve().parents[3]


def cff_fonts(paths):
    for p in paths:
        files = [p] if p.is_file() else sorted(p.rglob("*.otf"))
        for f in files:
            try:
                font = TTFont(f, lazy=True)
            except Exception:
                continue
            if "CFF " in font or "CFF2" in font:
                yield f


def drawings(path):
    font = TTFont(path)
    glyphs = font.getGlyphSet()
    out = {}
    for name in font.getGlyphOrder():
        pen = DecomposingRecordingPen(glyphs)
        glyphs[name].draw(pen)
        out[name] = (pen.value, glyphs[name].width)
    return out


def main():
    out_dir = Path(sys.argv[1])
    paths = [Path(a) for a in sys.argv[2:]] or [ROOT / "corpus"]
    ref_dir, ours_dir = out_dir / "cffsubr", out_dir / "ours"
    ref_dir.mkdir(parents=True, exist_ok=True)
    ours_dir.mkdir(parents=True, exist_ok=True)

    fonts, names = [], set()
    for f in cff_fonts(paths):
        if f.name in names:
            continue
        try:
            font = TTFont(f)
            cffsubr.subroutinize(font)
            font.save(ref_dir / f.name)
        except Exception as e:
            print(f"cffsubr failed on {f}: {e}", file=sys.stderr)
            continue
        names.add(f.name)
        fonts.append(f)

    cmd = ["cargo", "run", "-q", "--release", "-p", "fontcraft-cff", "--example", "subr_measure", "--",
           "--reference", str(ref_dir), "--out", str(ours_dir)] + [str(f) for f in fonts]
    table = subprocess.run(cmd, cwd=ROOT, check=True, capture_output=True, text=True).stdout
    (out_dir / "measure.tsv").write_text(table)

    rows = [line.split("\t") for line in table.splitlines()[1:]]
    ok = [r for r in rows if len(r) == 9]
    errors = [r for r in rows if len(r) != 9]
    different = [r[0] for r in ok if r[7] != "equal"]
    over = [(r[0], r[6]) for r in ok if r[6] != "-" and float(r[6]) > 1.02]
    ours_total = sum(int(r[4]) for r in ok if r[5] != "-")
    ref_total = sum(int(r[5]) for r in ok if r[5] != "-")

    capture = io.StringIO()
    handler = logging.StreamHandler(capture)
    logging.getLogger("fontTools").addHandler(handler)
    logging.getLogger("fontTools").setLevel(logging.WARNING)
    warned, redrawn_differ = [], []
    for f in fonts:
        ours = ours_dir / f.name
        if not ours.exists():
            continue
        capture.truncate(0)
        capture.seek(0)
        try:
            same = drawings(ours) == drawings(f)
        except Exception as e:
            warned.append((f.name, f"exception: {e}"))
            continue
        if capture.getvalue():
            warned.append((f.name, capture.getvalue().strip().splitlines()[0]))
        if not same:
            redrawn_differ.append(f.name)

    print(f"fonts: {len(fonts)}; measured: {len(ok)}; errors: {len(errors)}")
    print(f"outlines different (read-fonts): {len(different)}; (fontTools): {len(redrawn_differ)}")
    print(f"fontTools warnings: {len(warned)}")
    print(f"more than 2% above cffsubr: {len(over)}")
    if ref_total:
        print(f"bytes: ours {ours_total}, cffsubr {ref_total}, ratio {ours_total / ref_total:.4f}")
    for name, msg in warned[:20]:
        print(f"  warning {name}: {msg}")
    for name, ratio in over[:20]:
        print(f"  over {name}: {ratio}")
    for r in errors[:20]:
        print(f"  error {' '.join(r)}")


if __name__ == "__main__":
    main()
