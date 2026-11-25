#!/usr/bin/env python3
"""Compare FontCraft's TrueType autohinter with ttfautohint over the corpus.

Oracle: ttfautohint (the program in the `ttfautohint-py` package, run as a black box on
FontCraft's own unhinted build) and FreeType's bytecode interpreter (`freetype-py`), which hints
both fonts. Needs a Python with ttfautohint-py, freetype-py and fonttools:

    uv venv ENV && uv pip install --python ENV/bin/python ttfautohint-py freetype-py fonttools
    ENV/bin/python -I crates/hinting/crosscheck/tt_autohint.py \
        --cli target/release/fontcraft-cli --corpus corpus --out DIR

Every source of the corpus is built as a static TrueType font (the first instance, or the one
`INSTANCE` names), unhinted and with Autohint on. Sources that build the same outlines count
once. At 9 to 30 PPM, in FreeType's default and monochrome modes, the script reports per font
and in total:

  * zone extremes: on-curve nodes of flat extremes inside an alignment zone that lie on a whole
    pixel row, the row most extremes of that zone are on;
  * x-height and cap-height rows: the height most extremes of that zone have, equal to
    ttfautohint's at the same size;
  * stem widths: the heights of horizontal bars of A to Z, a to z and 0 to 9, equal to
    ttfautohint's within 1/16 pixel;
  * the size of the hinted font against ttfautohint's.

`--runs` repeats it with the TTFAutohint options that FontCraft and ttfautohint are both given
(`RUNS`); `--reference FONT...` hints unhinted TrueType fonts with ttfautohint alone, for a check
of the script without a FontCraft build.
"""

from __future__ import annotations

import argparse
import hashlib
import importlib.util
import json
import subprocess
import sys
from collections import Counter
from concurrent.futures import ThreadPoolExecutor
from pathlib import Path

import freetype
from fontTools.ttLib import TTFont

PPEMS = range(9, 31)
MODES = {"default": freetype.FT_LOAD_TARGET_NORMAL, "mono": freetype.FT_LOAD_TARGET_MONO}
SUFFIXES = {".glyphs", ".glyphspackage", ".designspace", ".ufo", ".ufoz"}
# The instance built of a source with many: (end of the source's path, instance name).
INSTANCE = [
    ("inter/src/Inter-Roman.glyphspackage", "Regular"),
    ("inter/src/Inter-Italic.glyphspackage", "Italic"),
    ("oswald/sources/Oswald.glyphs", "Regular"),
]
RUNS = {
    "default": "",
    "natural": "--stem-width-mode=nnn",
    "quantized": "--stem-width-mode=qqq",
    "strong": "--stem-width-mode=sss",
    "range": "--hinting-range-min=20 --hinting-range-max=25",
    "composites": "--composites",
}
LETTERS = [chr(c) for c in range(ord("A"), ord("Z") + 1)] + [chr(c) for c in range(ord("a"), ord("z") + 1)]
BAR_CHARS = LETTERS + list("0123456789")
TIMEOUT_S = 900
ON_ROW = 1 / 32
SAME_WIDTH = 1 / 16


def ttfautohint_program() -> Path:
    spec = importlib.util.find_spec("ttfautohint")
    if spec is None or spec.origin is None:
        sys.exit("ttfautohint-py is not installed in this Python (or give --ttfautohint)")
    return Path(spec.origin).parent / "ttfautohint"


# ── building ────────────────────────────────────────────────────────────────────────────────────


def sources(corpus: Path) -> list[Path]:
    out: list[Path] = []
    stack = [corpus.resolve()]
    while stack:
        for p in sorted(stack.pop().iterdir()):
            if p.suffix in SUFFIXES:
                out.append(p)
            elif p.is_dir() and p.name != ".git":
                stack.append(p)
    return sorted(out)


