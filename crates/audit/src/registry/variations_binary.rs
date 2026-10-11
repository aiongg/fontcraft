//! Registry entries of the binary-scope `check.variations.*` checks (M12.54), in the order of the
//! inventory. Each follows the FontBakery check its row names; the runners are in
//! `checks/variations_binary.rs`.

use crate::CheckGroup::Variations;
use crate::CheckScope::Binary;
use crate::Precondition::{Table, Variable};
use crate::Profile::{Opentype, Strict, Universal};
use crate::Severity::{Fail, Info, Warn};
use crate::checks::variations_binary as vb;
use crate::{CheckSpec, FixSpec, Runner};

const fn fix(id: &'static str, command: &'static str, safe: bool, description: &'static str) -> Option<FixSpec> {
    Some(FixSpec { id, command: Some(command), safe, description })
}

const CLEAN_INSTANCES: Option<FixSpec> = fix(
    "cleanInstances",
    "tables.cleanInstances",
    false,
    "Sort the named instances, drop repeats, write every record in one form and point the default instance at the font's own names.",
);

/// The fields every entry here shares; each sets the rest and takes these with `..BASE`.
const BASE: CheckSpec = CheckSpec {
    id: "",
    label: "",
    description: "",
    group: Variations,
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
    preconditions: &[Variable],
    run: Runner::Binary(vb::avar_missing),
};

