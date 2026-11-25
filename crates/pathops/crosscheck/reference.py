"""Reference booleans for `cargo xtask overlap`: skia-pathops, as fontTools runs it.
Run locally; not part of the gate.

Reads the layers file the xtask writes (one JSON object per line with SVG paths in "input",
"shift" and "band"), runs one operation on each line with skia-pathops (nonzero rule, outer
contours counter-clockwise, as fontTools' removeOverlaps calls it) and writes one JSON object per
result: the layer's key, the operation, the second operand it ran with, the result as an SVG path,
the time the operation took, or the error. Union runs on "input" alone; difference and
intersection run on "input" and each of "shift" (the layer moved) and "band" (a bar across it).

Usage (with a Python that has skia-pathops and fontTools):

    python -I crates/pathops/crosscheck/reference.py [--op union|difference|intersection] \\
        LAYERS.jsonl REFERENCE.jsonl
"""

import argparse
import json
import time

import pathops
from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.svgLib.path import parse_path

OPS = {"difference": pathops.PathOp.DIFFERENCE, "intersection": pathops.PathOp.INTERSECTION}
PAIRS = ("shift", "band")


def read(svg):
    path = pathops.Path()
    parse_path(svg, path.getPen())
    return path


def union(rec, _pair):
    path = read(rec["input"])
    start = time.perf_counter()
    path.simplify(fix_winding=True, keep_starting_points=False, clockwise=False)
    return path, time.perf_counter() - start


def boolean(op):
    def run(rec, pair):
        one, two = read(rec["input"]), read(rec[pair])
        start = time.perf_counter()
        path = pathops.op(one, two, op, fix_winding=True, keep_starting_points=False, clockwise=False)
        return path, time.perf_counter() - start

    return run


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--op", choices=["union", *OPS], default="union")
    parser.add_argument("layers")
    parser.add_argument("reference")
    args = parser.parse_args()
    run, pairs = (union, [None]) if args.op == "union" else (boolean(OPS[args.op]), PAIRS)
    results = errors = 0
    with open(args.layers) as inp, open(args.reference, "w") as out:
        for line in inp:
            rec = json.loads(line)
            for pair in pairs:
                row = {k: rec[k] for k in ("source", "glyph", "layer")}
                row["op"] = args.op
                if pair is not None:
                    row["pair"] = pair
                try:
                    path, seconds = run(rec, pair)
                    row["ms"] = seconds * 1000.0
                    pen = SVGPathPen(None, ntos=repr)
                    path.draw(pen)
                    row["d"] = pen.getCommands()
                    results += 1
                except Exception as e:  # report every failure; the run goes on
                    row["error"] = f"{type(e).__name__}: {e}"
                    errors += 1
                out.write(json.dumps(row) + "\n")
    print(f"{args.op}: {results} results, {errors} errors")


if __name__ == "__main__":
    main()
