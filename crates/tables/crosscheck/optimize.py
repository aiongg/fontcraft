"""Cross-check the glyf, gvar, variation-store, GPOS compaction and COLR steps of `fontcraft-cli optimize`
against fontTools. Run locally; not part of the gate.

Each step runs on every font under the paths given (by default corpus/) that has the table it
works on, and fontTools does the same work on the same font:

    gvar-iup:    ours `optimize FONT --gvar-iup`; theirs: every tuple's inferred deltas filled
                 in, rounded and optimised (`calcInferredDeltas`, `roundDeltas`, `optimize`, as
                 the instancer runs them), tuples without effect dropped. Each glyph's tuples
                 must list the same points with the same deltas.
    implied:     ours `optimize FONT --drop-implied-oncurves`; theirs: `dropImpliedOnCurvePoints`
                 on each glyph without instructions, given the default glyph and the glyph at the
                 peak of every gvar tuple (instanced by fontTools). Glyphs whose point numbers
                 other data uses (components of point-matched or instructed composites, GPOS
                 anchors and GDEF attachment points and carets on contour points) are left out
                 on both sides, as ours keeps them. The glyf points must match.
    varstore:    ours `optimize FONT --varstore`; theirs `fonttools varLib.varStore FONT OUT`.
                 The TTX of GDEF and GPOS must be equal.
    gpos:        ours `optimize FONT --gpos-compaction LEVEL`; theirs `fonttools otlLib.optimize
                 FONT --gpos-compression-level LEVEL` (LEVEL from `--gpos-level`, default 5).
                 Every pair lookup must hold the same subtables with the same classes (each
                 class as its set of glyph names) and the same non-zero values; extension
                 subtables are looked through. Prints both GPOS sizes. Ours keeps GPOS when it
                 would not get smaller, which shows as "kept" and counts as equal when
                 fontTools' GPOS is no smaller either. Fonts fontTools fails on count apart.
    colr:        ours `optimize FONT --colr-layer-reuse`; theirs `unbuildColrV1` then
                 `buildColrV1`. The TTX of COLR must be equal. `--build N` writes N random
                 COLR 1 fonts without layer reuse (the corpus holds none).

    VENV/bin/python -I crates/tables/crosscheck/optimize.py [--cli PATH] [--build N] [--gpos-level L] [--only STEP] OUT_DIR [FONT_OR_DIR ...]

VENV needs fontTools 4.66.1 (the fontmake tool environment has it). PATH defaults to
target/release/fontcraft-cli. Prints one line per font and step, then the totals. Each run
writes its intermediate files to a directory of its own under OUT_DIR, so runs can share it.
"""

import argparse
import os
import random
import subprocess
import sys
import tempfile

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
    flags = flag if isinstance(flag, list) else [flag]
    r = subprocess.run([cli, "optimize", font, *flags, "-o", out], capture_output=True, text=True)
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


