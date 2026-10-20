"""Cross-check the TTX writer and reader of `mort` and `morx` against fontTools' ttx. Run
locally; not part of the gate.

For every font that holds `mort` or `morx` (under the paths given, by default corpus/), and for
the fonts `--build` writes, each of those tables is checked twice:

    dump:  `fontcraft-cli dump FONT -t TAG -o OURS` against `ttx -t TAG -o THEIRS`: the table
           elements must be equal line for line, comments included.
    read:  `fontcraft-cli tables import FONT THEIRS -o OUT` against fontTools' compile of THEIRS
           into the same font: the table's bytes must be equal.

The corpus holds no font with either table, so `--build N` writes N fonts of each table with
fontTools into OUT_DIR/built: random chains of every subtable kind (rearrangement, contextual,
ligature, noncontextual, insertion) over a 40-glyph font, compiled by fontTools from text the
script writes. A random table fontTools refuses to compile is skipped and counted.

    VENV/bin/python -I crates/tables/crosscheck/morph.py [--cli PATH] [--build N] OUT_DIR [FONT_OR_DIR ...]

VENV needs fontTools 4.66.1 (the fontmake tool environment has it). PATH defaults to
target/release/fontcraft-cli.
"""

import argparse
import os
import random
import subprocess
import sys

from fontTools.fontBuilder import FontBuilder
from fontTools.ttLib import TTFont

TAGS = ("morx", "mort")
GLYPHS = [".notdef"] + [f"g{i:02d}" for i in range(1, 40)]


def block(text, tag):
    start = text.find(f"  <{tag}>")
    end = text.find(f"  </{tag}>", start)
    return None if start < 0 or end < 0 else text[start:end]


def transition(rng, kind, states, lookups):
    out = [f'<NewState value="{rng.randrange(states)}"/>']
    flags = {
        "Rearrangement": ["MarkFirst", "DontAdvance", "MarkLast"],
        "Contextual": ["SetMark", "DontAdvance"],
        "Ligature": ["SetComponent", "DontAdvance"],
        "Insertion": ["SetMark", "DontAdvance", "CurrentIsKashidaLike", "MarkedIsKashidaLike", "CurrentInsertBefore", "MarkedInsertBefore"],
    }[kind]
    chosen = [f for f in flags if rng.random() < 0.3]
    if chosen:
        out.append(f'<Flags value="{",".join(chosen)}"/>')
    if kind == "Rearrangement":
        out.append(f'<Verb value="{rng.randrange(16)}"/>')
    elif kind == "Contextual":
        for name in ("MarkIndex", "CurrentIndex"):
            out.append(f'<{name} value="{rng.randrange(lookups) if rng.random() < 0.3 else 65535}"/>')
    elif kind == "Ligature" and rng.random() < 0.3:
        for i in range(rng.randint(1, 3)):
            store = ' Flags="Store"' if rng.random() < 0.3 else ""
            out.append(f'<Action GlyphIndexDelta="{rng.randint(-len(GLYPHS) + 1, 3)}"{store}/>')
    elif kind == "Insertion":
        for name in ("CurrentInsertionAction", "MarkedInsertionAction"):
            for _ in range(rng.choice([0, 0, 1, 2, 3])):
                out.append(f'<{name} glyph="{rng.choice(GLYPHS[1:])}"/>')
    return out


def state_table(rng, kind):
    classes = rng.randint(5, 10)
    states = rng.randint(2, 5)
    lookups = rng.randint(1, 3)
    out = ["<StateTable>"]
    for g in rng.sample(GLYPHS[1:], 12):
        out.append(f'<GlyphClass glyph="{g}" value="{rng.randrange(1, classes)}"/>')
    out.append(f'<GlyphClass glyph="{GLYPHS[1]}" value="{classes - 1}"/>')
    used = set()
    for s in range(states):
        out.append(f'<State index="{s}">')
        for c in range(classes):
            body = transition(rng, kind, states, lookups)
            used.update(int(x.split('"')[1]) for x in body if "Index value" in x and "65535" not in x)
            out.append(f'<Transition onGlyphClass="{c}">' + "".join(body) + "</Transition>")
        out.append("</State>")
    if kind == "Contextual":
        for i in range(max(used) + 1 if used else 0):
            out.append(f'<PerGlyphLookup index="{i}">')
            for g in rng.sample(GLYPHS[1:], rng.randint(1, 6)):
                out.append(f'<Lookup glyph="{g}" value="{rng.choice(GLYPHS[1:])}"/>')
            out.append("</PerGlyphLookup>")
    if kind == "Ligature":
        out.append("<LigComponents>")
        for i in range(len(GLYPHS) + 4):
            out.append(f'<LigComponent index="{i}" value="{rng.randrange(4)}"/>')
        out.append("</LigComponents><Ligatures>")
        for i in range(12):
            out.append(f'<Ligature index="{i}" glyph="{rng.choice(GLYPHS[1:])}"/>')
        out.append("</Ligatures>")
    out.append("</StateTable>")
    return out


