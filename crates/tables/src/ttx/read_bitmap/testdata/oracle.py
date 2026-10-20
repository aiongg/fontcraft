"""Writes the inputs and expected tables of the M12.7c reader tests (`read_bitmap` and
`read_misc`).

Run with a Python that has fontTools (the version named in `ttx/mod.rs`, `TTX_VERSION`):

    python -I oracle.py

Each case takes table elements the writer tests store (`../../color/testdata`,
`../../misc/testdata`) or written here, under the glyph order `.notdef A B C`, with a `maxp`,
a `post` table that keeps the glyph names and a `name` table. fontTools compiles them, reads the font back
and writes it as TTX: that text is `<name>.ttx`, so every input is in the form `ttx` writes.
`<name>.digest` holds the tag (`_` for a space), byte length and SHA-256 of each table of the
case that fontTools compiles from `<name>.ttx`, one table a line.

`python -I oracle.py forms` writes only the M12.7e cases, `forms_row` and `forms_bitwise`:
bitmap tables of bit depths 1, 2 and 4 whose images `ttx -z row` and `ttx -z bitwise` write.
"""

import hashlib
import io
import re
import sys
from pathlib import Path

from fontTools.fontBuilder import FontBuilder
from fontTools.ttLib import TTFont

HERE = Path(__file__).parent
TTX = HERE.parent.parent
OUT = {"bitmap": HERE, "misc": TTX / "read_misc/testdata"}
ORDER = [".notdef", "A", "B", "C"]

EBSC = """  <EBSC>
    <header version="2.0" numSizes="2"/>
    <bitmapScaleTable index="0">
      <sbitLineMetrics direction="hori">
        <ascender value="9"/>
        <descender value="-2"/>
        <widthMax value="200"/>
        <caretSlopeNumerator value="1"/>
        <caretSlopeDenominator value="0"/>
        <caretOffset value="0"/>
        <minOriginSB value="0"/>
        <minAdvanceSB value="1"/>
        <maxBeforeBL value="9"/>
        <minAfterBL value="-2"/>
        <pad1 value="0"/>
        <pad2 value="0"/>
      </sbitLineMetrics>
      <sbitLineMetrics direction="vert">
        <ascender value="5"/>
        <descender value="-5"/>
        <widthMax value="10"/>
        <caretSlopeNumerator value="0"/>
        <caretSlopeDenominator value="1"/>
        <caretOffset value="0"/>
        <minOriginSB value="0"/>
        <minAdvanceSB value="0"/>
        <maxBeforeBL value="0"/>
        <minAfterBL value="0"/>
        <pad1 value="0"/>
        <pad2 value="0"/>
      </sbitLineMetrics>
      <ppemX value="9"/>
      <ppemY value="9"/>
      <substitutePpemX value="10"/>
      <substitutePpemY value="10"/>
    </bitmapScaleTable>
    <bitmapScaleTable index="1">
      <sbitLineMetrics direction="hori">
        <ascender value="7"/>
        <descender value="-1"/>
        <widthMax value="8"/>
        <caretSlopeNumerator value="1"/>
        <caretSlopeDenominator value="0"/>
        <caretOffset value="0"/>
        <minOriginSB value="0"/>
        <minAdvanceSB value="0"/>
        <maxBeforeBL value="7"/>
        <minAfterBL value="-1"/>
        <pad1 value="0"/>
        <pad2 value="0"/>
      </sbitLineMetrics>
      <sbitLineMetrics direction="vert">
        <ascender value="4"/>
        <descender value="-4"/>
        <widthMax value="8"/>
        <caretSlopeNumerator value="0"/>
        <caretSlopeDenominator value="1"/>
        <caretOffset value="0"/>
        <minOriginSB value="0"/>
        <minAdvanceSB value="0"/>
        <maxBeforeBL value="0"/>
        <minAfterBL value="0"/>
        <pad1 value="0"/>
        <pad2 value="0"/>
      </sbitLineMetrics>
      <ppemX value="7"/>
      <ppemY value="7"/>
      <substitutePpemX value="10"/>
      <substitutePpemY value="10"/>
    </bitmapScaleTable>
  </EBSC>
"""

