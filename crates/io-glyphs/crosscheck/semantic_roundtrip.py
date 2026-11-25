"""Sorts the differences of the source round trip into cosmetic and data changes.

`cargo xtask roundtrip --mode model --out OUT` writes what FontCraft makes of every corpus source;
this script compares each `.glyphs` file and `.glyphspackage` there with the original in two ways:
as property lists (every key, also those glyphsLib does not read) and as glyphsLib objects (what
the data means). Each file ends in one class:

- identical: the same bytes;
- cosmetic, layout: the property lists are equal (white space, key order, quoting, number
  spelling);
- cosmetic, same objects: keys differ and the objects are equal, and every key we dropped holds a
  value glyphsLib's objects give anyway (a default written out, a key of an older spelling);
- unread keys: the objects are equal, but we dropped or changed a key glyphsLib does not read:
  each needs a look;
- data, added only: the objects only gained values (a key the format requires and a hand-written
  file lacks): each pattern needs a look;
- data: the objects differ;
- not read: glyphsLib or the property list parser cannot read one side. Both sides failing the
  same way is listed apart: then only the property lists were compared.

Differences are grouped by pattern (glyph names and indices removed) with a count and an example.

Run in a Python environment with glyphsLib, isolated:
    ENV/bin/python -I crates/io-glyphs/crosscheck/semantic_roundtrip.py corpus OUT [--list N]
"""

import collections
import enum
import pathlib
import re
import sys

import glyphsLib
import openstep_plist

# Object attributes that lead back up, or hold where the file was read from.
SKIPPED = {"parent", "_parent", "font", "_font", "_glyph", "_layer", "filepath", "_owner", "_masters_cache", "_glyphs_cache"}
MAX_DEPTH = 40


def plist(text):
    return openstep_plist.loads(text, use_numbers=True)


def scalar(value):
    """A scalar as it is compared: a number and the text that spells it are the same."""
    if isinstance(value, bool):
        return float(value)
    if isinstance(value, (int, float)):
        return float(value)
    if isinstance(value, str):
        try:
            return float(value) if re.fullmatch(r"-?\d+(\.\d+)?([eE][-+]?\d+)?", value) else value
        except ValueError:
            return value
    return value


def plist_differences(a, b, path=""):
    """(path, kind, ours, original) of every difference; `a` is the original, `b` ours."""
    if isinstance(a, dict) and isinstance(b, dict):
        for key in a:
            if key not in b:
                yield f"{path}.{key}", "dropped", None, a[key]
            else:
                yield from plist_differences(a[key], b[key], f"{path}.{key}")
        for key in b:
            if key not in a:
                yield f"{path}.{key}", "added", b[key], None
    elif isinstance(a, (list, tuple)) and isinstance(b, (list, tuple)):
        if len(a) != len(b):
            yield path, f"length {len(a)} became {len(b)}", b, a
            return
        for n, (x, y) in enumerate(zip(a, b)):
            yield from plist_differences(x, y, f"{path}[{n}]")
    elif scalar(a) != scalar(b):
        yield path, "changed", b, a


def is_object(value):
    return type(value).__module__.startswith("glyphsLib") and hasattr(value, "__dict__") and not isinstance(value, enum.Enum)


def flatten(value, path, out, trail):
    """`out[path] = scalar` for everything reachable from `value`."""
    if id(value) in trail or len(trail) > MAX_DEPTH:
        return
    below = trail | {id(value)}
    if is_object(value):
        for key, item in vars(value).items():
            if key not in SKIPPED and not callable(item):
                flatten(item, f"{path}.{key.lstrip('_')}", out, below)
        # Some classes are lists or dictionaries with attributes of their own.
    if isinstance(value, dict) or hasattr(value, "items") and callable(getattr(value, "keys", None)):
        for key in value.keys():
            flatten(value[key], f"{path}.{key}", out, below)
    elif isinstance(value, (list, tuple, set, frozenset)):
        items = sorted(value, key=str) if isinstance(value, (set, frozenset)) else value
        out[f"{path}#"] = len(items)
        for n, item in enumerate(items):
            # Objects with an identity of their own are told apart by it, so that one more or one
            # less does not shift the rest.
            name = next((getattr(item, a) for a in ("layerId", "name") if is_object(item) and isinstance(getattr(item, a, None), str)), None)
            flatten(item, f"{path}[{n if name is None else name}]", out, below)
    elif not is_object(value):
        if value is None or isinstance(value, (str, int, float, bool)):
            out[path] = scalar(value)
        else:
            out[path] = str(value)


