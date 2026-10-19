"""Writes schema.rs, the table layouts the TTX writer for structured tables walks.

The layouts are fontTools' `ttLib.tables.otData` (MIT): field types, names, counts and
conditions, in its order. Run with a Python that has fontTools:

    python -I gen_schema.py > schema.rs

then `cargo fmt`. Layouts that use a field type the writer does not read are left out and
listed on stderr.
"""

import re
import sys

from fontTools.ttLib.tables.otData import otData
from fontTools.ttLib.tables.otTables import _equivalents

# The otData ranges the writer covers: the common layout tables, the Apple tables, the COLR
# layouts, then the avar layouts. Variable composites, multi-variation stores and the Apple
# glyph metamorphosis tables (mort, morx) are left out.
RANGES = [("LookupOrder", "TSIC"), ("COLR", "AxisValueMap")]
EXTRA = {"AxisValueMap", "AxisSegmentMap", "avar"}
MORPH = {
    "mort",
    "MortChain",
    "MortSubtable",
    "morx",
    "MorxChain",
    "MorphFeature",
    "MorxSubtable",
    "StateHeader",
    "RearrangementMorph",
    "ContextualMorph",
    "LigatureMorph",
    "NoncontextualMorph",
    "InsertionMorph",
    "MorphClass",
}
SKIP = MORPH | {
    "AATLookupSegment",
    "MultiVarData",
    "MultiVarStore",
    "SparseVarRegionAxis",
    "SparseVarRegion",
    "SparseVarRegionList",
    "VARC",
    "AxisIndicesList",
    "VarCompositeGlyphs",
}

SIMPLE = {
    "int16": "I16",
    "uint8": "U8",
    "uint16": "U16",
    "uint24": "U24",
    "uint32": "U32",
    "Version": "Version",
    "Tag": "Tag",
    "GlyphID": "Glyph",
    "NameID": "NameId",
    "DeciPoints": "DeciPoints",
    "Fixed": "Fixed",
    "F2Dot14": "F2Dot14",
    "ValueRecord": "ValueRecord",
    "DeltaValue": "DeltaValue",
    "VarIdxMapValue": "VarIdxMapValue",
    "VarDataValue": "VarDataValue",
    "LookupFlag": "LookupFlag",
    "STATFlags": "StatFlags",
    "VarIndex": "VarIndex",
    "char64": "Char64",
    "CIDGlyphMap": "CidGlyphMap",
    "GlyphCIDMap": "GlyphCidMap",
    "Angle": "Angle",
    "BiasedAngle": "BiasedAngle",
    "ExtendMode": "ExtendMode",
    "CompositeMode": "CompositeMode",
}
COUNT = {"uint8": "Count8", "uint16": "Count16", "uint32": "Count32"}
OFFSET = {"Offset": "Off16", "LOffset": "Off32", "Offset24": "Off24"}
OFFSET_TO = {"OffsetTo": "Off16", "LOffsetTo": "Off32", "LOffset24To": "Off24"}

base_of = {}
for base, alts in _equivalents.items():
    for alt in alts:
        base_of[alt] = base

format_re = re.compile(r"([A-Za-z0-9]+)Format(\d+)$")


def selected():
    names = [n for n, _ in otData]
    spans = [(names.index(first), names.index(stop)) for first, stop in RANGES]
    for i, (name, fields) in enumerate(otData):
        inside = any(start <= i < stop for start, stop in spans)
        if (inside or name in EXTRA) and name not in SKIP:
            yield name, fields


def resolve(name):
    return base_of.get(name, name)


class Unsupported(Exception):
    pass


