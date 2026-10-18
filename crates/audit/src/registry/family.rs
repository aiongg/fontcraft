//! Registry entries of the `check.family.*` checks (M12.58). Every one reads two or more compiled
//! fonts; the runners are in `checks/family.rs`, which also says which grouping each one uses.

use crate::CheckGroup::Family;
use crate::CheckScope;
use crate::Precondition::ProjectValue;
use crate::Profile::{Opentype, Strict, Universal};
use crate::Severity::{Fail, Info, Warn};
use crate::checks::family as f;
use crate::{CheckSpec, FixSpec, Runner, Threshold};

/// The fields every family check shares; each entry sets the rest and takes these with `..BASE`.
const BASE: CheckSpec = CheckSpec {
    id: "",
    label: "",
    description: "",
    group: Family,
    scope: CheckScope::Family,
    family: None,
    severity: Fail,
    profile: Strict,
    thresholds: &[],
    fix: None,
    skip_variable: false,
    capped: false,
    codes: &[],
    code_severities: &[],
    preconditions: &[],
    run: Runner::Family(f::max_four_fonts_per_family_name),
};

/// The Font Info fix of `docs/spec/21-qa.md` § 5; report-only until the Font Info commands exist.
const SYNC_FAMILY_VALUES: Option<FixSpec> = Some(FixSpec {
    id: "syncFamilyValues",
    command: Some("font.info.syncFamilyValues"),
    safe: false,
    description: "Copy the regular face's value to every font of the family.",
});

/// Turns on the stricter rules of the legacy family comparison.
const STRICT: &[Threshold] = &[Threshold {
    name: "strict",
    default: 0.0,
    min: 0.0,
    max: 1.0,
    doc: "1 adds the stricter rules of the legacy family comparison; 0 runs the basic rule only.",
}];

