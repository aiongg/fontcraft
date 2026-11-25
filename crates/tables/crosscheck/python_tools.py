#!/usr/bin/env python3
"""Measure the command-line tools against the Python reference tools, over the corpus.

Oracles: fontTools 4.66.1 (cu2qu, pens.statisticsPen, ttLib.reorderGlyphs, diff, pyftsubset) and
AFDKO 5.0.1 (ttxn, ttfcomponentizer, ttfdecomponentizer, otf2otc, otc2otf). The script runs them
and reads what they write; it copies no code of theirs.

Measures (`--measure`, all by default):

  ttxn         `ttxn FONT` against `dump --normalize`: the text of each table element
  cu2qu        `fonttools cu2qu UFO` against `outlines to-quadratic`: each glyph's points
  componentize `ttfcomponentizer` against `outlines componentize --reference` on fontmake builds
               of the corpus UFOs whose components were decomposed: each glyph's glyf entry
  decompose    `ttfdecomponentizer` against `outlines decompose`: each glyph's glyf entry
  statistics   `fonttools pens.statisticsPen` against `metrics --statistics`: the numbers
  reorder      `ttLib.reorderGlyphs` (the glyph order reversed) against `glyphs reorder`: the TTX
               of each table
  diff         `fonttools diff` (-l, and the table diffs) against `diff`: the verdict per table
  collection   `otf2otc` and `otc2otf` against `collection pack` and `unpack`: the collection's
               bytes, and the tables of each member
  dehint       `pyftsubset --no-hinting` with everything else kept against `dehint`: the hint
               tables, the glyf instructions and the CFF charstrings and Private dicts

A case is one font, UFO or pair of fonts; it gives one result per table, glyph set or file:

  equal      the two outputs agree
  expected   they differ for a reason `EXPECTED` gives (a choice of ours, or a fault of the tool)
  different  they differ and nothing explains it: a bug, or a difference to write into 20-cli.md
  reference  the reference tool refused the input or failed
  failed     our command refused an input the reference tool accepts

    python3 -I crates/tables/crosscheck/python_tools.py --cli target/release/fontcraft-cli \\
        --corpus corpus --out DIR [--measure NAME ...] [--jobs 4] [--limit N] [--match TEXT]
        [--reference-only] [--fresh]

The reference outputs stay under DIR/ref and are reused; our outputs and results (DIR/ours,
DIR/cases) are reused while they are newer than the command-line tool. The report is
DIR/report.md, the results of every case DIR/results.json. `--reference-only` runs the reference
half alone and needs no --cli. A Python without fontTools starts the script again with the
fontmake tool's Python.
"""

from __future__ import annotations

import argparse
import concurrent.futures as cf
import hashlib
import json
import multiprocessing
import os
import re
import subprocess
import sys
import traceback
from collections import Counter, defaultdict
from pathlib import Path

TOOLS = Path.home() / ".local/share/uv/tools"
try:
    import fontTools  # noqa: F401
except ImportError:
    _py = TOOLS / "fontmake/bin/python"
    if not _py.exists() or os.environ.get("CROSSCHECK_REEXEC"):
        sys.exit("fontTools is not installed for this Python, nor a fontmake tool environment")
    os.environ["CROSSCHECK_REEXEC"] = "1"
    os.execv(str(_py), [str(_py), "-I", *sys.argv])

from fontTools.ttLib import TTFont

REPO = Path(__file__).resolve().parents[3]
FONTTOOLS = TOOLS / "fontmake/bin/fonttools"
FONTMAKE = TOOLS / "fontmake/bin/fontmake"
AFDKO = TOOLS / "afdko/bin"
TIMEOUT_S = 600
MAX_JOBS = 4
MAX_DETAIL = 400
# Tables whose bytes record when or by whom a font was written.
EPHEMERAL_HEAD = ("checkSumAdjustment", "modified")
HINT_TABLES = ("cvt ", "cvar", "fpgm", "prep", "hdmx", "VDMX")
ENV = {**os.environ, "PATH": f"{AFDKO}:{os.environ.get('PATH', '')}", "SOURCE_DATE_EPOCH": "1000000000"}

# (measure, pattern on "result name: detail", reason) of each difference that is explained. The first
# entry that matches wins; every reason is also in docs/spec/20-cli.md.
EXPECTED: list[tuple[str, str, str]] = []


