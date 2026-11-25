"""Second step of the shaping measure: shapes sample strings with HarfBuzz.

Reads every font of a folder (`*.ttf`, `*.otf`) with its sidecars, as the engine test
`shaping_fonts` writes them, and writes `expected.tsv` there: one line per shaping request with
HarfBuzz's glyphs, clusters (UTF-8 byte offsets) and positions. The test
`crates/shaping/tests/harfbuzz.rs` shapes the same requests with FontCraft and compares. Any other
font can be put in the folder; a variable font is also shaped at two locations per axis.

Columns: font, location (`wght=700,opsz=14`), direction (`ltr`, `rtl`, `ttb`), ISO 15924 script
(empty: guessed), language, features (`liga=0`, `ss01=1@3:8` for a byte range, comma-separated),
text as UTF-8 hex, glyphs (`gid:cluster:x advance:y advance:x offset:y offset`, space-separated).

It also checks, inside HarfBuzz alone, that Edit View's path equals the exported font with its
`GPOS` features off: `NAME.edit.ttf` and `NAME.full.ttf` give the same glyph ids and clusters.

    uv venv ENV; uv pip install --python ENV/bin/python uharfbuzz fonttools
    ENV/bin/python -I crates/shaping/crosscheck/harfbuzz.py FOLDER
"""

import io
import pathlib
import random
import sys
import unicodedata

import uharfbuzz as hb
from fontTools.ttLib import TTFont
from fontTools.ttLib.tables._c_m_a_p import CmapSubtable
from fontTools.unicodedata import script as script_of

SAMPLES = [
    "Hamburgefonstiv 0123456789 (fi) ffi ffl AVATAR To. “quotes” — 1/2 3⁄4",
    "Áèȫi̇́ Ångström ệ q̣̇ x͡y",
    "İstanbul ışık Ţară Ștefan paral·lel IJsland Ĳ",
    "Съешь же ещё этих йї",
    "Ξεσκεπάζω ᾄ Ώ",
    "بسم الله لا مَّم",
    "שָׁלוֹם בְּרִא",
    "क्षि र्क श्री",
    "กำ ปี่ น้ำ",
    "한글 한글 漢字かな",
    "a‍b a‌b a­b ◌́ ́ a️",
]
LANGUAGES = ["tr", "ro", "nl", "ca", "sr", "az", "x-hbscLATN-hbotTRK", "x-hbotROM"]
RANDOM_LINES = 60
LINE_LENGTH = 14
MAX_SAMPLE_CHARS = 400


class Face:
    def __init__(self, path, private_use):
        self.data = path.read_bytes()
        font = TTFont(io.BytesIO(self.data), lazy=True)
        self.tags = set(font.keys())
        self.cmap = dict(font.getBestCmap() or {}) if "cmap" in font else {}
        self.axes = [(a.axisTag, a.minValue, a.defaultValue, a.maxValue) for a in font["fvar"].axes] if "fvar" in font else []
        self.features = {t: sorted({r.FeatureTag for r in self.records(font, t)}) for t in ("GSUB", "GPOS")}
        if private_use:
            # The glyphs FontCraft reaches through a map beside `cmap`.
            order = font.getGlyphOrder()
            mapping = dict(self.cmap)
            for cp, gid in private_use.items():
                mapping.setdefault(cp, order[gid])
            table = font["cmap"]
            sub = CmapSubtable.newSubtable(12)
            sub.platformID, sub.platEncID, sub.language, sub.cmap = 3, 10, 0, mapping
            table.tables = [t for t in table.tables if t.format == 14] + [sub]
            out = io.BytesIO()
            font.save(out, reorderTables=None)
            self.data = out.getvalue()
        self.face = hb.Face(self.data)
        self.characters = sorted(cp for cp in self.cmap if cp > 0x20) + sorted(private_use)

    @staticmethod
    def records(font, tag):
        table = font[tag].table if tag in font else None
        return table.FeatureList.FeatureRecord if table is not None and table.FeatureList else []

    def shape(self, text, direction="ltr", script="", language="", features=(), location=None):
        font = hb.Font(self.face)
        if location:
            font.set_variations(location)
        buffer = hb.Buffer()
        buffer.add_utf8(text.encode())
        buffer.direction = direction
        if script:
            buffer.script = script
        if language:
            buffer.language = language
        else:
            # An unset language would be guessed from the process's locale.
            buffer.language = "und"
        buffer.guess_segment_properties()
        wanted = {}
        for tag, value, span in features:
            wanted[tag] = bool(value) if span is None and value < 2 else value if span is None else [(span[0], span[1], value)]
        hb.shape(font, buffer, wanted)
        return [
            (i.codepoint, i.cluster, p.x_advance, p.y_advance, p.x_offset, p.y_offset)
            for i, p in zip(buffer.glyph_infos, buffer.glyph_positions)
        ]


def first_script(text):
    for c in text:
        s = script_of(c)
        if s not in ("Zyyy", "Zinh", "Zzzz"):
            return s
    return ""


def direction_of(text):
    for c in text:
        kind = unicodedata.bidirectional(c)
        if kind in ("R", "AL"):
            return "rtl"
        if kind == "L":
            return "ltr"
    return "ltr"


