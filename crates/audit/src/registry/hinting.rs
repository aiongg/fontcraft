//! Registry entries of the `check.hinting.*` checks (M12.55).

use crate::Precondition::{SourceData, Table, TrueTypeOutlines};
use crate::Profile::{Strict, Universal};
use crate::Severity::{Fail, Info, Warn};
use crate::checks::hinting as h;
use crate::{CheckGroup, CheckScope, CheckSpec, FixSpec, Runner};

const fn fix(id: &'static str, command: Option<&'static str>, safe: bool, description: &'static str) -> Option<FixSpec> {
    Some(FixSpec { id, command, safe, description })
}

const RECALCULATE: Option<&str> = Some("tables.recalculate");
const AUTOHINT: Option<FixSpec> =
    fix("autohint", Some("hint.ps.autohintGlyph"), false, "Run the PostScript autohinter again on the reported glyphs.");

/// The fields the binary hinting checks share.
const BINARY: CheckSpec = CheckSpec {
    id: "",
    label: "",
    description: "",
    group: CheckGroup::Hinting,
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
    run: Runner::Binary(h::integer_ppem),
};

/// The checks, in the order of the inventory.
pub(super) const CHECKS: &[CheckSpec] = &[
    CheckSpec {
        id: "check.hinting.integerPpem",
        label: "Integer sizes for instructed fonts",
        description: "A TrueType font with instructions does not set head.flags bit 3, so scalers may use fractional sizes the instructions were not written for.",
        fix: fix("patchTable", RECALCULATE, true, "Set bit 3 of head.flags."),
        codes: &["bad-flags"],
        preconditions: &[Table("fpgm")],
        run: Runner::Binary(h::integer_ppem),
        ..BINARY
    },
    CheckSpec {
        id: "check.hinting.smartDropout",
        label: "Smart dropout control",
        description: "The 'prep' program of a TrueType font does not switch on dropout control with smart dropout. Fonts hinted with a visual hinting tool are skipped.",
        fix: fix("patchTable", RECALCULATE, true, "Append the instructions that switch on smart dropout control to the 'prep' program."),
        codes: &["lacks-smart-dropout"],
        preconditions: &[TrueTypeOutlines],
        run: Runner::Binary(h::smart_dropout),
        ..BINARY
    },
    CheckSpec {
        id: "check.hinting.sizeImpact",
        label: "Hinting size",
        description: "Reports the file size with and without hinting and the difference in bytes and percent.",
        severity: Info,
        codes: &["size-impact"],
        run: Runner::Binary(h::size_impact),
        ..BINARY
    },
    CheckSpec {
        id: "check.hinting.gasp",
        label: "Rendering ranges (gasp)",
        description: "A TrueType font has no readable 'gasp' table (failure), no range for all sizes, further ranges, or a range for all sizes without every rendering flag (warnings); the ranges are listed.",
        profile: Strict,
        fix: fix("patchTable", RECALCULATE, false, "Write one range for all sizes with grid-fitting and smoothing flags all set."),
        codes: &["lacks-gasp", "empty", "lacks-ffff-range", "non-ffff-range", "unset-flags", "ranges"],
        code_severities: &[("lacks-ffff-range", Warn), ("non-ffff-range", Warn), ("unset-flags", Warn), ("ranges", Info)],
        preconditions: &[TrueTypeOutlines],
        run: Runner::Binary(h::gasp),
        ..BINARY
    },
    CheckSpec {
        id: "check.hinting.autohinterTraces",
        label: "Automatic hinter traces",
        description: "A font hinted by the automatic TrueType hinter does not record the hinter's options in its version string, or records an outdated hinter version.",
        profile: Strict,
        codes: &["lacks-version-strings", "lacks-ttfa-params", "version-not-detected", "old-ttfa"],
        code_severities: &[("version-not-detected", Info), ("old-ttfa", Warn)],
        run: Runner::Binary(h::autohinter_traces),
        ..BINARY
    },
    CheckSpec {
        id: "check.hinting.missingStemHints",
        label: "Glyphs without stem hints",
        description: "A glyph of a CFF font draws an outline but has no stem hints at all.",
        profile: Strict,
        fix: AUTOHINT,
        capped: true,
        codes: &["missing-hints"],
        preconditions: &[Table("CFF ")],
        run: Runner::Binary(h::missing_stem_hints),
        ..BINARY
    },
    CheckSpec {
        id: "check.hinting.stemHintSanity",
        label: "Stem hints outside the glyph",
        description: "A CFF stem hint has a negative width, is wider than the glyph's box or reaches past it; edge hints are tested against the box as well.",
        profile: Strict,
        fix: AUTOHINT,
        capped: true,
        codes: &["negative-width", "wider-than-glyph", "above-glyph", "below-glyph", "edge-outside-glyph"],
        preconditions: &[Table("CFF ")],
        run: Runner::Binary(h::stem_hint_sanity),
        ..BINARY
    },
    CheckSpec {
        id: "check.hinting.forceBold",
        label: "ForceBold",
        description: "The CFF private dictionary sets ForceBold, which is harmless but obsolete.",
        severity: Warn,
        profile: Strict,
        fix: fix("recompileCff", Some("tables.recompileCff"), true, "Rewrite the CFF table without ForceBold."),
        codes: &["force-bold"],
        preconditions: &[Table("CFF ")],
        run: Runner::Binary(h::force_bold),
        ..BINARY
    },
    CheckSpec {
        id: "check.hinting.blueValuesCount",
        label: "Alignment zone counts",
        description: "BlueValues is missing or empty, or a zone array has an odd number of values or more zones than allowed (7 for BlueValues and FamilyBlues, 5 for OtherBlues and FamilyOtherBlues).",
        codes: &["no-blue-values", "too-many-zones", "odd-count"],
        preconditions: &[Table("CFF ")],
        run: Runner::Binary(h::blue_values_count),
        ..BINARY
    },
    CheckSpec {
        id: "check.hinting.blueValuesOrder",
        label: "Alignment zone order",
        description: "In an alignment-zone pair the first value is larger than the second.",
        fix: fix("sortZones", Some("font.info.sortZones"), true, "Sort each alignment-zone pair."),
        codes: &["unordered-pair"],
        preconditions: &[Table("CFF ")],
        run: Runner::Binary(h::blue_values_order),
        ..BINARY
    },
    CheckSpec {
        id: "check.hinting.blueZoneSeparation",
        label: "Alignment zones too close",
        description: "Two bottom zones or two top zones are closer together than 2 × BlueFuzz + 1 units, so the hinter treats them as one.",
        codes: &["zones-too-close"],
        preconditions: &[Table("CFF ")],
        run: Runner::Binary(h::blue_zone_separation),
        ..BINARY
    },
    CheckSpec {
        id: "check.hinting.blueZoneHeight",
        label: "Alignment zones too tall",
        description: "An alignment zone is at least 1 / BlueScale units tall, which switches overshoot suppression off at the wrong sizes.",
        codes: &["zone-too-tall"],
        preconditions: &[Table("CFF ")],
        run: Runner::Binary(h::blue_zone_height),
        ..BINARY
    },
    CheckSpec {
        id: "check.hinting.blueZonesOutsideBBox",
        label: "Alignment zones outside the font",
        description: "The lowest or highest alignment zone lies outside the font's bounding box, so no glyph can reach it.",
        profile: Strict,
        codes: &["below-bbox", "above-bbox"],
        preconditions: &[Table("CFF ")],
        run: Runner::Binary(h::blue_zones_outside_bbox),
        ..BINARY
    },
    CheckSpec {
        id: "check.hinting.zonesOverlapInSource",
        label: "Overlapping alignment zones",
        description: "In a master, two alignment zones overlap or touch, or a zone contains no metric line.",
        scope: CheckScope::Source,
        severity: Warn,
        thresholds: &[h::MIN_ZONE_GAP],
        codes: &["zones-overlap", "zone-without-metric"],
        preconditions: &[SourceData],
        run: Runner::Source(h::zones_overlap_in_source),
        ..BINARY
    },
    CheckSpec {
        id: "check.hinting.stemsUndefined",
        label: "No standard stems",
        description: "A master defines no horizontal or no vertical standard stem, so the autohinter has nothing to normalise stem widths to.",
        scope: CheckScope::Source,
        severity: Info,
        profile: Strict,
        codes: &["no-horizontal-stems", "no-vertical-stems"],
        preconditions: &[SourceData],
        run: Runner::Source(h::stems_undefined),
        ..BINARY
    },
];
