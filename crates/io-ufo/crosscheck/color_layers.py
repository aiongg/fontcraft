"""Cross-check FontCraft's UFO colour layer list against glyphsLib (M11.5c). Run locally; not part
of the gate.

For every corpus Glyphs source with full-colour layers, glyphsLib's `to_ufos(minimal=True)` (the
UFOs fontmake compiles) and `fontcraft-cli convert` each write UFOs. Per master, the script
compares the glyph entries of `com.github.googlei18n.ufo2ft.colorLayers` that are paints (numbers
to six significant digits), `com.github.googlei18n.ufo2ft.colorPalettes`, and the outlines of the
layer glyphs the entries name. Palette-only entries (`[[glyph, index], ...]`) are not compared:
FontCraft writes palette layers in the `colorLayerMapping` form glyphsLib's full UFOs use.

Then each glyphsLib UFO goes through FontCraft and back (`convert IN.ufo OUT.ufo`); the script
checks that the list, the palettes and the layer glyphs come back as they were and that no layer
glyph is lost or added.

Usage (with the fontmake tool environment's Python, which has glyphsLib and ufoLib2):

    python -I crates/io-ufo/crosscheck/color_layers.py OUT_DIR [--cli PATH] [SOURCE.glyphs ...]

With no sources it uses the corpus colour sources. `--cli` defaults to target/release/fontcraft-cli.
"""

import math
import subprocess
import sys
from pathlib import Path

import glyphsLib
import ufoLib2
from fontTools.designspaceLib import DesignSpaceDocument

REPO = Path(__file__).resolve().parents[3]
LAYERS = "com.github.googlei18n.ufo2ft.colorLayers"
PALETTES = "com.github.googlei18n.ufo2ft.colorPalettes"
CORPUS = sorted((REPO / "corpus/fontc/resources/testdata/glyphs3").glob("COLR*.glyphs")) + [
    REPO / "corpus/fontc/resources/testdata/glyphs3/color-and-bracket-glyph-order.glyphs",
]


def plain(v):
    """A lib value as plain Python lists, dicts and numbers."""
    if isinstance(v, dict):
        return {str(k): plain(x) for k, x in v.items()}
    if isinstance(v, str):
        return v
    if hasattr(v, "__iter__"):
        return [plain(x) for x in v]
    return v


def close(a, b):
    if isinstance(a, dict) and isinstance(b, dict):
        return a.keys() == b.keys() and all(close(a[k], b[k]) for k in a)
    if isinstance(a, list) and isinstance(b, list):
        return len(a) == len(b) and all(close(x, y) for x, y in zip(a, b))
    if isinstance(a, (int, float)) and isinstance(b, (int, float)) and not isinstance(a, bool):
        return math.isclose(a, b, rel_tol=1e-6, abs_tol=1e-6)
    return a == b


def paint_entries(lib):
    return {k: v for k, v in plain(lib.get(LAYERS, {})).items() if isinstance(v, dict)}


def named_glyphs(value, out):
    """The glyph names a paint names (`Glyph` keys), into `out`."""
    if isinstance(value, dict):
        if isinstance(value.get("Glyph"), str):
            out.add(value["Glyph"])
        for v in value.values():
            named_glyphs(v, out)
    elif isinstance(value, list):
        for v in value:
            named_glyphs(v, out)
    return out


def outline(ufo, name):
    if name not in ufo:
        return None
    g = ufo[name]
    contours = sorted(tuple((p.x, p.y, p.segmentType) for p in c) for c in g.contours)
    return g.width, sorted(g.unicodes), contours, sorted((c.baseGlyph, tuple(c.transformation)) for c in g.components)


def compare(want, got):
    """Lines saying how UFO `got` differs from `want` in its paint entries and layer glyphs."""
    out = []
    a, b = paint_entries(want.lib), paint_entries(got.lib)
    for k in sorted(set(a) | set(b)):
        if not close(a.get(k), b.get(k)):
            out.append(f"entry {k}: {a.get(k)} != {b.get(k)}")
    if a and not close(plain(want.lib.get(PALETTES)), plain(got.lib.get(PALETTES))):
        out.append(f"palettes: {plain(want.lib.get(PALETTES))} != {plain(got.lib.get(PALETTES))}")
    for name in sorted(named_glyphs(a, set()) | named_glyphs(b, set())):
        if outline(want, name) != outline(got, name):
            out.append(f"glyph {name}: {outline(want, name)} != {outline(got, name)}")
    made = lambda ufo: {g.name for g in ufo if ".color" in g.name}
    if made(want) != made(got):
        out.append(f"layer glyphs: {sorted(made(want))} != {sorted(made(got))}")
    return out


def fontcraft_ufos(path):
    if path.suffix == ".ufo":
        return [ufoLib2.Font.open(path)]
    doc = DesignSpaceDocument.fromfile(path)
    return [ufoLib2.Font.open(Path(path).parent / s.filename) for s in doc.sources if not s.layerName]


def main():
    args = sys.argv[1:]
    if not args:
        sys.exit(__doc__)
    out = Path(args.pop(0))
    cli = REPO / "target/release/fontcraft-cli"
    if args[:1] == ["--cli"]:
        cli = Path(args[1])
        args = args[2:]
    sources = [Path(a) for a in args] or CORPUS
    totals = {"sources": 0, "equal": 0, "different": 0, "round trip equal": 0, "round trip different": 0, "failed": 0}
    for src in sources:
        font = glyphsLib.GSFont(str(src))
        ref = glyphsLib.to_ufos(font, minimal=True)
        if not any(paint_entries(u.lib) for u in ref):
            continue
        totals["sources"] += 1
        stem = src.stem
        target = out / "fontcraft" / (stem + (".ufo" if len(ref) == 1 else ".designspace"))
        target.parent.mkdir(parents=True, exist_ok=True)
        run = subprocess.run([str(cli), "convert", str(src), str(target)], capture_output=True, text=True)
        if run.returncode != 0:
            totals["failed"] += 1
            print(f"{stem}: convert failed: {run.stderr.strip()[:300]}")
            continue
        ours = fontcraft_ufos(target)
        lines = [f"master {i}: {line}" for i, (r, o) in enumerate(zip(ref, ours)) for line in compare(r, o)]
        totals["different" if lines else "equal"] += 1
        print(f"{stem}: {'equal to glyphsLib' if not lines else 'differs from glyphsLib'}")
        for line in lines[:12]:
            print("   ", line)
        # glyphsLib's UFOs through FontCraft and back.
        rt_lines = []
        for i, r in enumerate(ref):
            ufo_in = out / "glyphslib" / f"{stem}-{i}.ufo"
            ufo_out = out / "roundtrip" / f"{stem}-{i}.ufo"
            ufo_in.parent.mkdir(parents=True, exist_ok=True)
            ufo_out.parent.mkdir(parents=True, exist_ok=True)
            r.save(ufo_in, overwrite=True)
            run = subprocess.run([str(cli), "convert", str(ufo_in), str(ufo_out)], capture_output=True, text=True)
            if run.returncode != 0:
                rt_lines.append(f"master {i}: convert failed: {run.stderr.strip()[:300]}")
                continue
            rt_lines += [f"master {i}: {line}" for line in compare(ufoLib2.Font.open(ufo_in), ufoLib2.Font.open(ufo_out))]
        totals["round trip different" if rt_lines else "round trip equal"] += 1
        for line in rt_lines[:12]:
            print("    round trip:", line)
    print(", ".join(f"{k} {v}" for k, v in totals.items()))


if __name__ == "__main__":
    main()
