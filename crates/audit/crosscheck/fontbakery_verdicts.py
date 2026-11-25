#!/usr/bin/env python3
"""Compare FontBakery's verdicts with FontCraft's audit, check by check, over the corpus.

Oracle: FontBakery 1.1.0 (Apache-2.0, github.com/fonttools/fontbakery). This script runs the
FontBakery executable and reads its JSON report; it copies no FontBakery code.

For every compiled font under the corpus (TTF, OTF, WOFF, WOFF2), and for fontc's builds of the
real-world corpus sources, the script runs

  * FontBakery, through a generated profile holding every FontBakery check that a row of
    `spec/inventory/21-qa-checks.toml` names in its `ref` ("fontbakery <check id>"), and
  * `fontcraft-cli check --profile strict`, which runs every FontCraft check,

and compares the verdicts of each FontBakery check with those of the FontCraft check(s) the
inventory maps it to. A verdict is "flagged" (WARN, FAIL or FATAL upstream; warn or fail here) or
"clean" (PASS, INFO or SKIP upstream; pass, info or skipped here). An ERROR on either side is its
own verdict. Each disagreement is classified through `KNOWN` below as

  * `purpose`: a difference we chose, written down in docs/spec/21-qa.md or the check's row;
  * `data`: the input, not the check, explains it (a conformance-test font the upstream tool
    cannot load, a check that needs the network, ...);
  * `ours`: anything not in `KNOWN`; each one is a bug to fix or to list with its reason.

Usage (Python run isolated, from the repository root):

  python3 -I crates/audit/crosscheck/fontbakery_verdicts.py \
      --cli target/release/fontcraft-cli --corpus corpus --out ~/.cache/scratch/fontcraft/M12.81

Reports land in --out: `report.md` (agreement per check, the disagreements by class),
`disagreements.json`, and the raw reports of both tools under `fb/` and `fc/` (reused on a rerun
unless --fresh is given).
"""

from __future__ import annotations

import argparse
import concurrent.futures as cf
import hashlib
import json
import os
import re
import subprocess
import sys
import tomllib
from collections import Counter, defaultdict
from pathlib import Path

REPO = Path(__file__).resolve().parents[3]
INVENTORY = REPO / "spec" / "inventory" / "21-qa-checks.toml"
FB_DEFAULT = Path.home() / ".local/share/uv/tools/fontbakery/bin/fontbakery"
FONTTOOLS_PY = Path.home() / ".local/share/uv/tools/afdko/bin/python"
FONT_SUFFIXES = {".ttf", ".otf", ".woff", ".woff2"}
# Real-world sources that fontc builds; the corpus test fixtures are left out.
FONTC_SOURCES = [
    "oswald/sources/*.glyphs",
    "markow/sources/*.glyphs",
    "inter/src/*.glyphspackage",
    "openplotfont/fonts/*.glyphspackage",
]
TIMEOUT_S = 600
# Fonts on which each check is first run alone, to find the ones that need optional extras.
PROBE_FONTS = {"Oswald-Regular.ttf", "InterVariable.ttf", "Oswald[wght].ttf"}
# Mapped checks left out of the FontBakery run, with the reason.
NOT_RUN = {
    "fontvalidator": "wraps an external validator that is not installed with FontBakery",
}

FB_FLAGGED = {"WARN", "FAIL", "FATAL"}
FB_CLEAN = {"PASS", "INFO", "SKIP"}
FC_FLAGGED = {"warn", "fail"}
FC_CLEAN = {"pass", "info", "skipped"}

# FontCraft checks that run only with --network.
NETWORK_CHECKS = {"check.files.auditorVersion", "check.names.nameCollision"}

# Known disagreements: (FontBakery check, direction) -> (class, reason). The direction is
# "fb" when only FontBakery flags, "fc" when only FontCraft flags, "error" when either side
# errors. A value may also be a function (font path, FontBakery check result, our checks) ->
# (class, reason) or None, for differences that hold for some fonts only.
AOTS = "/aots/"


def aots_only(cls: str, why: str):
    """A difference explained for the fonts of fontTools' OpenType conformance suite only."""
    return lambda font, fb, ours: (cls, why) if AOTS in f"/{font}" else None


PROJECT_VALUE = "the expected value is a project value (21-qa.md § 2); without one our check skips"
MERGED_OWN = "our merged check also reports a case this upstream check does not test"
SCHEME = ("our check tests one vertical-metrics scheme, chosen by parameter (21-qa.md § 2); "
          "each vendor check tests its own scheme")
OWN_TOOL = ("upstream wraps an external program; ours is our own implementation (21-qa.md § 9) "
            "and tests what our parser, dump format or rasteriser needs")
NO_TABLE = "the font lacks the table; upstream fails, our check skips and the table check reports it"

