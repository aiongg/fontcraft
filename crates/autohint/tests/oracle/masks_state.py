"""Expected values for the tests in crates/autohint/src/{masks,state}.rs.

Runs otfautohint's own functions (AFDKO 5.0.1, Apache-2.0) on the inputs those tests build and
prints the results as JSON. A test tool, not shipped. Run it with the Python that has AFDKO:

    python -I masks_state.py
"""

import json
from types import SimpleNamespace

from afdko.otfautohint.autohint import ACOptions
from afdko.otfautohint.glyphData import glyphData, stem
from afdko.otfautohint.hinter import dimensionHinter, vhinter
from afdko.otfautohint.hintstate import (
    glyphHintState,
    hintSegment,
    instanceStemState,
    links,
    stemValue,
)

ST = hintSegment.sType
out = {}


class PE:
    """Stands for a path element: a weak-referenceable key with a position."""

    def __init__(self, subpath, offset):
        self.position = (subpath, offset)


# 1. addSegment, compactLists, remExtraBends, cleanup
pes = [PE(0, i) for i in range(6)]
hs = glyphHintState()
made = []
for fr, to, loc, pe1, pe2, typ, bonus, mid1, mid2 in [
    (0, 100, 50, None, 0, ST.LINE, 0, False, False),
    (20, 60, 50, 1, None, ST.CURVE, 5, False, False),
    (200, 300, 50.000001, 2, None, ST.BEND, 0, False, False),
    (90, 400, 50, None, 3, ST.LINE, 2, False, False),
    (300, 0, 50, 4, None, ST.LINE, 0, False, False),
    (10, 30, 120, 5, None, ST.BEND, 0, False, False),
    (100, 0, 120, 0, 1, ST.LINE, 0, True, False),
]:
    before = set(map(id, hs.increasingSegs + hs.decreasingSegs))
    hs.addSegment(fr, to, loc, pes[pe1] if pe1 is not None else None,
                  pes[pe2] if pe2 is not None else None, typ, bonus, False,
                  mid1, mid2, "")
    made += [s for s in hs.increasingSegs + hs.decreasingSegs
             if id(s) not in before]
idx = {id(s): i for i, s in enumerate(made)}
out["segments_inc_after_add"] = [idx[id(s)] for s in hs.increasingSegs]
hs.compactLists()
hs.remExtraBends()
hs.cleanup()
out["segments"] = [
    [s.min, s.max, s.bonus,
     idx[id(s.replacedBy)] if s.replacedBy is not None else None, s.deleted]
    for s in made
]
out["segments_inc"] = [idx[id(s)] for s in hs.increasingSegs]
out["segments_dec"] = [idx[id(s)] for s in hs.decreasingSegs]
out["segments_pe"] = [
    [[idx[id(s)] for s in lst] for lst in (st.s_segs, st.m_segs, st.e_segs)]
    for st in (hs.peStates[p] for p in pes)
]


# 2. unconflict
def unconflict(weights, sc, cur, pin):
    fake = SimpleNamespace(hs=SimpleNamespace(weights=weights))
    fake.unconflict = lambda *a: dimensionHinter.unconflict(fake, *a)
    return dimensionHinter.unconflict(fake, sc, cur, pin)


def matrix(n, pairs):
    m = [[False] * n for _ in range(n)]
    for a, b in pairs:
        m[a][b] = m[b][a] = True
    return m


out["unconflict"] = [
    unconflict([10, 1, 1, 10], matrix(4, [(0, 1), (1, 2), (2, 3)]), None, None),
    unconflict([1, 10, 10, 1], matrix(4, [(0, 1), (1, 2), (2, 3)]), None, None),
    unconflict([5, 5, 5, 5, 5], matrix(5, [(0, 4), (1, 2)]),
               [True] * 5, [False, False, False, False, True]),
]

# 3. links.mark and links.shuffle
lpes = [PE(i, 0) for i in range(5)]
segs = [hintSegment(0, 0, 10, p, ST.LINE, 0, False, True, "") for p in lpes]
values = [stemValue(0, 10, 1, 0, segs[a], segs[b]) for a, b in
          [(0, 3), (3, 4), (1, 2), (0, 0), (4, 0)]]
