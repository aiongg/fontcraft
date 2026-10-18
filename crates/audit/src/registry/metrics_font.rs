//! Registry entries of the font-wide `check.metrics.*` checks (M12.48).

use crate::CheckGroup::Metrics;
use crate::CheckScope::Binary;
use crate::Precondition::Table;
use crate::Profile::{Opentype, Strict, Universal};
use crate::Severity::{Fail, Info, Warn};
use crate::checks::metrics_font as m;
use crate::{CheckSpec, FixSpec, Runner};

/// The fields every entry here shares; each entry sets the rest.
const BINARY: CheckSpec = CheckSpec {
    id: "",
    label: "",
    description: "",
    group: Metrics,
    scope: Binary,
    family: None,
    severity: Fail,
    profile: Opentype,
    thresholds: &[],
    fix: None,
    skip_variable: false,
    capped: false,
    codes: &[],
    code_severities: &[],
    preconditions: &[],
    run: Runner::Binary(no_check),
};

fn no_check(_: &crate::BinaryCtx<'_>, _: &crate::Params<'_>, _: &mut crate::Sink) {}

const fn patch(safe: bool, description: &'static str) -> Option<FixSpec> {
    Some(FixSpec { id: "patchTable", command: Some("tables.recalculate"), safe, description })
}

const SYNC_VERTICAL: Option<FixSpec> = Some(FixSpec {
    id: "syncVerticalMetrics",
    command: Some("font.info.syncVerticalMetrics"),
    safe: false,
    description: "Recompute the typo, hhea and win vertical metrics from one scheme.",
});

