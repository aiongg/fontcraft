"""Writes the expected TTX text for the fonts of `fonts.otasm`, with fontTools' `ttx` writer.

Run with a Python that has fontTools (the version named in `ttx/mod.rs`, `TTX_VERSION`):

    python -I oracle.py fonts.otasm

Each font of the description is assembled exactly as the Rust tests assemble it (the
directives are listed in `tests.rs`), and its structured tables are written to `<font>.ttx`
next to the description, without comments and trailing spaces.
"""

import io
import re
import struct
import sys
from pathlib import Path

from fontTools.ttLib import TTFont

TABLES = ["GDEF", "GSUB", "GPOS", "BASE", "JSTF", "MATH", "STAT", "fvar", "avar", "meta"]
SIZES = {"u8": 1, "i8": 1, "u16": 2, "i16": 2, "u24": 3, "u32": 4, "i32": 4, "fixed": 4, "f2": 2}


def parse(path):
    fonts = {}
    font = table = None
    for raw in Path(path).read_text(encoding="utf-8").splitlines():
        words = raw.split(None, 1)
        if words and words[0] in ("utf16", "ascii"):
            items = [(words[0], words[1] if len(words) > 1 else "")]
        else:
            items = [tuple(raw.split("#", 1)[0].split())] if raw.split("#", 1)[0].split() else []
        for item in items:
            if item[0] == "font":
                font = fonts.setdefault(item[1], {})
            elif item[0] == "table":
                table = font.setdefault(item[1].replace("_", " "), [])
            else:
                table.append(item)
    return fonts


def number(text):
    return int(text, 0)


def assemble(lines):
    labels, pos = {}, 0
    for item in lines:
        op, args = item[0], item[1:]
        if op.startswith("@"):
            labels[op[1:]] = pos
        elif op in SIZES:
            pos += SIZES[op] * len(args)
        elif op == "tag":
            pos += 4 * len(args)
        elif op in ("o16", "o24", "o32"):
            pos += int(op[1:]) // 8
        elif op == "pstr":
            pos += sum(1 + len(s) for s in args)
        elif op == "utf16":
            pos += len(args[0].encode("utf-16-be"))
        elif op == "ascii":
            pos += len(args[0].encode("ascii"))
        else:
            raise ValueError(op)
    out = bytearray()
    for item in lines:
        op, args = item[0], item[1:]
        if op.startswith("@"):
            continue
        if op in ("u8", "i8", "u16", "i16", "u32", "i32"):
            code = {"u8": "B", "i8": "b", "u16": "H", "i16": "h", "u32": "I", "i32": "i"}[op]
            out += b"".join(struct.pack(">" + code, number(a)) for a in args)
        elif op == "u24":
            out += b"".join(struct.pack(">I", number(a))[1:] for a in args)
        elif op == "fixed":
            out += b"".join(struct.pack(">i", round(float(a) * 65536)) for a in args)
        elif op == "f2":
            out += b"".join(struct.pack(">h", round(float(a) * 16384)) for a in args)
        elif op == "tag":
            out += b"".join(a.replace("_", " ").encode("latin-1") for a in args)
        elif op in ("o16", "o24", "o32"):
            value = labels[args[0]] - labels[args[1]]
            out += struct.pack(">I", value)[4 - int(op[1:]) // 8 :]
        elif op == "pstr":
            out += b"".join(bytes([len(s)]) + s.encode("ascii") for s in args)
        elif op == "utf16":
            out += args[0].encode("utf-16-be")
        elif op == "ascii":
            out += args[0].encode("ascii")
    return bytes(out)


def sfnt(tables):
    tags = sorted(tables)
    head = struct.pack(">IHHHH", 0x00010000, len(tags), 0, 0, 0)
    offset = 12 + 16 * len(tags)
    directory, body = b"", b""
    for tag in tags:
        data = tables[tag]
        directory += struct.pack(">4sIII", tag.encode("latin-1"), 0, offset + len(body), len(data))
        body += data + b"\0" * (-len(data) % 4)
    return head + directory + body


def without_comments(text):
    text = re.sub(r"<!--.*?-->", "\x00", text, flags=re.S)
    lines = []
    for line in text.split("\n"):
        if "\x00" in line and not line.replace("\x00", "").strip():
            continue
        lines.append(line.replace("\x00", "").rstrip())
    return "\n".join(lines)


def main():
    spec = Path(sys.argv[1])
    for name, tables in parse(spec).items():
        font = TTFont(io.BytesIO(sfnt({tag: assemble(lines) for tag, lines in tables.items()})))
        out = io.StringIO()
        font.saveXML(out, tables=[t for t in TABLES if t in tables])
        spec.with_name(f"{name}.ttx").write_text(without_comments(out.getvalue()), encoding="utf-8")


main()
