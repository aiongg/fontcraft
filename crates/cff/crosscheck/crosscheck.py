"""Cross-check fontcraft-cff against the reference CFF writer (ufo2ft and fontTools, as fontmake
runs them with --optimize-cff 1). Run locally; not part of the gate.

For every source, the reference compiles an OTF while a hook records exactly what its charstring
pen receives (glyph order, advance widths, drawing calls after quadratic-to-cubic conversion).
The recording and the Top/Private DICT values the reference wrote become a JSON file; the Rust
example `cff_from_json` builds a CFF table from it, computing widths, FontBBox, charset,
charstrings and offsets itself; the two tables are compared byte for byte.

Usage (with the fontmake tool environment's Python, which has ufo2ft and ufoLib2):

    python -I crates/cff/crosscheck/crosscheck.py OUT_DIR [SOURCE.ufo ...]

With no sources it uses every UFO under corpus/ plus three synthetic fonts.
"""

import io
import json
import random
import subprocess
import sys
from pathlib import Path

import ufo2ft
import ufoLib2
from fontTools.cffLib import CFFFontSet
from fontTools.pens.basePen import BasePen
from fontTools.pens.pointPen import PointToSegmentPen
from fontTools.ttLib import TTFont
from ufo2ft import outlineCompiler
from ufo2ft.constants import CFFOptimization

REPO = Path(__file__).resolve().parents[3]
TOP_KEYS = ["version", "Notice", "Copyright", "FullName", "FamilyName", "Weight", "isFixedPitch",
            "ItalicAngle", "UnderlinePosition", "UnderlineThickness", "PaintType", "FontMatrix",
            "StrokeWidth"]
COMPUTED_TOP = {"FontBBox", "charset", "CharStrings", "Private", "Encoding", "CharstringType"}
PRIVATE_KEYS = ["BlueValues", "OtherBlues", "FamilyBlues", "FamilyOtherBlues", "BlueScale",
                "BlueShift", "BlueFuzz", "StdHW", "StdVW", "StemSnapH", "StemSnapV", "ForceBold",
                "LanguageGroup"]
COMPUTED_PRIVATE = {"defaultWidthX", "nominalWidthX"}


class Recorder(BasePen):
    def __init__(self, glyph_set):
        super().__init__(glyph_set)
        self.ops = []

    def _moveTo(self, p):
        self.ops.append(["M", *p])

    def _lineTo(self, p):
        self.ops.append(["L", *p])

    def _curveToOne(self, a, b, c):
        self.ops.append(["C", *a, *b, *c])

    def _closePath(self):
        pass

    def _endPath(self):
        pass


RECORDED = {}
_original_compile_glyphs = outlineCompiler.OutlineOTFCompiler.compileGlyphs


def _recording_compile_glyphs(self):
    order, glyphs = [], []
    for name in self.glyphOrder:
        glyph = self.allGlyphs[name]
        pen = Recorder(self.allGlyphs)
        if glyph.lib.get(outlineCompiler.EXPLICIT_CLOSING_LINE_KEY):
            glyph.drawPoints(PointToSegmentPen(pen, outputImpliedClosingLine=True))
        else:
            glyph.draw(pen)
        order.append(name)
        glyphs.append({"advance": float(glyph.width), "path": pen.ops})
    RECORDED["glyphs"] = glyphs
    return _original_compile_glyphs(self)


outlineCompiler.OutlineOTFCompiler.compileGlyphs = _recording_compile_glyphs


def plain(v):
    if isinstance(v, bytes):
        return v.decode("latin-1")
    if isinstance(v, list):
        return [plain(x) for x in v]
    return v