def run_cli(cli: Path, source: Path, steps: list[tuple[str, dict]]) -> tuple[list[dict], str]:
    """The results of engine commands on `source`, and what went wrong."""
    args = [str(cli), "run", "--in", str(source)]
    for command, params in steps:
        args += ["--cmd", command, "--params", json.dumps(params)]
    try:
        r = subprocess.run(args, capture_output=True, text=True, timeout=TIMEOUT_S, check=False)
    except subprocess.TimeoutExpired:
        return [], "timed out"
    lines = []
    for line in r.stdout.splitlines():
        try:
            lines.append(json.loads(line))
        except ValueError:
            continue
    return lines, "" if r.returncode == 0 else (r.stderr.strip().splitlines() or ["failed"])[-1]


def build(cli: Path, source: Path, options: str, out: Path) -> dict:
    """FontCraft's unhinted and autohinted fonts of `source`, or why there are none."""
    instance = next((name for end, name in INSTANCE if str(source).endswith(end)), None)
    out.mkdir(parents=True, exist_ok=True)
    common = {"formats": ["ttf"], "timestamp": 0, "versionStamp": False, "instances": [instance] if instance else True}
    steps: list[tuple[str, dict]] = []
    if options:
        steps.append(("font.customParameters.add", {"scope": "font", "name": "TTFAutohint options", "value": options}))
    steps.append(("font.compile", common | {"autohint": False, "destination": str(out / "unhinted")}))
    steps.append(("font.compile", common | {"autohint": True, "destination": str(out / "autohint")}))
    results, error = run_cli(cli, source, steps)
    fonts = [r["result"]["fonts"] for r in results if r.get("command") == "font.compile" and r["result"].get("fonts")]
    if len(fonts) != 2:
        return {"skip": f"no build: {error or 'no static font'}"[:200]}
    unhinted, hinted = fonts[0][0], fonts[1][0]
    target = {"instance": unhinted["instance"]} if unhinted["source"] == "instance" else {}
    blues, error = run_cli(cli, source, [("hint.ps.getFontHints", target)])
    if not blues:
        return {"skip": f"no zones: {error}"[:200]}
    return {
        "unhinted": unhinted["files"][0]["path"],
        "autohint": hinted["files"][0]["path"],
        "zones": zones(blues[0]["result"]),
        "warnings": hinted.get("warnings", []),
    }


def zones(hints: dict) -> list[tuple[float, float, bool]]:
    """(bottom, top, is a top zone) of each alignment zone."""
    pairs = lambda v: list(zip(v[::2], v[1::2]))  # noqa: E731
    blue = pairs(hints.get("blueValues") or [])
    out = [(a, b, i > 0) for i, (a, b) in enumerate(blue)]
    out += [(a, b, False) for a, b in pairs(hints.get("otherBlues") or [])]
    return out


def ttfautohint(program: Path, font: Path, options: str, out: Path) -> str:
    """Hints `font` into `out`; the program's message when it refuses."""
    if out.exists():
        return ""
    try:
        r = subprocess.run([str(program), *options.split(), str(font), str(out)], capture_output=True, text=True, timeout=TIMEOUT_S, check=False)
    except subprocess.TimeoutExpired:
        return "ttfautohint timed out"
    if r.returncode != 0:
        out.unlink(missing_ok=True)
        return "ttfautohint: " + " ".join(r.stderr.split())[:160]
    return ""


# ── what is measured ────────────────────────────────────────────────────────────────────────────


def contours(glyph) -> list[list[tuple[int, int, int, bool]]]:
    """(point number, x, y, on curve) of each contour."""
    out, start = [], 0
    for end in glyph.endPtsOfContours:
        out.append([(i, *glyph.coordinates[i], bool(glyph.flags[i] & 1)) for i in range(start, end + 1)])
        start = end + 1
    return out


