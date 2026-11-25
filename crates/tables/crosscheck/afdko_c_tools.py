#!/usr/bin/env python3
"""Measure the command-line tools against AFDKO's C tools, over the corpus.

Oracles: AFDKO 5.0.1 (sfntedit, spot, sfntdiff, tx). The script runs them and reads what they
write; it copies no code of theirs.

Measures (`--measure`, all by default):

  sfntedit  -x, -d, -a and -f against `tables extract`, `delete`, `add` and `checksum --fix`: the
            bytes of the files
  spot      the cases of spot_tx.py beside this script (the listings and the CFF literal view of
            `dump`, `glyphs list`, `tables list`)
  sfntdiff  -d 1 against `diff --detail offset`: the verdict per table, and where it starts to differ
  mtx       `tx -mtx -1` against `metrics --precision real`: the advance and box of each glyph
  afm       `tx -afm` against `metrics --format afm`: the lines, comments left out
  goadb     `tx -dump -4 -gf` against `glyphs rename --goadb`: the glyph names
  nohints   `tx -dump -6 -n` against `dehint`, its result read back with `tx -dump -6`: the paths

    python3 -I crates/tables/crosscheck/afdko_c_tools.py --cli target/release/fontcraft-cli \\
        --corpus corpus --out DIR [--measure NAME ...] [--jobs 4] [--limit N] [--match TEXT]
        [--reference-only] [--fresh]

The harness (cases, resuming, statuses, report) is python_tools.py beside this script. `tx -afm`
opens a temporary file in the system's temporary folder whatever TMPDIR says: where that folder is
read-only, the afm measure has only `reference` results.
"""

from __future__ import annotations

import importlib.util
import json
import re
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
_spec = importlib.util.spec_from_file_location("python_tools", HERE / "python_tools.py")
h = importlib.util.module_from_spec(_spec)
sys.modules["python_tools"] = h
_spec.loader.exec_module(h)

from fontTools.ttLib import TTFont  # noqa: E402  (python_tools may have restarted the script)

AFDKO, run, OursFailed, ReferenceFailed = h.AFDKO, h.run, h.OursFailed, h.ReferenceFailed
# Deleted by the sfntedit measure: the first of these the font has. The edited font is only
# compared byte for byte, so it need not stay usable.
DELETABLE = ("DSIG", "GSUB", "GPOS", "post")


def file_tag(tag: str) -> str:
    return re.sub(r"[^A-Za-z0-9]", "_", tag)


class SfntEdit:
    def cases(self, corpus):
        return [(corpus.rel(p), [p]) for p in corpus.fonts()]

    @staticmethod
    def edits(font: Path) -> tuple[list[str], str | None]:
        tags = [t.decode("latin-1") for t in h.tags(font)]
        return tags, next((t for t in DELETABLE if t in tags), None)

    def reference(self, inputs, ref):
        font = inputs[0]
        tags, gone = self.edits(font)
        if not tags:
            raise ReferenceFailed("not a plain sfnt")
        parts = [f"{t.strip()}={ref / ('x.' + file_tag(t))}" for t in tags]
        run([AFDKO / "sfntedit", "-x", ",".join(parts), font, ref / "extracted.font"])
        if gone:
            run([AFDKO / "sfntedit", "-d", gone.strip(), font, ref / "deleted.font"])
        # `name` goes back in with other bytes, so that the directory changes.
        if "name" in tags:
            data = (ref / "x.name").read_bytes()
            (ref / "new.name").write_bytes(data + bytes(5))
            run([AFDKO / "sfntedit", "-a", f"name={ref / 'new.name'}", font, ref / "added.font"])
        run([AFDKO / "sfntedit", "-f", font, ref / "fixed.font"])

    def compare(self, cli, inputs, ref, ours):
        font = inputs[0]
        tags, gone = self.edits(font)
        run([cli, "tables", "extract", font, *(f"{t.strip()}={ours / ('x.' + file_tag(t))}" for t in tags)], error=OursFailed)
        same = {t: (ours / f"x.{file_tag(t)}").read_bytes() == (ref / f"x.{file_tag(t)}").read_bytes() for t in tags}
        out = h.compare_maps(same, dict.fromkeys(tags, True), "extract")
        steps = []
        if gone:
            steps.append(("delete", ["delete", font, gone.strip()], "deleted.font"))
        if "name" in tags:
            steps.append(("add", ["add", font, f"name={ref / 'new.name'}"], "added.font"))
        steps.append(("fix", ["checksum", font, "--fix"], "fixed.font"))
        for name, args, file in steps:
            run([cli, "tables", *args, "-o", ours / file], error=OursFailed)
            a, b = (ours / file).read_bytes(), (ref / file).read_bytes()
            detail = ""
            if a != b:
                at = next((i for i, (x, y) in enumerate(zip(a, b)) if x != y), min(len(a), len(b)))
                order = [t.decode("latin-1") for t in h.tags(ours / file)], [t.decode("latin-1") for t in h.tags(ref / file)]
                detail = f"{len(a)} and {len(b)} bytes, first difference at {at}" + (
                    "" if order[0] == order[1] else f"; directory {order[0]} against {order[1]}")
            out.append((name, "different" if detail else "equal", detail))
        return out


