"""Writes the expected outputs of the variable feature syntax tests.

Each `../data/variable_syntax/cases/NAME.fea` is converted with glyphsLib's variable feature converter
(the one fontmake runs for a variable build) and written to `expected/NAME.fea` beside it;
a case that glyphsLib refuses is written as `expected/NAME.err` holding the
message. The first line of a case names its font: `# font: mapped` (three
axes with Axis Mappings) or `# font: plain` (one weight axis, no mapping).

Run with fontmake's Python, isolated:
    <fontmake venv>/bin/python -I crates/features/tests/oracle/convert.py
"""

import pathlib
import sys

import glyphsLib
from glyphsLib.builder.variable_features import VariableFeatureConverter

here = pathlib.Path(__file__).resolve().parent.parent / "data" / "variable_syntax"
fonts = {name: glyphsLib.GSFont(str(here / f"{name}.glyphs")) for name in ("mapped", "plain")}

for case in sorted((here / "cases").glob("*.fea")):
    text = case.read_text()
    first = text.split("\n", 1)[0]
    name = first.removeprefix("# font:").strip()
    if name not in fonts:
        sys.exit(f"{case.name}: the first line must be `# font: mapped` or `# font: plain`")
    out = here / "expected" / case.name
    err = out.with_suffix(".err")
    try:
        converted = VariableFeatureConverter(fonts[name]).convert(text)
    except ValueError as e:
        err.write_text(f"{e}\n")
        out.unlink(missing_ok=True)
        continue
    out.write_text(converted)
    err.unlink(missing_ok=True)