# Single-byte codes, two lead bytes whose second bytes have gaps, and a lead byte whose glyph
# array equals another's.
CMAP2 = """  <cmap>
    <tableVersion version="0"/>
    <cmap_format_2 platformID="1" platEncID="1" language="0">
      <map code="0x41" name="A"/>
      <map code="0x42" name="B"/>
      <map code="0x8140" name="C"/>
      <map code="0x8142" name="A"/>
      <map code="0x8240" name="C"/>
      <map code="0x8242" name="A"/>
      <map code="0x8350" name="B"/>
    </cmap_format_2>
    <cmap_format_4 platformID="3" platEncID="1" language="0">
      <map code="0x41" name="A"/>
    </cmap_format_4>
  </cmap>
"""


def line_metrics(direction):
    names = ["ascender", "descender", "widthMax", "caretSlopeNumerator", "caretSlopeDenominator", "caretOffset"]
    names += ["minOriginSB", "minAdvanceSB", "maxBeforeBL", "minAfterBL", "pad1", "pad2"]
    values = dict(ascender=8, descender=-2, widthMax=9, caretSlopeNumerator=1) if direction == "hori" else {}
    body = "".join(f'      <{n} value="{values.get(n, 0)}"/>\n' for n in names)
    return f'      <sbitLineMetrics direction="{direction}">\n{body}      </sbitLineMetrics>\n'


def metrics(kind, height, width):
    if kind == "small":
        fields = [("height", height), ("width", width), ("BearingX", 0), ("BearingY", height), ("Advance", width + 1)]
        name = "SmallGlyphMetrics"
    else:
        fields = [("height", height), ("width", width), ("horiBearingX", 0), ("horiBearingY", height), ("horiAdvance", width + 1)]
        fields += [("vertBearingX", -1), ("vertBearingY", 0), ("vertAdvance", height + 1)]
        name = "BigGlyphMetrics"
    body = "".join(f'<{n} value="{v}"/>' for n, v in fields)
    return f"<{name}>{body}</{name}>"


# Strikes of bit depth 2, 4 and 1 whose rows do not end on a byte: (bit depth, glyphs, index
# subtables). A glyph is (name, image format, metrics kind or None, height, width, raw hex); an
# index subtable is (index format, image format, glyph ids, extra XML).
FORM_STRIKES = [
    (2, [("A", 2, "small", 3, 3, "b6db6f"), ("B", 7, "big", 2, 5, "9c3e5a"), ("C", 6, "big", 2, 3, "e418")],
     [(1, 2, [1], ""), (3, 7, [2], ""), (1, 6, [3], "")]),
    (4, [("A", 5, None, 2, 3, "123456"), ("B", 5, None, 2, 3, "abcdef"), ("C", 1, "small", 2, 3, "f0a05b70")],
     [(2, 5, [1, 2], '<imageSize value="3"/>' + metrics("big", 2, 3)), (1, 1, [3], "")]),
    (1, [(".notdef", 7, "big", 3, 5, "a5f3"), ("A", 2, "small", 2, 9, "ff80c0")],
     [(3, 7, [0], ""), (1, 2, [1], "")]),
]


def forms_tables():
    """EBDT and EBLC in the raw form, from `FORM_STRIKES`."""
    data, loc = ['  <EBDT>\n    <header version="2.0"/>\n'], ['  <EBLC>\n    <header version="2.0"/>\n']
    for i, (depth, glyphs, subtables) in enumerate(FORM_STRIKES):
        data.append(f'    <strikedata index="{i}">\n')
        for name, fmt, kind, height, width, raw in glyphs:
            m = metrics(kind, height, width) if kind else ""
            data.append(f'<ebdt_bitmap_format_{fmt} name="{name}">{m}<rawimagedata>{raw}</rawimagedata></ebdt_bitmap_format_{fmt}>\n')
        data.append("    </strikedata>\n")
        ids = [g for s in subtables for g in s[2]]
        loc.append(f'    <strike index="{i}">\n      <bitmapSizeTable>\n')
        loc.append(line_metrics("hori") + line_metrics("vert"))
        for n, v in [("colorRef", 0), ("startGlyphIndex", min(ids)), ("endGlyphIndex", max(ids)), ("ppemX", 9), ("ppemY", 9), ("bitDepth", depth), ("flags", 1)]:
            loc.append(f'        <{n} value="{v}"/>\n')
        loc.append("      </bitmapSizeTable>\n")
        for index_format, image_format, gids, extra in subtables:
            locs = "".join(f'<glyphLoc id="{g}" name="{ORDER[g]}"/>' for g in gids)
            loc.append(f'<eblc_index_sub_table_{index_format} imageFormat="{image_format}" firstGlyphIndex="{min(gids)}" lastGlyphIndex="{max(gids)}">{extra}{locs}</eblc_index_sub_table_{index_format}>\n')
        loc.append("    </strike>\n")
    data.append("  </EBDT>\n")
    loc.append("  </EBLC>\n")
    return "".join(data) + "".join(loc)


