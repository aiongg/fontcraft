#!/usr/bin/env python3
"""The threshold survey of docs/spec/21-qa.md question 5: how often each check fires on the corpus.

Runs `audit.run` without its result cap (`maxResults`), through `fontcraft-cli run`, with

  * the `universal` profile on every corpus source (.glyphs, .glyphspackage, .ufo, .designspace),
  * the `outlines` profile on every compiled font of the corpus (TTF, OTF, WOFF, WOFF2),

and counts, per check, the results, the warnings and failures, and the share of each font's glyphs
it fires on. A check that fires on more than 20% of the glyphs of a font is listed with those fonts:
it is reviewed, and its threshold set or kept with a reason (21-qa.md § 10, question 5).

Fonts with fewer than 20 glyphs, and the fonts of the conformance suite (`aots`), are counted but
left out of the 20% rule.

    python3 -I crates/audit/crosscheck/threshold_survey.py --cli target/release/fontcraft-cli \\
        --corpus corpus --out DIR [--jobs 4] [--limit N] [--match TEXT] [--kind sources|binaries]
        [--baseline DIR] [--fresh]

Results of a font are reused while they are newer than the command-line tool. `--baseline DIR` is
the --out of an earlier run: the report then shows its warnings per glyph beside this run's. The
report is DIR/report.md, the numbers DIR/survey.json.
"""

from __future__ import annotations

import argparse
import concurrent.futures as cf
import hashlib
import json
import os
import plistlib
import re
import subprocess
import sys
from collections import Counter, defaultdict
from pathlib import Path

REPO = Path(__file__).resolve().parents[3]
MAX_JOBS = 4
TIMEOUT_S = 900
MAX_RESULTS = 1_000_000
SHARE = 0.20
MIN_GLYPHS = 20
BINARIES = {".ttf", ".otf", ".woff", ".woff2"}
SOURCES = {".glyphs", ".glyphspackage", ".ufo", ".designspace"}


def corpus_files(corpus: Path) -> tuple[list[Path], list[Path]]:
    sources, binaries = [], []
    for root, dirs, files in os.walk(corpus, followlinks=True):
        dirs[:] = sorted(d for d in dirs if d != ".git")
        packages = [d for d in dirs if Path(d).suffix in (".ufo", ".glyphspackage")]
        sources += [Path(root) / d for d in packages]
        dirs[:] = [d for d in dirs if d not in packages]
        for f in sorted(files):
            suffix = Path(f).suffix.lower()
            if suffix in BINARIES:
                binaries.append(Path(root) / f)
            elif suffix in SOURCES:
                sources.append(Path(root) / f)
    seen, unique = set(), []
    for p in binaries:
        digest = hashlib.sha1(p.read_bytes()).digest()
        if digest not in seen:
            seen.add(digest)
            unique.append(p)
    return sources, unique


def ufo_glyphs(ufo: Path) -> int:
    try:
        with open(ufo / "glyphs" / "contents.plist", "rb") as fh:
            return len(plistlib.load(fh))
    except (OSError, ValueError, plistlib.InvalidFileException):
        return 0


def glyph_count(path: Path) -> int:
    """Glyphs of the file, read without FontCraft; 0 when that takes more than a glance."""
    try:
        if path.suffix == ".ufo":
            return ufo_glyphs(path)
        if path.suffix == ".glyphspackage":
            return len(list((path / "glyphs").glob("*.glyph")))
        if path.suffix == ".glyphs":
            return len(re.findall(r"^\s*glyphname = ", path.read_text("utf-8", "replace"), re.M))
        if path.suffix == ".designspace":
            names = re.findall(r'filename="([^"]+\.ufo)"', path.read_text("utf-8", "replace"))
            return max((ufo_glyphs(path.parent / n) for n in names), default=0)
        data = path.read_bytes()
        if data[:4] in (b"\0\1\0\0", b"OTTO", b"true"):
            for i in range(min(int.from_bytes(data[4:6], "big"), 512)):
                rec = data[12 + 16 * i:28 + 16 * i]
                if rec[:4] == b"maxp":
                    at = int.from_bytes(rec[8:12], "big")
                    return int.from_bytes(data[at + 4:at + 6], "big")
    except OSError:
        pass
    return 0


