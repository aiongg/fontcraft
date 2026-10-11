"""Expected values for the unit tests of crates/autohint/src/glyph.rs and bezier.rs.

Runs otfautohint's glyphData.py (AFDKO 5.0.1) and fontTools' bezierTools on the small
synthetic glyphs the tests build, and prints each value the tests compare, with floats in their
shortest round-trip form. Run locally; not part of the gate and not shipped.

Usage (with the Python AFDKO is installed in):

    python -I crates/autohint/tests/oracle/glyph_data.py
"""

from fontTools.misc.bezierTools import solveCubic, approximateCubicArcLength

from afdko.otfautohint.glyphData import glyphData, norm_float, pathElement, pt, stem


def draw(gd, contours):
    """contours: lists of ("m", x, y), ("l", x, y), ("c", x1, y1, x2, y2, x3, y3), ("z",)."""
    for contour in contours:
        for op in contour:
            if op[0] == "m":
                gd.moveTo(op[1:3])
            elif op[0] == "l":
                gd.lineTo(op[1:3])
            elif op[0] == "c":
                gd.curveTo(op[1:3], op[3:5], op[5:7])
            elif op[0] == "z":
                gd.closePath()
    return gd


def glyph(contours, name="g"):
    return draw(glyphData(roundCoords=False, name=name), contours)


def ref(pe):
    if pe is None:
        return None
    sub = pe.segment_sub if isinstance(pe.segment_sub, int) else None
    return (pe.position[0], pe.position[1], sub)


def p(x):
    return (float(x[0]), float(x[1]))


BOWL = [
    [("m", 250, 0), ("c", 388, 0, 500, 112, 500, 250), ("c", 500, 388, 388, 500, 250, 500),
     ("c", 112, 500, 0, 388, 0, 250), ("c", 0, 112, 112, 0, 250, 0), ("z",)],
    [("m", 250, 100), ("c", 167, 100, 100, 167, 100, 250), ("c", 100, 333, 167, 400, 250, 400),
     ("c", 333, 400, 400, 333, 400, 250), ("c", 400, 167, 333, 100, 260, 100), ("z",)],
]

CUP = [[("m", 0, 0), ("c", 30, 100, 70, 100, 100, 0), ("l", 100, -200), ("l", 0, -200), ("z",)]]

TINY = [[("m", 0, 0), ("l", 100, 0), ("l", 101, 1), ("l", 101, 100), ("l", 0, 100), ("z",)]]

S_CURVE = [[("m", 0, 0), ("c", 150, 50, -50, 150, 100, 200), ("l", 0, 200), ("z",)]]

T_ORIG = [
    [("m", 0, 0), ("l", 100, 0), ("l", 100, 300), ("l", 0, 300), ("z",)],
    [("m", 0, 200), ("l", 300, 200), ("l", 300, 300), ("l", 0, 300), ("z",)],
]
T_UNION = [[("m", 0, 0), ("l", 100, 0), ("l", 100, 200), ("l", 300, 200), ("l", 300, 300),
            ("l", 0, 300), ("z",)]]


