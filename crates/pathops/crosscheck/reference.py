"""Reference overlap removal for `cargo xtask overlap`: skia-pathops, as fontTools runs it.
Run locally; not part of the gate.

Reads the layers file the xtask writes (one JSON object per line with an SVG path in "input"),
removes overlaps from each with skia-pathops (nonzero rule, outer contours counter-clockwise, as
fontTools' removeOverlaps calls it) and writes one JSON object per line: the layer's key, the
result as an SVG path, the time the operation took, or the error.

Usage (with a Python that has skia-pathops and fontTools):

    python -I crates/pathops/crosscheck/reference.py LAYERS.jsonl REFERENCE.jsonl
"""

import json
import sys
import time

import pathops
from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.svgLib.path import parse_path


def main(src, dst):
    with open(src) as inp, open(dst, "w") as out:
        for line in inp:
            rec = json.loads(line)
            row = {k: rec[k] for k in ("source", "glyph", "layer")}
            try:
                path = pathops.Path()
                parse_path(rec["input"], path.getPen())
                start = time.perf_counter()
                path.simplify(fix_winding=True, keep_starting_points=False, clockwise=False)
                row["ms"] = (time.perf_counter() - start) * 1000.0
                pen = SVGPathPen(None, ntos=repr)
                path.draw(pen)
                row["d"] = pen.getCommands()
            except Exception as e:  # report every failure; the run goes on
                row["error"] = f"{type(e).__name__}: {e}"
            out.write(json.dumps(row) + "\n")


if __name__ == "__main__":
    main(sys.argv[1], sys.argv[2])