class Spot:
    """spot_tx.py runs both halves; the reference step only finds the fonts spot cannot open."""

    def cases(self, corpus):
        return [(corpus.rel(p), [p]) for p in corpus.fonts()]

    def reference(self, inputs, ref):
        run([AFDKO / "spot", "-T", inputs[0]])

    def compare(self, cli, inputs, ref, ours):
        font, out = str(inputs[0]), []
        script = [sys.executable, "-I", HERE / "spot_tx.py"]
        for args in ([cli, AFDKO / "spot", AFDKO / "tx", font],
                     ["--listings", cli, AFDKO / "spot", AFDKO / "tx", h.TOOLS / "fontmake/bin/ttx", font]):
            text = run([*script, *args], ok=(0, 1), error=OursFailed)
            lines = text.splitlines()
            for i, line in enumerate(lines):
                m = re.match(r"(equal|different|failed) +(.*)$", line)
                if not m:
                    continue
                status, rest = m.group(1), m.group(2).replace(font, "").strip()
                name, _, detail = rest.partition(": ")
                if status == "different":
                    detail += " " + " ".join(x.strip() for x in lines[i + 1:i + 3] if x.startswith("  "))
                if status == "failed" and re.search(r"reference (?!0,)", detail):
                    status = "reference"
                out.append((name, status, h.short(detail.strip())))
        return out


class SfntDiff:
    def cases(self, corpus):
        return [(f"{corpus.rel(a)} + {b.name}", [a, b]) for a, b in corpus.pairs()]

    def reference(self, inputs, ref):
        text = run([AFDKO / "sfntdiff", "-d", "1", *inputs], ok=(0, 1))
        verdict, tag = {}, None
        for line in text.splitlines():
            if m := re.match(r"'(.{4})' differs", line):
                tag = m.group(1)
                verdict[tag] = None
            elif tag and (m := re.match(r"< [0-9a-f]{8}\+([0-9a-f]{8})=", line)):
                verdict[tag] = int(m.group(1), 16)
        verdict.pop("sfnt", None)
        (ref / "verdict.json").write_text(json.dumps(verdict))

    def compare(self, cli, inputs, ref, ours):
        want = json.loads((ref / "verdict.json").read_text())
        doc = json.loads(run([cli, "diff", *inputs, "--detail", "offset", "--json"], ok=(0, 1), error=OursFailed))
        # sfntdiff compares the tables both fonts have.
        got = {t["tag"]: t.get("firstOffset") for t in doc["tables"].values() if t["a"] and t["b"] and not t["equal"]}
        out = h.compare_maps(dict.fromkeys(got, True), dict.fromkeys(want, True), "verdict")
        both = [t for t in want if t in got]
        return out + h.compare_maps({t: got[t] for t in both}, {t: want[t] for t in both}, "offset")


OUTLINES = (b"glyf", b"CFF ")


def outline_fonts(corpus):
    return [(corpus.rel(p), [p]) for p in corpus.fonts() if any(h.has_table(p, t) for t in OUTLINES)]


class Mtx:
    ROW = re.compile(r"glyph\[(\d+)\] \{.*,(-?[\d.]+),\{(-?[\d.]+),(-?[\d.]+),(-?[\d.]+),(-?[\d.]+)\}\}$")

    def cases(self, corpus):
        return outline_fonts(corpus)

    def reference(self, inputs, ref):
        text = run([AFDKO / "tx", "-mtx", "-1", inputs[0]])
        rows = {m.group(1): [float(v) for v in m.groups()[1:]] for m in map(self.ROW.match, text.splitlines()) if m}
        if not rows:
            raise ReferenceFailed("no glyph lines")
        (ref / "mtx.json").write_text(json.dumps(rows))

    def compare(self, cli, inputs, ref, ours):
        want = json.loads((ref / "mtx.json").read_text())
        doc = json.loads(run([cli, "metrics", inputs[0], "--precision", "real", "--format", "json"], error=OursFailed))
        got = {str(g["gid"]): [g["advance"], *(g.get("bounds") or [0, 0, 0, 0])] for g in doc["glyphs"]}
        # tx prints at most two decimals.
        near = {k: want[k] if k in got and all(abs(x - y) < 0.011 for x, y in zip(got[k], want[k])) else got.get(k) for k in want}
        return h.compare_maps(near, want, "glyphs")


