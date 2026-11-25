"""Writes the reference anchors of the propagation measure
(`crates/engine/tests/engine/corpus_propagation.rs`).

For every Glyphs source of the corpus, glyphsLib's `propagate_all_anchors` (the algorithm fontc
follows) is run, and the anchors of every glyph on every master layer are written as JSON:
`{source path relative to the corpus: {glyph: {master id: [[name, x, y], ...]}}}`, with
`{"error": message}` for a source the library cannot load. The library is run, never read.

Run in a Python environment with glyphsLib, isolated:
    ENV/bin/python -I crates/components/crosscheck/glyphslib_anchors.py corpus OUT.json
"""

import json
import pathlib
import sys

import glyphsLib
from glyphsLib.builder.transformations.propagate_anchors import propagate_all_anchors


def anchors(path):
    font = glyphsLib.GSFont(str(path))
    # The function itself ignores the parameter; the library's builder checks it.
    flag = font.customParameters["Propagate Anchors"]
    if flag is None or bool(flag):
        propagate_all_anchors(font)
    masters = {m.id for m in font.masters}
    return {
        g.name: {l.layerId: [[a.name, a.position.x, a.position.y] for a in l.anchors] for l in g.layers if l.layerId in masters}
        for g in font.glyphs
    }


def main():
    root, out_path = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2])
    out = {}
    for path in sorted([*root.glob("**/*.glyphs"), *root.glob("**/*.glyphspackage")]):
        rel = path.relative_to(root).as_posix()
        try:
            out[rel] = anchors(path)
        except Exception as e:  # every refusal is an outcome
            out[rel] = {"error": f"{type(e).__name__}: {e}"}
    out_path.write_text(json.dumps(out))
    print(f"{len(out)} sources, {sum('error' in v for v in out.values())} not loaded")


main()
