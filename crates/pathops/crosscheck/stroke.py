"""Reference strokes for `cargo xtask offset`: skia-pathops. Run locally; not part of the gate.

Reads the layers file the xtask writes (one JSON object per line: the closed paths in "input", the
same paths left open in "open", as SVG paths) and writes one JSON object per layer and operation
with the result as an SVG path, or the error:

    grow      the glyph united with a mitred stroke of its outline, twice --grow wide
    shrink    the glyph less a mitred stroke of its outline, twice --shrink wide
    stroke    a mitred stroke of its closed paths, twice --stroke wide
    JOIN-CAP  strokes of the open paths, the same width, for each join (miter, round, bevel) and
              cap (butt, round, square)

Skia draws a stroke as its two edges, the inner one backwards, filled with the nonzero rule. Where
a path turns tighter than half the width, the inner edge folds over and cancels part of the stroke
(a circle of radius 5 stroked 40 wide comes out with a hole). So the reference here is the union of
Skia's stroke of the whole path and of its strokes, with butt ends, of every stretch from the middle
of one segment to the middle of the next: no stretch turns far enough to cancel itself.

Usage (with a Python that has skia-pathops and fontTools):

    python -I crates/pathops/crosscheck/stroke.py [--grow 20] [--shrink 10] [--stroke 30] \\
        [--only OP,OP] LAYERS.jsonl REFERENCE.jsonl
"""

import argparse
import json

import pathops
from fontTools.misc.bezierTools import splitCubicAtT, splitQuadraticAtT
from fontTools.pens.recordingPen import DecomposingRecordingPen
from fontTools.pens.svgPathPen import SVGPathPen
from fontTools.svgLib.path import parse_path

MITER_LIMIT = 4.0
JOINS = {"miter": pathops.LineJoin.MITER_JOIN, "round": pathops.LineJoin.ROUND_JOIN, "bevel": pathops.LineJoin.BEVEL_JOIN}
CAPS = {"butt": pathops.LineCap.BUTT_CAP, "round": pathops.LineCap.ROUND_CAP, "square": pathops.LineCap.SQUARE_CAP}
CLOSED = ("grow", "shrink", "stroke")


def read(svg):
    """The contours of an SVG path, as (segments, closed); a segment is a list of 2 to 4 points."""
    pen = DecomposingRecordingPen({})
    parse_path(svg, pen)
    contours, segments, start, last = [], [], None, None
    for verb, points in pen.value:
        if verb == "moveTo":
            segments, start, last = [], points[0], points[0]
        elif verb in ("closePath", "endPath"):
            closed = verb == "closePath"
            if closed and last != start:
                segments.append([last, start])
            segments = [s for s in segments if any(p != s[0] for p in s)]
            if segments:
                contours.append((segments, closed))
            segments, last = [], start
        elif verb == "qCurveTo" and len(points) > 2:
            off, end = list(points[:-1]), points[-1]
            for i, c in enumerate(off):
                to = end if i == len(off) - 1 else tuple((a + b) / 2 for a, b in zip(c, off[i + 1]))
                segments.append([last, c, to])
                last = to
        else:
            segments.append([last, *points])
            last = points[-1]
    return contours


def halves(segment):
    if len(segment) == 2:
        (a, b), (c, d) = segment
        mid = ((a + c) / 2, (b + d) / 2)
        return [segment[0], mid], [mid, segment[1]]
    split = splitQuadraticAtT if len(segment) == 3 else splitCubicAtT
    return [list(half) for half in split(*segment, 0.5)]


def draw(segments, closed):
    path = pathops.Path()
    pen = path.getPen()
    pen.moveTo(segments[0][0])
    for i, s in enumerate(segments):
        if len(s) == 2:
            if not (closed and i == len(segments) - 1):
                pen.lineTo(s[1])
        elif len(s) == 3:
            pen.qCurveTo(*s[1:])
        else:
            pen.curveTo(*s[1:])
    pen.closePath() if closed else pen.endPath()
    return path


def stroked(segments, closed, width, cap, join):
    path = draw(segments, closed)
    path.stroke(width, cap, join, MITER_LIMIT)
    # Skia keeps arcs as conics.
    path.convertConicsToQuads()
    path.simplify(fix_winding=True, keep_starting_points=False, clockwise=False)
    return path


def unite(paths):
    """The union of simple paths, two by two."""
    paths = [p for p in paths if len(list(p.segments))]
    if not paths:
        return pathops.Path()
    while len(paths) > 1:
        merged = [
            pathops.op(a, b, pathops.PathOp.UNION, fix_winding=True, keep_starting_points=False)
            for a, b in zip(paths[::2], paths[1::2])
        ]
        paths = merged + (paths[-1:] if len(paths) % 2 else [])
    return paths[0]


def stroke(contours, width, cap, join):
    butt = pathops.LineCap.BUTT_CAP
    parts = []
    for segments, closed in contours:
        parts.append(stroked(segments, closed, width, cap, join))
        halved = [halves(s) for s in segments]
        pairs = list(zip(halved, halved[1:] + halved[:1])) if closed else list(zip(halved, halved[1:]))
        stretches = [[a[1], b[0]] for a, b in pairs]
        if not closed:
            stretches += [[halved[0][0]], [halved[-1][1]]]
        parts.extend(stroked(s, False, width, butt, join) for s in stretches)
    return unite(parts)


def fill(contours):
    path = pathops.Path()
    for segments, closed in contours:
        path.addPath(draw(segments, closed))
    path.simplify(fix_winding=True, keep_starting_points=False, clockwise=False)
    return path


def result(op, rec, args):
    if op in CLOSED:
        contours = read(rec["input"])
        if op == "stroke":
            return stroke(contours, 2 * args.stroke, CAPS["butt"], JOINS["miter"])
        width, how = (2 * args.grow, pathops.PathOp.UNION) if op == "grow" else (2 * args.shrink, pathops.PathOp.DIFFERENCE)
        outline = stroke(contours, width, CAPS["butt"], JOINS["miter"])
        return pathops.op(fill(contours), outline, how, fix_winding=True, keep_starting_points=False)
    join, cap = op.split("-")
    return stroke(read(rec["open"]), 2 * args.stroke, CAPS[cap], JOINS[join])


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--grow", type=float, default=20.0)
    parser.add_argument("--shrink", type=float, default=10.0)
    parser.add_argument("--stroke", type=float, default=30.0)
    parser.add_argument("--only", help="operations, separated by commas")
    parser.add_argument("layers")
    parser.add_argument("reference")
    args = parser.parse_args()
    ops = [*CLOSED, *(f"{j}-{c}" for j in JOINS for c in CAPS)]
    if args.only:
        ops = [op for op in ops if op in args.only.split(",")]
    results, errors = dict.fromkeys(ops, 0), dict.fromkeys(ops, 0)
    with open(args.layers) as inp, open(args.reference, "w") as out:
        for line in inp:
            rec = json.loads(line)
            for op in ops:
                row = {k: rec[k] for k in ("source", "glyph", "layer")}
                row["op"] = op
                try:
                    pen = SVGPathPen(None, ntos=repr)
                    result(op, rec, args).draw(pen)
                    row["d"] = pen.getCommands()
                    results[op] += 1
                except Exception as e:  # report every failure; the run goes on
                    row["error"] = f"{type(e).__name__}: {e}"
                    errors[op] += 1
                out.write(json.dumps(row) + "\n")
    for op in ops:
        print(f"{op}: {results[op]} results, {errors[op]} errors")


if __name__ == "__main__":
    main()