def main():
    print("# stems")
    stems = [stem(10, 90), stem(500, 480), stem(-21, -42), stem(100, 50), stem(500, 520),
             stem(479, 500)]
    for s in stems:
        print("stem", tuple(s), "ghost", s.isGhost(), "bad", s.isBad(),
              "rel", s.relVals(), "rel_after", s.relVals(stem(3.33333, 7.77777)),
              "dist", [s.distance(v) for v in (0, 50, 485, 600)])
    for a in stems:
        print("overlaps", tuple(a), [a.overlaps(b) for b in stems])
        print("ghost_compat", tuple(a), [a.ghostCompat(b) for b in stems])

    print("# round")
    for v in (0.35, 1.00005, 2.00015, 0.12345, -3.14159265, 1e-5, 12.34565, 2.675, 1e6 + 0.00005,
              -0.00004):
        print(repr(v), repr(round(v, 4)), repr(round(v, 1)), repr(round(v, 6)), norm_float(v))

    print("# solveCubic")
    for args in ((1, 1, -6, 0), (-10.0, -9.0, 48.0, -29.0), (1.0, -4.5, 6.75, -3.375),
                 (2.0, -3.0, 0.5, 0.1), (1.0, 0.0, 1.0, -1.0), (1e-12, 1.0, -3.0, 2.0)):
        print(args, [repr(x) for x in solveCubic(*args)])

    print("# arc length")
    for seg in (((0, 0), (25, 100), (75, 100), (100, 0)), ((0, 0), (50, 0), (100, 50), (100, 100))):
        print(seg, repr(approximateCubicArcLength(*seg)))

    print("# bowl")
    g = glyph(BOWL)
    print("lens", [len(sp) for sp in g.subpaths])
    print("close", [[pe.isClose() for pe in sp] for sp in g.subpaths])
    c = g.nextForHints(g)
    order = []
    while c:
        order.append(ref(c))
        c = g.nextForHints(c)
    print("hint order", order)
    b = g.getBounds(None)
    print("glyph bounds", b.bounds, [[ref(x) for x in r] for r in b.extpes])

    print("# cup")
    g = glyph(CUP)
    pe = g.subpaths[0][0]
    bs = pe.getBounds()
    print("lb", bs.lb, "bounds", bs.bounds, "extpts", [[p(x) for x in r] for r in bs.extpts],
          "tmap", bs.tmap)
    print("farthest y before", bs.farthestExtreme(True))
    print("farthest x before", bs.farthestExtreme(False))
    sb = g.getBounds(0)
    print("subpath bounds", sb.bounds, [[ref(x) for x in r] for r in sb.extpes])
    print("first element bounds after", pe.getBounds().bounds, "lb", pe.getBounds().lb)
    print("farthest y after", pe.getBounds().farthestExtreme(True))
    print("contains (50,75)", pe.containsPoint(pt(50, 75), 0.01, True))
    print("contains (50,70)", pe.containsPoint(pt(50, 70), 0.01, True))
    print("contains on line", g.subpaths[0][1].containsPoint(pt(100, -50), 0.01, True))
    print("at .25", pe.atT(.25), "assoc", repr(pe.getAssocFactor()), repr(pe.getAssocFactor(True)))
    print("short", [x.isShort() for x in g.subpaths[0]], "tiny", [x.isTiny() for x in g.subpaths[0]])
    print("slope", pe.slopePoint(0), pe.slopePoint(1))
    second = pe.splitAt(0.25)
    print("split first", p(pe.s), p(pe.cs), p(pe.ce), p(pe.e), "second", p(second.s),
          p(second.cs), p(second.ce), p(second.e))

    print("# edits")
    print("short", pathElement(pt(0, 0), pt(17, 0)).isShort(),
          pathElement(pt(0, 0), pt(5, 5)).isShort(),
          "tiny", pathElement(pt(0, 0), pt(1.5, -1.5)).isTiny())
    line = pathElement(pt(0, 0), pt(300, 0))
    line.convertToCurve(.25)
    print("to curve", line.cs, line.ce, line.relVals())

    print("# tiny")
    g = glyph(TINY)
    print("lens", [len(sp) for sp in g.subpaths])
    sp = g.subpaths[0]
    print("next skip", ref(g.nextInSubpath(sp[0], skipTiny=True)))
    print("prev skip", ref(g.prevInSubpath(sp[2], skipTiny=True)))
    print("next wrap", ref(g.nextInSubpath(sp[-1])))
    print("next no wrap", ref(g.nextInSubpath(sp[-1], closeWrapOK=False)))
    print("prev of first", ref(g.prevInSubpath(sp[0])))
    print("next slope", g.nextSlopePoint(sp[0]), "prev slope", g.prevSlopePoint(sp[2]))

    print("# s-curve")
    g = glyph(S_CURVE)
    pe = g.subpaths[0][0]
    print("split", pe.splitAtInflectionsForSegs())
    for spe in pe.segment_sub:
        print("part", ref(spe), p(spe.s), p(spe.cs), p(spe.ce), p(spe.e))
    c = g.next(g, segSub=True)
    walk = []
    while c:
        walk.append(ref(c))
        c = g.next(c, segSub=True)
    print("walk segsub", walk)
    print("next in subpath from part 0",
          ref(g.nextInSubpath(pe.segment_sub[0], segSub=True)))
    print("prev in subpath from line",
          ref(g.prevInSubpath(g.subpaths[0][1], segSub=True)))
    print("prev in subpath from part 0",
          ref(g.prevInSubpath(pe.segment_sub[0], segSub=True)))

    print("# associate")
    orig = glyph(T_ORIG)
    derived = glyph(T_UNION)
    derived.associatePath(orig)
    for sp in derived.subpaths:
        for pe in sp:
            print("pe", ref(pe), p(pe.s), p(pe.e), "->", ref(pe.association))


if __name__ == "__main__":
    main()
