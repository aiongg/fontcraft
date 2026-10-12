"""Expected values for the segment, flex and overlap tests of crates/autohint (package M10.9).

Runs otfautohint's own functions (AFDKO 5.0.1) on the synthetic glyphs the Rust tests build and
prints what they produce. A test tool, not shipped. Run it with the Python that has AFDKO:

    python segments.py
"""

from afdko.otfautohint.autohint import ACOptions
from afdko.otfautohint.fdTools import FDDict
from afdko.otfautohint.glyphData import glyphData
from afdko.otfautohint.hinter import hhinter, vhinter

# Each glyph is a list of contours; a contour is a start point and a list of steps, a step being
# one point (a line) or three (a curve). Every contour is closed.
GLYPHS = {
    "stem": [((100, 0), [[(180, 0)], [(180, 700)], [(100, 700)]])],
    "bowl": [
        ((250, -10), [[(390, -10), (480, 100), (480, 250)], [(480, 400), (390, 510), (250, 510)],
                      [(110, 510), (20, 400), (20, 250)], [(20, 100), (110, -10), (250, -10)]]),
        ((250, 60), [[(160, 60), (105, 140), (105, 250)], [(105, 360), (160, 440), (250, 440)],
                     [(340, 440), (395, 360), (395, 250)], [(395, 140), (340, 60), (250, 60)]]),
    ],
    "flexbar": [((0, 0), [[(600, 0)], [(600, 100)], [(500, 100), (400, 95), (300, 95)],
                          [(200, 95), (100, 100), (0, 100)]])],
    "flexdip": [((0, 0), [[(600, 0)], [(600, 100)], [(500, 95), (400, 95), (300, 95)],
                          [(200, 95), (100, 95), (0, 100)]])],
    "bump": [((0, 0), [[(600, 0)], [(600, 100)], [(500, 105), (400, 105), (300, 105)],
                       [(200, 105), (100, 105), (0, 100)]])],
    "tee": [
        ((0, 400), [[(500, 400)], [(500, 500)], [(0, 500)]]),
        ((200, 0), [[(300, 0)], [(300, 450)], [(200, 450)]]),
    ],
}

# The tee with its overlap removed, as a boolean union draws it.
TEE_UNION = [((200, 0), [[(300, 0)], [(300, 400)], [(500, 400)], [(500, 500)], [(0, 500)],
                         [(0, 400)], [(200, 400)]])]


def draw(contours, name):
    gl = glyphData(roundCoords=True, name=name)
    for start, steps in contours:
        gl.moveTo(start)
        for st in steps:
            if len(st) == 1:
                gl.lineTo(st[0])
            else:
                gl.curveTo(*st)
        gl.closePath()
    return gl


def options():
    o = ACOptions()
    o.overlapForcing = False
    return o


def fddict():
    d = FDDict(0)
    d.BlueFuzz = 1
    d.DominantH = []
    d.DominantV = []
    return d


def seglist(segs):
    return [(s.loc, s.min, s.max, int(s.type)) for s in segs]


def dump_segs(name, gl, opts=None):
    for cls in (hhinter, vhinter):
        h = cls(opts or options())
        h.setGlyph([fddict()], None, [gl], name)
        h.startHint()
        h.genSegs()
        h.stopHint()
        print(name, h.aDesc(), "increasing", seglist(h.hs.increasingSegs))
        print(name, h.aDesc(), "decreasing", seglist(h.hs.decreasingSegs))


def flex_marks(gl):
    return [(pe.position, pe.flex) for pe in gl if pe.flex]


def main():
    for name in ("stem", "bowl"):
        dump_segs(name, draw(GLYPHS[name], name))

    for name in ("flexbar", "flexdip", "bump"):
        gl = draw(GLYPHS[name], name)
        o = options()
        hh, vh = hhinter(o), vhinter(o)
        hh.setGlyph([fddict()], None, [gl], name)
        vh.setGlyph([fddict()], None, [gl], name)
        hh.addFlex()
        vh.addFlex(inited=True)
        print(name, "flex", flex_marks(gl))
        dump_segs(name, gl)

    orig = draw(GLYPHS["tee"], "tee")
    union = draw(TEE_UNION, "tee")
    union.roundCoords = False
    union.associatePath(orig)
    print("tee association", [(pe.position, pe.association.position if pe.association else None)
                              for pe in union])
    o = options()
    o.overlapForcing = True
    try:
        dump_segs("tee", draw(GLYPHS["tee"], "tee"), o)
    except ImportError as e:
        print("tee overlap segments need booleanOperations:", e)


if __name__ == "__main__":
    main()