lk = links(SimpleNamespace(subpaths=[0] * 5))
lk.mark(values, False)
out["links"] = lk.links
out["shuffle"] = lk.shuffle()


# 4. instanceStemState.addToLoc and bestLocation
def in_band(loc, isBottom=False):
    return (-15 <= loc <= 0) if isBottom else (500 <= loc <= 515)


iss = instanceStemState(
    0, SimpleNamespace(options=SimpleNamespace(roundCoords=True),
                       inBand=in_band))
iss.addToLoc(10, 30, strong=True, seg=segs[0])
iss.addToLoc(10, 5)
iss.addToLoc(0, 20.5)
iss.addToLoc(10, 30, strong=True, seg=segs[0])
iss.addToLoc(505, 12)
out["best_location"] = [iss.bestLocation(True), iss.bestLocation(False)]

# 5. convertToMasks and makePEMask, vertical stems, one master and two
gd = glyphData(True, "test")
gd.moveTo((100, 0))
gd.lineTo((100, 500))
gd.curveTo((100, 600), (230, 650), (230, 500))
gd.lineTo((230, 0))
gd.lineTo((480, 0))
gd.lineTo((480, 300))
gd.lineTo((400, 300))
gd.lineTo((400, 0))
gd.closePath()
els = gd.subpaths[0]
out["elements"] = len(els)


def mask_case(stems_per_master):
    opts = ACOptions()
    h = vhinter(opts)
    h.keepHints = False
    h.setGlyph([None] * len(stems_per_master), None,
               [gd] * len(stems_per_master), "test")
    hs = h.hs
    ghost_seg = hintSegment(480, 0, 0, None, ST.GHOST, 0, True, True, "")
    vals = []
    for i, (lloc, uloc, val) in enumerate([(100, 180, 100), (150, 230, 40),
                                           (400, 480, 200)]):
        lo = hintSegment(lloc, 0, 10, els[0], ST.LINE, 0, True, False, "")
        up = hintSegment(uloc, 0, 10, els[0], ST.LINE, 0, True, True, "")
        v = stemValue(lloc, uloc, val, 0, lo, up)
        v.idx = i
        vals.append(v)
    up3 = hintSegment(480, 0, 10, els[0], ST.LINE, 0, True, True, "")
    v3 = stemValue(480, 480, 20, 0, ghost_seg, up3, isGhost=True)
    v3.idx = 3
    vals.append(v3)

    def seg(e, place, v, suppressed=False):
        s = hintSegment(vals[v].lloc, 0, 10, els[e], ST.LINE, 0, True,
                        True, "")
        s.hintval = vals[v]
        s.suppressed = suppressed
        getattr(hs.getPEStateForced(els[e]), place + "_segs").append(s)

    for e, place, v, *sup in [(0, "s", 0), (1, "s", 0), (1, "e", 1),
                              (2, "s", 1), (4, "s", 2), (4, "m", 3),
                              (5, "s", 0), (5, "e", 2), (6, "s", 2),
                              (7, "s", 0), (7, "e", 1), (7, "m", 2, True)]:
        seg(e, place, v, *sup)
    hs.stems = tuple([stem(*s) for s in sl] for sl in stems_per_master)
    hs.weights = [100, 40, 200, 20]
    hs.mainValues = [vals[2], vals[0], vals[1], vals[3]]
    h.convertToMasks()
    return {
        "good": hs.goodMask,
        "has_overlaps": hs.hasOverlaps,
        "overlaps": hs.stemOverlaps,
        "ghost_compat": hs.ghostCompat,
        "main": hs.mainMask,
        "masks": [(hs.peStates[e].mask or [])
                  if e in hs.peStates else None for e in els],
    }


one = [(100, 180), (150, 230), (400, 480), (480, 460)]
two = [(160, 240), (140, 220), (410, 490), (490, 470)]
out["masks_one_master"] = mask_case([one])
out["masks_two_masters"] = mask_case([one, two])

print(json.dumps(out))