TITLE = "The command-line tools against the Python reference tools"


class ReferenceFailed(Exception):
    """The reference tool refused the input."""


class OursFailed(Exception):
    """Our command failed."""


def run(cmd, cwd=None, ok=(0,), error=ReferenceFailed) -> str:
    try:
        r = subprocess.run([str(c) for c in cmd], cwd=cwd, env=ENV, stdin=subprocess.DEVNULL,
                           capture_output=True, timeout=TIMEOUT_S)
    except subprocess.TimeoutExpired:
        raise error(f"more than {TIMEOUT_S} s") from None
    except OSError as e:
        raise error(str(e)) from None
    if r.returncode not in ok:
        text = (r.stderr or r.stdout).decode("utf-8", "replace").strip()
        raise error(f"exit {r.returncode}: {text[-300:]}")
    return r.stdout.decode("utf-8", "replace")


def slug(text: str) -> str:
    return f"{Path(text).stem[:40]}-{hashlib.sha1(text.encode()).hexdigest()[:8]}"


def short(value) -> str:
    text = str(value)
    return text if len(text) <= MAX_DETAIL else text[:MAX_DETAIL] + "..."


def open_font(path) -> TTFont:
    return TTFont(path, recalcBBoxes=False, recalcTimestamp=False, lazy=True)


def first_difference(a: str, b: str) -> str:
    """The first differing line of ours (`a`) and theirs (`b`)."""
    la, lb = a.splitlines(), b.splitlines()
    for i in range(max(len(la), len(lb))):
        x = la[i] if i < len(la) else "<end>"
        y = lb[i] if i < len(lb) else "<end>"
        if x != y:
            return short(f"line {i + 1}: ours {x.strip()[:150]!r}, theirs {y.strip()[:150]!r}")
    return ""


