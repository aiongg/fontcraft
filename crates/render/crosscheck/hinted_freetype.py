#!/usr/bin/env python3
"""Compare FontCraft's hinted outlines with FreeType's, glyph by glyph and size by size.

Oracle: FreeType's TrueType interpreter (version 40), through `freetype-py`. The fonts are

  * the corpus's compiled TrueType fonts with glyph programs (each distinct `glyf` once),
  * FontCraft's exports of the corpus sources that carry TrueType instructions, and of a synthetic
    font with every instruction type, direction and rounding mode, when --cli is given,
  * FontCraft's autohinted builds left by `crates/hinting/crosscheck/tt_autohint.py` (--autohinted DIR).

    cargo build --release -p fontcraft-render --example hinted_dump
    ENV/bin/python -I crates/render/crosscheck/hinted_freetype.py --corpus corpus --out DIR \
        --dump target/release/examples/hinted_dump --cli target/release/fontcraft-cli

Each font's grid-fitted outlines at 8 to 72 PPM are compared in two intents: grayscale with
FreeType's default mode, DirectWrite with its LCD mode (FreeType 2.13 has no mode without symmetric
smoothing, so GDI ClearType is only run). A glyph at a size is `equal` when every segment is the
same to 1/64 pixel, `close` when no point is more than 1/8 pixel away, else `differs`; `failed` when
only one of the two ran its program. In every intent the script counts the glyph sizes skrifa drew
unhinted, with the messages. The Python needs freetype-py and fonttools (the environment of
`tt_autohint.py` does). `--reference` runs FreeType's half alone.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import subprocess
import sys
from collections import Counter
from pathlib import Path

import freetype
from fontTools.ttLib import TTFont, TTLibError

INTENTS = {"grayscale": freetype.FT_LOAD_TARGET_NORMAL, "directwrite": freetype.FT_LOAD_TARGET_LCD, "cleartype": None}
FIRST, LAST = 8, 72
CLOSE = 8  # 64ths of a pixel
TIMEOUT_S = 3600
SUFFIXES = {".glyphs", ".glyphspackage", ".designspace", ".ufo"}


def instructed(path: Path) -> tuple[str, int, int] | None:
    """(hash of glyf, glyphs, glyphs with a program) of a static TrueType font with programs."""
    try:
        font = TTFont(path, lazy=True)
        if "glyf" not in font or "fvar" in font:
            return None
        glyf = font["glyf"]
        programs = 0
        for name in font.getGlyphOrder():
            g = glyf[name]
            g.expand(glyf)
            programs += bool(getattr(g, "program", None) and g.program.getBytecode())
        if not programs:
            return None
        return hashlib.sha256(font.reader["glyf"]).hexdigest(), len(font.getGlyphOrder()), programs
    except (TTLibError, KeyError, AssertionError, ValueError, OSError, AttributeError):
        return None


def freetype_path(face, gid: int, target: int) -> str:
    face.load_glyph(gid, target | freetype.FT_LOAD_NO_BITMAP | freetype.FT_LOAD_NO_AUTOHINT)
    out: list[str] = []
    at = lambda *ps: " ".join(f"{p.x} {p.y}" for p in ps)  # noqa: E731

    def move(a, _):
        if out:
            out.append("Z")
        out.append("M " + at(a))

    def line(a, _):
        out.append("L " + at(a))

    def conic(a, b, _):
        out.append("Q " + at(a, b))

    def cubic(a, b, c, _):
        out.append("C " + at(a, b, c))

    face.glyph.outline.decompose(None, move_to=move, line_to=line, conic_to=conic, cubic_to=cubic)
    if out:
        out.append("Z")
    return " ".join(out)


def normal(path: str) -> list[tuple[str, tuple[float, ...]]]:
    """The path's segments without the closing lines, which one side writes and the other may not."""
    segments: list[tuple[str, tuple[float, ...]]] = []
    start: tuple[float, ...] = ()
    words = path.split()
    i = 0
    while i < len(words):
        name = words[i]
        n = {"M": 2, "L": 2, "Q": 4, "C": 6, "Z": 0}[name]
        values = tuple(float(w) for w in words[i + 1 : i + 1 + n])
        i += 1 + n
        if name == "M":
            start = values
        if name == "Z":
            if segments and segments[-1][0] == "L" and segments[-1][1] == start:
                segments.pop()
            continue
        segments.append((name, values))
    return segments


def verdict(a: str, b: str) -> str:
    x, y = normal(a), normal(b)
    if x == y:
        return "equal"
    if [n for n, _ in x] != [n for n, _ in y]:
        return "differs"
    far = max((abs(p - q) for (_, u), (_, v) in zip(x, y) for p, q in zip(u, v)), default=0.0)
    return "equal" if far < 1 else "close" if far <= CLOSE else "differs"


