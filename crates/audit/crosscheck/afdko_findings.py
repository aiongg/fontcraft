#!/usr/bin/env python3
"""Compare the findings of AFDKO's checkers with FontCraft's, finding by finding, over the corpus.

Oracles: AFDKO 5.0.1 (Apache-2.0): checkoutlinesufo and comparefamily. The script runs them and
reads what they print; it copies no code of theirs.

Measures (`--measure`, both by default):

  outlines  `checkoutlinesufo --all` on a copy of each corpus UFO against `check outlines`. A
            finding is a glyph and a kind of problem. One result per kind.
  family    `comparefamily -d` on each family of the corpus (the static fonts below one folder that
            share a typographic family name and a kind of outline, two or more) against
            `check family`. A finding is a numbered test and a font. One result per test that
            reports in either tool; the checks of a test come from the tables of
            docs/spec/21-qa.md. A finding that names no font counts for the font "-".

    python3 -I crates/audit/crosscheck/afdko_findings.py --cli target/release/fontcraft-cli \\
        --corpus corpus --out DIR [--measure NAME ...] [--jobs 4] [--limit N] [--match TEXT]
        [--reference-only] [--fresh]

The harness (cases, resuming, statuses, report) is crates/tables/crosscheck/python_tools.py.
"""

from __future__ import annotations

import importlib.util
import json
import re
import shutil
import sys
from collections import defaultdict
from pathlib import Path

REPO = Path(__file__).resolve().parents[3]
_spec = importlib.util.spec_from_file_location("python_tools", REPO / "crates/tables/crosscheck/python_tools.py")
h = importlib.util.module_from_spec(_spec)
sys.modules["python_tools"] = h
_spec.loader.exec_module(h)

AFDKO, run, OursFailed, ReferenceFailed = h.AFDKO, h.run, h.OursFailed, h.ReferenceFailed
SPEC = REPO / "docs/spec/21-qa.md"
MAX_LISTED = 6

# What checkoutlinesufo prints -> (kind, our check).
KINDS = [
    (re.compile(r"Coincident points at"), "coincident", "check.outlines.coincidentPoints"),
    (re.compile(r"Flat curve at"), "flat curve", "check.outlines.flatCurve"),
    (re.compile(r"Colinear line at"), "collinear line", "check.outlines.collinearLines"),
    (re.compile(r"There is an overlap"), "overlap", "check.outlines.overlap"),
    (re.compile(r"is too small: bounding box"), "tiny contour", "check.outlines.tinyContour"),
    (re.compile(r"has no contours"), "no contours", "check.outlines.noContours"),
]
BASIC = {"coincident", "flat curve", "collinear line"}


def listed(names) -> str:
    names = sorted(names)
    return ", ".join(names[:MAX_LISTED]) + (f", {len(names) - MAX_LISTED} more" if len(names) > MAX_LISTED else "")


def compare_sets(name: str, ours: set, theirs: set) -> tuple[str, str, str]:
    if ours == theirs:
        return (name, "equal", "")
    parts = [f"{len(ours & theirs)} in both"]
    if ours - theirs:
        parts.append(f"ours only {len(ours - theirs)}: {listed(ours - theirs)}")
    if theirs - ours:
        parts.append(f"theirs only {len(theirs - ours)}: {listed(theirs - ours)}")
    return (name, "different", h.short("; ".join(parts)))


class Outlines:
    def cases(self, corpus):
        return [(corpus.rel(p), [p]) for p in corpus.ufos()]

    def reference(self, inputs, ref):
        copy = ref / "in.ufo"
        if copy.exists():
            shutil.rmtree(copy)
        # The tool stores the hashes of the glyphs it checked in the UFO.
        shutil.copytree(inputs[0], copy)
        text = run([AFDKO / "checkoutlinesufo", "--all", copy])
        found: dict[str, list[str]] = defaultdict(list)
        for line in text.splitlines():
            m = re.match(r"(\S+) (.*)$", line.strip())
            if m:
                for pattern, kind, _ in KINDS:
                    if pattern.search(m.group(2)):
                        found[kind].append(m.group(1))
        (ref / "findings.json").write_text(json.dumps(found))
        composites = []
        for glif in (copy / "glyphs").glob("*.glif"):
            body = glif.read_text("utf-8", "replace")
            if "<component" in body and (m := re.search(r'<glyph [^>]*name="([^"]+)"', body)):
                composites.append(m.group(1))
        (ref / "composites.json").write_text(json.dumps(composites))

    def compare(self, cli, inputs, ref, ours):
        text = run([cli, "check", "outlines", inputs[0], "--all", "--format", "json"], ok=(0, 1), error=OursFailed)
        (ours / "check.json").write_text(text)
        doc = json.loads(text)
        want = json.loads((ref / "findings.json").read_text())
        composites = set(json.loads((ref / "composites.json").read_text()))
        out = []
        for _, kind, check in KINDS:
            rows = [r for r in doc["results"] if r["check"] == check]
            if any(r["code"] == "too-many-results" for r in rows):
                out.append((kind, "different", "ours collapsed the results of this check (audit.resultCap)"))
                continue
            mine = {r["location"].get("glyph", "-") for r in rows if r["severity"] in ("warn", "fail")}
            theirs = set(want.get(kind, []))
            name, status, detail = compare_sets(kind, mine, theirs)
            if status == "different" and kind in BASIC and mine <= theirs and theirs - mine <= composites:
                status = "expected"
                detail += " (the tool decomposes components; ours reports the glyph that holds the path)"
            out.append((name, status, detail))
        return out


