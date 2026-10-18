//! Registry entries of the width `check.metrics.*` checks (M12.49).

use crate::CheckGroup::Metrics;
use crate::CheckScope::{Binary, Glyph};
use crate::Precondition::{SourceData, Table};
use crate::Profile::{Outlines, Strict, Universal};
use crate::Severity::{Fail, Warn};
use crate::checks::metrics_widths as m;
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
    profile: Universal,
    thresholds: &[],
    fix: None,
    skip_variable: false,
    capped: false,
    codes: &[],
    code_severities: &[],
    preconditions: &[Table("hmtx")],
    run: Runner::Binary(no_check),
};

fn no_check(_: &crate::BinaryCtx<'_>, _: &crate::Params<'_>, _: &mut crate::Sink) {}

const fn update_metrics(safe: bool, description: &'static str) -> Option<FixSpec> {
    Some(FixSpec { id: "updateMetrics", command: Some("glyph.updateMetrics"), safe, description })
}

/// The width checks, in the order of the inventory.
pub(super) const CHECKS: &[CheckSpec] = &[
    CheckSpec {
        id: "check.metrics.baseGlyphWidth",
        label: "Base glyphs without width",
        description: "An encoded glyph that is not a mark and not a character that may draw nothing (a space, a format or control character, a private-use character) has an advance width of zero.",
        codes: &["zero-width"],
        run: Runner::Binary(m::base_glyph_width),
        ..BINARY
    },
    CheckSpec {
        id: "check.metrics.markWidths",
        label: "Mark widths",
        description: "A non-spacing combining mark (Mn, Me) has a non-zero advance, or a spacing modifier (Sk, Lm) has none.",
        profile: Strict,
        fix: Some(FixSpec {
            id: "zeroMarkWidths",
            command: Some("spacing.setWidth"),
            safe: true,
            description: "Set the advance of the non-spacing mark to zero in the source.",
        }),
        codes: &["non-spacing-not-zero", "spacing-zero"],
        run: Runner::Binary(m::mark_widths),
        ..BINARY
    },
    CheckSpec {
        id: "check.metrics.whitespaceWidths",
        label: "Space widths",
        description: "The space has no advance, or the no-break space has a different one; in a proportional font, another encoded space character departs from its conventional advance (a warning).",
        fix: update_metrics(true, "Give the no-break space the advance of the space in the source."),
        codes: &["zero-space", "different-widths", "bad-whitespace-advance"],
        code_severities: &[("bad-whitespace-advance", Warn)],
        run: Runner::Binary(m::whitespace_widths),
        ..BINARY
    },
    CheckSpec {
        id: "check.metrics.mathSignWidths",
        label: "Math sign widths",
        description: "The encoded mathematical operators and relations do not all share the most common advance among them.",
        severity: Warn,
        codes: &["width-outliers"],
        run: Runner::Binary(m::math_sign_widths),
        ..BINARY
    },
    CheckSpec {
        id: "check.metrics.tabularWidths",
        label: "Tabular widths",
        description: "The figures the tnum feature gives do not share one advance; or in a group of tabular figures found by name, glyphs depart from the width more than half the group shares; or figurespace is not as wide as the tabular one.",
        codes: &["tnum-unequal", "group-unequal", "figure-space"],
        run: Runner::Binary(m::tabular_widths),
        ..BINARY
    },
    CheckSpec {
        id: "check.metrics.digitWidths",
        label: "Figure widths",
        description: "The encoded figures 0-9 do not all have the advance of the zero, a figure is missing, or comma and period differ in advance.",
        profile: Strict,
        codes: &["missing-digit", "bad-digit-width", "comma-period"],
        run: Runner::Binary(m::digit_widths),
        ..BINARY
    },
    CheckSpec {
        id: "check.metrics.fixedFractionWidths",
        label: "En and em widths",
        description: "The en dash or a dot leader is not half the em wide, or the em dash is not one em wide.",
        profile: Strict,
        codes: &["not-half-em", "not-em"],
        preconditions: &[Table("hmtx"), Table("head")],
        run: Runner::Binary(m::fixed_fraction_widths),
        ..BINARY
    },
    CheckSpec {
        id: "check.metrics.ligatureWidths",
        label: "Ligature widths",
        description: "A ligature of the liga, clig, dlig, hlig or rlig feature is not wider than its first component, or its left or right sidebearing differs from that of its first or last component by more than `tolerance` units.",
        severity: Warn,
        profile: Strict,
        thresholds: &[m::TOLERANCE],
        capped: true,
        codes: &["not-wider", "left-sidebearing", "right-sidebearing"],
        preconditions: &[Table("hmtx"), Table("GSUB")],
        run: Runner::Binary(m::ligature_widths),
        ..BINARY
    },
    CheckSpec {
        id: "check.metrics.accentedWidths",
        label: "Accented glyph widths",
        description: "A glyph whose name is a base glyph's name with an accent added differs in advance from that base glyph by more than `tolerance` units.",
        severity: Warn,
        profile: Strict,
        thresholds: &[m::TOLERANCE],
        fix: update_metrics(false, "Give the accented glyph the width of its base glyph in the source."),
        capped: true,
        codes: &["width-differs"],
        run: Runner::Binary(m::accented_widths),
        ..BINARY
    },
    CheckSpec {
        id: "check.metrics.metricsKeyOutOfSync",
        label: "Metrics keys out of sync",
        description: "A glyph's sidebearings or width no longer match the metrics key or formula stored for them.",
        scope: Glyph,
        severity: Warn,
        fix: update_metrics(true, "Recompute the sidebearings and width from the metrics keys."),
        codes: &["out-of-sync", "unlinked", "key-error"],
        preconditions: &[SourceData],
        run: Runner::Glyph(m::metrics_key_out_of_sync),
        ..BINARY
    },
    CheckSpec {
        id: "check.metrics.negativeAdvance",
        label: "Negative advance width",
        description: "A master or attribute layer of a glyph has a negative width, which cannot be compiled.",
        scope: Glyph,
        profile: Outlines,
        codes: &["negative-width"],
        preconditions: &[SourceData],
        run: Runner::Glyph(m::negative_advance),
        ..BINARY
    },
];