def find_report(text: str) -> dict | None:
    """The result document among the lines `fontcraft-cli run` prints."""
    for line in reversed(text.splitlines()):
        try:
            doc = json.loads(line)
        except ValueError:
            continue
        for candidate in (doc, doc.get("result") if isinstance(doc, dict) else None):
            if isinstance(candidate, dict) and "checks" in candidate and "results" in candidate:
                return candidate
    return None


def survey_one(job) -> dict:
    cli, cli_time, path, rel, kind, out, fresh = job
    profile = "universal" if kind == "sources" else "outlines"
    record = out / kind / f"{path.stem[:40]}-{hashlib.sha1(rel.encode()).hexdigest()[:8]}.json"
    if not fresh and record.exists() and record.stat().st_mtime > cli_time:
        return json.loads(record.read_text())
    params = {"target": str(path), "profile": profile, "maxResults": MAX_RESULTS, "includePassed": True}
    row = {"font": rel, "kind": kind, "glyphs": glyph_count(path)}
    try:
        r = subprocess.run([str(cli), "run", "--cmd", "audit.run", "--params", json.dumps(params)],
                           stdin=subprocess.DEVNULL, capture_output=True, timeout=TIMEOUT_S)
        doc = find_report(r.stdout.decode("utf-8", "replace"))
        if doc is None:
            row["error"] = (r.stderr.decode("utf-8", "replace") or f"exit {r.returncode}").strip()[-300:]
    except subprocess.TimeoutExpired:
        doc, row["error"] = None, f"more than {TIMEOUT_S} s"
    if doc is not None:
        checks: dict[str, dict] = {}
        for res in doc["results"]:
            c = checks.setdefault(res["check"], {"results": 0, "serious": 0, "glyphs": set(), "codes": Counter()})
            c["results"] += 1
            if res["severity"] in ("warn", "fail"):
                c["serious"] += 1
            c["codes"][res["code"]] += 1
            if glyph := res.get("location", {}).get("glyph"):
                c["glyphs"].add(glyph)
        row["checks"] = {k: {**v, "glyphs": len(v["glyphs"]), "codes": dict(v["codes"])} for k, v in sorted(checks.items())}
        row["ran"] = sum(1 for c in doc["checks"] if c.get("status") != "skipped")
        row["errors"] = [c["id"] for c in doc["checks"] if c.get("status") == "error"]
    record.parent.mkdir(parents=True, exist_ok=True)
    record.write_text(json.dumps(row))
    return row


def totals(rows: list[dict], kind: str) -> tuple[int, int]:
    picked = [r for r in rows if r["kind"] == kind and "checks" in r]
    return sum(c["serious"] for r in picked for c in r["checks"].values()), sum(r["glyphs"] for r in picked)