def test_tables() -> dict[str, list[str]]:
    """"single N" and "family N" -> our check ids, from the two tables of 21-qa.md."""
    out: dict[str, list[str]] = {}
    group = None
    for line in SPEC.read_text("utf-8").splitlines():
        if m := re.match(r"\| (single-face|family) test \| FontCraft check", line):
            group = "single" if m.group(1) == "single-face" else "family"
        elif group and (m := re.match(r"\| (\d+) \| (.*) \|$", line)):
            out[f"{group} {m.group(1)}"] = [f"check.{c}" for c in re.findall(r"`([A-Za-z0-9.]+)`", m.group(2))]
        elif not line.startswith("|"):
            group = None
    return out


class Family:
    def cases(self, corpus):
        groups: dict[tuple, dict[str, Path]] = defaultdict(dict)
        for p in corpus.fonts():
            tags = h.tags(p)
            if "aots" in p.parts or b"fvar" in tags or b"name" not in tags or b"OS/2" not in tags:
                continue
            try:
                name = h.open_font(p)["name"]
            except Exception:  # a test font fontTools cannot read
                continue
            family, ps = name.getDebugName(16) or name.getDebugName(1), name.getDebugName(6)
            if not family or not ps or not re.fullmatch(r"[A-Za-z0-9_-]+", ps):
                continue
            rel = Path(corpus.rel(p)).parts
            # The folder of one release: oswald/legacy/3.0, inter/docs/font-files, ...
            folder = "/".join(rel[:min(3, len(rel) - 1)])
            unhinted = "unhinted" in p.name
            groups[(folder, family, h.magic(p), unhinted)].setdefault(ps, p)
        out = []
        for (folder, family, kind, unhinted), fonts in sorted(groups.items(), key=lambda kv: str(kv[0])):
            if len(fonts) >= 2:
                label = f"{folder}: {family} ({'CFF' if kind == b'OTTO' else 'TrueType'}{', unhinted' if unhinted else ''}, {len(fonts)} fonts)"
                out.append((label, [list(fonts.items())]))
        return out

    @staticmethod
    def folder(inputs, dest: Path) -> list[str]:
        dest.mkdir(exist_ok=True)
        for ps, path in inputs[0]:
            copy = dest / f"{ps}{path.suffix.lower()}"
            if not copy.exists():
                shutil.copy(path, copy)
        return sorted((ps for ps, _ in inputs[0]), key=len, reverse=True)

    def reference(self, inputs, ref):
        names = self.folder(inputs, ref / "fonts")
        text = run([AFDKO / "comparefamily", "-d", ref / "fonts"], ok=(0, 1))
        (ref / "report.txt").write_text(text)
        if "Single Face Test 1" not in text or "Family Test 1" not in text:
            raise ReferenceFailed(f"the report is cut short: {text[-200:]!r}")
        found: dict[str, set[str]] = defaultdict(set)
        test, message = None, []

        def flush():
            if test and message:
                text = " ".join(message)
                # The longest names first, so that one name is not found inside another.
                hit = set()
                for n in names:
                    if re.search(rf"(?<![A-Za-z0-9_-]){re.escape(n)}(?![A-Za-z0-9_-])", text):
                        hit.add(n)
                found[test].update(hit or {"-"})
            message.clear()

        for line in text.splitlines():
            if m := re.match(r"(Single Face|Family) Test (\d+)\b", line):
                flush()
                test = f"{'single' if m.group(1) == 'Single Face' else 'family'} {m.group(2)}"
            elif re.match(r"\s*(Error|Warning)\b", line):
                flush()
                message.append(line)
            elif message:
                message.append(line)
        flush()
        (ref / "findings.json").write_text(json.dumps({k: sorted(v) for k, v in found.items()}))

    def compare(self, cli, inputs, ref, ours):
        fonts = sorted((ref / "fonts").iterdir())
        text = run([cli, "check", "family", *fonts, "--format", "json"], ok=(0, 1), error=OursFailed)
        (ours / "check.json").write_text(text)
        doc = json.loads(text)
        names = {f["id"]: Path(f["path"] or f["name"]).stem for f in doc["target"]["fonts"]}
        flagged: dict[str, set[str]] = defaultdict(set)
        for r in doc["results"]:
            if r["severity"] in ("warn", "fail"):
                where = r["location"]
                ids = where.get("fonts") or ([where["font"]] if where.get("font") else [])
                flagged[r["check"]].update(names.get(i, i) for i in ids)
                if not ids:
                    flagged[r["check"]].add("-")
        want = {k: set(v) for k, v in json.loads((ref / "findings.json").read_text()).items()}
        out, tables = [], test_tables()
        tests_of: dict[str, set[str]] = defaultdict(set)
        for test, checks in tables.items():
            for c in checks:
                tests_of[c].add(test)
        for test, checks in tables.items():
            mine, theirs = set().union(*(flagged[c] for c in checks)) if checks else set(), want.get(test, set())
            if not (mine or theirs):
                continue
            name, status, detail = compare_sets(test, mine, theirs)
            # One check of ours may stand for several tests: a font it flags may be about another one.
            if status == "different" and theirs <= mine and all(
                    all(any(font in want.get(t, set()) for t in tests_of[c] - {test})
                        for c in checks if font in flagged[c]) for font in mine - theirs):
                status = "expected"
                detail += " (the check also stands for another test, in which the tool reports the font)"
            out.append((name, status, detail))
        return out


h.MEASURES = {"outlines": Outlines(), "family": Family()}
h.TITLE = "FontCraft's findings against AFDKO's checkers"
# (measure, pattern on "result name: detail", reason) of each difference that is explained; every
# reason is also in docs/spec/21-qa.md § 9.
h.EXPECTED = [
    ("family", r"^single 6: ", "the tool reads its expected URL and foundry code from the environment and reports both "
                               "in every run without them; ours skips without the project values"),
]

if __name__ == "__main__":
    sys.exit(h.main(__doc__))