KNOWN: dict[tuple[str, str], object] = {
    # Project values and vendor policy.
    ("microsoft/fstype", "fb"): ("purpose", PROJECT_VALUE + " (fsType)"),
    ("googlefonts/fstype", "fb"): ("purpose", PROJECT_VALUE + " (fsType)"),
    ("typenetwork/glyph_coverage", "fb"): ("purpose", PROJECT_VALUE + " (glyphSet)"),
    ("opentype/vendor_id", "fc"): ("purpose", "upstream reads the vendor registry over the network and "
                                    "skips under --skip-network; ours uses the bundled registry"),
    ("googlefonts/name/version_format", "fb"): ("purpose", "vendor version-string format; ours "
                                                 "reports only the version mismatch as a failure"),
    ("microsoft/version", "fb"): ("purpose", "vendor version-string format; ours reports only the "
                                  "version mismatch as a failure"),
    ("googlefonts/name/version_format", "fc"): ("purpose", MERGED_OWN + ": name ID 5 against head.fontRevision"),
    ("microsoft/version", "fc"): ("purpose", MERGED_OWN + ": name ID 5 against head.fontRevision"),
    ("adobefonts/unsupported_tables", "fb"): ("purpose", "the allowed tables are a project value "
                                              "(allowedTables); without one our check skips"),
    ("notofonts/cmap/alien_codepoints", "fb"): ("purpose", "private-use code points are information "
                                                "here; a failure only under the Noto policy"),
    ("typenetwork/PUA_encoded_glyphs", "fb"): ("purpose", "private-use code points are information here"),
    ("googlefonts/has_ttfautohint_params", "fc"): ("purpose", MERGED_OWN + ": an old autohinter version"),
    # Vertical metrics: one scheme by parameter.
    **{(c, "fc"): ("purpose", SCHEME) for c in (
        "googlefonts/cjk_vertical_metrics", "microsoft/vertical_metrics", "typenetwork/vertical_metrics",
        "typenetwork/font_is_centered_vertically")},
    ("typenetwork/vertical_metrics", "fb"): ("purpose", SCHEME),
    # Merged checks whose codes do not tell the upstream checks apart.
    ("typenetwork/family/valid_strikeout", "fc"): ("purpose", MERGED_OWN + ": values that differ from the computed default (strict)"),
    ("typenetwork/family/valid_underline", "fc"): ("purpose", MERGED_OWN + ": values that differ from the computed default (strict)"),
    ("vtt_volt_data", "fc"): ("purpose", "merged with upstream's unwanted_tables, which also names FFTM and other editor tables"),
    ("opentype/mac_style", "fc"): ("purpose", "fsselection and mac_style are one check by design; the result is about fsSelection"),
    # External programs.
    ("ots", "fb"): ("purpose", OWN_TOOL),
    ("ttx_roundtrip", "fb"): ("purpose", OWN_TOOL),
    ("freetype_rasterizer", "fb"): ("purpose", OWN_TOOL),
    # Upstream cannot see it.
    ("unique_glyphnames", "fc"): ("purpose", "duplicate names in a CFF charset or post table; fontTools "
                                  "renames duplicates while loading, so upstream never sees them"),
    # The OpenType conformance-suite fonts: synthetic tables and shapes.
    ("mandatory_glyphs", "fc"): aots_only("data", "conformance-suite font whose .notdef is a zero-area outline"),
    ("caps_vertically_centered", "fc"): aots_only("data", "conformance-suite font: upstream finds capitals "
                                                  "by glyph name and skips; ours by cmap, where they are arbitrary shapes"),
    ("cmap/format_12", "fb"): aots_only("data", "conformance-suite font with no cmap format 4 subtable, which "
                                        "our cmapSubtables check reports"),
}


def only_codes(codes: set[str], cls: str, why: str):
    """A difference explained when upstream reports these codes and no others."""
    return lambda font, fb, ours: (cls, why) if fb.get("codes") and set(fb["codes"]) <= codes else None


def only_ours(codes: set[str], cls: str, why: str):
    """A difference explained when our flags carry these codes and no others."""
    def f(font, fb, ours):
        got = {c for o in ours.values() for c in o.get("codes", [])}
        return (cls, why) if got and got <= codes else None
    return f


CORPUS = [Path(".")]  # set by main()


def lacks(tag: bytes, cls: str, why: str):
    """A difference explained when the font (TrueType or CFF, not WOFF) has no `tag` table."""
    def f(font, fb, ours):
        try:
            with open(font if font.is_absolute() else CORPUS[0] / font, "rb") as fh:
                head = fh.read(12)
                count = int.from_bytes(head[4:6], "big") if len(head) == 12 else 0
                tags = {fh.read(16)[:4] for _ in range(min(count, 512))}
        except OSError:
            return None
        return (cls, why) if head[:4] in (b"\0\1\0\0", b"OTTO", b"true") and tag not in tags else None
    return f


ONE_FILE = "the corpus run hands upstream one file at a time, so its family check cannot see the other styles"
KNOWN[("opentype/STAT/ital_axis", "fb")] = only_codes({"missing-roman"}, "data", ONE_FILE)
CUTOFF = ("upstream passes a font once it finds more than 100 cases, counted over the decomposed "
          "glyphs; ours counts each layer's own points and collapses past audit.resultCap (also 100)")


def cutoff_or(other=None):
    """Upstream gave up past its false-positive cutoff; otherwise ask `other`."""
    return lambda font, fb, ours: ("purpose", CUTOFF) if fb.get("by_design") else (
        other(font, fb, ours) if other else None)


KNOWN[("outline_alignment_miss", "fc")] = cutoff_or(aots_only("data", "conformance-suite font with arbitrary shapes"))
for _c in ("outline_short_segments", "outline_colinear_vectors", "outline_jaggy_segments",
           "outline_semi_vertical"):
    KNOWN.setdefault((_c, "fc"), cutoff_or())
KNOWN[("opentype/fsselection", "fc")] = ("purpose", "ours takes the style from the name table, upstream "
                                         "from the file name; on these fonts the two disagree")
