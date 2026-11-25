"""Compare the interpolatable masters FontCraft writes with fontmake's (M2.9).

    fontcraft-cli compile SOURCE --format ttf-interpolatable --output-layout by-format --output-dir ours
    (cd theirs && fontmake SOURCE -o ttf-interpolatable)
    python -I masters_check.py ours/master_ttf_interpolatable theirs/master_ttf_interpolatable

(and the same with otf). Run it with fontmake's own Python, which has fontTools. Per pair of fonts
of one name: glyphs only in one, and glyphs whose structure (contours, on-curve flags, components,
or charstring operators), coordinates or advance differ. Per folder: the glyphs whose structure is
not the same in every font that has them, which must be none. Exit status 1 when any is listed.
"""

import argparse
import pathlib
import sys

from fontTools.pens.recordingPen import RecordingPen
from fontTools.ttLib import TTFont


def drawn(font, name):
    """(structure, coordinates, advance) of one glyph."""
    advance = font["hmtx"][name][0]
    if "glyf" in font:
        g = font["glyf"][name]
        if g.isComposite():
            parts = [c.getComponentInfo() for c in g.components]
            return ("components", [p[0] for p in parts], [p[1][:4] for p in parts]), [p[1][4:] for p in parts], advance
        if g.numberOfContours <= 0:
            return ("empty",), [], advance
        return ("contours", list(g.endPtsOfContours), [f & 1 for f in g.flags]), [tuple(p) for p in g.coordinates], advance
    pen = RecordingPen()
    font.getGlyphSet()[name].draw(pen)
    ops = [op for op, _ in pen.value]
    return ("charstring", ops), [p for _, pts in pen.value for p in pts], advance


def load(folder):
    return {p.name: TTFont(p) for p in sorted(pathlib.Path(folder).iterdir()) if p.suffix in (".ttf", ".otf")}


def names(font):
    return font.getGlyphOrder()


def agree(fonts):
    """Glyphs whose structure differs between two fonts of a folder."""
    first, bad = {}, set()
    for font in fonts.values():
        for name in names(font):
            advance = font["hmtx"][name][0]
            structure = drawn(font, name)[0]
            if advance == 0xFFFF and structure[0] in ("empty", "charstring"):
                continue  # no source of this glyph
            if first.setdefault(name, structure) != structure:
                bad.add(name)
    return sorted(bad)


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("ours")
    parser.add_argument("theirs")
    args = parser.parse_args()
    ours, theirs = load(args.ours), load(args.theirs)
    failed = False

    def report(what, items):
        nonlocal failed
        if items:
            failed = True
            shown = " ".join(items[:12]) + (f" and {len(items) - 12} more" if len(items) > 12 else "")
            print(f"  {what}: {shown}")

    for label, fonts in (("ours", ours), ("theirs", theirs)):
        print(f"{label}: {len(fonts)} fonts")
        report("structure differs between the fonts", agree(fonts))
    report("fonts only in ours", sorted(set(ours) - set(theirs)))
    report("fonts only in theirs", sorted(set(theirs) - set(ours)))
    for name in sorted(set(ours) & set(theirs)):
        a, b = ours[name], theirs[name]
        print(f"{name}: {len(names(a))} glyphs")
        report("glyphs only in ours", sorted(set(names(a)) - set(names(b))))
        report("glyphs only in theirs", sorted(set(names(b)) - set(names(a))))
        structure, points, advances = [], [], []
        for glyph in names(a):
            if glyph not in names(b):
                continue
            x, y = drawn(a, glyph), drawn(b, glyph)
            if x[0] != y[0]:
                structure.append(glyph)
            elif x[1] != y[1]:
                points.append(glyph)
            if x[2] != y[2]:
                advances.append(glyph)
        report("structure", structure)
        report("coordinates", points)
        report("advances", advances)
        report("tables only in ours", sorted(set(a.keys()) - set(b.keys())))
        report("tables only in theirs", sorted(set(b.keys()) - set(a.keys())))
    sys.exit(1 if failed else 0)


if __name__ == "__main__":
    main()