def numbered(font):
    """Names of the glyphs whose point numbers other data uses."""
    glyf = font["glyf"]
    roots = []
    for name in glyf.keys():
        g = glyf[name]
        if g.isComposite():
            instructed = hasattr(g, "program") and g.program.getBytecode()
            if instructed or any(hasattr(c, "firstPt") for c in g.components):
                roots.extend(c.glyphName for c in g.components)

    def mark(coverage, i, anchor):
        if anchor is not None and anchor.Format == 2:
            roots.append(coverage.glyphs[i])

    if "GPOS" in font and font["GPOS"].table.LookupList:
        for lookup in font["GPOS"].table.LookupList.Lookup:
            subs = lookup.SubTable
            if lookup.LookupType == 9:
                subs = [e.ExtSubTable for e in subs]
            for s in subs:
                kind = s.LookupType
                if kind == 3:
                    for i, r in enumerate(s.EntryExitRecord):
                        mark(s.Coverage, i, r.EntryAnchor)
                        mark(s.Coverage, i, r.ExitAnchor)
                elif kind in (4, 5, 6):
                    marks = s.MarkArray if kind != 6 else s.Mark1Array
                    cov1 = s.MarkCoverage if kind != 6 else s.Mark1Coverage
                    for i, r in enumerate(marks.MarkRecord):
                        mark(cov1, i, r.MarkAnchor)
                    if kind == 4:
                        for i, r in enumerate(s.BaseArray.BaseRecord):
                            for a in r.BaseAnchor:
                                mark(s.BaseCoverage, i, a)
                    elif kind == 6:
                        for i, r in enumerate(s.Mark2Array.Mark2Record):
                            for a in r.Mark2Anchor:
                                mark(s.Mark2Coverage, i, a)
                    else:
                        for i, attach in enumerate(s.LigatureArray.LigatureAttach):
                            for c in attach.ComponentRecord:
                                for a in c.LigatureAnchor:
                                    mark(s.LigatureCoverage, i, a)
    if "GDEF" in font:
        gdef = font["GDEF"].table
        if getattr(gdef, "AttachList", None):
            for i, p in enumerate(gdef.AttachList.AttachPoint):
                if p.PointCount:
                    roots.append(gdef.AttachList.Coverage.glyphs[i])
        if getattr(gdef, "LigCaretList", None):
            for i, lig in enumerate(gdef.LigCaretList.LigGlyph):
                for c in lig.CaretValue:
                    mark(gdef.LigCaretList.Coverage, i, c)
    seen = set()
    while roots:
        name = roots.pop()
        if name in seen or name not in glyf:
            continue
        seen.add(name)
        if glyf[name].isComposite():
            roots.extend(c.glyphName for c in glyf[name].components)
    return seen


def their_implied(font):
    from fontTools.ttLib.tables._g_l_y_f import dropImpliedOnCurvePoints

    # The glyph at the peak of every gvar tuple stands in for the masters.
    sets = []
    if "gvar" in font:
        peaks = set()
        for variations in font["gvar"].variations.values():
            for v in variations:
                peaks.add(tuple(sorted((tag, peak) for tag, (_, peak, _) in v.axes.items())))
        # The glyf glyph set itself: a VARC table would otherwise put its own glyphs in front.
        from fontTools.ttLib.ttGlyphSet import _TTGlyphSetGlyf

        sets = [_TTGlyphSetGlyf(font, dict(peak)) for peak in sorted(peaks)]
    glyf = font["glyf"]
    keep = numbered(font)
    for name in glyf.keys():
        g = glyf[name]
        if g.numberOfContours < 1 or (hasattr(g, "program") and g.program.getBytecode()) or name in keep:
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


def has_class_kerning(font):
    for lookup in font["GPOS"].table.LookupList.Lookup:
        for s in pair_subtables(lookup):
            if s.Format == 2:
                return True
    return False


def pair_subtables(lookup):
    subs = lookup.SubTable
    if lookup.LookupType == 9:
        subs = [e.ExtSubTable for e in subs if e.ExtensionLookupType == 2]
    elif lookup.LookupType != 2:
        subs = []
    return subs


def value_view(v):
    if v is None:
        return None
    out = []
    for k, x in sorted(v.__dict__.items()):
        if not x:
            continue
        if hasattr(x, "__dict__"):
            x = tuple(sorted((a, tuple(b) if isinstance(b, list) else b) for a, b in x.__dict__.items()))
        out.append((k, x))
    return tuple(out)


def gpos_view(font):
    view = []
    for index, lookup in enumerate(font["GPOS"].table.LookupList.Lookup):
        for s in pair_subtables(lookup):
            if s.Format != 2:
                view.append((index, "format 1"))
                continue
            c1, c2 = {}, {}
            for g in s.Coverage.glyphs:
                c1.setdefault(s.ClassDef1.classDefs.get(g, 0), set()).add(g)
            for g, c in s.ClassDef2.classDefs.items():
                c2.setdefault(c, set()).add(g)
            values = {}
            for i, row in enumerate(s.Class1Record):
                for j, rec in enumerate(row.Class2Record):
                    v = (value_view(getattr(rec, "Value1", None)), value_view(getattr(rec, "Value2", None)))
                    if any(v):
                        values[(i, j)] = v
            view.append((index, [sorted(c1[k]) for k in sorted(c1)], [sorted(c2[k]) for k in sorted(c2)], values))
    return view