KNOWN[("transformed_components", "fc")] = only_ours({"scaled-or-rotated"}, "purpose",
    "a source does not say whether its export will be hinted, so ours warns on any scale or rotation; "
    "upstream flags those only on hinted fonts (the row's guess)")
KNOWN[("fontwerk/style_linking", "fc")] = ("purpose", MERGED_OWN + ": a subfamily name other than "
                                           "Regular, Bold, Italic or Bold Italic (office_ribz_req)")
KNOWN[("microsoft/office_ribz_req", "fc")] = ("purpose", MERGED_OWN + ": the subfamily name against the "
                                              "bold and italic bits (style_linking)")
KNOWN[("notofonts/unicode_range_bits", "fb")] = ("purpose", "ours compares each Unicode range bit with "
                                                 "the cmap only when its strict parameter is on, which "
                                                 "the strict profile leaves off")
for _c in ("name_length_req", "instances_name_length_req"):
    KNOWN[(_c, "fb")] = lacks(b"name", "purpose", NO_TABLE)
KNOWN[("googlefonts/name/mandatory_entries", "fb")] = only_codes({"missing-entry"}, "purpose",
    "vendor policy: name IDs 16 and 17 for every style outside Regular, Bold, Italic and Bold Italic; "
    "ours requires the records the OpenType specification requires")
KNOWN[("opentype/xavgcharwidth", "fb")] = only_codes({"missing-glyphs"}, "data", "the font lacks glyphs "
                                                    "that upstream's weighted average needs; ours skips")
KNOWN[("notofonts/hmtx/encoded_latin_digits", "fb")] = only_codes({"missing-digit"}, "purpose",
    "vendor policy: every font encodes the digits; ours compares digit widths only when there are digits")
KNOWN[("overlapping_path_segments", "fb")] = (
    "purpose", "upstream compares end points only, so it also flags zero-length segments and curves "
    "that share their ends but not their handles; ours needs the handles to match too")

KNOWN[("outline_direction", "fb")] = lambda font, fb, ours: (
    "purpose", "upstream reports the single-point contours autohinters leave as counter-clockwise; "
    "ours reports them as stray points") if fb.get("single_points") else None

for _c in ("family/vertical_metrics", "family/win_ascent_and_descent", "googlefonts/vertical_metrics",
           "linegaps", "opentype/family/panose_familytype", "opentype/monospace", "os2_metrics_match_hhea",
           "typenetwork/font_is_centered_vertically", "typoascender_exceeds_Agrave",
           "adobefonts/nameid_1_win_english", "opentype/varfont/valid_default_instance_nameids",
           "notofonts/cmap/unexpected_subtables", "opentype/varfont/distinct_instance_records"):
    KNOWN.setdefault((_c, "fb"), ("purpose", NO_TABLE))



def fonts(names: tuple[str, ...], cls: str, why: str, other=None):
    """A difference explained on these fonts only (by file name); otherwise ask `other`."""
    def f(font, fb, ours):
        if font.name in names:
            return (cls, why)
        if callable(other):
            return other(font, fb, ours)
        return other
    return f


def also(key: tuple[str, str], entry) -> None:
    """Adds an explanation to the ones KNOWN has for `key`: the first that answers wins."""
    first = KNOWN.get(key)
    if first is None:
        KNOWN[key] = entry
    elif callable(first):
        KNOWN[key] = lambda font, fb, ours: first(font, fb, ours) or entry(font, fb, ours)


NO_OS2 = "the font has no OS/2 table; upstream fails, our check skips and check.tables.requiredTables reports it"
also(("opentype/xavgcharwidth", "fb"), only_codes({"lacks-OS/2"}, "purpose", NO_OS2))
also(("typenetwork/family/valid_underline", "fb"), lacks(b"OS/2", "purpose", NO_OS2))
also(("fontwerk/names_match_default_fvar", "fb"), only_codes(
    {"missing-default-name-id"}, "purpose", "no named instance sits at the default location, which OpenType does not "
    "ask for; ours compares the names of the instance that does"))
also(("instances_name_length_req", "fb"), fonts(
    ("I.otf",), "purpose", "ours reports the longest STAT style name in place of each named instance "
    "(name/family_and_style_max_length agrees)"))
also(("opentype/maxadvancewidth", "fc"), fonts(
    ("I.otf", "Amstelvar-avar2.subset.ttf"), "data", "an earlier upstream check recompiles the open font, which "
    "recalculates hhea.advanceWidthMax; run alone, the upstream check fails these fonts too"))
also(("missing_small_caps_glyphs", "fc"), ("purpose", "upstream tests that the glyphs the lookups name exist; ours "
                                           "also that every letter has a small cap"))
also(("cjk_not_enough_glyphs", "fb"), aots_only("data", "conformance-suite font whose only cmap subtable is in a "
                                                "legacy encoding, which upstream reads as Unicode"))
also(("case_mapping", "fc"), aots_only("data", "conformance-suite font whose cmap subtables disagree; the two tools "
                                       "take different ones"))
for _c in ("notofonts/cmap/alien_codepoints", "typenetwork/PUA_encoded_glyphs"):
    also((_c, "fc"), fonts(("cmap10_font1.otf",), "purpose", "upstream tests the private-use area of the Basic "
                           "Multilingual Plane only; ours also planes 15 and 16"))