def extremes(glyph) -> list[tuple[int, int, bool]]:
    """(point, y, is a top) of the on-curve points of each flat run that is a local extreme."""
    out = []
    for c in contours(glyph):
        n = len(c)
        start = next((i for i in range(n) if c[i][2] != c[(i + 1) % n][2]), None)
        if start is None:
            continue
        ring = [c[(start + 1 + k) % n] for k in range(n)]
        k = 0
        while k < n:
            end = k
            while end + 1 < n and ring[end + 1][2] == ring[k][2]:
                end += 1
            before, after, y = ring[k - 1][2], ring[(end + 1) % n][2], ring[k][2]
            if (before < y and after < y) or (before > y and after > y):
                out += [(p[0], y, before < y) for p in ring[k : end + 1] if p[3]]
            k = end + 1
    return out


def bars(glyph, upem: int) -> list[tuple[int, int]]:
    """(point on the lower edge, point on the upper edge) of each horizontal bar: two flat
    straight segments that overlap by half and lie less than a quarter em apart, nothing between."""
    flat = []
    for c in contours(glyph):
        for a, b in zip(c, c[1:] + c[:1]):
            if a[3] and b[3] and a[2] == b[2] and abs(a[1] - b[1]) >= upem * 0.08:
                flat.append((a[2], min(a[1], b[1]), max(a[1], b[1]), a[0], b[1] > a[1]))
    out = []
    for lo in flat:
        above = [
            hi
            for hi in flat
            if lo[0] < hi[0] < lo[0] + upem * 0.25
            and min(lo[2], hi[2]) - max(lo[1], hi[1]) >= 0.5 * min(lo[2] - lo[1], hi[2] - hi[1])
        ]
        # TrueType fills clockwise: a bar's upper edge runs right, its lower edge left.
        hi = min(above, default=None, key=lambda h: h[0])
        if hi and hi[4] and not lo[4]:
            out.append((lo[3], hi[3]))
    return out


class Hinted:
    """Hinted point heights of a font, in pixels."""

    def __init__(self, path: str):
        self.face = freetype.Face(path)

    def heights(self, glyph: str, ppem: int, mode: str) -> list[float]:
        self.face.set_pixel_sizes(0, ppem)
        # Without NO_AUTOHINT FreeType hints a font that has no programs with its own autohinter.
        self.face.load_glyph(
            self.face.get_name_index(glyph.encode()),
            MODES[mode] | freetype.FT_LOAD_NO_BITMAP | freetype.FT_LOAD_NO_AUTOHINT,
        )
        return [p[1] / 64 for p in self.face.glyph.outline.points]


def describe(path: str, zone_list: list[tuple[float, float, bool]]) -> dict:
    """What an unhinted font offers to measure."""
    font = TTFont(path)
    glyf, cmap = font["glyf"], font.getBestCmap() or {}
    upem = font["head"].unitsPerEm
    simple = {n: glyf[n] for n in font.getGlyphOrder() if glyf[n].numberOfContours > 0}
    if not any(cmap.get(ord(c)) in simple for c in LETTERS):
        return {"skip": "no outline for a letter A to Z or a to z"}
    if not font["post"].formatType == 2.0:
        return {"skip": "no glyph names"}
    os2 = font["OS/2"]
    named = {"x": getattr(os2, "sxHeight", 0), "cap": getattr(os2, "sCapHeight", 0)}
    found: dict[str, list[tuple[int, int]]] = {}
    for name, g in simple.items():
        if name == ".notdef":
            continue
        for point, y, top in extremes(g):
            for z, (lo, hi, is_top) in enumerate(zone_list):
                if is_top == top and lo <= y <= hi:
                    found.setdefault(name, []).append((point, z))
                    break
    row_zone = {
        k: next((z for z, (lo, hi, top) in enumerate(zone_list) if top and v and lo <= v <= hi), None) for k, v in named.items()
    }
    bar_list = {}
    for c in BAR_CHARS:
        name = cmap.get(ord(c))
        if name in simple and (found_bars := bars(simple[name], upem)):
            bar_list[name] = found_bars
    data = font.reader["glyf"] + str(upem).encode()
    return {"extremes": found, "rows": row_zone, "bars": bar_list, "hash": hashlib.sha256(data).hexdigest()}


