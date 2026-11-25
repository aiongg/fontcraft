"""Writes the expected texts of the token measure (`../tokens_oracle.rs`).

Each line of `../data/tokens/cases.txt` is feature code with number, glyph property or predicate
tokens; it is expanded on each master of `font.glyphs` beside it with glyphsLib's `TokenExpander`
and written to `expected.tsv`: code, master id, text. A case the library refuses has `!` and the
exception's name as its text.

Run in a Python environment with glyphsLib, isolated:
    ENV/bin/python -I crates/features/tests/oracle/tokens.py
"""

import pathlib

import glyphsLib
from glyphsLib.builder.tokens import TokenExpander

here = pathlib.Path(__file__).resolve().parent.parent / "data" / "tokens"
font = glyphsLib.GSFont(str(here / "font.glyphs"))
rows = []
for code in (here / "cases.txt").read_text().splitlines():
    for master in font.masters:
        try:
            text = TokenExpander(font, master).expand(code)
        except Exception as e:  # every refusal is an outcome
            text = f"!{type(e).__name__}"
        rows.append(f"{code}\t{master.id}\t{text}")
(here / "expected.tsv").write_text("\n".join(rows) + "\n")
print(f"{len(rows)} expansions, {sum(r.split(chr(9))[2].startswith('!') for r in rows)} refused")