also(("outline_direction", "fb"), fonts(
    ("JetBrainsMono.woff2", "JetBrainsMono-Italic.woff2", "Oswald.glyphs"), "purpose", "mirrored components: ours "
    "reports them in check.components.transformed, not in the direction check, which tests each layer's own paths"))
also(("outline_alignment_miss", "fb"), only_codes(
    {"skip-cap-x-height-alignment"}, "purpose", "upstream also warns that an OS/2 version below 2 has no x-height "
    "and cap height to test; ours tests the lines the font has"))
also(("outline_alignment_miss", "fb"), fonts(
    ("Amstelvar-avar2.subset.ttf", "Oswald-Regular.ttf"), "purpose", "upstream counts implied on-curve points and the "
    "points of decomposed components; ours counts each layer's own nodes"))
also(("outline_short_segments", "fb"), fonts(
    ("NotoSansArabic-Regular.quadratic.subset.ttf",), "purpose", "upstream measures each quadratic curve; ours the "
    "whole spline between two on-curve points"))
also(("overlapping_path_segments", "fc"), fonts(
    ("Oswald[wght].ttf", "Oswald[wght].woff2", "Oswald.glyphs"), "data", "the segments coincide in a master other than "
    "the default one, which upstream reads"))
also(("microsoft/vertical_metrics", "fb"), ("purpose", SCHEME))
also(("microsoft/fvar_STAT_axis_ranges", "fb"), fonts(
    ("Amstelvar-avar2.subset.ttf",), "purpose", "upstream warns that STAT has no axis values; ours does so in "
    "statValueQuality"))

# FontBakery conditions that make upstream skip for a reason outside the check itself: how the
# corpus is run (data), or a scope we chose to widen (purpose).
SKIP_CONDITIONS = {
    "network": ("data", "upstream needs the network for this check and skips under --skip-network"),
    "style": ("data", "upstream takes the style from the file name and skips when the name holds no "
              "style it knows; ours reads the style from the font"),
    "is_ttf": ("purpose", "upstream runs this check on TrueType outlines only; ours also on CFF outlines"),
    "not is_italic": ("data", "the font sets the italic style bits with an italic angle of 0; upstream "
                      "skips italic fonts, ours skips by the italic angle"),
    "not is_variable_font": ("purpose", "upstream skips variable fonts; ours checks their default outlines"),
}

# Codes of a merged FontCraft check that belong to one upstream check; a merged check's result
# counts for an upstream check only when one of its codes belongs to it or to none listed here.
# "" marks a code that belongs to no upstream check (a FontCraft addition); a tuple, a code that
# several upstream checks share.
# A code may carry "#<name ID>" when the finding names one record; without its own entry it
# belongs where its bare code does.
MANDATORY_NAMES = ("googlefonts/name/mandatory_entries", "typenetwork/name/mandatory_entries")
CODE_OWNER = {
    "check.outlines.pathDirection": {"counter-same-direction": "", "nested-clockwise": ""},
    "check.layout.gdefMarkClass": {"mark-chars": "opentype/gdef_mark_chars",
                                   "non-mark-chars": "opentype/gdef_non_mark_chars",
                                   "spacing-mark-glyphs": "opentype/gdef_spacing_marks"},
    "check.names.requiredRecords": {"missing#1": ("name_id_1", "adobefonts/nameid_1_win_english", *MANDATORY_NAMES),
                                    "missing#2": ("name_id_2", *MANDATORY_NAMES), "missing#4": MANDATORY_NAMES,
                                    "missing#6": MANDATORY_NAMES, "missing": (),
                                    "missing-recommended": "", "missing-recommended#0": "microsoft/copyright",
                                    "missing-recommended#7": "microsoft/trademark",
                                    "missing-recommended#8": "microsoft/manufacturer"},
    "check.names.postScriptNameCharacters": {"bad-psname": "opentype/postscript_name"},
    "check.names.postScriptNameConsistency": {"ps-cff-name-mismatch": "opentype/name/postscript_vs_cff"},
    "check.metrics.digitWidths": {"bad-digit-width": "notofonts/hmtx/encoded_latin_digits",
                                  "missing-digit": "notofonts/hmtx/encoded_latin_digits",
                                  "comma-period": "notofonts/hmtx/comma_period"},
    "check.names.copyrightInDescription": {"copyright-on-description": "name/no_copyright_on_description"},
    "check.tables.fsSelection": {"bad-regular": "opentype/fsselection"},
    "check.variations.statValueQuality": {"axis-order-invalid": "microsoft/STAT_table_axis_order",
                                          "missing-axis-value": "googlefonts/STAT/compulsory_axis_values",
                                      "no-axis-values": ("microsoft/STAT_axis_values",
                                                             "microsoft/STAT_table_eliding_bit")},
    "check.variations.instanceNameIds": {"missing-axis-name": "", "missing-name": "fvar_name_entries"},
    "check.variations.statCoversInstances": {"ambiguous-axis-value": "microsoft/fvar_STAT_axis_ranges"},
    "check.metrics.whitespaceWidths": {"bad-whitespace-advance": "notofonts/hmtx/whitespace_advances",
                                       "zero-space": "whitespace_widths"},
    "check.variations.defaultInstanceNames": {
        "invalid-default-instance-postscript-name": "opentype/varfont/valid_default_instance_nameids"},
    "check.tables.weightClassRange": {"cff-below-250": ""},
    "check.names.familyAndStyleLength": {
        "nameid1-too-long": ("name/family_and_style_max_length", "name_length_req"),
        "familyname-plus-stat-entries-too-long": "name/family_and_style_max_length",
        "fvar-instance-too-long": ("name/family_and_style_max_length", "instances_name_length_req")},
    "check.tables.codePageRanges": {"no-code-pages": "opentype/code_pages", "bad-code-page-bit": "",
                                    "bad-range-bit": "notofonts/unicode_range_bits",
                                    "non-plane-0-without-format-12": ""},
    "check.metrics.tabularWidths": {"tnum-unequal": "tnum_glyphs_equal_widths", "group-unequal": "",
                                    "figure-space": ""},
}