def table_xml(rng, tag):
    out = [f"<{tag}>"]
    out.append('<Version value="0x00010000"/>' if tag == "mort" else '<Version value="2"/><Reserved value="0"/>')
    for c in range(rng.randint(1, 2)):
        out.append(f'<MorphChain index="{c}"><DefaultFlags value="0x0000000F"/>')
        out.append('<MorphFeature index="0"><FeatureType value="1"/><FeatureSetting value="0"/>'
                   '<EnableFlags value="0x00000001"/><DisableFlags value="0xFFFFFFFE"/></MorphFeature>')
        for s in range(rng.randint(1, 5)):
            kind = rng.choice(["Rearrangement", "Contextual", "Ligature", "Noncontextual", "Insertion"])
            out.append(f'<MorphSubtable index="{s}">')
            if tag == "mort":
                out.append(f'<CoverageFlags value="{rng.choice([0, 0x80, 0x20])}"/>')
            else:
                out.append(f'<TextDirection value="{rng.choice(["Horizontal", "Vertical", "Any"])}"/>')
                out.append(f'<ProcessingOrder value="{rng.choice(["LayoutOrder", "ReversedLayoutOrder", "LogicalOrder", "ReversedLogicalOrder"])}"/>')
            out.append(f'<SubFeatureFlags value="0x{1 << s:08x}"/><{kind}Morph>')
            if kind == "Noncontextual":
                out.append("<Substitution>")
                for g in rng.sample(GLYPHS[1:], rng.randint(1, 20)):
                    out.append(f'<Lookup glyph="{g}" value="{rng.choice(GLYPHS[1:])}"/>')
                out.append("</Substitution>")
            else:
                out.extend(state_table(rng, kind))
            out.append(f"</{kind}Morph></MorphSubtable>")
        out.append("</MorphChain>")
    out.append(f"</{tag}>")
    return "\n".join(out)


def build(out_dir, count):
    os.makedirs(out_dir, exist_ok=True)
    paths, refused = [], 0
    for tag in TAGS:
        for i in range(count):
            rng = random.Random(f"{tag}{i}")
            fb = FontBuilder(1000, isTTF=True)
            fb.setupGlyphOrder(GLYPHS)
            fb.setupMaxp()
            fb.font["maxp"].numGlyphs = len(GLYPHS)
            fb.setupPost()
            xml = os.path.join(out_dir, f"{tag}{i:03d}.ttx")
            with open(xml, "w") as f:
                f.write('<?xml version="1.0" encoding="UTF-8"?>\n<ttFont>\n' + table_xml(rng, tag) + "\n</ttFont>\n")
            try:
                fb.font.importXML(xml)
                path = os.path.join(out_dir, f"{tag}{i:03d}.ttf")
                fb.font.save(path)
                paths.append(path)
            except Exception:
                refused += 1
    return paths, refused


def fonts_in(paths):
    for p in paths:
        if os.path.isdir(p):
            for root, _, names in os.walk(p, followlinks=True):
                for n in sorted(names):
                    if n.lower().endswith((".ttf", ".otf")):
                        yield os.path.join(root, n)
        else:
            yield p


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--cli", default="target/release/fontcraft-cli")
    ap.add_argument("--build", type=int, default=0)
    ap.add_argument("out_dir")
    ap.add_argument("fonts", nargs="*")
    args = ap.parse_args()
    os.makedirs(args.out_dir, exist_ok=True)
    fonts = list(fonts_in(args.fonts or ["corpus"]))
    refused, checked = 0, 0
    if args.build:
        built, refused = build(os.path.join(args.out_dir, "built"), args.build)
        fonts += built
    counts = {"dump equal": 0, "dump different": 0, "read equal": 0, "read different": 0, "failed": 0}
    for n, font in enumerate(fonts):
        try:
            tt = TTFont(font, lazy=True)
            tags = [t for t in TAGS if t in tt]
        except Exception:
            continue
        for tag in tags:
            checked += 1
            stem = os.path.join(args.out_dir, f"{n:04d}.{tag}")
            ours, theirs, out = stem + ".ours.ttx", stem + ".theirs.ttx", stem + ".ours.ttf"
            try:
                subprocess.run([args.cli, "dump", font, "-t", tag, "-o", ours], check=True, capture_output=True)
                subprocess.run([sys.executable, "-m", "fontTools.ttx", "-q", "-t", tag, "-o", theirs, font], check=True, capture_output=True)
                a, b = (block(open(p, encoding="utf-8").read(), tag) for p in (ours, theirs))
                same = a is not None and a == b
                counts["dump equal" if same else "dump different"] += 1
                if not same:
                    print("dump differs:", font, tag)
                subprocess.run([args.cli, "tables", "import", font, theirs, "-o", out], check=True, capture_output=True)
                want = TTFont(font)
                want.importXML(theirs)
                same = TTFont(out).reader[tag] == want[tag].compile(want)
                counts["read equal" if same else "read different"] += 1
                if not same:
                    print("read differs:", font, tag)
            except Exception as e:
                counts["failed"] += 1
                print("failed:", font, tag, e)
    print(f"tables checked: {checked} ({refused} random tables refused by fontTools)")
    for k, v in counts.items():
        print(f"{k}: {v}")


if __name__ == "__main__":
    main()
