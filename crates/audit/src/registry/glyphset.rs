//! Registry entries of the `check.glyphset.*` checks and `check.outlines.contourCount` (M12.47).
//! Home profiles and fixes follow `docs/spec/21-qa.md` § 8 (Glyph set, names and encoding).

use crate::Precondition::{ProjectValue, Static, Table, TrueTypeOutlines};
use crate::Profile::{Opentype, Strict, Universal};
use crate::Severity::{Fail, Warn};
use crate::checks::glyphset as g;
use crate::{CheckGroup, CheckScope, CheckSpec, FixSpec, Runner};

/// The fields every glyph-set check of a compiled font shares.
const GLYPHSET: CheckSpec = CheckSpec {
    id: "",
    label: "",
    description: "",
    group: CheckGroup::Glyphset,
    scope: CheckScope::Binary,
    family: None,
    severity: Fail,
    profile: Universal,
    thresholds: &[],
    fix: None,
    skip_variable: false,
    capped: false,
    codes: &[],
    code_severities: &[],
    preconditions: &[],
    run: Runner::Binary(g::notdef),
};

const fn fix(id: &'static str, command: &'static str, safe: bool, description: &'static str) -> Option<FixSpec> {
    Some(FixSpec { id, command: Some(command), safe, description })
}

const ADD_MANDATORY: Option<FixSpec> = fix(
    "addMandatoryGlyphs",
    "glyph.addGlyphs",
    true,
    "Add the missing mandatory glyphs (.notdef with a drawing, space, no-break space) and put them in the required order.",
);
const REMOVE_CODEPOINTS: Option<FixSpec> = fix("removeCodepoints", "glyph.setUnicodes", true, "Remove the reported code points from their glyphs.");