def ty(t, name):
    if name.startswith("ValueFormat"):
        return "ValueFormat"
    if name.endswith("Count") or name in ("StructLength", "MorphType"):
        return COUNT[t]
    if name == "SubTable":
        return "SubTable"
    if name == "ExtSubTable":
        return "ExtSubTable"
    if name == "FeatureParams":
        return "FeatureParams"
    if t in SIMPLE:
        return SIMPLE[t]
    if t == "struct":
        return f'Struct("{resolve(name)}")'
    if t in OFFSET:
        return f'{OFFSET[t]}("{resolve(name)}")'
    m = re.match(r"(\w+)\((\w+)\)$", t)
    if m and m.group(1) in OFFSET_TO:
        return f'{OFFSET_TO[m.group(1)]}("{resolve(m.group(2))}")'
    if m and m.group(1) == "AATLookup":
        return f'AatLookup("{resolve(m.group(2))}")'
    if m and m.group(1) == "AATLookupWithDataOffset":
        return f'AatLookupData("{resolve(m.group(2))}")'
    if "(" in t or t[:1].islower() or t == "GlyphID32":
        raise Unsupported(t)
    return f'Struct("{resolve(t)}")'


def cond(aux):
    if not aux:
        return "Cond::Always"
    m = re.fullmatch(r"(\w+) & (0x[0-9A-Fa-f]+)", aux)
    if m:
        return f'Cond::Mask("{m.group(1)}", {int(m.group(2), 16):#x})'
    m = re.fullmatch(r"(\w+) >= (0x[0-9A-Fa-f]+|\d+)", aux)
    if m:
        return f'Cond::AtLeast("{m.group(1)}", {int(m.group(2), 0):#x})'
    if aux == "DeltaFormat in (1,2,3)":
        return 'Cond::In123("DeltaFormat")'
    raise Unsupported(aux)


def field(spec):
    t, name, repeat, aux, _ = spec
    kind = ty(t, name)
    if isinstance(repeat, int) and repeat:
        return f'Field {{ name: "{name}", ty: {kind}, rep: Rep::Fixed({repeat}), cond: Cond::Always }}'
    if repeat:
        assert isinstance(repeat, str), (name, repeat)
        return f'Field {{ name: "{name}", ty: {kind}, rep: Rep::Count("{repeat}", {int(aux or 0)}), cond: Cond::Always }}'
    return f'Field {{ name: "{name}", ty: {kind}, rep: Rep::One, cond: {cond(aux)} }}'


def main():
    plain = {}
    formats = {}
    skipped = []
    for name, fields in selected():
        try:
            m = format_re.match(name)
            if m:
                base, number = m.group(1), int(m.group(2))
                width = {"uint16": "U16", "uint8": "U8"}[fields[0][0]]
                body = [field(f) for f in fields[1:]]
                formats.setdefault(base, (width, []))[1].append((number, body))
            else:
                body = [field(f) for f in fields]
                if any(f[1] == "Format" for f in fields):
                    assert fields[0][1] == "Format", name
                plain[name] = body
        except Unsupported as e:
            skipped.append(f"{name}: {e}")
    out = []
    out.append("// Generated by gen_schema.py from fontTools' `ttLib.tables.otData` (MIT). Do not edit by hand.")
    out.append("")
    out.append("use super::engine::{Cond, Def, Field, Layout, Rep, Ty::*, Width};")
    out.append("")
    out.append("/// Every layout, sorted by name for [`def`].")
    out.append("static DEFS: &[Def] = &[")
    for name in sorted(set(plain) | set(formats)):
        if name in plain:
            out.append(f'    Def {{ name: "{name}", layout: Layout::Plain(&[')
            out.extend(f"        {f}," for f in plain[name])
            out.append("    ]) },")
        else:
            width, variants = formats[name]
            out.append(f'    Def {{ name: "{name}", layout: Layout::Formats(Width::{width}, &[')
            for number, body in sorted(variants):
                out.append(f"        ({number}, &[")
                out.extend(f"            {f}," for f in body)
                out.append("        ]),")
            out.append("    ]) },")
    out.append("];")
    out.append("")
    out.append("/// The layout named `name`.")
    out.append("pub(super) fn def(name: &str) -> Option<&'static Def> {")
    out.append("    DEFS.binary_search_by(|d| d.name.cmp(name)).ok().and_then(|i| DEFS.get(i))")
    out.append("}")
    print("\n".join(out))
    for s in skipped:
        print("left out:", s, file=sys.stderr)


main()
