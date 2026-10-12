"""Expected values for the glyph-driver tests of crates/autohint (src/driver.rs, src/masters.rs).

Runs otfautohint (AFDKO 5.0.1) on the synthetic glyphs the Rust tests build, through its
Python API, and prints the stems, masks, counter masks and flex of each result. Run it with the
Python of an AFDKO installation:

    python -I crates/autohint/tests/oracle/driver.py

A test tool only; nothing here ships.
"""

from afdko.otfautohint.autohint import ACOptions
from afdko.otfautohint.fdTools import FDDict, kBlueValueKeys
from afdko.otfautohint.glyphData import glyphData
from afdko.otfautohint.hinter import glyphHinter

# The zones and stems every test uses: BlueValues [-15 0 500 515 700 715], StdHW 50, StdVW 80,
# BlueFuzz 1.
BLUES = [-15, 0, 500, 515, 700, 715]


def fddict(font_name=None):
    d = FDDict(0, fontName=font_name)
    bvs = list(BLUES)
    bvs[0] = bvs[0] - bvs[1]
    for i in range(3, len(bvs), 2):
        bvs[i] = bvs[i] - bvs[i - 1]
    for key, value in zip(kBlueValueKeys, bvs):
        d.setInfo(key, value)
    d.setInfo('DominantH', [50])
    d.setInfo('DominantV', [80])
    d.setInfo('FlexOK', True)
    d.setInfo('BlueFuzz', 1)
    d.setInfo('OrigEmSqUnits', 1000)
    d.buildBlueLists()
    return d


def rect(pen, x0, y0, x1, y1):
    """A counter-clockwise rectangle, drawn as the Rust tests draw it."""
    pen.moveTo((x0, y0))
    pen.lineTo((x1, y0))
    pen.lineTo((x1, y1))
    pen.lineTo((x0, y1))
    pen.closePath()


def glyph(name, draw, dx=0):
    g = glyphData(roundCoords=True, name=name)
    draw(g, dx)
    return g


def staggered(pen, dx):
    # Two bars whose vertical stems overlap in x: hint substitution.
    rect(pen, 100 + dx, 0, 180 + dx, 300)
    rect(pen, 140 + dx, 400, 220 + dx, 700)


def m_stems(pen, dx):
    # Three equal stems: counter hinting for a glyph named "m".
    rect(pen, 50, 0, 130, 500)
    rect(pen, 250 + dx, 0, 330 + dx, 500)
    rect(pen, 450 + 2 * dx, 0, 530 + 2 * dx, 500)


def show(label, g):
    print('==', label)
    print('h', [(s.lb, s.rt) for s in g.hstems])
    print('v', [(s.lb, s.rt) for s in g.vstems])
    print('is_hm', g.is_hm)
    print('start', g.startmasks)
    for si, sp in enumerate(g.subpaths):
        for ei, pe in enumerate(sp):
            if pe.masks:
                print('mask', (si, ei), pe.masks)
            if pe.flex == 1:
                print('flex', (si, ei))
    print('cntr', g.cntr)


def run(label, names_and_glyphs, options=None):
    options = options or ACOptions()
    hinter = glyphHinter(options, None)
    name = names_and_glyphs[0][0]
    glyphs = tuple(g for _, g in names_and_glyphs)
    dicts = [fddict('master %d' % i) for i in range(len(glyphs))]
    _, result = hinter._hint(name, glyphs, dicts)
    if result is None:
        print('==', label, 'not hinted')
        return
    for i, g in enumerate(result):
        show('%s [%d]' % (label, i), g)


run('staggered', [('staggered', glyph('staggered', staggered))])
run('m', [('m', glyph('m', m_stems))])
run('staggered two masters', [('staggered', glyph('staggered', staggered)),
                               ('staggered', glyph('staggered', staggered, 10))])



def tee(pen, dx):
    # A bar whose left end is hidden inside a stem: the overlap is removed in every master
    # before segments are made, so the hidden end makes no stem.
    rect(pen, 100 + dx, 0, 180 + dx, 700)
    rect(pen, 140 + dx, 300, 500 + dx, 380)


run('tee two masters', [('tee', glyph('tee', tee)), ('tee', glyph('tee', tee, 10))])