def for_upstream(fb_check: str, check: str, res: dict) -> dict:
    """`res` as it bears on `fb_check`: flags raised only by codes of other checks do not count."""
    owners = CODE_OWNER.get(check)
    if not owners or res["status"] not in FC_FLAGGED:
        return res
    def owner(code):
        return owners[code] if code in owners else owners.get(code.split("#")[0], fb_check)

    mine = [c for c in res["codes"] if fb_check in ((o,) if isinstance(o := owner(c), str) else o)]
    return res if mine else {**res, "status": "pass", "codes": [], "other_codes": res["codes"]}


# ---------------------------------------------------------------- inputs


def load_mapping() -> dict[str, list[str]]:
    """FontBakery check id -> FontCraft check ids, from the inventory's `ref` fields."""
    rows = tomllib.loads(INVENTORY.read_text(encoding="utf-8")).get("check", [])
    mapping: dict[str, list[str]] = defaultdict(list)
    for row in rows:
        for part in str(row.get("ref", "")).split(";"):
            m = re.match(r"\s*fontbakery\s+([A-Za-z0-9_/]+)\s*$", part.split("(")[0])
            if m and row["id"] not in mapping[m.group(1)]:
                mapping[m.group(1)].append(row["id"])
    return dict(mapping)


def fb_known_checks(fb: Path) -> set[str]:
    py = fb.parent / "python"
    code = (
        "import warnings; warnings.simplefilter('ignore')\n"
        "from fontbakery.fonts_profile import load_all_checks, checks_by_id\n"
        "load_all_checks(); print('\\n'.join(sorted(checks_by_id)))\n"
    )
    out = subprocess.run([str(py), "-I", "-c", code], capture_output=True, text=True, check=True)
    return set(out.stdout.split())


def fb_version(fb: Path) -> str:
    out = subprocess.run([str(fb), "--version"], capture_output=True, text=True)
    return out.stdout.strip() or out.stderr.strip()


def write_profile(path: Path, checks: list[str]) -> None:
    body = json.dumps({"name": "fontcraft_crosscheck", "sections": {"crosscheck": checks}}, indent=1)
    path.write_text(f"PROFILE = {body}\n", encoding="utf-8")


def probe_extras(fb: Path, checks: list[str], fonts: list[Path], out: Path,
                 jobs: int) -> dict[str, str]:
    """Checks that make FontBakery stop because an optional extra is missing, with the extra.

    One such check aborts the whole run, so each check is first run alone on a few fonts.
    The answer is cached in `probe.json` beside the reports.
    """
    cache = out / "probe.json"
    key = {"checks": checks, "fonts": [str(f) for f in fonts]}
    if cache.exists():
        doc = json.loads(cache.read_text(encoding="utf-8"))
        if doc.get("key") == key:
            return doc["needs"]

    (out / "probe").mkdir(exist_ok=True)

    def one(check):
        # `-c` matches substrings of check ids, so each check gets a profile of its own.
        single = out / "probe" / f"{check.replace('/', '__')}.py"
        write_profile(single, [check])
        for font in fonts:
            r = subprocess.run(
                [str(fb), "check-profile", str(single), "-q", "-n", "--skip-network", str(font)],
                capture_output=True, text=True, timeout=TIMEOUT_S,
            )
            m = re.search(r"with the '([a-z0-9]+)' extra", r.stdout + r.stderr)
            if m:
                return check, m.group(1)
        return check, None

    with cf.ThreadPoolExecutor(max_workers=max(1, jobs)) as pool:
        needs = {c: e for c, e in pool.map(one, checks) if e}
    cache.write_text(json.dumps({"key": key, "needs": needs}, indent=1), encoding="utf-8")
    return needs


def corpus_fonts(corpus: Path) -> list[Path]:
    found = []
    for root, dirs, files in os.walk(corpus, followlinks=True):
        dirs[:] = sorted(d for d in dirs if d != ".git")
        for f in sorted(files):
            if Path(f).suffix.lower() in FONT_SUFFIXES:
                found.append(Path(root) / f)
    return found


def slug(path: Path) -> str:
    h = hashlib.sha1(str(path).encode()).hexdigest()[:8]
    return f"{path.stem}-{h}"


# ---------------------------------------------------------------- runs


