"""Expected values for the report tests of crates/autohint (src/report.rs).

Runs otfautohint's stem and zone analysis (AFDKO 5.0.1) on small synthetic glyphs and prints the
reports. Run it with the Python of an AFDKO installation:

    python -I crates/autohint/tests/oracle/report.py

A test tool only; nothing here ships.
"""
import sys
from afdko.otfautohint.autohint import ACOptions
from afdko.otfautohint.fdTools import FDDict
from afdko.otfautohint.glyphData import glyphData
from afdko.otfautohint.hinter import glyphHinter
from afdko.otfautohint.report import Report

K = 0.5523

def ellipse(pen, cx, cy, rx, ry, ccw):
    pts = [(cx, cy - ry), (cx + rx, cy), (cx, cy + ry), (cx - rx, cy)]
    if not ccw:
        pts = [pts[0], pts[3], pts[2], pts[1]]
    pen.moveTo(pts[0])
    for i in range(4):
        a, b = pts[i], pts[(i + 1) % 4]
        # control points along the tangents
        if a[0] == cx:  # a at bottom/top: tangent horizontal
            c1 = (a[0] + (b[0] - a[0]) * K, a[1])
            c2 = (b[0], b[1] - (b[1] - a[1]) * K)
        else:
            c1 = (a[0], a[1] + (b[1] - a[1]) * K)
            c2 = (b[0] - (b[0] - a[0]) * K, b[1])
        pen.curveTo(tuple(round(v) for v in c1), tuple(round(v) for v in c2), b)
    pen.closePath()

def poly(pen, pts):
    pen.moveTo(pts[0])
    for p in pts[1:]:
        pen.lineTo(p)
    pen.closePath()

def glyphs():
    out = []
    g = glyphData(True, 'l'); poly(g, [(100, 0), (180, 0), (180, 700), (100, 700)]); out.append(g)
    g = glyphData(True, 'H')
    poly(g, [(100, 0), (180, 0), (180, 330), (420, 330), (420, 0), (500, 0), (500, 700), (420, 700), (420, 400), (180, 400), (180, 700), (100, 700)])
    out.append(g)
    g = glyphData(True, 'o'); ellipse(g, 250, 250, 200, 260, True); ellipse(g, 250, 250, 120, 190, False); out.append(g)
    return out

def fddict():
    d = FDDict(0, 'test')
    for k, v in [('BaselineYCoord', 0), ('BaselineOvershoot', -12), ('LcHeight', 500), ('LcOvershoot', 12),
                 ('CapHeight', 700), ('CapOvershoot', 12), ('BlueFuzz', 1), ('OrigEmSqUnits', 1000)]:
        d.setInfo(k, v)
    d.setInfo('DominantV', [80])
    d.setInfo('DominantH', [70])
    d.buildBlueLists()
    return d

for mode in ('stems', 'allstems', 'zones'):
    o = ACOptions()
    o.hintAll = True; o.noFlex = True; o.allowNoBlues = True; o.logOnly = True
    o.report_stems = mode != 'zones'; o.report_zones = mode == 'zones'; o.report_all_stems = mode == 'allstems'
    d = fddict()
    glyphHinter.initialize(o, {0: {0: [d]}})
    rep = Report()
    for g in glyphs():
        name, r = glyphHinter.hint(g.glyphName if hasattr(g, 'glyphName') else g.name, (g,), [d])
        rep.glyphs[name] = r
    h, v, top, bot = rep._get_lists(o)
    print(mode)
    for label, reps, f in (('h', h, rep._sort_count), ('v', v, rep._sort_count), ('top', top, rep._sort_val_reversed), ('bot', bot, rep._sort_val)):
        reps.sort(key=f)
        print(' ', label, reps)
    for n, r in rep.glyphs.items():
        print('   ', n, 'h', sorted(r.hstems_pos), 'v', sorted(r.vstems_pos), 'char', sorted(r.char_zones), 'sz', sorted(r.stem_zone_stems))