pub(super) const CHECKS: &[CheckSpec] = &[
    CheckSpec {
        id: "check.outlines.contourCount",
        label: "Unusual contour count",
        description: "An encoded glyph of a static TrueType font has a number of contours that well-made fonts do not use for its character; no contours where some are expected is a failure.",
        group: CheckGroup::Outlines,
        capped: true,
        codes: &["contour-count", "no-contour", "lacks-cmap"],
        code_severities: &[("contour-count", Warn)],
        preconditions: &[TrueTypeOutlines, Static],
        run: Runner::Binary(g::contour_count),
        ..GLYPHSET
    },
    CheckSpec {
        id: "check.glyphset.notdef",
        label: ".notdef glyph",
        description: "The .notdef glyph is missing, is not the first glyph, has a code point (warnings) or is blank (failure).",
        fix: ADD_MANDATORY,
        codes: &["notdef-not-found", "notdef-not-first", "notdef-has-codepoint", "notdef-is-blank"],
        code_severities: &[("notdef-not-found", Warn), ("notdef-not-first", Warn), ("notdef-has-codepoint", Warn)],
        run: Runner::Binary(g::notdef),
        ..GLYPHSET
    },
    CheckSpec {
        id: "check.glyphset.whitespaceGlyphs",
        label: "Space glyphs",
        description: "There is no glyph for U+0020 SPACE or U+00A0 NO-BREAK SPACE.",
        fix: ADD_MANDATORY,
        codes: &["missing-whitespace-glyph"],
        run: Runner::Binary(g::whitespace_glyphs),
        ..GLYPHSET
    },
    CheckSpec {
        id: "check.glyphset.whitespaceInk",
        label: "Ink in whitespace glyphs",
        description: "A glyph mapped from a whitespace or invisible character has an outline; U+1680 OGHAM SPACE MARK is meant to draw.",
        fix: fix("clearGlyph", "edit.delete", false, "Remove all contours and components from the reported glyphs."),
        codes: &["has-ink"],
        run: Runner::Binary(g::whitespace_ink),
        ..GLYPHSET
    },
    CheckSpec {
        id: "check.glyphset.controlCharacters",
        label: "Control characters",
        description: "A TrueType font maps control characters U+0001 to U+001F other than U+000D, or has glyphs named after them.",
        fix: REMOVE_CODEPOINTS,
        codes: &["unacceptable"],
        preconditions: &[TrueTypeOutlines],
        run: Runner::Binary(g::control_characters),
        ..GLYPHSET
    },
    CheckSpec {
        id: "check.glyphset.truetypeLeadingGlyphs",
        label: "First glyphs of a TrueType font",
        description: "The first four glyphs of a TrueType font are not .notdef, NULL, CR and space, or the last three are not mapped from U+0000, U+000D and U+0020; the names .null and nonmarkingreturn and an OS/2 default character other than 0 give warnings.",
        profile: Strict,
        fix: ADD_MANDATORY,
        codes: &["wrong-glyph-order", "legacy-name", "wrong-encoding", "not-encoded", "default-char-not-zero"],
        code_severities: &[("legacy-name", Warn), ("default-char-not-zero", Warn)],
        run: Runner::Binary(g::truetype_leading_glyphs),
        ..GLYPHSET
    },
    CheckSpec {
        id: "check.glyphset.uniqueGlyphNames",
        label: "Duplicate glyph names",
        description: "Two glyphs share one name, which stops the font from installing on some systems.",
        codes: &["duplicated-glyph-names"],
        run: Runner::Binary(g::unique_glyph_names),
        ..GLYPHSET
    },
    CheckSpec {
        id: "check.glyphset.validGlyphNames",
        label: "Glyph name characters",
        description: "A glyph name uses characters other than A-Z, a-z, 0-9, period and underscore, starts with a digit or period, or is longer than 63 characters; names of 32 to 63 characters give warnings.",
        capped: true,
        codes: &["found-invalid-names", "legacy-long-names"],
        code_severities: &[("legacy-long-names", Warn)],
        run: Runner::Binary(g::valid_glyph_names),
        ..GLYPHSET
    },
    CheckSpec {
        id: "check.glyphset.glyphNameBufferSize",
        label: "Total glyph name size",
        description: "The glyph names of a CFF font together exceed 32,767 bytes, which crashed old Mac OS X versions; coming close gives a warning.",
        profile: Strict,
        codes: &["buffer-too-large", "buffer-near-limit"],
        code_severities: &[("buffer-near-limit", Warn)],
        preconditions: &[Table("CFF ")],
        run: Runner::Binary(g::glyph_name_buffer_size),
        ..GLYPHSET
    },
    CheckSpec {
        id: "check.glyphset.productionNames",
        label: "Production glyph names",
        description: "A glyph name, or a part of a ligature name, is a working name where the glyph database gives another final name, or cannot be mapped back to Unicode.",
        severity: Warn,
        profile: Strict,
        fix: fix("useProductionNames", "glyph.setProductionName", true, "Rename the glyphs to their production names on export."),
        capped: true,
        codes: &["working-name", "unmappable-name"],
        run: Runner::Binary(g::production_names),
        ..GLYPHSET
    },
    CheckSpec {
        id: "check.glyphset.unreachableGlyphs",
        label: "Unreachable glyphs",
        description: "A glyph has no code point, no substitution produces it, and no composite, COLR, MATH, JSTF or variation-sequence entry refers to it.",
        severity: Warn,
        fix: fix("removeGlyphs", "glyph.setExport", false, "Mark the reported glyphs as not exporting."),
        capped: true,
        codes: &["unreachable-glyphs"],
        run: Runner::Binary(g::unreachable_glyphs),
        ..GLYPHSET
    },
    CheckSpec {
        id: "check.glyphset.caseMapping",
        label: "Case partners",
        description: "A letter with a Unicode case mapping is present without its upper- or lowercase partner (a short list of often-missing partners is skipped).",
        capped: true,
        codes: &["missing-case-counterparts"],
        run: Runner::Binary(g::case_mapping),
        ..GLYPHSET
    },
    CheckSpec {
        id: "check.glyphset.emptyLetters",
        label: "Empty letters",
        description: "An encoded letter has no ink. The Hangul fillers are exempt; empty Hangul syllables outside KS X 1001 give a warning.",
        capped: true,
        codes: &["empty-letter", "empty-hangul-letter"],
        code_severities: &[("empty-hangul-letter", Warn)],
        run: Runner::Binary(g::empty_letters),
        ..GLYPHSET
    },
    CheckSpec {
        id: "check.glyphset.softHyphen",
        label: "Soft hyphen",
        description: "The font maps U+00AD SOFT HYPHEN; text engines draw the hyphen there themselves.",
        severity: Warn,
        fix: REMOVE_CODEPOINTS,
        codes: &["softhyphen"],
        run: Runner::Binary(g::soft_hyphen),
        ..GLYPHSET
    },
    CheckSpec {
        id: "check.glyphset.rupee",
        label: "Indian rupee sign",
        description: "A font that supports an Indic script lacks U+20B9 INDIAN RUPEE SIGN.",
        codes: &["missing-rupee"],
        run: Runner::Binary(g::rupee),
        ..GLYPHSET
    },
    CheckSpec {
        id: "check.glyphset.cjkGlyphCount",
        label: "Too few CJK glyphs",
        description: "The font claims CJK support in OS/2 but maps fewer than 150 CJK characters, so some are probably mis-encoded.",
        severity: Warn,
        codes: &["cjk-not-enough-glyphs"],
        preconditions: &[Table("OS/2")],
        run: Runner::Binary(g::cjk_glyph_count),
        ..GLYPHSET
    },
    CheckSpec {
        id: "check.glyphset.arabicHighHamza",
        label: "Arabic high hamza",
        description: "U+0674 ARABIC LETTER HIGH HAMZA is a mark in GDEF (failure) or is not drawn at the size of the letter hamza U+0621 (warning).",
        codes: &["mark-in-gdef", "glyph-area"],
        code_severities: &[("glyph-area", Warn)],
        run: Runner::Binary(g::arabic_high_hamza),
        ..GLYPHSET
    },
    CheckSpec {
        id: "check.glyphset.arabicSpacingSymbols",
        label: "Arabic spacing symbols",
        description: "One of the spacing Arabic symbols U+FBB2 to U+FBC2 is a mark in GDEF.",
        fix: fix("setGdefClasses", "glyph.setCategory", true, "Class the reported glyphs as base glyphs."),
        codes: &["mark-in-gdef"],
        preconditions: &[Table("GDEF")],
        run: Runner::Binary(g::arabic_spacing_symbols),
        ..GLYPHSET
    },
    CheckSpec {
        id: "check.glyphset.unicodeCmapMissing",
        label: "Unicode cmap",
        description: "The font has no cmap table, or no Unicode subtable (platform 0 encoding 3, or platform 3 encoding 1).",
        profile: Opentype,
        codes: &["no-cmap", "no-unicode-cmap"],
        run: Runner::Binary(g::unicode_cmap_missing),
        ..GLYPHSET
    },
    CheckSpec {
        id: "check.glyphset.doubleEncodedGlyphs",
        label: "Glyphs with several code points",
        description: "One glyph is mapped from more than one code point, so text extracted from documents is ambiguous.",
        severity: Warn,
        profile: Strict,
        capped: true,
        codes: &["double-mapped"],
        run: Runner::Binary(g::double_encoded_glyphs),
        ..GLYPHSET
    },
    CheckSpec {
        id: "check.glyphset.privateUseCodepoints",
        label: "Private use code points",
        description: "Glyphs are mapped from the private use areas (warning) or from surrogate code points (failure).",
        profile: Strict,
        capped: true,
        codes: &["pua-encoded", "surrogate-encoded"],
        code_severities: &[("pua-encoded", Warn)],
        run: Runner::Binary(g::private_use_codepoints),
        ..GLYPHSET
    },
    CheckSpec {
        id: "check.glyphset.minimumCoverage",
        label: "Minimum coverage",
        description: "The font does not map every character of the project's reference glyph set.",
        profile: Strict,
        codes: &["missing-codepoints"],
        preconditions: &[ProjectValue("glyphSet")],
        run: Runner::Binary(g::minimum_coverage),
        ..GLYPHSET
    },
    CheckSpec {
        id: "check.glyphset.rendersOwnName",
        label: "Family name renders",
        description: "The font cannot show its own family name (name ID 1) with its own glyphs.",
        profile: Strict,
        codes: &["render-own-name"],
        preconditions: &[Table("name")],
        run: Runner::Binary(g::renders_own_name),
        ..GLYPHSET
    },
    CheckSpec {
        id: "check.glyphset.duplicateCodepoint",
        label: "Duplicate code points",
        description: "Two exporting glyphs of the source claim the same code point, so only one can be reached.",
        scope: CheckScope::Source,
        codes: &["duplicate-codepoint"],
        run: Runner::Source(g::duplicate_codepoint),
        ..GLYPHSET
    },
    CheckSpec {
        id: "check.glyphset.nameCodepointMismatch",
        label: "Code point and name disagree",
        description: "An exporting glyph's code points do not include the one its name implies (a uniXXXX or uXXXXX name, or a name the glyph database knows).",
        severity: Warn,
        scope: CheckScope::Source,
        fix: fix("updateGlyphInfo", "glyph.updateGlyphInfo", false, "Reset the glyph's code point from its name."),
        codes: &["codepoint-mismatch"],
        run: Runner::Source(g::name_codepoint_mismatch),
        ..GLYPHSET
    },
];
