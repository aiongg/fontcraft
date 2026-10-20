"""Cross-check the glyf, gvar, variation-store and COLR steps of `fontcraft-cli optimize`
against fontTools. Run locally; not part of the gate.

Each step runs on every font under the paths given (by default corpus/) that has the table it
works on, and fontTools does the same work on the same font:

    gvar-iup:    ours `optimize FONT --gvar-iup`; theirs: every tuple's inferred deltas filled
                 in, rounded and optimised (`calcInferredDeltas`, `roundDeltas`, `optimize`, as
                 the instancer runs them), tuples without effect dropped. Each glyph's tuples
                 must list the same points with the same deltas.
    implied:     ours `optimize FONT --drop-implied-oncurves`; theirs: `dropImpliedOnCurvePoints`
                 on each glyph without instructions, given the default glyph and the glyph at the
                 peak of every gvar tuple (instanced by fontTools). The glyf points must match.
    varstore:    ours `optimize FONT --varstore`; theirs `fonttools varLib.varStore FONT OUT`.
                 The TTX of GDEF and GPOS must be equal.
    colr:        ours `optimize FONT --colr-layer-reuse`; theirs `unbuildColrV1` then
                 `buildColrV1`. The TTX of COLR must be equal. `--build N` writes N random
                 COLR 1 fonts without layer reuse (the corpus holds none).

    VENV/bin/python -I crates/tables/crosscheck/optimize.py [--cli PATH] [--build N] OUT_DIR [FONT_OR_DIR ...]

VENV needs fontTools 4.66.1 (the fontmake tool environment has it). PATH defaults to
target/release/fontcraft-cli. Prints one line per font and step, then the totals.
"""

import argparse
import os
import random
import subprocess
import sys

from fontTools.ttLib import TTFont


def fonts_under(paths):
    for p in paths:
        if os.path.isfile(p):
            yield p
            continue
        for root, _, files in os.walk(p, followlinks=True):
            for f in sorted(files):
                if f.lower().endswith((".ttf", ".otf")):
                    yield os.path.join(root, f)


def ours(cli, font, flag, out):
    r = subprocess.run([cli, "optimize", font, flag, "-o", out], capture_output=True, text=True)
    return r.returncode == 0, r.stderr.strip()


def ttx_of(path, tags, out):
    args = [sys.executable, "-I", "-m", "fontTools.ttx", "-q", "-o", out]
    for t in tags:
        args += ["-t", t]
    subprocess.run(args + [path], check=True, capture_output=True)
    with open(out, encoding="utf-8") as f:
        return [line for line in f if not line.startswith("<ttFont") and "<!--" not in line]


def coords_and_ends(font, name):
    glyf = font["glyf"]
    coords, ctrl = glyf._getCoordinatesAndControls(name, font["hmtx"].metrics, font["vmtx"].metrics if "vmtx" in font else None)
    ends = ctrl.endPts if ctrl.numberOfContours >= 0 else list(range(len(ctrl.components)))
    return coords, ends


def their_gvar(font):
    gvar = font["gvar"]
    for name, variations in gvar.variations.items():
        coords, ends = coords_and_ends(font, name)
        kept = []
        for v in variations:
            v.calcInferredDeltas(coords, ends)
            v.roundDeltas()
            v.optimize(coords, ends)
            if v.hasImpact():
                kept.append(v)
        gvar.variations[name] = kept


def gvar_view(font):
    view = {}
    for name, variations in font["gvar"].variations.items():
        view[name] = [(sorted(v.axes.items()), [(i, tuple(d)) for i, d in enumerate(v.coordinates) if d is not None]) for v in variations]
    return view


def check_gvar(cli, font, out_dir):
    ok, err = ours(cli, font, "--gvar-iup", os.path.join(out_dir, "ours.ttf"))
    if not ok:
        return f"ours failed: {err}"
    theirs = TTFont(font)
    their_gvar(theirs)
    a, b = gvar_view(TTFont(os.path.join(out_dir, "ours.ttf"))), gvar_view(theirs)
    differ = [n for n in b if a.get(n) != b[n]]
    return "equal" if not differ else f"{len(differ)} of {len(b)} glyphs differ, first {differ[:5]}"


def their_implied(font):
    from fontTools.ttLib.tables._g_l_y_f import dropImpliedOnCurvePoints

    # The glyph at the peak of every gvar tuple stands in for the masters.
    sets = []
    if "gvar" in font:
        peaks = set()
        for variations in font["gvar"].variations.values():
            for v in variations:
                peaks.add(tuple(sorted((tag, peak) for tag, (_, peak, _) in v.axes.items())))
        sets = [font.getGlyphSet(location=dict(peak), normalized=True) for peak in sorted(peaks)]
    glyf = font["glyf"]
    for name in glyf.keys():
        g = glyf[name]
        if g.numberOfContours < 1 or (hasattr(g, "program") and g.program.getBytecode()):
            continue
        masters = [s[name]._getGlyphAndOffset()[0] for s in sets]
        dropImpliedOnCurvePoints(g, *masters)


def check_implied(cli, font, out_dir):
    ok, err = ours(cli, font, "--drop-implied-oncurves", os.path.join(out_dir, "ours.ttf"))
    if not ok:
        return f"ours failed: {err}"
    theirs = TTFont(font)
    their_implied(theirs)
    mine = TTFont(os.path.join(out_dir, "ours.ttf"))
    differ = []
    for name in theirs["glyf"].keys():
        a, b = mine["glyf"][name], theirs["glyf"][name]
        if a.numberOfContours > 0 and list(a.coordinates) != list(b.coordinates):
            differ.append(name)
    return "equal" if not differ else f"{len(differ)} glyphs differ, first {differ[:5]}"