def objects(path):
    out = {}
    flatten(glyphsLib.GSFont(str(path)), "", out, frozenset())
    return out


EMPTY = (None, "", 0.0)


def object_differences(a, b):
    for key in sorted(a.keys() | b.keys()):
        x, y = a.get(key), b.get(key)
        if x != y and not (x in EMPTY and y in EMPTY):
            yield key, "changed" if key in a and key in b else "dropped" if key in a else "added", y, x


def pattern(path):
    path = re.sub(r"\[[^\]]*\]", "[]", path)
    path = re.sub(r"\.(layers|kerning\w*)\.[^.]+", r".\1[]", path)
    return re.sub(r"(\.kerning\w*\[\])\..*", r"\1...", path)


def files_of(path):
    """The property list texts of a source, by file."""
    if path.is_dir():
        return {
            f.relative_to(path).as_posix(): f.read_text(encoding="utf-8")
            for f in sorted(path.rglob("*"))
            if f.is_file() and f.suffix in (".plist", ".glyph", ".plistx", "")
        }
    return {"": path.read_text(encoding="utf-8")}


def raw_differences(original, ours):
    a, b = files_of(original), files_of(ours)
    out = []
    for name in sorted(a.keys() | b.keys()):
        if name not in b:
            out.append((name, "file dropped", None, None))
        elif name not in a:
            out.append((name, "file added", None, None))
        elif a[name] != b[name]:
            out.extend((f"{name}{p}", kind, y, x) for p, kind, y, x in plist_differences(plist(a[name]), plist(b[name])))
    return out, a == b


def compare(original, ours):
    """(class, differences) of one source."""
    try:
        raw, same = raw_differences(original, ours)
    except Exception as e:
        return "not read", [("", f"property list: {type(e).__name__}: {e}", None, None)]
    if same:
        return "identical", []
    if not raw:
        return "cosmetic, layout", []
    results = []
    for path in (original, ours):
        try:
            results.append(objects(path))
        except Exception as e:
            results.append(f"{type(e).__name__}: {e}")
    if all(isinstance(r, str) for r in results):
        return "not read by glyphsLib, both sides", raw
    if any(isinstance(r, str) for r in results):
        return "not read", [("", f"original: {str(results[0])[:120]}, ours: {str(results[1])[:120]}", None, None)] + raw
    differences = list(object_differences(*results))
    if differences:
        return "data" if any(d[1] != "added" for d in differences) else "data, added only", differences
    lost = [d for d in raw if d[1] != "added" and not (d[1] == "dropped" and scalar(d[3]) in EMPTY)]
    return ("unread keys", lost) if lost else ("cosmetic, same objects", raw)


def short(value):
    text = repr(value)
    return text if len(text) < 60 else text[:57] + "..."


def main():
    args = [a for a in sys.argv[1:] if a != "--list"]
    listed = int(args.pop()) if "--list" in sys.argv else 40
    corpus, out = pathlib.Path(args[0]), pathlib.Path(args[1])
    counts = collections.Counter()
    patterns = collections.defaultdict(lambda: collections.defaultdict(lambda: [0, set(), ""]))
    for ours in sorted([*out.glob("**/*.glyphs"), *out.glob("**/*.glyphspackage")]):
        rel = ours.relative_to(out)
        cls, differences = compare(corpus / rel, ours)
        counts[cls] += 1
        if cls not in ("identical", "cosmetic, layout"):
            print(f"{cls}: {rel}: {len(differences)} differences")
        for path, kind, y, x in differences:
            entry = patterns[cls][f"{pattern(path)} {kind}"]
            entry[0] += 1
            entry[1].add(str(rel))
            entry[2] = entry[2] or f"{rel}{path}: {short(x)} became {short(y)}"
    for cls, rows in sorted(patterns.items()):
        print(f"\n== {cls}: {len(rows)} patterns")
        ordered = sorted(rows.items(), key=lambda r: (-len(r[1][1]), -r[1][0]))
        for text, (n, sources, example) in ordered[:listed]:
            print(f"{n} x {text} in {len(sources)} files (e.g. {example})")
        if len(ordered) > listed:
            print(f"{len(ordered) - listed} more patterns")
    print("\n" + "\n".join(f"{n} {cls}" for cls, n in sorted(counts.items())))
    print(
        f"{sum(counts.values())} sources, {counts['data']} with data changes, "
        f"{counts['data, added only']} with added data only, {counts['unread keys']} with unread keys"
    )


main()