def compare_maps(ours: dict, theirs: dict, what: str) -> list[tuple[str, str, str]]:
    """One result for two maps: equal, or the count of differing keys and the first one."""
    keys = [k for k in {**theirs, **ours} if ours.get(k) != theirs.get(k)]
    if not keys:
        return [(what, "equal", "")]
    k = keys[0]
    return [(what, "different", short(f"{len(keys)} of {len(theirs)}; {k}: ours {ours.get(k)!r}"[:MAX_DETAIL // 2]
                                      + f" theirs {theirs.get(k)!r}"[:MAX_DETAIL // 2]))]


# ---------------------------------------------------------------- inputs


class Corpus:
    def __init__(self, root: Path, match: str | None, limit: int):
        self.root, self.match, self.limit = root, match, limit
        self._fonts: list[Path] | None = None

    def _pick(self, paths):
        if self.match:
            paths = [p for p in paths if self.match in str(p)]
        return paths[: self.limit] if self.limit else paths

    def rel(self, path: Path) -> str:
        return str(path.relative_to(self.root)) if path.is_relative_to(self.root) else str(path)

    def fonts(self) -> list[Path]:
        """Every TTF and OTF, files with the same bytes once."""
        if self._fonts is None:
            seen, out = set(), []
            for root, dirs, files in os.walk(self.root, followlinks=True):
                dirs[:] = sorted(d for d in dirs if d != ".git")
                for f in sorted(files):
                    path = Path(root) / f
                    if path.suffix.lower() in (".ttf", ".otf"):
                        digest = hashlib.sha1(path.read_bytes()).digest()
                        if digest not in seen:
                            seen.add(digest)
                            out.append(path)
            self._fonts = out
        return self._pick(self._fonts)

    def ttf(self) -> list[Path]:
        return [p for p in self.fonts() if has_table(p, b"glyf")]

    def ufos(self) -> list[Path]:
        out = []
        for root, dirs, _ in os.walk(self.root, followlinks=True):
            dirs[:] = sorted(d for d in dirs if d != ".git")
            out += [Path(root) / d for d in dirs if d.endswith(".ufo")]
            dirs[:] = [d for d in dirs if not d.endswith(".ufo")]
        return self._pick(sorted(out))

    def pairs(self) -> list[tuple[Path, Path]]:
        """Neighbours in one folder, with the same kind of outlines."""
        by_dir: dict[Path, list[Path]] = defaultdict(list)
        for p in self.fonts():
            if "aots" not in p.parts:
                by_dir[p.parent].append(p)
        out = []
        for members in by_dir.values():
            out += [(a, b) for a, b in zip(members, members[1:]) if magic(a) == magic(b)]
        return out


def magic(path: Path) -> bytes:
    with open(path, "rb") as fh:
        return fh.read(4)


def tags(path: Path) -> list[bytes]:
    with open(path, "rb") as fh:
        head = fh.read(12)
        if head[:4] not in (b"\0\1\0\0", b"OTTO", b"true"):
            return []
        return [fh.read(16)[:4] for _ in range(min(int.from_bytes(head[4:6], "big"), 512))]


def has_table(path: Path, tag: bytes) -> bool:
    return tag in tags(path)


# ---------------------------------------------------------------- measures
#
# Each measure has `cases(corpus) -> [(name, inputs)]`, `reference(inputs, ref)`, which writes the
# reference output into the folder `ref`, and `compare(cli, inputs, ref, ours)`, which runs our
# command into the folder `ours` and gives [(result name, status, detail)].


class Ttxn:
    def cases(self, corpus):
        return [(corpus.rel(p), [p]) for p in corpus.fonts()]

    def reference(self, inputs, ref):
        run([AFDKO / "ttxn", "-o", ref / "dump.ttx", inputs[0]])
        if not (ref / "dump.ttx").exists():
            raise ReferenceFailed("no output")
        text = (ref / "dump.ttx").read_text("utf-8", "replace")
        if "</ttFont>" not in text:
            raise ReferenceFailed("the dump is cut short")

    @staticmethod
    def elements(text: str) -> dict[str, str]:
        out: dict[str, str] = {}
        tag, body = None, []
        for line in text.splitlines():
            if tag is None:
                m = re.fullmatch(r"  <([A-Za-z0-9_.]+)( [^>]*)?>", line)
                if m:
                    tag, body = m.group(1), []
                else:
                    m = re.fullmatch(r"  <([A-Za-z0-9_.]+)( [^>]*)?/>", line)
                    if m:
                        out[m.group(1)] = line
            elif line == f"  </{tag}>":
                out[tag], tag = "\n".join(x.rstrip() for x in body if x.strip()), None
            else:
                body.append(line)
        return out

    def compare(self, cli, inputs, ref, ours):
        run([cli, "dump", inputs[0], "--normalize", "-o", ours / "dump.ttx"], error=OursFailed)
        a = self.elements((ours / "dump.ttx").read_text("utf-8", "replace"))
        b = self.elements((ref / "dump.ttx").read_text("utf-8", "replace"))
        out = []
        for tag in {**b, **a}:
            if tag in a and tag in b:
                d = first_difference(a[tag], b[tag])
                out.append((tag, "different" if d else "equal", d))
            else:
                out.append((tag, "different", "ours only" if tag in a else "theirs only"))
        return out


def ufo_points(path: Path) -> dict[str, list]:
    from fontTools.pens.recordingPen import RecordingPointPen
    from fontTools.ufoLib import UFOReader

    out = {}
    glyphs = UFOReader(path, validate=False).getGlyphSet(validateRead=False)
    for name in glyphs.keys():
        pen = RecordingPointPen()
        glyphs[name].drawPoints(pen)
        rows = []
        for op, args, kwargs in pen.value:
            if op == "addPoint":
                (x, y), kind = args[0], args[1]
                rows.append((round(x, 3) + 0.0, round(y, 3) + 0.0, kind))
            elif op == "addComponent":
                rows.append((args[0], tuple(round(v, 6) for v in args[1])))
            else:
                rows.append(op)
        out[name] = rows
    return out


class Cu2qu:
    def cases(self, corpus):
        return [(corpus.rel(p), [p]) for p in corpus.ufos()]

    def reference(self, inputs, ref):
        run([FONTTOOLS, "cu2qu", "-o", ref / "out.ufo", inputs[0]])

    def compare(self, cli, inputs, ref, ours):
        run([cli, "outlines", "to-quadratic", inputs[0], "-o", ours / "out.ufo"], error=OursFailed)
        return compare_maps(ufo_points(ours / "out.ufo"), ufo_points(ref / "out.ufo"), "glyphs")


def glyf_entries(path: Path) -> dict[str, tuple]:
    font = open_font(path)
    glyf = font["glyf"]
    out = {}
    for name in font.getGlyphOrder():
        g = glyf[name]
        code = g.program.getBytecode().hex() if hasattr(g, "program") else ""
        if g.isComposite():
            parts = [(c.glyphName, c.flags & ~0x0021, c.getComponentInfo()[1]) for c in g.components]
            out[name] = ("composite", parts, code)
        elif g.numberOfContours > 0:
            flags = [f & 1 for f in g.flags]
            out[name] = ("simple", list(g.coordinates), flags, list(g.endPtsOfContours), code)
        else:
            out[name] = ("empty",)
    return out


class Decompose:
    def cases(self, corpus):
        out = []
        for p in corpus.ttf():
            try:
                font = open_font(p)
                if any(font["glyf"][g].isComposite() for g in font.getGlyphOrder()):
                    out.append((corpus.rel(p), [p]))
            except Exception:  # a test font fontTools cannot read
                continue
        return out

    def reference(self, inputs, ref):
        run([AFDKO / "ttfdecomponentizer", "-o", ref / "out.ttf", inputs[0]])
        glyf_entries(ref / "out.ttf")

    def compare(self, cli, inputs, ref, ours):
        run([cli, "outlines", "decompose", inputs[0], "-o", ours / "out.ttf"], error=OursFailed)
        return compare_maps(glyf_entries(ours / "out.ttf"), glyf_entries(ref / "out.ttf"), "glyf")


class Componentize:
    def cases(self, corpus):
        out = []
        for p in corpus.ufos():
            glyphs = p / "glyphs"
            if glyphs.is_dir() and any("<component" in g.read_text("utf-8", "replace")
                                       for g in sorted(glyphs.glob("*.glif"))[:5000]):
                out.append((corpus.rel(p), [p]))
        return out

    def reference(self, inputs, ref):
        run([FONTMAKE, "-u", inputs[0], "-o", "ttf", "--output-path", ref / "built.ttf",
             "--keep-overlaps"])
        run([AFDKO / "ttfdecomponentizer", "-o", ref / "plain.ttf", ref / "built.ttf"])
        # ttfcomponentizer takes the UFO it finds in the font's folder.
        if not (ref / inputs[0].name).exists():
            (ref / inputs[0].name).symlink_to(inputs[0])
        run([AFDKO / "ttfcomponentizer", "-o", ref / "out.ttf", ref / "plain.ttf"])
        glyf_entries(ref / "out.ttf")

    def compare(self, cli, inputs, ref, ours):
        run([cli, "outlines", "componentize", ref / "plain.ttf", "--reference", inputs[0],
             "-o", ours / "out.ttf"], error=OursFailed)
        return compare_maps(glyf_entries(ours / "out.ttf"), glyf_entries(ref / "out.ttf"), "glyf")


class Statistics:
    """Both sides read in font units: the reference tool scales each glyph to the em first."""

    FIELDS = ("area", "meanX", "meanY", "varianceX", "varianceY", "stddevX", "stddevY", "covariance",
              "correlation", "slant")
    FONT = {"weight": "weight", "weight (perceptual)": "weightPerceptual", "width": "width",
            "slant": "slant", "slant angle": "slantAngle", "slant (perceptual)": "slantPerceptual",
            "slant (perceptual) angle": "slantPerceptualAngle"}

    def cases(self, corpus):
        return [(corpus.rel(p), [p]) for p in corpus.fonts()
                if has_table(p, b"glyf") or has_table(p, b"CFF ") or has_table(p, b"CFF2")]

    def reference(self, inputs, ref):
        from fontTools.pens.statisticsPen import StatisticsPen

        font = open_font(inputs[0])
        glyphs = font.getGlyphSet()
        rows = {}
        for i, name in enumerate(font.getGlyphOrder()):
            pen = StatisticsPen(glyphset=glyphs)
            glyphs[name].draw(pen)
            rows[str(i)] = {f: float(getattr(pen, f)) for f in self.FIELDS} if pen.area else None
        text = run([FONTTOOLS, "pens.statisticsPen", "-q", inputs[0]])
        font_rows = {k.strip(): float(v) for k, _, v in (line.partition(":") for line in text.splitlines()) if v}
        (ref / "statistics.json").write_text(json.dumps({"glyphs": rows, "font": font_rows}))

    @staticmethod
    def close(x, y) -> bool:
        # The reference tool prints six significant digits for the font.
        return abs(x - y) <= max(1e-3, 2e-5 * max(abs(x), abs(y)))

    def compare(self, cli, inputs, ref, ours):
        text = run([cli, "metrics", inputs[0], "--statistics", "--precision", "real", "--format", "json"],
                   error=OursFailed)
        (ours / "statistics.json").write_text(text)
        doc, want = json.loads(text), json.loads((ref / "statistics.json").read_text())
        bad, total = [], 0
        rows = {str(g["gid"]): g for g in doc["glyphs"]}
        for gid, w in want["glyphs"].items():
            if w is None or not all(abs(v) < 1e300 for v in w.values()):
                continue
            total += 1
            s = rows.get(gid, {}).get("statistics")
            if not s:
                bad.append(f"glyph {gid}: ours has no statistics")
                continue
            got = {"area": s["area"], "meanX": s["mean"][0], "meanY": s["mean"][1],
                   "varianceX": s["variance"][0], "varianceY": s["variance"][1],
                   "stddevX": s["stddev"][0], "stddevY": s["stddev"][1], "covariance": s["covariance"],
                   "correlation": s["correlation"], "slant": s["slant"]}
            off = [f for f in self.FIELDS if not self.close(float(got[f] or 0.0), w[f])]
            if off:
                bad.append(f"glyph {gid} {off[0]}: ours {got[off[0]]}, theirs {w[off[0]]}")
        out = [("glyphs", "different" if bad else "equal", short(f"{len(bad)} of {total}; {bad[0]}") if bad else "")]
        fs, bad = doc.get("fontStatistics") or {}, []
        for name, key in self.FONT.items():
            if name in want["font"] and not self.close(float(fs.get(key) or 0.0), want["font"][name]):
                bad.append(f"{name}: ours {fs.get(key)}, theirs {want['font'][name]}")
        out.append(("font", "different" if bad else "equal", short("; ".join(bad))))
        return out


def table_xml(font: TTFont, tag: str, scratch: Path) -> str:
    path = scratch / f"xml.{os.getpid()}.{hashlib.sha1(tag.encode()).hexdigest()[:8]}.ttx"
    font.saveXML(path, tables=[tag], newlinestr="\n")
    text = path.read_text("utf-8")
    path.unlink()
    if tag == "head":
        for field in EPHEMERAL_HEAD:
            text = re.sub(rf"<{field} .*?/>", "", text)
    return text.split("\n", 2)[-1]


def compare_fonts(ours: Path, theirs: Path, scratch: Path, skip=()) -> list[tuple[str, str, str]]:
    """The TTX of each table; a table neither font can decompile is compared by its bytes."""
    a, b, out = open_font(ours), open_font(theirs), []
    ta, tb = ({t for t in f.reader.keys()} for f in (a, b))
    for tag in sorted(ta | tb):
        if tag in skip:
            continue
        if tag not in ta or tag not in tb:
            out.append((tag, "different", "ours only" if tag in ta else "theirs only"))
            continue
        if a.reader[tag] == b.reader[tag]:
            out.append((tag, "equal", ""))
            continue
        try:
            d = first_difference(table_xml(a, tag, scratch), table_xml(b, tag, scratch))
        except Exception as e:
            d = f"the bytes differ, and the table cannot be decompiled: {e}"
        out.append((tag, "different" if d else "equal", d))
    return out


class Reorder:
    @staticmethod
    def order(font: TTFont) -> list[str]:
        names = font.getGlyphOrder()
        return names[:1] + names[:0:-1]

    def cases(self, corpus):
        return [(corpus.rel(p), [p]) for p in corpus.fonts()]

    def reference(self, inputs, ref):
        from fontTools.ttLib.reorderGlyphs import reorderGlyphs

        font = TTFont(inputs[0], recalcBBoxes=False, recalcTimestamp=False)
        order = self.order(font)
        try:
            reorderGlyphs(font, order)
            font.save(ref / "out.font", reorderTables=None)
        except Exception as e:
            raise ReferenceFailed(f"{type(e).__name__}: {e}") from None
        (ref / "order.txt").write_text("\n".join(order) + "\n")

    def compare(self, cli, inputs, ref, ours):
        run([cli, "glyphs", "reorder", inputs[0], "--order-file", ref / "order.txt", "-o", ours / "out.font"],
            error=OursFailed)
        return compare_fonts(ours / "out.font", ref / "out.font", ours)


class Diff:
    def cases(self, corpus):
        return [(f"{corpus.rel(a)} + {b.name}", [a, b]) for a, b in corpus.pairs()]

    def reference(self, inputs, ref):
        text = run([FONTTOOLS, "diff", "-l", "--color", "never", *inputs], ok=(0, 1))
        verdict, section = {}, ""
        for line in text.splitlines():
            m = re.match(r"^(\*?) +(.{4}): (SAME|DIFF)", line)
            if m:
                verdict[m.group(2)] = "same" if m.group(3) == "SAME" else "different"
            elif line.startswith("Tables only in file"):
                section = "onlyA" if "file1" in line else "onlyB"
            elif line.startswith("Tables in both"):
                section = ""
            elif section and (m := re.match(r"^\*? +(.{4})(:| |$)", line)):
                verdict[m.group(1)] = section
        if not verdict:
            raise ReferenceFailed(f"no verdict in: {text[:200]!r}")
        fields = {}
        for tag, v in verdict.items():
            if v == "different":
                text = run([FONTTOOLS, "diff", "--color", "never", "-t", tag, "--", *inputs], ok=(0, 1, 2))
                fields[tag] = bool(re.search(r"^[-+](?![-+])", text, re.M))
        (ref / "verdict.json").write_text(json.dumps({"bytes": verdict, "fields": fields}))

    def compare(self, cli, inputs, ref, ours):
        want = json.loads((ref / "verdict.json").read_text())
        out = []
        doc = json.loads(run([cli, "diff", *inputs, "--summary", "--json"], ok=(0, 1), error=OursFailed))
        got = {}
        for t in doc["tables"].values():
            got[t["tag"]] = t["status"] if t["status"] not in ("same", "different") else ("same" if t["equal"] else "different")
        out += compare_maps(got, want["bytes"], "bytes")
        doc = json.loads(run([cli, "diff", *inputs, "--json"], ok=(0, 1), error=OursFailed))
        mine = {t["tag"]: t["status"] == "different" for t in doc["tables"].values()}
        out += compare_maps({t: mine.get(t) for t in want["fields"]}, want["fields"], "fields")
        return out


class Collection:
    def cases(self, corpus):
        return [(f"{corpus.rel(a)} + {b.name}", [a, b]) for a, b in corpus.pairs()]

    def reference(self, inputs, ref):
        # otc2otf writes beside its input.
        run([AFDKO / "otf2otc", "-o", ref / "pack.ttc", *inputs])
        run([AFDKO / "otc2otf", ref / "pack.ttc"], cwd=ref)

    @staticmethod
    def members(folder: Path) -> dict[str, dict[str, str]]:
        out = {}
        for path in sorted(folder.iterdir()):
            if path.suffix.lower() in (".ttf", ".otf"):
                font = open_font(path)
                rows = {}
                for tag in font.reader.keys():
                    data = font.reader[tag]
                    if tag == "head" and len(data) >= 36:
                        data = data[:8] + bytes(4) + data[12:28] + bytes(8) + data[36:]
                    rows[tag] = hashlib.sha1(data).hexdigest()
                out[path.name] = rows
        return out

    def compare(self, cli, inputs, ref, ours):
        run([cli, "collection", "pack", *inputs, "-o", ours / "pack.ttc"], error=OursFailed)
        a, b = (ours / "pack.ttc").read_bytes(), (ref / "pack.ttc").read_bytes()
        at = next((i for i, (x, y) in enumerate(zip(a, b)) if x != y), min(len(a), len(b)))
        out = [("pack", "equal" if a == b else "different",
                "" if a == b else f"{len(a)} and {len(b)} bytes, first difference at {at}")]
        folder = ours / "unpacked"
        folder.mkdir(exist_ok=True)
        run([cli, "collection", "unpack", ref / "pack.ttc", "--output-dir", folder], error=OursFailed)
        return out + compare_maps(self.members(folder), self.members(ref), "unpack")


class Dehint:
    def cases(self, corpus):
        return [(corpus.rel(p), [p]) for p in corpus.fonts()
                if has_table(p, b"glyf") or has_table(p, b"CFF ")]

    def reference(self, inputs, ref):
        run([FONTTOOLS, "subset", inputs[0], "--glyphs=*", "--no-hinting", "--desubroutinize", "--layout-features=*",
             "--layout-scripts=*", "--name-IDs=*", "--name-languages=*", "--name-legacy", "--notdef-outline",
             "--glyph-names", "--legacy-kern", "--legacy-cmap", "--symbol-cmap", "--drop-tables=",
             "--passthrough-tables", "--no-prune-unicode-ranges", f"--output-file={ref / 'out.font'}"])
        self.state(ref / "out.font")

    @staticmethod
    def state(path: Path) -> dict[str, dict]:
        font = open_font(path)
        out: dict[str, dict] = {"tables": {t: t in font.reader.keys() for t in HINT_TABLES}}
        if "glyf" in font.reader.keys():
            out["glyf"] = glyf_entries(path)
            maxp = font["maxp"]
            names = ("maxZones", "maxTwilightPoints", "maxStorage", "maxFunctionDefs", "maxInstructionDefs",
                     "maxStackElements", "maxSizeOfInstructions")
            out["maxp"] = {n: getattr(maxp, n) for n in names if hasattr(maxp, n)}
        if "CFF " in font.reader.keys():
            cff = font["CFF "].cff
            cff.desubroutinize()
            top = cff.topDictIndex[0]
            programs = {}
            for name in font.getGlyphOrder():
                cs = top.CharStrings[name]
                cs.decompile()
                programs[name] = [v.hex() if isinstance(v, bytes) else (round(v, 4) if isinstance(v, float) else v) for v in cs.program]
            out["charstrings"] = programs
            dicts = [fd.Private for fd in top.FDArray] if hasattr(top, "FDArray") else [top.Private]
            out["private"] = {f"{i}.{k}": v for i, p in enumerate(dicts) for k, v in sorted(p.rawDict.items()) if k != "Subrs"}
        return out

    def compare(self, cli, inputs, ref, ours):
        run([cli, "dehint", inputs[0], "-o", ours / "out.font"], error=OursFailed)
        a, b = self.state(ours / "out.font"), self.state(ref / "out.font")
        out = []
        for part in b:
            out += compare_maps(a.get(part, {}), b[part], part)
        return out


MEASURES = {"ttxn": Ttxn(), "cu2qu": Cu2qu(), "componentize": Componentize(), "decompose": Decompose(),
            "statistics": Statistics(), "reorder": Reorder(), "diff": Diff(), "collection": Collection(),
            "dehint": Dehint()}


# ---------------------------------------------------------------- running


def classify(measure: str, name: str, status: str, detail: str) -> tuple[str, str]:
    if status == "different":
        for m, pattern, why in EXPECTED:
            if m == measure and re.search(pattern, f"{name}: {detail}"):
                return "expected", f"{why} ({detail})"
    return status, detail


def work(job) -> dict:
    """One case, in a process of its own: the reference half if it is not there yet, then ours."""
    measure, name, inputs, out, cli, cli_time, fresh = job
    m = MEASURES[measure]
    key = slug(f"{measure}:{name}")
    ref, ours = out / "ref" / measure / key, out / "ours" / measure / key
    record = out / "cases" / measure / f"{key}.json"
    for d in (ref, ours, record.parent):
        d.mkdir(parents=True, exist_ok=True)
    done = ref / "done.json"
    result = {"measure": measure, "case": name, "results": []}
    try:
        failed = None
        if fresh or not done.exists():
            try:
                m.reference(inputs, ref)
                done.write_text("{}")
            except ReferenceFailed as e:
                failed = short(e)
                # A temporary folder the tool cannot write may be writable on the next run.
                if not re.search(r"temp file|temporary file|tmpfile|Read-only", failed):
                    done.write_text(json.dumps({"failed": failed}))
        else:
            failed = json.loads(done.read_text()).get("failed")
        if failed is not None:
            result["results"] = [("case", "reference", failed)]
        elif cli is None:
            result["results"] = [("case", "pending", "the reference half is there")]
        elif not fresh and record.exists() and record.stat().st_mtime > cli_time:
            return json.loads(record.read_text())
        else:
            result["results"] = m.compare(cli, inputs, ref, ours)
    except OursFailed as e:
        result["results"] = [("case", "failed", short(e))]
    except Exception:
        result["results"] = [("case", "failed", "script: " + short(traceback.format_exc()[-300:]))]
    result["results"] = [(n, *classify(measure, n, s, d)) for n, s, d in result["results"]]
    if cli is not None or failed_reference(result):
        record.write_text(json.dumps(result))
    return result


def failed_reference(result) -> bool:
    return any(s == "reference" for _, s, _ in result["results"])


def versions() -> dict[str, str]:
    def first(cmd):
        try:
            r = subprocess.run([str(c) for c in cmd], env=ENV, stdin=subprocess.DEVNULL, capture_output=True, timeout=60)
            return (r.stdout or r.stderr).decode("utf-8", "replace").strip().splitlines()[0]
        except (OSError, IndexError, subprocess.TimeoutExpired):
            return "not installed"

    afdko = first([AFDKO / "python", "-I", "-c", "import importlib.metadata as m; print(m.version('afdko'))"])
    return {"fontTools": fontTools.version, "fontmake": first([FONTMAKE, "--version"]), "AFDKO": afdko}


STATUSES = ["equal", "expected", "different", "failed", "reference", "pending"]


def report(out: Path, results: list[dict], cli) -> str:
    totals: dict[str, Counter] = defaultdict(Counter)
    cases: Counter = Counter()
    groups: dict[tuple, list] = defaultdict(list)
    for r in results:
        cases[r["measure"]] += 1
        for name, status, detail in r["results"]:
            totals[r["measure"]][status] += 1
            if status not in ("equal", "pending"):
                groups[(r["measure"], status, name)].append((r["case"], detail))
    lines = [f"# {TITLE}", "",
             ", ".join(f"{k} {v}" for k, v in versions().items()) + f"; ours: {cli or 'not run'}", "",
             "| Measure | Cases | " + " | ".join(STATUSES) + " |", "|---|---|" + "---|" * len(STATUSES)]
    for m in MEASURES:
        if cases[m]:
            lines.append(f"| {m} | {cases[m]} | " + " | ".join(str(totals[m][s]) for s in STATUSES) + " |")
    summary = "\n".join(lines[4:])
    lines += ["", "## Differences", ""]
    for (m, status, name), items in sorted(groups.items(), key=lambda kv: (kv[0][0], kv[0][1], -len(kv[1]))):
        lines.append(f"- **{m}** {status} `{name}` ({len(items)})")
        lines += [f"  - {case}: {short(detail).replace(chr(10), ' ')}" for case, detail in items[:5]]
    (out / "report.md").write_text("\n".join(lines) + "\n", encoding="utf-8")
    (out / "results.json").write_text(json.dumps(results, indent=1), encoding="utf-8")
    return summary


def main(doc: str = __doc__) -> int:
    ap = argparse.ArgumentParser(description=doc.split("\n\n")[0])
    ap.add_argument("--cli", type=Path, default=REPO / "target/release/fontcraft-cli")
    ap.add_argument("--corpus", type=Path, default=REPO / "corpus")
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--measure", nargs="*", choices=list(MEASURES), default=list(MEASURES))
    ap.add_argument("--jobs", type=int, default=MAX_JOBS)
    ap.add_argument("--limit", type=int, default=0, help="only the first N inputs of each kind")
    ap.add_argument("--match", help="only inputs whose path holds this text")
    ap.add_argument("--reference-only", action="store_true", help="run the reference tools only")
    ap.add_argument("--fresh", action="store_true", help="run both halves again")
    args = ap.parse_args()

    out = args.out.expanduser().resolve()
    out.mkdir(parents=True, exist_ok=True)
    cli = None if args.reference_only else args.cli.resolve()
    if cli is not None and not cli.exists():
        ap.error(f"{cli} does not exist: build it, or pass --reference-only")
    cli_time = cli.stat().st_mtime if cli else 0.0
    corpus = Corpus(args.corpus.resolve(), args.match, args.limit)
    jobs = []
    for measure in args.measure:
        for name, inputs in MEASURES[measure].cases(corpus):
            jobs.append((measure, name, inputs, out, cli, cli_time, args.fresh))
    results = []
    # Forked workers see the measures another script put in MEASURES.
    context = multiprocessing.get_context("fork")
    with cf.ProcessPoolExecutor(max_workers=max(1, min(args.jobs, MAX_JOBS)), mp_context=context) as pool:
        for i, r in enumerate(pool.map(work, jobs), 1):
            results.append(r)
            if i % 100 == 0 or i == len(jobs):
                print(f"{i} of {len(jobs)} cases ({r['measure']})", file=sys.stderr, flush=True)
    print(report(out, results, cli))
    print(f"report: {out / 'report.md'}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