def check_varstore(cli, font, out_dir):
    o, t = os.path.join(out_dir, "ours.ttf"), os.path.join(out_dir, "theirs.ttf")
    ok, err = ours(cli, font, "--varstore", o)
    if not ok:
        return f"ours failed: {err}"
    subprocess.run([sys.executable, "-I", "-m", "fontTools.varLib.varStore", font, t], check=True, capture_output=True)
    a = ttx_of(o, ["GDEF", "GPOS"], os.path.join(out_dir, "ours.ttx"))
    b = ttx_of(t, ["GDEF", "GPOS"], os.path.join(out_dir, "theirs.ttx"))
    sizes = f"GDEF {len(TTFont(font).reader['GDEF'])} -> ours {len(TTFont(o).reader['GDEF'])}, theirs {len(TTFont(t).reader['GDEF'])}"
    return ("equal" if a == b else f"TTX differs at line {next(i for i, (x, y) in enumerate(zip(a + [''], b + [''])) if x != y)}") + f"; {sizes}"


def check_colr(cli, font, out_dir):
    from fontTools.colorLib.builder import buildColrV1
    from fontTools.colorLib.unbuilder import unbuildColrV1

    o, t = os.path.join(out_dir, "ours.ttf"), os.path.join(out_dir, "theirs.ttf")
    ok, err = ours(cli, font, "--colr-layer-reuse", o)
    if not ok:
        return f"ours failed: {err}"
    theirs = TTFont(font)
    colr = theirs["COLR"].table
    glyphs = unbuildColrV1(colr.LayerList, colr.BaseGlyphList)
    colr.LayerList, colr.BaseGlyphList = buildColrV1(glyphs, theirs.getReverseGlyphMap())
    theirs.save(t)
    a = ttx_of(o, ["COLR"], os.path.join(out_dir, "ours.ttx"))
    b = ttx_of(t, ["COLR"], os.path.join(out_dir, "theirs.ttx"))
    return "equal" if a == b else "TTX differs"


def build_colr(path, seed):
    from fontTools.colorLib.builder import buildCOLR
    from fontTools.fontBuilder import FontBuilder
    from fontTools.pens.ttGlyphPen import TTGlyphPen

    rnd = random.Random(seed)
    names = [".notdef"] + [f"c{i}" for i in range(12)] + [f"p{i}" for i in range(8)]
    fb = FontBuilder(1000, isTTF=True)
    fb.setupGlyphOrder(names)
    fb.setupCharacterMap({0x41 + i: f"c{i}" for i in range(12)})
    pen = TTGlyphPen(None)
    pen.moveTo((0, 0))
    pen.lineTo((100, 0))
    pen.lineTo((100, 100))
    pen.closePath()
    box = pen.glyph()
    fb.setupGlyf({n: box for n in names})
    fb.setupHorizontalMetrics({n: (500, 0) for n in names})
    fb.setupHorizontalHeader(ascent=800, descent=-200)
    fb.setupNameTable({"familyName": "Colr Check", "styleName": "Regular"})
    fb.setupOS2()
    fb.setupPost()
    layers = [{"Format": 10, "Glyph": f"p{rnd.randrange(8)}", "Paint": {"Format": 2, "PaletteIndex": rnd.randrange(4), "Alpha": 1.0}} for _ in range(10)]
    glyphs = {}
    for i in range(12):
        start = rnd.randrange(6)
        glyphs[f"c{i}"] = {"Format": 1, "Layers": layers[start : start + 2 + rnd.randrange(5)]}
    fb.font["COLR"] = buildCOLR(glyphs, version=1, glyphMap=fb.font.getReverseGlyphMap(), allowLayerReuse=False)
    fb.setupCPAL([[(1, 0, 0, 1), (0, 1, 0, 1), (0, 0, 1, 1), (0, 0, 0, 1)]])
    fb.save(path)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--cli", default="target/release/fontcraft-cli")
    ap.add_argument("--build", type=int, default=0)
    ap.add_argument("out_dir")
    ap.add_argument("paths", nargs="*", default=["corpus"])
    a = ap.parse_args()
    os.makedirs(a.out_dir, exist_ok=True)
    fonts = list(fonts_under(a.paths))
    for i in range(a.build):
        p = os.path.join(a.out_dir, f"colr-{i}.ttf")
        build_colr(p, i)
        fonts.append(p)
    totals = {}
    for font in fonts:
        try:
            f = TTFont(font, lazy=True)
            tags = set(f.keys())
            store = "GDEF" in tags and getattr(f["GDEF"].table, "VarStore", None) is not None
            colr1 = "COLR" in tags and f["COLR"].version == 1
        except Exception:
            continue
        steps = []
        if "gvar" in tags:
            steps.append(("gvar-iup", check_gvar))
        if "glyf" in tags:
            steps.append(("implied", check_implied))
        if store:
            steps.append(("varstore", check_varstore))
        if colr1:
            steps.append(("colr", check_colr))
        for step, check in steps:
            try:
                result = check(a.cli, font, a.out_dir)
            except Exception as e:
                result = f"error: {e}"
            totals.setdefault(step, [0, 0])
            totals[step][0 if result.startswith("equal") else 1] += 1
            print(f"{step:9} {font}: {result}")
    for step, (same, other) in sorted(totals.items()):
        print(f"{step}: {same} equal, {other} different or failed")


if __name__ == "__main__":
    main()