def build_with_fontc(corpus: Path, out: Path, fresh: bool) -> list[tuple[Path, Path]]:
    """Build each real-world source with fontc; returns (source, binary) pairs that built."""
    built = []
    dest = out / "fontc"
    dest.mkdir(parents=True, exist_ok=True)
    for pattern in FONTC_SOURCES:
        for src in sorted(corpus.glob(pattern)):
            ttf = dest / f"{slug(src)}.ttf"
            if fresh or not ttf.exists():
                build_dir = dest / f"{slug(src)}.build"
                r = subprocess.run(
                    ["fontc", "--build-dir", str(build_dir), "-o", str(ttf), str(src)],
                    capture_output=True, text=True, timeout=TIMEOUT_S,
                )
                if r.returncode != 0 or not ttf.exists():
                    print(f"fontc failed on {src}: {r.stderr.strip()[-300:]}", file=sys.stderr)
                    continue
            built.append((src, ttf))
    return built


def unwrap(fonttools_python: Path, font: Path, dest_dir: Path) -> Path | None:
    """FontBakery reads only TTF and OTF: a WOFF or WOFF2 file is decoded with fontTools first.

    Our audit still reads the original file, so the comparison also covers our WOFF decoders.
    """
    dest_dir.mkdir(parents=True, exist_ok=True)
    dest = dest_dir / f"{slug(font)}.sfnt"
    if not dest.exists():
        code = ("import sys\nfrom fontTools.ttLib import TTFont\n"
                "f = TTFont(sys.argv[1]); f.flavor = None; f.save(sys.argv[2])\n")
        r = subprocess.run([str(fonttools_python), "-I", "-c", code, str(font), str(dest)],
                           capture_output=True, text=True, timeout=TIMEOUT_S)
        if r.returncode != 0:
            return None
    # FontBakery picks its handling by extension.
    with open(dest, "rb") as fh:
        ext = ".otf" if fh.read(4) == b"OTTO" else ".ttf"
    named = dest.with_suffix(ext)
    if not named.exists():
        named.symlink_to(dest.name)
    return named


def run_fb(fb: Path, profile: Path, font: Path | None, dest: Path, fresh: bool) -> Path:
    if font is not None and (fresh or not dest.exists()):
        subprocess.run(
            [str(fb), "check-profile", str(profile), "-q", "-n", "--skip-network",
             "--json", str(dest), str(font)],
            capture_output=True, text=True, timeout=TIMEOUT_S,
        )
    return dest


def run_fc(cli: Path, font: Path, dest: Path, fresh: bool) -> Path:
    if fresh or not dest.exists():
        r = subprocess.run(
            [str(cli), "check", str(font), "--profile", "strict", "--format", "json",
             "--include-passed", "--fail-on", "fail"],
            capture_output=True, text=True, timeout=TIMEOUT_S,
        )
        if r.stdout.strip().startswith("{"):
            dest.write_text(r.stdout, encoding="utf-8")
        else:
            dest.write_text(json.dumps({"runError": r.stderr.strip()[-2000:], "exit": r.returncode}))
    return dest


# ---------------------------------------------------------------- verdicts


def only_single_points(text: str) -> bool:
    """Is every glyph upstream's direction check lists one whose contour is a single point?"""
    listed = {kind: {m.group(1) for m in re.finditer(r"\* (.+?) has a " + kind, text)}
              for kind in ("counter-clockwise outer contour", "path with no bounds")}
    turned, points = listed.values()
    return bool(turned) and turned <= points and "more." not in text


def fb_results(path: Path) -> dict[str, dict]:
    """FontBakery check id -> {"status", "codes"} for one font."""
    try:
        doc = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return {}
    res = {}
    for section in doc.get("sections", []):
        for check in section.get("checks", []):
            key = check.get("key", [])
            m = re.search(r"FontBakeryCheck:([^>]+)>", str(key[1] if len(key) > 1 else ""))
            if not m:
                continue
            codes = sorted({
                str((log.get("message") or {}).get("code", ""))
                for log in check.get("logs", [])
                if log.get("status") in FB_FLAGGED | {"ERROR"}
            } - {""})
            unmet = set()
            for log in check.get("logs", []):
                msg = str((log.get("message") or {}).get("message", ""))
                if log.get("status") == "SKIP" and msg.startswith("Unfulfilled Conditions:"):
                    unmet |= {c.strip() for c in msg.split(":", 1)[1].split(",")}
            text = "\n".join(str((log.get("message") or {}).get("message", "")) for log in check.get("logs", []))
            by_design = any("probably by design" in str((log.get("message") or {}).get("message", ""))
                            for log in check.get("logs", []))
            res[m.group(1)] = {"status": check.get("result", "ERROR"), "codes": codes,
                               "unmet": sorted(unmet), "by_design": by_design,
                               "single_points": only_single_points(text)}
    return res


def fc_results(path: Path) -> tuple[dict[str, dict], str | None]:
    """FontCraft check id -> {"status", "codes"} for one font, and a run error if any."""
    try:
        doc = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, ValueError) as e:
        return {}, f"unreadable report: {e}"
    if "runError" in doc:
        return {}, doc["runError"] or f"exit {doc.get('exit')}"
    codes: dict[str, set] = defaultdict(set)
    for r in doc.get("results", []):
        if r.get("severity") in FC_FLAGGED and not r.get("ignored"):
            codes[r.get("check", "")].add(str(r.get("code", "")))
            name_id = (r.get("location") or {}).get("nameId")
            if isinstance(name_id, int):
                # The name ID tells apart upstream checks that each test one record.
                codes[r.get("check", "")].add(f"{r.get('code', '')}#{name_id}")
    collapsed = {r.get("check", "") for r in doc.get("results", []) if r.get("code") == "too-many-results"}
    res = {}
    for c in doc.get("checks", []):
        res[c["id"]] = {"status": c.get("status", "error"), "codes": sorted(codes.get(c["id"], ())),
                        "collapsed": c["id"] in collapsed}
    return res, None


