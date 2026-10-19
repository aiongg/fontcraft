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
"""

import hashlib
import io
import re
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


def blocks(path, tags):
    text = path.read_text(encoding="utf-8")
    out = []
    for tag in tags:
        m = re.search(rf"\n  <{tag}>\n.*?\n  </{tag}>\n", text, flags=re.S)
        out.append(m.group(0)[1:] + "\n")
    return "".join(out)


def case(out_dir, name, text, tags):
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
    back.saveXML(str(path), tables=["GlyphOrder", "maxp", "post"] + tags, newlinestr="\n")
    again = TTFont()
    again.importXML(str(path))
    lines = []
    for tag in tags:
        compiled = again.getTableData(tag)
        lines.append(f"{tag.replace(' ', '_')} {len(compiled)} {hashlib.sha256(compiled).hexdigest()}\n")
    (out_dir / f"{name}.digest").write_text("".join(lines), encoding="utf-8")


def main():
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