def blocks(path, tags):
    text = path.read_text(encoding="utf-8")
    out = []
    for tag in tags:
        m = re.search(rf"\n  <{tag}>\n.*?\n  </{tag}>\n", text, flags=re.S)
        out.append(m.group(0)[1:] + "\n")
    return "".join(out)


def case(out_dir, name, text, tags, image_form="raw"):
    fb = FontBuilder(1000, isTTF=True)
    fb.setupGlyphOrder(ORDER)
    fb.setupMaxp()
    fb.setupPost(keepGlyphNames=True)
    # `trak` names its tracks from `name` when it is written.
    fb.setupNameTable({"familyName": "Test", "styleName": "Regular"})
    font = fb.font
    font.importXML(io.StringIO('<?xml version="1.0" encoding="UTF-8"?>\n<ttFont>\n' + text + "</ttFont>\n"))
    data = io.BytesIO()
    font.save(data)
    back = TTFont(io.BytesIO(data.getvalue()))
    path = out_dir / f"{name}.ttx"
    back.saveXML(str(path), tables=["GlyphOrder", "maxp", "post"] + tags, newlinestr="\n", bitmapGlyphDataFormat=image_form)
    again = TTFont()
    again.importXML(str(path))
    lines = []
    for tag in tags:
        compiled = again.getTableData(tag)
        lines.append(f"{tag.replace(' ', '_')} {len(compiled)} {hashlib.sha256(compiled).hexdigest()}\n")
    (out_dir / f"{name}.digest").write_text("".join(lines), encoding="utf-8")


def main():
    if sys.argv[1:] == ["forms"]:
        for form in ["row", "bitwise"]:
            case(OUT["bitmap"], f"forms_{form}", forms_tables(), ["EBDT", "EBLC"], form)
        return
    color, misc = TTX / "color/testdata", TTX / "misc/testdata"
    case(OUT["bitmap"], "bitmap", blocks(color / "bitmap.ttx", ["EBDT", "EBLC", "bdat", "bloc"]), ["EBDT", "EBLC", "bdat", "bloc"])
    case(OUT["bitmap"], "cbdt", blocks(color / "cbdt.ttx", ["CBDT", "CBLC"]), ["CBDT", "CBLC"])
    case(OUT["bitmap"], "sbix", blocks(color / "color.ttx", ["sbix"]) + EBSC, ["sbix", "EBSC"])
    apple = ["ankr", "bsln", "cidg", "feat", "gcid", "lcar", "ltag", "opbd", "trak"]
    case(OUT["misc"], "apple", blocks(misc / "apple.ttx", apple) + blocks(misc / "misc.ttx", ["prop"]), apple + ["prop"])
    case(OUT["misc"], "apple2", blocks(misc / "apple2.ttx", ["bsln", "lcar", "opbd"]), ["bsln", "lcar", "opbd"])
    case(OUT["misc"], "colr1", blocks(color / "colr1.ttx", ["COLR"]), ["COLR"])
    case(OUT["misc"], "cmap2", CMAP2, ["cmap"])
    case(OUT["misc"], "misc", blocks(misc / "misc.ttx", ["DSIG", "FFTM", "LTSH", "VDMX"]), ["DSIG", "FFTM", "LTSH", "VDMX"])


main()