def measure(what: dict, path: str, mode: str) -> dict:
    """Zone extremes on their row, the row heights and the bar heights of one hinted font."""
    font = Hinted(path)
    on_row = total = 0
    rows: dict[str, float] = {}
    widths: dict[tuple[str, int, int], float] = {}
    for ppem in PPEMS:
        per_zone: dict[int, list[float]] = {}
        for name in sorted(set(what["extremes"]) | set(what["bars"])):
            y = font.heights(name, ppem, mode)
            for point, zone in what["extremes"].get(name, []):
                per_zone.setdefault(zone, []).append(round(y[point], 2))
            for i, (lo, hi) in enumerate(what["bars"].get(name, [])):
                widths[(name, i, ppem)] = y[hi] - y[lo]
        for zone, ys in per_zone.items():
            row = Counter(round(v) for v in ys).most_common(1)[0][0]
            on_row += sum(abs(v - row) <= ON_ROW for v in ys)
            total += len(ys)
        for key, zone in what["rows"].items():
            if zone in per_zone:
                rows[f"{key}{ppem}"] = Counter(per_zone[zone]).most_common(1)[0][0]
    return {"on": on_row, "of": total, "rows": rows, "widths": widths}


def compare(what: dict, ours: str, theirs: str) -> dict:
    out = {"size": Path(ours).stat().st_size, "ref_size": Path(theirs).stat().st_size}
    for mode in MODES:
        a, b = measure(what, ours, mode), measure(what, theirs, mode)
        r = {"on": a["on"], "of": a["of"], "ref_on": b["on"]}
        for key in ("x", "cap"):
            mine = {k: v for k, v in a["rows"].items() if k.startswith(key)}
            r[f"{key}_rows"] = [sum(abs(v - b["rows"][k]) <= ON_ROW for k, v in mine.items()), len(mine)]
            r[f"{key}_misses"] = [f"{k} {v:g} (ref {b['rows'][k]:g})" for k, v in mine.items() if abs(v - b["rows"][k]) > ON_ROW]
        same = [abs(v - b["widths"][k]) <= SAME_WIDTH for k, v in a["widths"].items()]
        r["bars"] = [sum(same), len(same)]
        out[mode] = r
    return out


def percent(pair) -> str:
    return f"{pair[0]}/{pair[1]} = {100 * pair[0] / pair[1]:.2f}%" if pair[1] else "none"


def total(results: list[dict], mode: str) -> dict[str, str]:
    add = lambda key: [sum(r[mode][key][0] for r in results), sum(r[mode][key][1] for r in results)]  # noqa: E731
    of = sum(r[mode]["of"] for r in results)
    return {
        "zone extremes": percent([sum(r[mode]["on"] for r in results), of]),
        "zone extremes, ttfautohint": percent([sum(r[mode]["ref_on"] for r in results), of]),
        "x rows": percent(add("x_rows")),
        "cap rows": percent(add("cap_rows")),
        "bar heights": percent(add("bars")),
        "size": percent([sum(r["size"] for r in results), sum(r["ref_size"] for r in results)]),
    }


# ── the runs ────────────────────────────────────────────────────────────────────────────────────