pub(super) const CHECKS: &[CheckSpec] = &[
    CheckSpec {
        id: "check.variations.axisRanges",
        label: "Registered axis ranges",
        description: "A named instance or an axis breaks the rules of the registered axes: weight outside 1 to 1000, width below 1 (above 1000 warns), an ital axis that does not run from 0 to 1, or (warning) a slnt axis that does not run from a negative value up to at least 0.",
        profile: Opentype,
        codes: &["wght-out-of-range", "wdth-out-of-range", "wdth-greater-than-1000", "invalid-ital-range", "unusual-slnt-range"],
        code_severities: &[("wdth-greater-than-1000", Warn), ("unusual-slnt-range", Warn)],
        run: Runner::Binary(vb::axis_ranges),
        ..BASE
    },
    CheckSpec {
        id: "check.variations.regularInstanceCoordinates",
        label: "Regular instance coordinates",
        description: "No instance is named Regular, or it does not sit at weight 400, width 100, slant 0 and italic 0; an optical size outside 10 to 16 warns.",
        profile: Opentype,
        codes: &["no-regular-instance", "wght-not-400", "wdth-not-100", "slnt-not-0", "ital-not-0", "opsz"],
        code_severities: &[("opsz", Warn)],
        run: Runner::Binary(vb::regular_instance_coordinates),
        ..BASE
    },
    CheckSpec {
        id: "check.variations.boldInstanceCoordinate",
        label: "Bold instance coordinate",
        description: "The weight axis reaches 700 but no instance is named Bold, or the Bold instance is not at weight 700.",
        profile: Universal,
        codes: &["no-bold-instance", "wght-not-700"],
        run: Runner::Binary(vb::bold_instance_coordinate),
        ..BASE
    },
    CheckSpec {
        id: "check.variations.slantDirection",
        label: "Slant direction",
        description: "H leans further right at the top of the slnt axis than at its bottom: the registered slant axis uses negative values for a rightward lean.",
        profile: Opentype,
        codes: &["positive-value-for-clockwise-lean"],
        run: Runner::Binary(vb::slant_direction),
        ..BASE
    },
    CheckSpec {
        id: "check.variations.foundryAxisTags",
        label: "Private axis tags",
        description: "A private axis tag does not start with an uppercase letter or uses characters other than uppercase letters and digits; a tag that equals a registered tag except for case warns.",
        profile: Opentype,
        codes: &["foundry-defined-similar-registered-name", "invalid-foundry-defined-tag-first-letter", "invalid-foundry-defined-tag-chars"],
        code_severities: &[("foundry-defined-similar-registered-name", Warn)],
        run: Runner::Binary(vb::foundry_axis_tags),
        ..BASE
    },
    CheckSpec {
        id: "check.variations.unsupportedItalAxis",
        label: "Italic axis",
        description: "The font has an ital axis, which some browsers did not support; the advice is separate upright and italic fonts.",
        profile: Strict,
        codes: &["unsupported-ital"],
        run: Runner::Binary(vb::unsupported_ital_axis),
        ..BASE
    },
    CheckSpec {
        id: "check.variations.axisHasNoVariation",
        label: "Axis without range",
        description: "An axis has equal minimum and maximum, so it varies nothing.",
        profile: Strict,
        fix: fix("dropAxis", "tables.dropAxis", true, "Remove the axis from fvar, avar, STAT and every variation region."),
        codes: &["axis-has-no-variation"],
        run: Runner::Binary(vb::axis_has_no_variation),
        ..BASE
    },
    CheckSpec {
        id: "check.variations.axisOrder",
        label: "Axis order",
        description: "Without STAT, the registered fvar axes are not in the order opsz, wdth, wght, ital, slnt (custom axes are listed); with STAT, its axis ordering is reported as information.",
        severity: Warn,
        profile: Strict,
        codes: &["axes-incorrect-order", "custom-axes", "stat-axis-order"],
        code_severities: &[("custom-axes", Info), ("stat-axis-order", Info)],
        run: Runner::Binary(vb::axis_order),
        ..BASE
    },
    CheckSpec {
        id: "check.variations.instanceRecords",
        label: "Instance records",
        description: "Some fvar instance records have a PostScript name ID and others do not, so they differ in size, or (warning) a record repeats an earlier one's coordinates and names.",
        profile: Opentype,
        fix: CLEAN_INSTANCES,
        codes: &["different-size-instance-records", "repeated-instance-record"],
        code_severities: &[("repeated-instance-record", Warn)],
        run: Runner::Binary(vb::instance_records),
        ..BASE
    },
    CheckSpec {
        id: "check.variations.duplicateInstanceNames",
        label: "Duplicate instance names",
        description: "Two named instances have the same subfamily name, or an instance's name has no Windows English record.",
        profile: Universal,
        codes: &["name-record-not-found", "duplicate-instance-names"],
        run: Runner::Binary(vb::duplicate_instance_names),
        ..BASE
    },
    CheckSpec {
        id: "check.variations.instancesInOrder",
        label: "Instance order",
        description: "Instances that agree on every axis but weight are not listed in rising weight order; some applications show them in stored order.",
        profile: Universal,
        fix: CLEAN_INSTANCES,
        codes: &["instances-not-in-order"],
        run: Runner::Binary(vb::instances_in_order),
        ..BASE
    },
    CheckSpec {
        id: "check.variations.instanceNameIds",
        label: "Instance and axis name IDs",
        description: "An fvar name reference is invalid: an axis or subfamily name ID outside 256 to 32767 (a subfamily may also use 2 or 17), a PostScript name ID that is not 6, 0xFFFF or in that range, or an ID with no record in the name table.",
        profile: Opentype,
        codes: &[
            "invalid-axis-nameid",
            "missing-axis-name",
            "invalid-subfamily-nameid",
            "missing-name",
            "invalid-postscript-nameid",
            "missing-postscript-name",
        ],
        run: Runner::Binary(vb::instance_name_ids),
        ..BASE
    },
    CheckSpec {
        id: "check.variations.defaultInstanceNames",
        label: "Default instance names",
        description: "The instance at the default location does not use the font's own names: its subfamily name must equal name ID 17 (or 2 without 17) and, when the records carry PostScript names, its PostScript name must equal name ID 6.",
        profile: Opentype,
        fix: CLEAN_INSTANCES,
        codes: &["invalid-default-instance-subfamily-name", "invalid-default-instance-postscript-name"],
        preconditions: &[Variable, Table("name")],
        run: Runner::Binary(vb::default_instance_names),
        ..BASE
    },
    CheckSpec {
        id: "check.variations.weightClassMatchesDefault",
        label: "Weight class of the default",
        description: "OS/2 usWeightClass differs from the default of the weight axis.",
        profile: Opentype,
        fix: fix("patchTable", "tables.recalculate", false, "Set usWeightClass to the default of the weight axis."),
        codes: &["bad-weight-class"],
        preconditions: &[Variable, Table("OS/2")],
        run: Runner::Binary(vb::weight_class_matches_default),
        ..BASE
    },
    CheckSpec {
        id: "check.variations.statAxisRecords",
        label: "STAT axis records",
        description: "STAT has no axis record for an axis that fvar defines.",
        profile: Opentype,
        codes: &["missing-axis-records"],
        preconditions: &[Variable, Table("STAT")],
        run: Runner::Binary(vb::stat_axis_records),
        ..BASE
    },
    CheckSpec {
        id: "check.variations.statItalAxis",
        label: "STAT italic axis",
        description: "An italic variable font has no ital axis in STAT (failure), or the STAT ital axis is set up wrongly (warnings): it is not the last axis, its value is not 0 for upright or 1 for italic, the upright value is not elidable, or the upright value does not link to 1.",
        profile: Opentype,
        codes: &[
            "no-stat",
            "missing-ital-axis",
            "ital-axis-not-last",
            "wrong-ital-axis-value",
            "wrong-ital-axis-flag",
            "wrong-ital-axis-linkedvalue",
        ],
        code_severities: &[
            ("ital-axis-not-last", Warn),
            ("wrong-ital-axis-value", Warn),
            ("wrong-ital-axis-flag", Warn),
            ("wrong-ital-axis-linkedvalue", Warn),
        ],
        run: Runner::Binary(vb::stat_ital_axis),
        ..BASE
    },
    CheckSpec {
        id: "check.variations.statItalicNaming",
        label: "Italic in STAT names",
        description: "A STAT axis value on an axis other than ital is named with the word Italic; `allowOnSlnt` also allows it on slnt.",
        profile: Universal,
        thresholds: &[vb::ALLOW_ON_SLNT],
        codes: &["bad-italic"],
        preconditions: &[Table("STAT")],
        run: Runner::Binary(vb::stat_italic_naming),
        ..BASE
    },
    CheckSpec {
        id: "check.variations.statCoversInstances",
        label: "STAT covers the instances",
        description: "The variable font has no STAT table, or STAT cannot name every instance: an instance uses an axis value no STAT axis value names, or its name record is missing.",
        profile: Universal,
        codes: &["missing-STAT-table", "missing-name-id", "missing-fvar-instance-axis-value"],
        run: Runner::Binary(vb::stat_covers_instances),
        ..BASE
    },
    CheckSpec {
        id: "check.variations.statValueQuality",
        label: "STAT value quality",
        description: "STAT is poorly built: a format 1 value repeats within an axis, a value named Regular is not elidable, the axis ordering is not opsz, wght, wdth, slnt, ital, a weight or width class value inside the axis range has no STAT value, or (warning) there are no axis values.",
        profile: Strict,
        codes: &["duplicate-axis-value", "regular-not-elidable", "no-axis-values", "axis-order-invalid", "missing-axis-value"],
        code_severities: &[("no-axis-values", Warn)],
        preconditions: &[Table("STAT")],
        run: Runner::Binary(vb::stat_value_quality),
        ..BASE
    },
    CheckSpec {
        id: "check.variations.avarMissing",
        label: "No avar table",
        description: "The variable font has no avar table, so every axis progresses linearly between masters; ignore when that is intended.",
        severity: Warn,
        profile: Universal,
        codes: &["missing-avar"],
        run: Runner::Binary(vb::avar_missing),
        ..BASE
    },
    CheckSpec {
        id: "check.variations.hvarMissing",
        label: "No HVAR table",
        description: "The variable font has no HVAR table, so text engines must compute advance changes from the outlines.",
        profile: Strict,
        fix: fix("recompile", "file.export", true, "Export the font again with the HVAR table generated."),
        codes: &["lacks-HVAR"],
        run: Runner::Binary(vb::hvar_missing),
        ..BASE
    },
    CheckSpec {
        id: "check.variations.duplexedAxisReflow",
        label: "Duplexed axis reflow",
        description: "An axis that must leave text length unchanged (GRAD, ROND) changes a glyph's advance in gvar, or kerning in the designspace.",
        profile: Universal,
        codes: &["axis-causes-reflow", "kern-causes-reflow"],
        run: Runner::Binary(vb::duplexed_axis_reflow),
        ..BASE
    },
    CheckSpec {
        id: "check.variations.staticInstantiation",
        label: "Static instance",
        description: "A static instance cannot be made at the middle of every axis: a variation table cannot be read, or a glyph's outline or advance cannot be computed there.",
        profile: Strict,
        codes: &["unreadable-table", "glyph-fails"],
        run: Runner::Binary(vb::static_instantiation),
        ..BASE
    },
    CheckSpec {
        id: "check.variations.customAxisRange",
        label: "Custom axis range",
        description: "A known custom axis is outside its documented range: ytlc (lowercase height) outside 0 to 1000.",
        profile: Strict,
        codes: &["invalid-range"],
        run: Runner::Binary(vb::custom_axis_range),
        ..BASE
    },
];