/// The font-wide metrics checks, in the order of the inventory.
pub(super) const CHECKS: &[CheckSpec] = &[
    CheckSpec {
        id: "check.metrics.unitsPerEm",
        label: "Units per em",
        description: "head.unitsPerEm is outside 16 to 16384, or is neither a power of two, 1000 nor 2000.",
        severity: Fail,
        profile: Opentype,
        codes: &["out-of-range", "suboptimal"],
        code_severities: &[("suboptimal", Warn)],
        preconditions: &[Table("head")],
        run: Runner::Binary(m::units_per_em),
        ..BINARY
    },
    CheckSpec {
        id: "check.metrics.italicAngle",
        label: "Italic angle",
        description: "post.italicAngle is implausible: over 90, 30 or 20 degrees, of the wrong sign for the measured lean, non-zero in an upright style or zero in an italic one.",
        severity: Fail,
        profile: Opentype,
        codes: &["positive", "negative", "over-90-degrees", "over-30-degrees", "over-20-degrees", "zero-italic", "non-zero-upright", "empty-glyphs"],
        code_severities: &[("positive", Warn), ("negative", Warn), ("over-30-degrees", Warn), ("over-20-degrees", Warn), ("empty-glyphs", Warn)],
        preconditions: &[Table("post")],
        run: Runner::Binary(m::italic_angle),
        ..BINARY
    },
    CheckSpec {
        id: "check.metrics.italicBitVsAngle",
        label: "Italic bit and angle",
        description: "The italic bit of head.macStyle is set while the italic angle is 4 degrees or less, or clear while it is more.",
        severity: Warn,
        profile: Strict,
        codes: &["bit-without-angle", "angle-without-bit"],
        preconditions: &[Table("head"), Table("post")],
        run: Runner::Binary(m::italic_bit_vs_angle),
        ..BINARY
    },
    CheckSpec {
        id: "check.metrics.caretSlope",
        label: "Caret slope",
        description: "hhea.caretSlopeRise and caretSlopeRun describe an angle more than 0.1 degrees away from post.italicAngle, or the rise is 0.",
        severity: Fail,
        profile: Opentype,
        fix: patch(true, "Set rise 1 and run 0 for upright fonts, rise = unitsPerEm and the matching run for slanted ones."),
        codes: &["zero-rise", "mismatch"],
        code_severities: &[("mismatch", Warn)],
        preconditions: &[Table("hhea"), Table("post"), Table("head")],
        run: Runner::Binary(m::caret_slope),
        ..BINARY
    },
    CheckSpec {
        id: "check.metrics.maxAdvanceWidth",
        label: "Largest advance width",
        description: "hhea.advanceWidthMax differs from the largest advance in hmtx.",
        severity: Fail,
        profile: Opentype,
        fix: patch(true, "Recompute hhea.advanceWidthMax from hmtx."),
        codes: &["mismatch"],
        preconditions: &[Table("hhea"), Table("hmtx")],
        run: Runner::Binary(m::max_advance_width),
        ..BINARY
    },
    CheckSpec {
        id: "check.metrics.xAvgCharWidth",
        label: "Average character width",
        description: "OS/2.xAvgCharWidth differs by more than 1 from the value its table version defines.",
        severity: Warn,
        profile: Opentype,
        fix: patch(true, "Recompute OS/2.xAvgCharWidth for the table version."),
        codes: &["close", "wrong"],
        code_severities: &[("close", Info)],
        preconditions: &[Table("OS/2"), Table("hmtx")],
        run: Runner::Binary(m::x_avg_char_width),
        ..BINARY
    },
    CheckSpec {
        id: "check.metrics.monospaceFlags",
        label: "Monospace markers",
        description: "post.isFixedPitch, PANOSE and the CFF isFixedPitch disagree with the advances.",
        severity: Fail,
        profile: Opentype,
        fix: patch(false, "Set post.isFixedPitch, the PANOSE proportion and the CFF isFixedPitch from the advances."),
        codes: &[
            "number-of-hmetrics",
            "mono-bad-panose",
            "mono-outliers",
            "mono-bad-is-fixed-pitch",
            "bad-is-fixed-pitch",
            "bad-panose",
            "cff-is-fixed-pitch",
        ],
        code_severities: &[("number-of-hmetrics", Warn), ("mono-outliers", Warn)],
        preconditions: &[Table("hhea"), Table("hmtx"), Table("OS/2"), Table("post")],
        run: Runner::Binary(m::monospace_flags),
        ..BINARY
    },
    CheckSpec {
        id: "check.metrics.fontBBox",
        label: "Font bounding box",
        description: "The head box reaches beyond 2 em horizontally or 1.5 em vertically, or differs from the CFF FontBBox.",
        severity: Warn,
        profile: Strict,
        fix: patch(true, "Recompute the head box and the CFF FontBBox from the glyphs."),
        codes: &["improbable", "head-cff-differ"],
        preconditions: &[Table("head")],
        run: Runner::Binary(m::font_bbox),
        ..BINARY
    },
    CheckSpec {
        id: "check.metrics.typoMetricsVsEm",
        label: "Typo metrics and em",
        description: "OS/2.sTypoAscender minus sTypoDescender is not the em size; in a CID-keyed font sTypoLineGap is not the em size.",
        severity: Warn,
        profile: Strict,
        codes: &["typo-height", "cid-line-gap"],
        preconditions: &[Table("OS/2"), Table("head")],
        run: Runner::Binary(m::typo_metrics_vs_em),
        ..BINARY
    },
    CheckSpec {
        id: "check.metrics.hheaMatchesTypo",
        label: "hhea matches typo metrics",
        description: "hhea ascender, descender or line gap differs from the OS/2 typo values. Skipped for CJK fonts.",
        severity: Fail,
        profile: Universal,
        fix: SYNC_VERTICAL,
        codes: &["ascender", "descender", "line-gap"],
        preconditions: &[Table("OS/2"), Table("hhea")],
        run: Runner::Binary(m::hhea_matches_typo),
        ..BINARY
    },
    CheckSpec {
        id: "check.metrics.winAscentDescent",
        label: "Windows ascent and descent",
        description: "OS/2.usWinAscent or usWinDescent is smaller than the glyph extents, which clips glyphs on Windows, or more than twice them. Skipped for CJK fonts.",
        severity: Fail,
        profile: Universal,
        fix: SYNC_VERTICAL,
        codes: &["ascent", "descent"],
        preconditions: &[Table("OS/2"), Table("head")],
        run: Runner::Binary(m::win_ascent_descent),
        ..BINARY
    },
    CheckSpec {
        id: "check.metrics.lineGaps",
        label: "Line gaps",
        description: "hhea.lineGap or OS/2.sTypoLineGap is not 0.",
        severity: Warn,
        profile: Universal,
        fix: SYNC_VERTICAL,
        codes: &["hhea-line-gap", "typo-line-gap"],
        preconditions: &[Table("OS/2"), Table("hhea")],
        run: Runner::Binary(m::line_gaps),
        ..BINARY
    },
    CheckSpec {
        id: "check.metrics.useTypoMetrics",
        label: "Use typo metrics",
        description: "OS/2.fsSelection bit 7 (USE_TYPO_METRICS) is not set (CJK fonts excepted), or is set in an OS/2 table older than version 4.",
        severity: Fail,
        profile: Strict,
        fix: patch(false, "Set fsSelection bit 7 and raise the OS/2 version to 4."),
        codes: &["not-set", "set-below-version-4"],
        preconditions: &[Table("OS/2")],
        run: Runner::Binary(m::use_typo_metrics),
        ..BINARY
    },
    CheckSpec {
        id: "check.metrics.typoAscenderVsAgrave",
        label: "Typo ascender and Agrave",
        description: "OS/2.sTypoAscender is lower than the top of Agrave, so macOS falls back to a taller line height.",
        severity: Warn,
        profile: Universal,
        codes: &["below-agrave"],
        preconditions: &[Table("OS/2")],
        run: Runner::Binary(m::typo_ascender_vs_agrave),
        ..BINARY
    },
    CheckSpec {
        id: "check.metrics.capsVerticallyCentred",
        label: "Capitals centred vertically",
        description: "The capitals are not centred between hhea.ascender and hhea.descender within 18% of 1.2 em.",
        severity: Warn,
        profile: Universal,
        codes: &["not-centred"],
        preconditions: &[Table("hhea"), Table("head")],
        run: Runner::Binary(m::caps_vertically_centred),
        ..BINARY
    },
    CheckSpec {
        id: "check.metrics.verticalMetricsScheme",
        label: "Vertical metrics scheme",
        description: "The vertical metrics do not follow the house scheme chosen by `scheme`: Google Fonts (its CJK scheme for CJK fonts), Microsoft or Type Network.",
        severity: Fail,
        profile: Strict,
        thresholds: &[m::SCHEME],
        codes: &[
            "typo-line-gap",
            "hhea-line-gap",
            "hhea-range",
            "hhea-range-high",
            "typo-ascender",
            "hhea-ascender",
            "win-ascent",
            "typo-descender",
            "hhea-descender",
            "cjk-use-typo-metrics",
            "cjk-typo-ascender",
            "cjk-typo-descender",
            "cjk-hhea-range",
            "hhea-win-ascent",
            "hhea-win-descent",
            "hhea-typo-ascender",
            "hhea-typo-descender",
            "hhea-typo-line-gap",
            "use-typo-metrics-recommended",
            "use-typo-metrics-off",
            "typo-hhea-sum",
            "uncentred",
        ],
        code_severities: &[("hhea-range-high", Warn), ("cjk-hhea-range", Warn), ("use-typo-metrics-recommended", Warn)],
        preconditions: &[Table("OS/2"), Table("hhea"), Table("head")],
        run: Runner::Binary(m::vertical_metrics_scheme),
        ..BINARY
    },
    CheckSpec {
        id: "check.metrics.strikeoutAndScripts",
        label: "Strikeout and script fields",
        description: "The strikeout size or underline thickness is 0, or an OS/2 strikeout, subscript or superscript field is more than 5 units from the default derived from the font's metrics.",
        severity: Fail,
        profile: Strict,
        fix: patch(false, "Write the derived strikeout, subscript and superscript values."),
        codes: &["strikeout-size-zero", "underline-thickness-zero", "differs-from-default"],
        code_severities: &[("differs-from-default", Warn)],
        preconditions: &[Table("OS/2"), Table("head"), Table("post")],
        run: Runner::Binary(m::strikeout_and_scripts),
        ..BINARY
    },
];