def fb_verdict(status: str | None) -> str:
    if status is None:
        return "absent"
    if status in FB_FLAGGED:
        return "flagged"
    if status in FB_CLEAN:
        return "clean"
    return "error"


def fc_verdict(statuses: list[str]) -> str:
    if not statuses:
        return "absent"
    if any(s == "error" for s in statuses):
        return "error"
    if any(s in FC_FLAGGED for s in statuses):
        return "flagged"
    return "clean"


def classify(fb_check: str, direction: str, font: Path, fb_res: dict, ours: dict) -> tuple[str, str]:
    if direction == "error" and fb_res.get("status") == "ERROR":
        return ("data", "the upstream check raised an error on this font, so it gives no verdict")
    if direction == "error" and not fb_res:
        return ("data", "FontBakery could not load this font")
    unmet = SKIP_CONDITIONS.keys() & set(fb_res.get("unmet", ()))
    if direction == "fc" and unmet:
        return SKIP_CONDITIONS[min(unmet)]
    if direction == "fb" and any(o.get("collapsed") for o in ours.values()):
        return ("purpose", "ours found more than audit.resultCap cases and collapsed them into one "
                "information result (21-qa.md § 3)")
    if direction == "fb" and ours and all(c.startswith("check.family.") and o["status"] == "skipped"
                                          for c, o in ours.items()):
        return ("data", ONE_FILE + "; ours skips a family check given a single font")
    entry = KNOWN.get((fb_check, direction))
    if callable(entry):
        got = entry(font, fb_res, ours)
        if got:
            return got
    elif entry:
        return entry
    return ("ours", "not yet explained")


# ---------------------------------------------------------------- main


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    ap.add_argument("--cli", type=Path, default=REPO / "target/release/fontcraft-cli")
    ap.add_argument("--fontbakery", type=Path, default=FB_DEFAULT)
    ap.add_argument("--corpus", type=Path, default=REPO / "corpus")
    ap.add_argument("--out", type=Path, default=Path.home() / ".cache/scratch/fontcraft/fontbakery-verdicts")
    ap.add_argument("--fonttools-python", type=Path, default=FONTTOOLS_PY,
                    help="a Python with fontTools and brotli, to decode WOFF and WOFF2 for FontBakery")
    ap.add_argument("--jobs", type=int, default=6)
    ap.add_argument("--limit", type=int, default=0, help="only the first N fonts (for a trial run)")
    ap.add_argument("--no-fontc", action="store_true", help="skip the fontc builds of the sources")
    ap.add_argument("--fresh", action="store_true", help="rerun both tools instead of reusing reports")
    args = ap.parse_args()

    out = args.out.expanduser()
    CORPUS[0] = args.corpus.resolve()
    for d in ("fb", "fc"):
        (out / d).mkdir(parents=True, exist_ok=True)

    mapping = load_mapping()
    known = fb_known_checks(args.fontbakery)
    missing_upstream = sorted(c for c in mapping if c not in known)
    compared = sorted(c for c in mapping if c in known and c not in NOT_RUN)
    profile = out / "crosscheck_profile.py"
    write_profile(profile, compared)
    version = fb_version(args.fontbakery)

    fonts = [(f, f) for f in corpus_fonts(args.corpus.resolve())]
    probes = [f for f, _ in fonts if f.name in PROBE_FONTS]
    needs_extra = probe_extras(args.fontbakery, compared, probes, out, args.jobs)
    for check, extra in needs_extra.items():
        NOT_RUN[check] = f"needs FontBakery's optional '{extra}' dependencies, not installed"
    compared = [c for c in compared if c not in needs_extra]
    write_profile(profile, compared)
    if not args.no_fontc:
        fonts += build_with_fontc(args.corpus.resolve(), out, args.fresh)
    if args.limit:
        fonts = fonts[: args.limit]

    def work(pair):
        label, font = pair
        s = slug(label)
        fb_font = font
        if font.suffix.lower() in {".woff", ".woff2"}:
            fb_font = unwrap(args.fonttools_python, font, out / "unwrapped")
        fbp = run_fb(args.fontbakery, profile, fb_font, out / "fb" / f"{s}.json", args.fresh)
        fcp = run_fc(args.cli, font, out / "fc" / f"{s}.json", args.fresh)
        return label, font, fbp, fcp

    with cf.ThreadPoolExecutor(max_workers=max(1, args.jobs)) as pool:
        runs = list(pool.map(work, fonts))

    per_check: dict[str, Counter] = defaultdict(Counter)
    severity_mismatch: Counter = Counter()
    disagreements = []
    vendor_only: Counter = Counter()
    fc_run_errors, fb_run_errors = [], []
    for label, font, fbp, fcp in runs:
        rel = str(label.relative_to(args.corpus.resolve())) if label.is_relative_to(args.corpus.resolve()) else str(label)
        fbr = fb_results(fbp)
        fcr, fc_err = fc_results(fcp)
        if fc_err:
            fc_run_errors.append({"font": rel, "error": fc_err[-400:]})
        if not fbr:
            fb_run_errors.append(rel)
        for fb_check in compared:
            ours = {c: for_upstream(fb_check, c, fcr[c]) for c in mapping[fb_check] if c in fcr}
            for c in mapping[fb_check]:
                if c not in fcr and c in NETWORK_CHECKS and not fc_err:
                    # Not run without --network, as FontBakery's network checks under --skip-network.
                    ours[c] = {"status": "skipped", "codes": []}
            if not ours and all(c.startswith("check.vendor.") for c in mapping[fb_check]):
                vendor_only[fb_check] += 1
                continue
            fbv = fb_verdict(fbr.get(fb_check, {}).get("status") if fbr else None)
            fcv = fc_verdict([o["status"] for o in ours.values()]) if not fc_err else "error"
            if fbv == "absent" and fcv == "absent":
                continue
            if fbv == fcv:
                per_check[fb_check]["agree-" + fbv] += 1
                if fbv == "flagged":
                    fb_sev = fbr[fb_check]["status"].replace("FATAL", "FAIL").lower()
                    fc_sev = "fail" if any(o["status"] == "fail" for o in ours.values()) else "warn"
                    if fb_sev != fc_sev:
                        severity_mismatch[(fb_check, f"{fb_sev}->{fc_sev}")] += 1
                continue
            if "error" in (fbv, fcv) or "absent" in (fbv, fcv):
                direction = "error"
            elif fbv == "flagged":
                direction = "fb"
            else:
                direction = "fc"
            cls, why = classify(fb_check, direction, Path(rel), fbr.get(fb_check, {}), ours)
            per_check[fb_check][cls] += 1
            disagreements.append({
                "font": rel, "fb_check": fb_check, "direction": direction, "class": cls, "reason": why,
                "fb": fbr.get(fb_check), "fc": ours,
            })

    (out / "disagreements.json").write_text(json.dumps(disagreements, indent=1), encoding="utf-8")
    write_report(out, version, len(runs), compared, missing_upstream, mapping, per_check,
                 disagreements, severity_mismatch, fc_run_errors, fb_run_errors, vendor_only)
    totals = Counter()
    for c in per_check.values():
        totals.update(c)
    print(f"FontBakery {version}; {len(runs)} fonts; {len(compared)} checks compared")
    print("  " + ", ".join(f"{k} {v}" for k, v in sorted(totals.items())))
    print(f"report: {out / 'report.md'}")
    return 0