def describe(ufo, tolerance):
    """Compile with the reference; return (json description, reference CFF bytes, notes)."""
    RECORDED.clear()
    otf = ufo2ft.compileOTF(ufo, optimizeCFF=CFFOptimization.SPECIALIZE, roundTolerance=tolerance,
                            removeOverlaps=False)
    buf = io.BytesIO()
    otf.save(buf)
    font = TTFont(io.BytesIO(buf.getvalue()))
    data = font.reader["CFF "]
    cff = font["CFF "].cff
    top = cff.topDictIndex[0]
    notes = sorted(set(top.rawDict) - set(TOP_KEYS) - COMPUTED_TOP)
    notes += sorted(set(top.Private.rawDict) - set(PRIVATE_KEYS) - COMPUTED_PRIVATE)
    names = font.getGlyphOrder()
    glyphs = RECORDED["glyphs"]
    assert len(names) == len(glyphs), (len(names), len(glyphs))
    for name, g in zip(names, glyphs):
        g["name"] = name
    info = ufo.info
    widths = None
    if info.postscriptDefaultWidthX is not None or info.postscriptNominalWidthX is not None:
        widths = [top.Private.rawDict.get("defaultWidthX", 0), top.Private.rawDict.get("nominalWidthX", 0)]
    desc = {
        "name": cff.fontNames[0],
        "top": {k: plain(top.rawDict[k]) for k in TOP_KEYS if k in top.rawDict},
        "private": {k: plain(top.Private.rawDict[k]) for k in PRIVATE_KEYS if k in top.Private.rawDict},
        "widths": widths,
        "round_tolerance": tolerance,
        "specialise": True,
        "glyphs": glyphs,
    }
    return desc, data, notes


def first_difference(ours, theirs):
    """A short description of where two CFF tables differ, decoded with fontTools."""
    def load(data):
        cff = CFFFontSet()
        cff.decompile(io.BytesIO(data), None)
        return cff
    try:
        a, b = load(ours), load(theirs)
    except Exception as e:  # noqa: BLE001 - report, do not stop the run
        return f"cannot decode ours: {e}"
    ta, tb = a.topDictIndex[0], b.topDictIndex[0]
    for key in sorted(set(ta.rawDict) | set(tb.rawDict)):
        if key in ("CharStrings", "Private", "charset"):
            continue
        if ta.rawDict.get(key) != tb.rawDict.get(key):
            return f"Top DICT {key}: ours {ta.rawDict.get(key)!r}, reference {tb.rawDict.get(key)!r}"
    pa, pb = ta.Private.rawDict, tb.Private.rawDict
    for key in sorted(set(pa) | set(pb)):
        if pa.get(key) != pb.get(key):
            return f"Private {key}: ours {pa.get(key)!r}, reference {pb.get(key)!r}"
    if ta.charset != tb.charset:
        return "charset names differ"
    csa, csb = ta.CharStrings, tb.CharStrings
    for name in tb.charset:
        x, y = csa[name], csb[name]
        x.decompile()
        y.decompile()
        if x.program != y.program:
            return f"charstring {name}: ours {x.program[:24]}, reference {y.program[:24]}"
    return "same content, different layout (offsets, string order or charset format)"


def synthetic_fonts():
    rng = random.Random(5176)

    def font(family, upm=1000):
        ufo = ufoLib2.Font()
        ufo.info.familyName = family
        ufo.info.styleName = "Regular"
        ufo.info.unitsPerEm = upm
        ufo.info.versionMajor, ufo.info.versionMinor = 2, 5
        return ufo

    def rect(glyph, x0, y0, x1, y1):
        pen = glyph.getPen()
        pen.moveTo((x0, y0))
        pen.lineTo((x1, y0))
        pen.lineTo((x1, y1))
        pen.lineTo((x0, y1))
        pen.closePath()

    # 1. A glyph order that is a prefix of the ISOAdobe charset: the predefined charset id.
    from fontTools.cffLib import cffStandardStrings
    std = font("Standard")
    order = list(cffStandardStrings[:60])
    for i, name in enumerate(order):
        g = std.newGlyph(name)
        g.width = 500 + 10 * (i % 3)
        if name != "space":
            rect(g, 50, 0, 450, 700 - i)
    std.lib["public.glyphOrder"] = order
    yield "synthetic-standard", std, 0.5

    # 2. Random outlines: axis-aligned and free segments, long contours, fractional points,
    #    scattered widths, blue zones and stems, 2048 units per em, an italic angle.
    rnd = font("Random", upm=2048)
    rnd.info.italicAngle = -12.5
    rnd.info.postscriptWeightName = "Regular"
    rnd.info.copyright = "Copyright 2026 nobody"
    rnd.info.postscriptBlueValues = [-12, 0, 1000, 1024]
    rnd.info.postscriptOtherBlues = [-400, -380]
    rnd.info.postscriptStemSnapH = [80, 92]
    rnd.info.postscriptStemSnapV = [96, 110]
    rnd.info.postscriptBlueScale = 0.0625
    names = [".notdef"] + [f"glyph{i:03d}" for i in range(400)]
    for gi, name in enumerate(names):
        g = rnd.newGlyph(name)
        g.width = rng.choice([0, 600, 600, 600, 1200, 1250, 2400, 3100, rng.randint(0, 4000)])
        pen = g.getPen()
        for _ in range(rng.randint(0, 3)):
            x, y = rng.uniform(-200, 1500), rng.uniform(-300, 1500)
            pen.moveTo((x, y))
            for _ in range(rng.randint(1, 60)):
                kind = rng.random()
                step = lambda: rng.choice([0, 0, rng.uniform(-300, 300), rng.randint(-200, 200)])
                if kind < 0.35:
                    x, y = x + step(), y + step()
                    pen.lineTo((x, y))
                else:
                    pts = []
                    for _ in range(3):
                        x, y = x + step(), y + step()
                        pts.append((x, y))
                    pen.curveTo(*pts)
            pen.closePath()
        if gi % 50 == 7:
            # A lone point: an open one-point contour, drawn as a bare move.
            pp = g.getPointPen()
            pp.beginPath()
            pp.addPoint((rng.randint(0, 500), rng.randint(0, 500)), "move")
            pp.endPath()
    yield "synthetic-random", rnd, 0.5
    yield "synthetic-random-tol0", rnd, 0.0
    yield "synthetic-random-tol03", rnd, 0.3

    # 3. Many glyphs: a long SID range (charset format 2) and 3-byte offsets.
    many = font("Many")
    for i in range(4000):
        g = many.newGlyph(".notdef" if i == 0 else f"u{i:05d}")
        g.width = 400 + (i % 13) * 20
        rect(g, i % 300, 0, i % 300 + 100, 500 + i % 200)
    yield "synthetic-many", many, 0.5


