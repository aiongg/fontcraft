"""Writes the reader tests' input and expected tables.

Run with a Python that has fontTools (the version named in `ttx/mod.rs`, `TTX_VERSION`):

    python -I oracle.py

Each `<name>.ttx` here is made of table elements the writer tests store (`../../otdata`,
`../../var`, `../../color`, `../../misc`) under a glyph order: the order of the font the writer tests build
from `fonts.otasm`, or a short one that holds every glyph the tables name. fontTools reads each
file (`TTFont().importXML`) and compiles every table in it; `<name>.digest` holds the tag (`_`
for a space), byte length and SHA-256 of each, one table a line.
"""

import hashlib
import io
import re
from pathlib import Path

from fontTools.ttLib import TTFont

HERE = Path(__file__).parent
TTX = HERE.parent.parent
HEAD = '<?xml version="1.0" encoding="UTF-8"?>\n<ttFont sfntVersion="\\x00\\x01\\x00\\x00" ttLibVersion="4.66">\n\n'

# Two axes, for the variation stores of `var.ttx`.
FVAR = """  <fvar>
    <Axis>
      <AxisTag>wght</AxisTag>
      <Flags>0x0</Flags>
      <MinValue>100.0</MinValue>
      <DefaultValue>400.0</DefaultValue>
      <MaxValue>900.0</MaxValue>
      <AxisNameID>256</AxisNameID>
    </Axis>
    <Axis>
      <AxisTag>wdth</AxisTag>
      <Flags>0x0</Flags>
      <MinValue>75.0</MinValue>
      <DefaultValue>100.0</DefaultValue>
      <MaxValue>125.0</MaxValue>
      <AxisNameID>257</AxisNameID>
    </Axis>
  </fvar>

"""


def otasm_orders():
    """The glyph order of each font of `fonts.otasm`, assembled as the writer tests do."""
    source = (TTX / "otdata/testdata/oracle.py").read_text(encoding="utf-8")
    namespace = {"__name__": "otasm"}
    exec(compile(source.rsplit("\nmain()", 1)[0], "oracle.py", "exec"), namespace)
    fonts = namespace["parse"](TTX / "otdata/testdata/fonts.otasm")
    orders = {}
    for name, tables in fonts.items():
        data = namespace["sfnt"]({tag: namespace["assemble"](lines) for tag, lines in tables.items()})
        orders[name] = TTFont(io.BytesIO(data)).getGlyphOrder()
    return orders


def blocks(path, tags):
    text = path.read_text(encoding="utf-8")
    out = []
    for tag in tags:
        m = re.search(rf"\n  <{tag}>\n.*?\n  </{tag}>\n", text, flags=re.S)
        out.append(m.group(0)[1:] + "\n")
    return "".join(out)


def glyph_order(names):
    lines = ["  <GlyphOrder>\n"]
    lines += [f'    <GlyphID id="{i}" name="{n}"/>\n' for i, n in enumerate(names)]
    return "".join(lines) + "  </GlyphOrder>\n\n"


def main():
    orders = otasm_orders()
    inputs = {
        "layout": (orders["layout"], "", TTX / "otdata/testdata/layout.ttx", ["GDEF", "GSUB", "GPOS", "BASE", "JSTF", "MATH", "meta"]),
        "variable": (orders["variable"], "", TTX / "otdata/testdata/variable.ttx", ["GDEF", "GSUB", "STAT", "fvar", "avar"]),
        "var": ([".notdef", "A", "B", "C"], FVAR, TTX / "var/testdata/var.ttx", ["HVAR", "VVAR", "MVAR"]),
        "color": ([".notdef", "A", "B", "C"], "", TTX / "color/testdata/color.ttx", ["COLR", "CPAL", "SVG"]),
        "kern": ([".notdef", "A", "B", "C"], "", TTX / "misc/testdata/misc.ttx", ["kern"]),
        "apple_kern": ([".notdef", "A", "B", "C"], "", TTX / "misc/testdata/apple_kern.ttx", ["kern"]),
    }
    for name, (order, extra, path, tags) in inputs.items():
        text = HEAD + glyph_order(order) + extra + blocks(path, tags) + "</ttFont>\n"
        (HERE / f"{name}.ttx").write_text(text, encoding="utf-8")
        font = TTFont()
        font.importXML(str(HERE / f"{name}.ttx"))
        lines = []
        for tag in font.keys():
            if tag == "GlyphOrder":
                continue
            data = font.getTableData(tag)
            lines.append(f"{tag.replace(' ', '_')} {len(data)} {hashlib.sha256(data).hexdigest()}\n")
        (HERE / f"{name}.digest").write_text("".join(lines), encoding="utf-8")


main()
