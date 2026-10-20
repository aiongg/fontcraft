"""The size and readability of variable CFF2 exports against fontmake's (M12.25c). Run locally;
not part of the gate.

For every source given (.glyphs, .glyphspackage, .designspace):

1. `fontcraft-cli compile SOURCE --variable --format otf` builds our CFF2 fonts (specialised and
   subroutinised);
2. fontmake builds `-o variable-cff2` twice: with its defaults (specialised by fontTools,
   subroutinised by cffsubr) and with `--optimize-cff 1` (specialised only);
3. fontTools draws every glyph of our fonts at the default location and at each axis's minimum
   and maximum, with logging captured: any warning or error is a failure;
4. fonts are matched by file name (else the first of each); the CFF2 table sizes are compared
   with fontmake's default build, and each of our charstrings, with its subroutines expanded, is
   compared token for token with the `--optimize-cff 1` build's.

Usage (with the fontmake tool environment's Python, which has fontTools; fontmake is taken from
the same folder):

    python -I crates/cff/crosscheck/cff2_export_check.py FONTCRAFT_CLI OUT_DIR SOURCE...
"""

import io
import logging
import subprocess
import sys
from pathlib import Path

from fontTools.pens.recordingPen import RecordingPen
from fontTools.ttLib import TTFont


def build_ours(cli, source, out):
    out.mkdir(parents=True, exist_ok=True)
    cmd = [cli, "compile", str(source), "--variable", "--format", "otf", "--output-dir", str(out)]
    r = subprocess.run(cmd, capture_output=True, text=True)
    if r.returncode != 0:
        return [], r.stderr.strip().splitlines()[-1:] or ["failed"]
    return sorted(out.glob("*.otf")), []


def build_fontmake(source, out, extra=()):
    out.mkdir(parents=True, exist_ok=True)
    fontmake = Path(sys.executable).parent / "fontmake"
    flag = "-m" if source.suffix == ".designspace" else "-g"
    cmd = [str(fontmake), flag, str(source), "-o", "variable-cff2", *extra, "--output-dir", str(out)]
    r = subprocess.run(cmd, capture_output=True, text=True)
    if r.returncode != 0:
        return [], r.stderr.strip().splitlines()[-1:] or ["failed"]
    return sorted(out.glob("*.otf")), []


def cff2_size(path):
    font = TTFont(str(path), lazy=True)
    return len(font.reader["CFF2"]) if "CFF2" in font.reader else None


def programs(path):
    """Each glyph's charstring tokens, subroutines expanded."""
    font = TTFont(str(path))
    cff = font["CFF2"].cff
    cff.desubroutinize()
    top = cff.topDictIndex[0]
    out = {}
    for name in font.getGlyphOrder():
        cs = top.CharStrings[name]
        cs.decompile()
        out[name] = list(cs.program)
    return out


def draw_problems(path):
    """Glyphs fontTools cannot draw, or draws with a logged message, at the default and at each
    axis's extremes."""
    stream = io.StringIO()
    handler = logging.StreamHandler(stream)
    root = logging.getLogger()
    root.addHandler(handler)
    problems = []
    try:
        font = TTFont(str(path))
        locations = [{}]
        for axis in font["fvar"].axes:
            locations += [{axis.axisTag: axis.minValue}, {axis.axisTag: axis.maxValue}]
        for location in locations:
            glyphs = font.getGlyphSet(location=location or None)
            for name in font.getGlyphOrder():
                try:
                    glyphs[name].draw(RecordingPen())
                except Exception as e:  # noqa: BLE001 - every failure is reported
                    problems.append(f"{name} at {location}: {e}")
    finally:
        root.removeHandler(handler)
    problems += [line for line in stream.getvalue().splitlines() if line.strip()]
    return problems


def main():
    if len(sys.argv) < 4:
        print(__doc__)
        return 2
    cli, out_dir, sources = sys.argv[1], Path(sys.argv[2]), [Path(s) for s in sys.argv[3:]]
    total_ours = total_theirs = 0
    failures = 0
    same = different = 0
    for source in sources:
        work = out_dir / source.stem
        ours, err_ours = build_ours(cli, source, work / "ours")
        theirs, err_theirs = build_fontmake(source, work / "fontmake")
        specialised, _ = build_fontmake(source, work / "fontmake-specialised", ("--optimize-cff", "1"))
        if err_ours or not ours:
            print(f"{source.name}: our build failed: {err_ours}")
            failures += 1
            continue
        for path in ours:
            problems = draw_problems(path)
            failures += bool(problems)
            for p in problems[:5]:
                print(f"{source.name}: {path.name}: {p}")
        by_name = {p.name: p for p in specialised}
        for path in ours if specialised else []:
            mine, ref = programs(path), programs(by_name.get(path.name, specialised[0]))
            for name, tokens in mine.items():
                if ref.get(name) == tokens:
                    same += 1
                else:
                    different += 1
        if err_theirs or not theirs:
            print(f"{source.name}: fontmake failed ({err_theirs}); sizes not compared")
            continue
        by_name = {p.name: p for p in theirs}
        for path in ours:
            other = by_name.get(path.name, theirs[0])
            a, b = cff2_size(path), cff2_size(other)
            if a is None or b is None:
                continue
            total_ours += a
            total_theirs += b
            print(f"{source.name}: {path.name}: CFF2 {a} bytes, fontmake {b} ({a / b:.3f})")
    print(f"charstrings equal to fontmake's specialised build: {same}, different: {different}")
    if total_theirs:
        print(f"total: ours {total_ours}, fontmake {total_theirs} ({total_ours / total_theirs:.3f}); {failures} with problems")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
