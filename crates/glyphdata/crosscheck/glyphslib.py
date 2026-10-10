"""What glyphsLib's glyph data says about glyph names, for `cargo xtask glyphdata --check`.

Reads glyph names, one per line, from standard input and writes one JSON object per name to
standard output: production name, Unicode value, category, subcategory, script and description
as glyphsLib's get_glyph returns them, and the case attribute of the record glyphsLib finds for
the name itself (get_glyph does not return case). The first line names the glyphsLib version.

Run with the fontmake tool environment's Python, which has glyphsLib:

    python -I crates/glyphdata/crosscheck/glyphslib.py < names.txt > out.jsonl
"""

import json
import sys

import glyphsLib
from glyphsLib import glyphdata


def main():
    out = sys.stdout
    out.write(json.dumps({"glyphsLib": glyphsLib.__version__}) + "\n")
    glyphdata.get_glyph("A")  # loads the data
    for line in sys.stdin:
        name = line.rstrip("\n")
        g = glyphdata.get_glyph(name)
        attributes = glyphdata._lookup_attributes(name, glyphdata.GLYPHDATA)
        out.write(
            json.dumps(
                {
                    "name": name,
                    "production": g.production_name,
                    "unicode": g.unicode,
                    "category": g.category,
                    "subCategory": g.subCategory,
                    "script": g.script,
                    "description": g.description,
                    "case": attributes.get("case"),
                }
            )
            + "\n"
        )


if __name__ == "__main__":
    main()