def sample_lines(face, extra):
    """The sample lines, and lines of characters drawn at random from one script of the font."""
    rng = random.Random(834)
    fixed = [line for line in SAMPLES if sum(ord(c) in face.cmap for c in line if c != " ") * 2 >= len(line.replace(" ", ""))]
    fixed += [line[:MAX_SAMPLE_CHARS] for line in extra]
    by_script = {}
    for cp in face.characters:
        by_script.setdefault(script_of(chr(cp)), []).append(cp)
    marks = [cp for cp in face.cmap if unicodedata.category(chr(cp)) == "Mn"]
    common = by_script.pop("Zyyy", []) + by_script.pop("Zinh", []) + by_script.pop("Zzzz", [])
    drawn = []
    scripts = sorted(by_script) or [""]
    for n in range(RANDOM_LINES):
        pool = by_script.get(scripts[n % len(scripts)], []) or common
        if not pool:
            break
        chars = []
        for _ in range(LINE_LENGTH):
            roll = rng.random()
            source = marks if roll < 0.15 and marks else common if roll < 0.25 and common else pool
            chars.append(chr(rng.choice(source)))
        drawn.append("".join(chars))
    return fixed, drawn


def requests(face, extra):
    """(text, direction, script, language, features, location) of every request."""
    fixed, drawn = sample_lines(face, extra)
    off = [(tag, 0, None) for tag in sorted(set(face.features["GPOS"]) | {"kern", "mark", "mkmk", "dist", "abvm", "blwm", "curs"})]
    for text in fixed + drawn:
        direction, script = direction_of(text), first_script(text)
        yield text, direction, script, "", [], None
        yield text, direction, script, "", off, None
    for text in fixed:
        direction, script = direction_of(text), first_script(text)
        for language in LANGUAGES:
            yield text, direction, script, language, [], None
        yield text, "ttb", script, "", [], None
        yield text, "rtl" if direction == "ltr" else "ltr", script, "", [], None
    # Every feature of the font, alone, on whole lines and on a byte range.
    for n, tag in enumerate(sorted(set(face.features["GSUB"]) | set(face.features["GPOS"]))):
        lines = [fixed[n % len(fixed)]] if fixed else []
        lines += [drawn[(n + k) % len(drawn)] for k in range(3)] if drawn else []
        for text in lines:
            direction, script = direction_of(text), first_script(text)
            yield text, direction, script, "", [(tag, 1, None)], None
            yield text, direction, script, "", [(tag, 0, None)], None
            yield text, direction, script, "", [(tag, 2, None)], None
            starts = [len(text[:k].encode()) for k in range(len(text) + 1)]
            a, b = starts[len(starts) // 4], starts[3 * len(starts) // 4]
            yield text, direction, script, "", [(tag, 1, (a, b))], None
    for tag, low, default, high in face.axes:
        for value in (low, high):
            if value == default:
                continue
            for text in (fixed + drawn)[:20]:
                yield text, direction_of(text), first_script(text), "", [], {tag: value}


def line_of(name, face, request):
    text, direction, script, language, features, location = request
    glyphs = face.shape(text, direction, script, language, features, location)
    return "\t".join(
        [
            name,
            ",".join(f"{t}={v:g}" for t, v in (location or {}).items()),
            direction,
            script,
            language,
            ",".join(f"{t}={v}" + (f"@{s[0]}:{s[1]}" if s else "") for t, v, s in features),
            text.encode().hex(),
            " ".join(":".join(map(str, g)) for g in glyphs),
        ]
    )


def edit_equals_full(folder, name, extra):
    """Edit View's font against the exported one with `GPOS` off; returns the differing texts."""
    edit_path, full_path = folder / f"{name}.edit.ttf", folder / f"{name}.full.ttf"
    if not (edit_path.exists() and full_path.exists()):
        return None
    edit, full = Face(edit_path, {}), Face(full_path, {})
    off = [(tag, 0, None) for tag in sorted(set(full.features["GPOS"]) | {"kern", "mark", "mkmk", "dist", "abvm", "blwm", "curs"})]
    fixed, drawn = sample_lines(edit, [])
    fixed += [t[:MAX_SAMPLE_CHARS] for t in extra if all(ord(c) < 0xF0000 for c in t)]
    differ = []
    for text in fixed + drawn:
        runs = [f.shape(text, direction_of(text), first_script(text), "", off) for f in (edit, full)]
        if [g[:2] for g in runs[0]] != [g[:2] for g in runs[1]]:
            differ.append(text)
    return len(fixed) + len(drawn), differ


def main():
    folder = pathlib.Path(sys.argv[1])
    print(f"HarfBuzz {hb.version_string()}")
    rows = []
    for path in sorted([*folder.glob("*.ttf"), *folder.glob("*.otf")]):
        name = path.name
        pua_path = path.with_suffix(".pua")
        private_use = {}
        if pua_path.exists():
            for row in pua_path.read_text().splitlines():
                cp, gid = row.split("\t")
                private_use[int(cp, 16)] = int(gid)
        texts = folder / (name.split(".")[0] + ".txt")
        extra = [t for t in texts.read_text().split("\n") if t] if texts.exists() else []
        face = Face(path, private_use)
        if not private_use:
            extra = [t for t in extra if all(ord(c) < 0xF0000 for c in t)]
        before = len(rows)
        rows.extend(line_of(name, face, r) for r in requests(face, extra))
        print(f"{name}: {len(rows) - before} requests")
    (folder / "expected.tsv").write_text("\n".join(rows) + "\n")
    print(f"total: {len(rows)} requests in {folder / 'expected.tsv'}")
    for path in sorted(folder.glob("*.edit.ttf")):
        name = path.name.removesuffix(".edit.ttf")
        texts = folder / f"{name}.txt"
        extra = [t for t in texts.read_text().split("\n") if t] if texts.exists() else []
        result = edit_equals_full(folder, name, extra)
        if result is None:
            continue
        count, differ = result
        print(f"{name}: Edit View's font against the export without GPOS: {count} lines, {len(differ)} differ")
        for text in differ[:10]:
            print(f"  {text!r}")


main()