def compare(args, path: Path, glyphs: int) -> dict:
    step = max(1, glyphs // args.glyphs) if args.glyphs else 1
    report: dict = {"font": str(path), "step": step}
    for intent, target in INTENTS.items():
        dump = subprocess.Popen(
            [str(args.dump), str(path), intent, str(FIRST), str(LAST), str(step)], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True
        )
        face = freetype.Face(str(path))
        counts, messages, examples = Counter(), Counter(), []
        size = 0
        for line in dump.stdout:
            head, _, message = line.rstrip("\n").partition("\t")
            ppem, gid, hinted, *rest = head.split(" ", 3)
            ours = rest[0] if rest else ""
            if message:
                messages[message.split(" at ")[0][:80]] += 1
            if target is None:
                counts["hinted" if hinted == "1" else "unhinted"] += 1
                continue
            if int(ppem) != size:
                size = int(ppem)
                face.set_pixel_sizes(0, size)
            try:
                theirs = freetype_path(face, int(gid), target)
                v = verdict(ours, theirs) if hinted == "1" or not message or "turn hinting off" in message else "failed"
            except freetype.FT_Exception as e:
                v = "equal" if hinted == "0" else "failed"
                messages[f"FreeType: {e}"[:80]] += 1
                theirs = ""
            counts[v] += 1
            if v in ("differs", "failed") and len(examples) < 20:
                examples.append({"ppem": size, "gid": int(gid), "fontcraft": ours[:400], "freetype": theirs[:400], "message": message})
        if dump.wait(TIMEOUT_S) != 0:
            counts["dump failed"] += 1
            messages[(dump.stderr.read().strip().splitlines() or ["?"])[-1][:120]] += 1
        report[intent] = {"counts": dict(counts), "messages": dict(messages), "examples": examples}
    return report


def exports(args) -> list[Path]:
    """FontCraft's TrueType exports of the corpus sources and the synthetic font (not autohinted)."""
    out = []
    fixture = args.out / "HintFixture.glyphs"
    fixture.write_text(synthetic())
    stack, files = [args.corpus.resolve()], [fixture]
    while stack:
        for p in sorted(stack.pop().iterdir()):
            if p.suffix in SUFFIXES:
                files.append(p)
            elif p.is_dir() and p.name != ".git":
                stack.append(p)
    for p in files:
        folder = args.out / "exports" / hashlib.sha256(str(p).encode()).hexdigest()[:12]
        params = {"formats": ["ttf"], "timestamp": 0, "versionStamp": False, "instances": False, "mastersAsInstances": True, "skipFeatures": True, "destination": str(folder)}
        if not folder.exists() and has_instructions(p):
            r = subprocess.run([str(args.cli), "run", "--in", str(p), "--cmd", "font.compile", "--params", json.dumps(params)], capture_output=True, text=True, timeout=TIMEOUT_S, check=False)
            if r.returncode != 0:
                print(f"no export of {p}: {(r.stderr.strip().splitlines() or ['?'])[-1][:200]}")
        out += sorted(folder.glob("*.ttf")) if folder.exists() else []
    return out


def synthetic() -> str:
    """A format 3 source: an E per instruction type, direction and rounding mode (`options` 0 round,
    1 up, 2 down, 3 GDI only, 4 no rounding), and a triple hint."""
    nodes = [(100, 0), (500, 0), (500, 80), (190, 84), (190, 310), (460, 310), (460, 390), (190, 390), (190, 615), (520, 620), (520, 712), (100, 712)]
    glyphs = []

    def glyph(name: str, hints: list[str]) -> None:
        path = ",\n".join(f"({x},{y},l)" for x, y in nodes)
        glyphs.append(
            f"{{\nglyphname = {name};\nlayers = (\n{{\nhints = (\n" + ",\n".join(hints) + f"\n);\nlayerId = m01;\nshapes = (\n{{\nclosed = 1;\nnodes = (\n{path}\n);\n}}\n);\nwidth = 600;\n}}\n);\n}}"
        )

    def hint(kind: str, vertical: bool, options: int, **fields: str) -> str:
        items = {"horizontal": "1"} if vertical else {}
        if options:
            items["options"] = str(options)
        items |= fields | {"type": kind}
        return "{\n" + "\n".join(f"{k} = {v};" for k, v in items.items()) + "\n}"

    for vertical in (True, False):
        d = "v" if vertical else "h"
        a, b, mid = ("(0,0)", "(0,10)", "(0,5)") if vertical else ("(0,0)", "(0,9)", "(0,3)")
        anchor = [hint("TTSnap", vertical, 0, origin=a), hint("TTSnap", vertical, 0, origin=b)]
        for r in range(5):
            glyph(f"snap{d}{r}", [hint("TTSnap", vertical, r, origin=mid)])
            glyph(f"stem{d}{r}", [anchor[0], hint("TTStem", vertical, r, origin=a, target="(0,2)" if vertical else mid)])
            glyph(f"free{d}{r}", [hint("TTStem", vertical, r, origin="(0,5)", target="(0,6)" if vertical else "(0,4)")])
            glyph(f"shift{d}{r}", [anchor[0], hint("TTShift", vertical, r, origin=a, target=mid)])
            glyph(f"mid{d}{r}", anchor + [hint("TTInterpolate", vertical, r, origin=a, other1=b, target=mid)])
    cells = '{\ndeltaV = {\n"{0, 0, 0, 0, 0, 0}" = {\n' + "".join(f'"{p}" = {8 if p % 2 else 16};\n' for p in range(8, 73, 3)) + "};\n};\n}"
    glyph("delta", [hint("TTSnap", True, 0, origin="(0,0)"), hint("TTDelta", True, 0, origin="(0,0)", settings=cells)])
    glyph("triple", [hint("TTStem", True, 128, origin=o, target=t) for o, t in (("(0,0)", "(0,2)"), ("(0,5)", "(0,6)"), ("(0,9)", "(0,10)"))])
    metrics = [("ascender", 800, 16), ("cap height", 700, 16), ("x-height", 500, 16), ("baseline", 0, -16), ("descender", -200, -16)]
    values = ",\n".join(f"{{\nover = {o};\npos = {p};\n}}" for _, p, o in metrics)
    kinds = ",\n".join(f'{{\ntype = "{k}";\n}}' for k, _, _ in metrics)
    return (
        '{\n.appVersion = "3343";\n.formatVersion = 3;\nfamilyName = "Hint Fixture";\nfontMaster = (\n{\nid = m01;\nmetricValues = (\n'
        + values
        + "\n);\nname = Regular;\nstemValues = (80,90);\n}\n);\nglyphs = (\n"
        + ",\n".join(glyphs)
        + "\n);\nmetrics = (\n"
        + kinds
        + "\n);\nstems = (\n{\nhorizontal = 1;\nname = h;\n},\n{\nname = v;\n}\n);\nunitsPerEm = 1000;\nversionMajor = 1;\nversionMinor = 0;\n}\n"
    )


def has_instructions(source: Path) -> bool:
    """A source whose text names a TrueType instruction, assembly or control value."""
    files = [source] if source.is_file() else [f for f in source.rglob("*") if f.is_file() and f.suffix in {".glyphs", ".glyph", ".glif", ".plist", ".designspace"}]
    marks = (b"TTInterpolate", b"TTStem", b"TTSnap", b"TTDelta", b"TTShift", b"public.truetype.instructions", b"cvt Table", b"TTFZones")
    return any(any(m in f.read_bytes() for m in marks) for f in files)


def reference(fonts: list[Path]) -> None:
    for path in fonts:
        face = freetype.Face(str(path))
        failed, drawn = Counter(), 0
        for size in (8, 12, 20, 40, 72):
            face.set_pixel_sizes(0, size)
            for gid in range(0, face.num_glyphs, max(1, face.num_glyphs // 100)):
                try:
                    drawn += len(normal(freetype_path(face, gid, freetype.FT_LOAD_TARGET_NORMAL))) >= 0
                except freetype.FT_Exception as e:
                    failed[str(e)] += 1
        print(f"{path.name}: {drawn} glyph sizes drawn, failed {dict(failed)}")


def main() -> None:
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--dump", type=Path, default=Path("target/release/examples/hinted_dump"))
    ap.add_argument("--cli", type=Path)
    ap.add_argument("--corpus", type=Path, default=Path("corpus"))
    ap.add_argument("--autohinted", type=Path, help="the --out folder of tt_autohint.py")
    ap.add_argument("--fonts", nargs="+", type=Path, help="only these fonts")
    ap.add_argument("--out", type=Path, required=True)
    ap.add_argument("--glyphs", type=int, default=300, help="about how many glyphs of each font (0: all)")
    ap.add_argument("--reference", action="store_true", help="list the fonts and run FreeType alone")
    args = ap.parse_args()
    args.out = args.out.resolve()
    args.out.mkdir(parents=True, exist_ok=True)
    fonts = args.fonts or sorted(p for p in args.corpus.resolve().rglob("*.ttf") if ".git" not in p.parts)
    if args.cli and not args.fonts:
        fonts += exports(args)
    if args.autohinted:
        fonts += sorted(args.autohinted.rglob("autohint/*.ttf"))
    seen: set[str] = set()
    todo = []
    for p in fonts:
        found = instructed(p)
        if found and found[0] not in seen:
            seen.add(found[0])
            todo.append((p, found[1]))
    print(f"FreeType {'.'.join(map(str, freetype.version()))}: {len(todo)} distinct fonts with glyph programs", flush=True)
    if args.reference:
        return reference([p for p, _ in todo])
    total: dict[str, Counter] = {i: Counter() for i in INTENTS}
    reports = []
    for path, glyphs in todo:
        r = compare(args, path, glyphs)
        reports.append(r)
        for i in INTENTS:
            total[i].update(r[i]["counts"])
        print(path.name, {i: r[i]["counts"] for i in INTENTS}, flush=True)
    (args.out / "report.json").write_text(json.dumps(reports, indent=1))
    for i, c in total.items():
        print(f"{i}: {dict(c)}")
    sys.exit(0)


if __name__ == "__main__":
    main()