def corpus_sources():
    for path in sorted((REPO / "corpus").rglob("*.ufo")):
        try:
            ufo = ufoLib2.Font.open(path)
        except Exception as e:  # noqa: BLE001
            print(f"skip {path.relative_to(REPO)}: cannot open ({e})")
            continue
        yield str(path.relative_to(REPO)), ufo, 0.5
        if path.name == "Float-Regular.ufo":
            yield str(path.relative_to(REPO)) + " (tolerance 0)", ufo, 0.0


def main():
    out = Path(sys.argv[1])
    out.mkdir(parents=True, exist_ok=True)
    if len(sys.argv) > 2:
        sources = [(p, ufoLib2.Font.open(p), 0.5) for p in sys.argv[2:]]
    else:
        sources = list(synthetic_fonts()) + list(corpus_sources())
    jobs = []
    for i, (label, ufo, tolerance) in enumerate(sources):
        try:
            desc, theirs, notes = describe(ufo, tolerance)
        except Exception as e:  # noqa: BLE001
            print(f"skip {label}: the reference cannot compile it ({type(e).__name__}: {e})")
            continue
        stem = out / f"{i:03d}"
        (stem.with_suffix(".json")).write_text(json.dumps(desc))
        (stem.with_suffix(".theirs.cff")).write_bytes(theirs)
        jobs.append((label, stem, notes, len(desc["glyphs"])))
    subprocess.run(["cargo", "run", "-q", "-p", "fontcraft-cff", "--example", "cff_from_json", "--"]
                   + [str(s.with_suffix(".json")) for _, s, _, _ in jobs], cwd=REPO, check=True)
    same = different = failed = 0
    glyphs = 0
    for label, stem, notes, count in jobs:
        theirs = stem.with_suffix(".theirs.cff").read_bytes()
        ours_path = Path(str(stem) + ".ours.cff")
        if not ours_path.exists():
            failed += 1
            err = Path(str(stem) + ".ours.err").read_text()
            print(f"FAILED    {label}: {err}")
            continue
        ours = ours_path.read_bytes()
        note = f" (entries not passed through: {', '.join(notes)})" if notes else ""
        if ours == theirs:
            same += 1
            glyphs += count
        else:
            different += 1
            print(f"DIFFERENT {label}: {len(ours)} vs {len(theirs)} bytes; {first_difference(ours, theirs)}{note}")
    print(f"\n{same} identical, {different} different, {failed} failed; {glyphs} glyphs in identical tables")


if __name__ == "__main__":
    main()