class Afm:
    def cases(self, corpus):
        return [(n, i) for n, i in outline_fonts(corpus) if h.has_table(i[0], b"CFF ")]

    @staticmethod
    def lines(text: str) -> str:
        return "\n".join(x.rstrip() for x in text.splitlines() if x.strip() and not x.startswith("Comment"))

    def reference(self, inputs, ref):
        text = run([AFDKO / "tx", "-afm", inputs[0]])
        if "StartFontMetrics" not in text:
            raise ReferenceFailed("no AFM text: tx could not write its temporary file")
        (ref / "font.afm").write_text(text)

    def compare(self, cli, inputs, ref, ours):
        text = run([cli, "metrics", inputs[0], "--format", "afm"], error=OursFailed)
        (ours / "font.afm").write_text(text)
        d = h.first_difference(self.lines(text), self.lines((ref / "font.afm").read_text()))
        return [("afm", "different" if d else "equal", d)]


class Goadb:
    ROW = re.compile(r"glyph\[(\d+)\] \{(.*?),")

    def cases(self, corpus):
        return [(n, i) for n, i in outline_fonts(corpus) if h.has_table(i[0], b"CFF ")]

    def reference(self, inputs, ref):
        font = TTFont(inputs[0], lazy=True)
        top = font["CFF "].cff.topDictIndex[0]
        if hasattr(top, "ROS"):
            raise ReferenceFailed("a CID-keyed font has no glyph names")
        names = [n for n in font.getGlyphOrder()[1:] if re.fullmatch(r"[A-Za-z_][A-Za-z0-9._]*", n)][:20]
        # Final name, then the source name that gets it; one row names a glyph the font lacks.
        rows = [f"{n}.renamed {n}" for n in names] + ["absent.final absent.source"]
        (ref / "goadb").write_text("\n".join(rows) + "\n")
        text = run([AFDKO / "tx", "-dump", "-4", "-gf", ref / "goadb", inputs[0]])
        got = [m.group(2) for m in map(self.ROW.match, text.splitlines()) if m]
        (ref / "names.json").write_text(json.dumps(got))

    def compare(self, cli, inputs, ref, ours):
        run([cli, "glyphs", "rename", inputs[0], "--goadb", ref / "goadb", "-o", ours / "out.otf"], error=OursFailed)
        got = TTFont(ours / "out.otf", lazy=True).getGlyphOrder()
        want = json.loads((ref / "names.json").read_text())
        return h.compare_maps(dict(enumerate(got)), dict(enumerate(want)), "names")


class NoHints:
    def cases(self, corpus):
        return [(n, i) for n, i in outline_fonts(corpus) if h.has_table(i[0], b"CFF ")]

    @staticmethod
    def glyphs(text: str) -> dict[str, str]:
        out, key = {}, None
        for line in text.splitlines():
            if m := re.match(r"glyph\[(\d+)\] ", line):
                key = m.group(1)
                out[key] = line
            elif key:
                out[key] += "\n" + line.strip()
        return out

    def reference(self, inputs, ref):
        (ref / "dump.txt").write_text(run([AFDKO / "tx", "-dump", "-6", "-n", inputs[0]]))

    def compare(self, cli, inputs, ref, ours):
        run([cli, "dehint", inputs[0], "-o", ours / "out.otf"], error=OursFailed)
        text = run([AFDKO / "tx", "-dump", "-6", ours / "out.otf"], error=OursFailed)
        return h.compare_maps(self.glyphs(text), self.glyphs((ref / "dump.txt").read_text()), "glyphs")


h.MEASURES = {"sfntedit": SfntEdit(), "spot": Spot(), "sfntdiff": SfntDiff(), "mtx": Mtx(), "afm": Afm(),
              "goadb": Goadb(), "nohints": NoHints()}
h.TITLE = "The command-line tools against AFDKO's C tools"
# Explained differences, as in python_tools.py; every reason is also in docs/spec/20-cli.md.
h.EXPECTED = []

if __name__ == "__main__":
    sys.exit(h.main(__doc__))