def run(args, name: str, options: str, corpus: Path, files: list[Path]) -> None:
    out = args.out / name
    skipped: list[tuple[str, str]] = []
    seen: dict[str, str] = {}
    results: list[dict] = []
    rel = lambda s: str(s.relative_to(corpus))  # noqa: E731

    def prepare(source: Path) -> tuple[Path, dict]:
        key = hashlib.sha256(rel(source).encode()).hexdigest()[:12]
        return source, build(args.cli, source, options, out / "fonts" / key)

    with ThreadPoolExecutor(args.jobs) as pool:
        built = list(pool.map(prepare, files))
    for source, b in built:
        what = b if "skip" in b else describe(b["unhinted"], b["zones"])
        if "skip" in what:
            skipped.append((rel(source), what["skip"]))
            continue
        if what["hash"] in seen:
            skipped.append((rel(source), f"same outlines as {seen[what['hash']]}"))
            continue
        seen[what["hash"]] = rel(source)
        ref = args.out / "ref" / f"{what['hash'][:16]}-{name}.ttf"
        ref.parent.mkdir(parents=True, exist_ok=True)
        if why := ttfautohint(args.ttfautohint, Path(b["unhinted"]), options, ref):
            skipped.append((rel(source), why))
            continue
        r = compare(what, b["autohint"], str(ref))
        r["source"] = rel(source)
        r["warnings"] = [w for w in b["warnings"] if "utohint" in w]
        results.append(r)
        print(f"  {r['source']}: zones {percent([r['default']['on'], r['default']['of']])}, x {percent(r['default']['x_rows'])}", flush=True)
    report = {"options": options, "fonts": results, "skipped": skipped}
    for mode in MODES:
        report[f"total {mode}"] = total(results, mode)
    (out / "report.json").write_text(json.dumps(report, indent=1))
    print(f"== {name} ({options or 'no options'}): {len(results)} fonts of {len(files)} sources")
    for reason, n in Counter(why.split(":")[0] if not why.startswith("same") else "same outlines" for _, why in skipped).most_common():
        print(f"   skipped {n}: {reason}")
    for mode in MODES:
        for key, value in report[f"total {mode}"].items():
            print(f"   {mode:8} {key}: {value}")


def reference(args, fonts: list[Path]) -> None:
    """ttfautohint alone, on unhinted fonts, with zones guessed from OS/2 (overshoot 2% of the em)."""
    args.out.mkdir(parents=True, exist_ok=True)
    for name, options in RUNS.items():
        for path in fonts:
            t = TTFont(path)
            over = t["head"].unitsPerEm * 0.02
            guessed = [(-over, 0, False)] + [(v, v + over, True) for v in (t["OS/2"].sxHeight, t["OS/2"].sCapHeight)]
            what = describe(str(path), guessed)
            if "skip" in what:
                print(path.name, "skipped:", what["skip"])
                continue
            out = args.out / f"{path.stem}-{name}.ttf"
            if why := ttfautohint(args.ttfautohint, path, options, out):
                print(path.name, why)
                continue
            for mode in MODES:
                m = measure(what, str(out), mode)
                x = [m["rows"].get(f"x{p}") for p in (9, 12, 15, 20, 30)]
                w = [v for (g, i, p), v in sorted(m["widths"].items()) if p == 20][:6]
                print(f"{path.name} {name} {mode}: zones {percent([m['on'], m['of']])}, x rows at 9 12 15 20 30: {x}, bars at 20: {w}, {out.stat().st_size} bytes")


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--cli", type=Path, default=Path("target/release/fontcraft-cli"))
    ap.add_argument("--corpus", type=Path, default=Path("corpus"))
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--ttfautohint", type=Path)
    ap.add_argument("--only", help="sources whose path holds this text")
    ap.add_argument("--runs", nargs="?", const=",".join(RUNS), default="default", help="runs of RUNS, comma-separated (alone: all)")
    ap.add_argument("--jobs", type=int, default=4)
    ap.add_argument("--reference", nargs="+", type=Path, metavar="FONT")
    args = ap.parse_args()
    args.ttfautohint = args.ttfautohint or ttfautohint_program()
    args.out = args.out.resolve()
    if args.reference:
        return reference(args, args.reference)
    corpus = args.corpus.resolve()
    files = [s for s in sources(args.corpus) if not args.only or args.only in str(s)]
    for name in args.runs.split(","):
        run(args, name, RUNS[name], corpus, files)
    return None


if __name__ == "__main__":
    main()