def write_report(out, version, n_fonts, compared, missing_upstream, mapping, per_check,
                 disagreements, severity_mismatch, fc_run_errors, fb_run_errors, vendor_only) -> None:
    cols = ["agree-flagged", "agree-clean", "agree-error", "purpose", "data", "ours"]
    lines = [
        "# FontBakery verdicts against the FontCraft audit",
        "",
        f"FontBakery {version}; {n_fonts} fonts; {len(compared)} FontBakery checks compared "
        f"(mapped from the inventory; {len(missing_upstream)} mapped ids are not FontBakery checks: "
        f"{', '.join(missing_upstream)}).",
        "",
        "Mapped checks not run:",
        "",
    ]
    lines += [f"- `{c}`: {why}" for c, why in sorted(NOT_RUN.items())]
    lines += [f"- `{c}`: mapped only to a `check.vendor.*` summary row, not built in (21-qa.md § 9)"
              for c in sorted(vendor_only)] + [""]
    totals = Counter()
    for c in per_check.values():
        totals.update(c)
    lines += ["## Totals", "", "| " + " | ".join(cols) + " |", "|" + "---|" * len(cols),
              "| " + " | ".join(str(totals[c]) for c in cols) + " |", ""]
    lines += ["## Per check", "", "| FontBakery check | FontCraft | " + " | ".join(cols) + " |",
              "|---|---|" + "---|" * len(cols)]
    for fb_check in compared:
        if fb_check in vendor_only:
            continue
        c = per_check.get(fb_check, Counter())
        lines.append(f"| `{fb_check}` | {', '.join(x.removeprefix('check.') for x in mapping[fb_check])} | "
                     + " | ".join(str(c[k]) for k in cols) + " |")
    lines += ["", "## Disagreements by class and reason", ""]
    grouped: dict[tuple, list] = defaultdict(list)
    for d in disagreements:
        grouped[(d["class"], d["fb_check"], d["direction"], d["reason"])].append(d)
    for (cls, fb_check, direction, why), items in sorted(grouped.items()):
        lines.append(f"- **{cls}** `{fb_check}` ({direction}, {len(items)}): {why}")
        for d in items[:4]:
            fb = d["fb"] or {}
            fc = {k.removeprefix("check."): (v["status"], v["codes"][:3]) for k, v in d["fc"].items()}
            lines.append(f"  - {d['font']}: fb {fb.get('status')} {fb.get('codes', [])[:3]}; fc {fc}")
    if severity_mismatch:
        lines += ["", "## Severity differences where both flag", ""]
        for (fb_check, change), n in sorted(severity_mismatch.items()):
            lines.append(f"- `{fb_check}` {change}: {n}")
    if fc_run_errors:
        lines += ["", "## FontCraft run errors", ""]
        lines += [f"- {e['font']}: {e['error']}" for e in fc_run_errors]
    if fb_run_errors:
        lines += ["", "## Fonts FontBakery could not report on", ""]
        lines += [f"- {f}" for f in fb_run_errors]
    (out / "report.md").write_text("\n".join(lines) + "\n", encoding="utf-8")


if __name__ == "__main__":
    sys.exit(main())