pub(super) const CHECKS: &[CheckSpec] = &[
    CheckSpec {
        id: "check.family.maxFourFontsPerFamilyName",
        label: "At most four fonts per family name",
        description: "More than four fonts share one family name (name ID 1); a style-linked group holds regular, italic, bold and bold italic at most.",
        profile: Opentype,
        codes: &["too-many"],
        run: Runner::Family(f::max_four_fonts_per_family_name),
        ..BASE
    },
    CheckSpec {
        id: "check.family.uniqueStyleLinking",
        label: "Unique style linking",
        description: "Two fonts with the same family name (ID 1) have the same bold and italic bits, so applications cannot tell them apart.",
        profile: Opentype,
        codes: &["same-style-bits"],
        run: Runner::Family(f::unique_style_linking),
        ..BASE
    },
    CheckSpec {
        id: "check.family.consistentFamilyName",
        label: "Consistent family name",
        description: "The fonts do not agree on the typographic family name (ID 16, or ID 1 when 16 is absent). With `strict`, ID 16 is also compared in every Windows language within each style-linked group.",
        profile: Opentype,
        thresholds: STRICT,
        codes: &["inconsistent-family-name", "differs-in-language"],
        run: Runner::Family(f::consistent_family_name),
        ..BASE
    },
    CheckSpec {
        id: "check.family.equalVersions",
        label: "Equal font versions",
        description: "The fonts of a family carry different font revisions (head table).",
        severity: Warn,
        profile: Opentype,
        codes: &["mismatch"],
        run: Runner::Family(f::equal_versions),
        ..BASE
    },
    CheckSpec {
        id: "check.family.panoseFamilyType",
        label: "PANOSE family type",
        description: "The fonts of a family differ in PANOSE family type (the first digit), or a font has no OS/2 table. With `strict`, a PANOSE of all zeros fails and one with only the first four digits set warns.",
        profile: Opentype,
        thresholds: STRICT,
        codes: &["lacks-os2", "inconsistency", "all-zero", "first-four-only"],
        code_severities: &[("inconsistency", Warn), ("first-four-only", Warn)],
        run: Runner::Family(f::panose_family_type),
        ..BASE
    },
    CheckSpec {
        id: "check.family.underlineConsistency",
        label: "Underline consistency",
        description: "The underline position or thickness (post table) differs between the fonts of a family, so underlines jump where styles are mixed on a line.",
        profile: Opentype,
        fix: SYNC_FAMILY_VALUES,
        codes: &["position-differs", "thickness-differs"],
        run: Runner::Family(f::underline_consistency),
        ..BASE
    },
    CheckSpec {
        id: "check.family.verticalMetrics",
        label: "Family vertical metrics",
        description: "The fonts of a family differ in a vertical metric: typo ascender, descender or line gap, win ascent or descent, or hhea ascender, descender or line gap.",
        profile: Universal,
        fix: SYNC_FAMILY_VALUES,
        codes: &["lacks-os2", "lacks-hhea", "mismatch"],
        run: Runner::Family(f::vertical_metrics),
        ..BASE
    },
    CheckSpec {
        id: "check.family.superfamilyVerticalMetrics",
        label: "Super-family vertical metrics",
        description: "The sibling families given together (fonts with different typographic family names) differ in a vertical metric. Lists the families found.",
        severity: Warn,
        codes: &["superfamily-vertical-metrics", "families"],
        code_severities: &[("families", Info)],
        run: Runner::Family(f::superfamily_vertical_metrics),
        ..BASE
    },
    CheckSpec {
        id: "check.family.unitsPerEm",
        label: "Family units per em",
        description: "The fonts of a family do not all use the same units per em.",
        profile: Universal,
        codes: &["inconsistent-upem"],
        run: Runner::Family(f::units_per_em),
        ..BASE
    },
    CheckSpec {
        id: "check.family.axisRanges",
        label: "Family axis ranges",
        description: "The variable fonts of a family differ in their axes, or in an axis's minimum, default or maximum.",
        profile: Opentype,
        codes: &["missing-axis", "axis-range-mismatch"],
        run: Runner::Family(f::axis_ranges),
        ..BASE
    },
    CheckSpec {
        id: "check.family.nameRecordSets",
        label: "Same name records in a style-linked group",
        description: "The fonts of a style-linked group do not carry the same set of name records (platform, encoding, language, name ID); IDs 16, 17, 18, 20, 21, 22 and those above 255 are exempt.",
        codes: &["differs"],
        run: Runner::Family(f::name_record_sets),
        ..BASE
    },
    CheckSpec {
        id: "check.family.styleLinkedGroupShape",
        label: "Style-linked group has a regular font",
        description: "A style-linked group has no regular or bold font to act as its base, or its base is the bold font although the group has more than two members.",
        profile: Universal,
        codes: &["no-base-font", "base-not-regular"],
        run: Runner::Family(f::style_linked_group_shape),
        ..BASE
    },
    CheckSpec {
        id: "check.family.boldLinkWeights",
        label: "Bold link weights",
        description: "Style linking would produce synthetic bolding: the regular weight class is outside 250 to 599, the bold weight is outside the range that regular weight allows, a bold font is not heavier than the regular one, or the group uses more than two weight classes. A bold weight that is not a multiple of 50 warns.",
        profile: Universal,
        codes: &[
            "regular-weight-out-of-range",
            "bold-weight-out-of-range",
            "bold-weight-not-round",
            "too-many-weights",
            "bold-same-weight",
            "bold-lighter",
            "regular-heavier",
        ],
        code_severities: &[("bold-weight-not-round", Warn)],
        run: Runner::Family(f::bold_link_weights),
        ..BASE
    },
    CheckSpec {
        id: "check.family.widthClass",
        label: "Width class in a style-linked group",
        description: "The fonts of a style-linked group differ in OS/2 width class.",
        severity: Warn,
        codes: &["differs"],
        run: Runner::Family(f::width_class),
        ..BASE
    },
    CheckSpec {
        id: "check.family.copyrightAndTrademark",
        label: "Copyright and trademark",
        description: "The copyright or trademark strings differ between the fonts of a family; when only the years differ this is a note.",
        severity: Warn,
        codes: &["copyright-differs", "copyright-years-only", "trademark-differs", "trademark-years-only"],
        code_severities: &[("copyright-years-only", Info), ("trademark-years-only", Info)],
        run: Runner::Family(f::copyright_and_trademark),
        ..BASE
    },
    CheckSpec {
        id: "check.family.macWindowsMenuNames",
        label: "Macintosh and Windows menu names",
        description: "For fonts that keep Macintosh names: the Macintosh and Windows family names of a bold or italic member of a style-linked group are the same, or those of the regular member differ.",
        codes: &["bold-italic-names-equal", "regular-names-differ"],
        preconditions: &[ProjectValue("keepMacNames")],
        run: Runner::Family(f::mac_windows_menu_names),
        ..BASE
    },
    CheckSpec {
        id: "check.family.layoutConsistency",
        label: "Layout consistency",
        description: "The fonts of a family do not have the same scripts, languages and features in GSUB and GPOS, or a feature uses different lookups in one font. Expected when the character sets differ.",
        codes: &["differs"],
        run: Runner::Family(f::layout_consistency),
        ..BASE
    },
    CheckSpec {
        id: "check.family.sizeFeatureGroups",
        label: "Optical size groups",
        description: "The optical-size data (the size feature) of a family is inconsistent: fonts with the same subfamily identifier have different menu names, size groups have different member counts (warning), or the design ranges of a group leave gaps or overlap.",
        codes: &["menu-names-differ", "group-sizes-differ", "ranges-not-contiguous"],
        code_severities: &[("group-sizes-differ", Warn)],
        run: Runner::Family(f::size_feature_groups),
        ..BASE
    },
    CheckSpec {
        id: "check.family.wwsBit",
        label: "WWS bit",
        description: "The WWS bit (OS/2 fsSelection bit 8) is set in some fonts of a family but not all, is set with an OS/2 version below 4, or is set while two fonts share the same weight, width and italic values.",
        codes: &["mixed", "old-os2-version", "duplicate-style"],
        run: Runner::Family(f::wws_bit),
        ..BASE
    },
    CheckSpec {
        id: "check.family.embeddingPermissions",
        label: "Embedding permissions",
        description: "The fonts of a family differ in OS/2 fsType.",
        codes: &["differs"],
        run: Runner::Family(f::embedding_permissions),
        ..BASE
    },
    CheckSpec {
        id: "check.family.glyphWidthRatio",
        label: "Glyph width ratio",
        description: "A glyph is more than three times as wide in one font of a family as in another (warning); a glyph of the compared glyph set has no width in one of the fonts.",
        codes: &["ratio", "missing-width"],
        code_severities: &[("ratio", Warn)],
        capped: true,
        run: Runner::Family(f::glyph_width_ratio),
        ..BASE
    },
    CheckSpec {
        id: "check.family.blueValueArraySizes",
        label: "Blue value array sizes",
        description: "Within the fonts of a style-linked group that share a glyph set, BlueValues or OtherBlues have a different number of entries.",
        codes: &["blue-values-count-differs", "other-blues-count-differs"],
        run: Runner::Family(f::blue_value_array_sizes),
        ..BASE
    },
    CheckSpec {
        id: "check.family.familyBlues",
        label: "Family blues",
        description: "FamilyBlues or FamilyOtherBlues are unusable: absent although the BlueValues differ (warning), not equal to the BlueValues of any font that uses them (warning), unique to one font, or over the zone limits.",
        codes: &["no-family-blues", "no-base-font", "unique", "too-many-zones", "odd-count"],
        code_severities: &[("no-family-blues", Warn), ("no-base-font", Warn)],
        run: Runner::Family(f::family_blues),
        ..BASE
    },
    CheckSpec {
        id: "check.family.blueScaleAndShift",
        label: "BlueScale and BlueShift",
        description: "BlueScale or BlueShift differs between the fonts of a style-linked group.",
        severity: Info,
        codes: &["blue-scale-differs", "blue-shift-differs"],
        run: Runner::Family(f::blue_scale_and_shift),
        ..BASE
    },
    CheckSpec {
        id: "check.family.codepointCoverage",
        label: "Code point coverage",
        description: "The fonts of a family do not encode the same characters; the upright fonts, or the italic ones, differ in glyph count (warning).",
        codes: &["glyphset-diverges", "different-glyph-count"],
        code_severities: &[("different-glyph-count", Warn)],
        run: Runner::Family(f::codepoint_coverage),
        ..BASE
    },
    CheckSpec {
        id: "check.family.italicsHaveUprights",
        label: "Italics have uprights",
        description: "An italic font has no upright partner with the same weight and width class in its family.",
        codes: &["missing-upright"],
        run: Runner::Family(f::italics_have_uprights),
        ..BASE
    },
    CheckSpec {
        id: "check.family.tabularFigureWidths",
        label: "Tabular figure widths",
        description: "Tabular glyphs do not have the same advance in every font of the family, so numeric columns shift when the style changes; a half-width tabular set is reported as information.",
        codes: &["inconsistent-widths", "half-widths"],
        code_severities: &[("half-widths", Info)],
        run: Runner::Family(f::tabular_figure_widths),
        ..BASE
    },
    CheckSpec {
        id: "check.family.duplicateFullNames",
        label: "Duplicate full names",
        description: "Two fonts have the same full name (ID 4) or the same PostScript name (ID 6).",
        codes: &["duplicate-full-names", "duplicate-postscript-names"],
        run: Runner::Family(f::duplicate_full_names),
        ..BASE
    },
];