def check_gpos(cli, font, out_dir, level):
    o, t = os.path.join(out_dir, "ours.ttf"), os.path.join(out_dir, "theirs.ttf")
    ok, err = ours(cli, font, ["--gpos-compaction", str(level)], o)
    if not ok:
        return f"ours failed: {err}"
    size = lambda p: len(TTFont(p).reader["GPOS"])
    r = subprocess.run([sys.executable, "-I", "-m", "fontTools.otlLib.optimize", font, "-o", t, "--gpos-compression-level", str(level)], capture_output=True, text=True)
    if r.returncode != 0:
        last = (r.stderr.strip().splitlines() or [""])[-1]
        return f"theirs failed: {last}; GPOS {size(font)} -> ours {size(o)}"
    sizes = f"GPOS {size(font)} -> ours {size(o)}, theirs {size(t)}"
    a, b = gpos_view(TTFont(o)), gpos_view(TTFont(t))
    if a == b:
        return f"equal; {sizes}"
    if size(o) == size(font) and a == gpos_view(TTFont(font)):
        # Ours keeps a table the step would not make smaller.
        return f"{'equal (kept, theirs no smaller)' if size(t) >= size(font) else 'kept'}; {sizes}"
    first = next((i for i, (x, y) in enumerate(zip(a, b)) if x != y), min(len(a), len(b)))
    return f"{len(a)} and {len(b)} pair subtables, first difference at subtable {first}; {sizes}"


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
    ap.add_argument("--gpos-level", type=int, default=5)
    ap.add_argument("--only", default=None, help="run only this step")
    ap.add_argument("out_dir")
    ap.add_argument("paths", nargs="*", default=["corpus"])
    a = ap.parse_args()
    os.makedirs(a.out_dir, exist_ok=True)
    fonts = list(fonts_under(a.paths))
    for i in range(a.build):
        p = os.path.join(a.out_dir, f"colr-{i}.ttf")
        build_colr(p, i)
        fonts.append(p)
    work = tempfile.mkdtemp(prefix="run-", dir=a.out_dir)
    totals = {}
    for font in fonts:
        try:
            f = TTFont(font, lazy=True)
            tags = set(f.keys())
            store = "GDEF" in tags and getattr(f["GDEF"].table, "VarStore", None) is not None
            colr1 = "COLR" in tags and f["COLR"].version == 1
            kerning = "GPOS" in tags and has_class_kerning(f)
        except Exception:
            continue
        steps = []
        if "gvar" in tags:
            steps.append(("gvar-iup", check_gvar))
        if "glyf" in tags:
            steps.append(("implied", check_implied))
        if store:
            steps.append(("varstore", check_varstore))
        if kerning:
            steps.append(("gpos", lambda cli, font, out_dir: check_gpos(cli, font, out_dir, a.gpos_level)))
        if colr1:
            steps.append(("colr", check_colr))
        steps = [s for s in steps if a.only in (None, s[0])]
        for step, check in steps:
            try:
                result = check(a.cli, font, work)
            except Exception as e:
                result = f"error: {e}"
            totals.setdefault(step, [0, 0, 0])
            totals[step][0 if result.startswith("equal") else 2 if result.startswith("theirs failed") else 1] += 1
            print(f"{step:9} {font}: {result}", flush=True)
    for step, (same, other, theirs) in sorted(totals.items()):
        print(f"{step}: {same} equal, {other} different or failed, {theirs} where fontTools failed")


if __name__ == "__main__":
    main()