def report(out: Path, rows: list[dict], baseline: list[dict] | None) -> str:
    lines = ["# Threshold survey", ""]
    for kind, profile in (("sources", "universal"), ("binaries", "outlines")):
        picked = [r for r in rows if r["kind"] == kind]
        if not picked:
            continue
        serious, glyphs = totals(rows, kind)
        lines += [f"## {kind.capitalize()} (`{profile}`)", "",
                  f"{len(picked)} fonts, {sum('error' in r for r in picked)} failed, {glyphs} glyphs; "
                  f"{serious} warnings and failures, {serious / max(glyphs, 1):.3f} per glyph."]
        if baseline:
            b_serious, b_glyphs = totals(baseline, kind)
            lines.append(f"Baseline: {b_serious} warnings and failures, {b_serious / max(b_glyphs, 1):.3f} per glyph.")
        per: dict[str, dict] = defaultdict(lambda: {"results": 0, "serious": 0, "fonts": 0, "codes": Counter(), "above": []})
        for r in picked:
            for check, c in r.get("checks", {}).items():
                p = per[check]
                p["results"] += c["results"]
                p["serious"] += c["serious"]
                p["fonts"] += 1
                p["codes"].update(c["codes"])
                if r["glyphs"] >= MIN_GLYPHS and "aots" not in r["font"] and c["glyphs"] > SHARE * r["glyphs"]:
                    p["above"].append((c["glyphs"] / r["glyphs"], r["font"]))
        before = Counter()
        for r in baseline or []:
            if r["kind"] == kind:
                for check, c in r.get("checks", {}).items():
                    before[check] += c["serious"]
        lines += ["", "| Check | Results | Warnings and failures | Before | Fonts | Fonts above 20% of glyphs | Codes |",
                  "|---|---|---|---|---|---|---|"]
        for check, p in sorted(per.items(), key=lambda kv: -kv[1]["results"]):
            codes = ", ".join(f"{k} {v}" for k, v in p["codes"].most_common(4))
            lines.append(f"| `{check.removeprefix('check.')}` | {p['results']} | {p['serious']} | "
                         f"{before[check] if baseline else '-'} | {p['fonts']} | {len(p['above'])} | {codes} |")
        lines += ["", "### Checks above 20% of a font's glyphs", ""]
        for check, p in sorted(per.items(), key=lambda kv: -len(kv[1]["above"])):
            if p["above"]:
                worst = sorted(p["above"], reverse=True)[:5]
                lines.append(f"- `{check}` ({len(p['above'])} fonts): " + "; ".join(f"{f} {s:.0%}" for s, f in worst))
        failed = [r for r in picked if "error" in r]
        if failed:
            lines += ["", "### Failed", ""] + [f"- {r['font']}: {r['error']}" for r in failed]
        lines.append("")
    (out / "report.md").write_text("\n".join(lines), encoding="utf-8")
    (out / "survey.json").write_text(json.dumps(rows), encoding="utf-8")
    return "\n".join(x for x in lines if x.startswith(("## ", "Baseline")) or " fonts, " in x)


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--cli", type=Path, default=REPO / "target/release/fontcraft-cli")
    ap.add_argument("--corpus", type=Path, default=REPO / "corpus")
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--kind", choices=["sources", "binaries"])
    ap.add_argument("--jobs", type=int, default=MAX_JOBS)
    ap.add_argument("--limit", type=int, default=0, help="only the first N files of each kind")
    ap.add_argument("--match", help="only files whose path holds this text")
    ap.add_argument("--baseline", type=Path, help="the --out of an earlier run")
    ap.add_argument("--fresh", action="store_true")
    args = ap.parse_args()

    out, corpus = args.out.expanduser().resolve(), args.corpus.resolve()
    out.mkdir(parents=True, exist_ok=True)
    cli = args.cli.resolve()
    if not cli.exists():
        ap.error(f"{cli} does not exist: build it first")
    jobs = []
    for kind, files in zip(("sources", "binaries"), corpus_files(corpus)):
        if args.kind not in (None, kind):
            continue
        files = [f for f in files if not args.match or args.match in str(f)]
        for f in files[: args.limit] if args.limit else files:
            jobs.append((cli, cli.stat().st_mtime, f, str(f.relative_to(corpus)), kind, out, args.fresh))
    rows = []
    with cf.ThreadPoolExecutor(max_workers=max(1, min(args.jobs, MAX_JOBS))) as pool:
        for i, row in enumerate(pool.map(survey_one, jobs), 1):
            rows.append(row)
            if i % 50 == 0 or i == len(jobs):
                print(f"{i} of {len(jobs)}", file=sys.stderr, flush=True)
    baseline = json.loads((args.baseline / "survey.json").read_text()) if args.baseline else None
    print(report(out, rows, baseline))
    print(f"report: {out / 'report.md'}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
