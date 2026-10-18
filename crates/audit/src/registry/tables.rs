//! Registry entries of the `check.tables.*` checks and `check.outlines.pointsOutsideBBox` (M12.51).

use crate::CheckGroup::{Outlines, Tables};
use crate::CheckScope::Binary;
use crate::Precondition::{CffOutlines, ProjectValue, Static, Table, TrueTypeOutlines};
use crate::Profile::{Opentype, Strict, Universal};
use crate::Severity::{Fail, Info, Warn};
use crate::checks::tables as t;
use crate::{CheckSpec, FixSpec, Runner};

/// The fields every entry here shares; each entry sets the rest.
const BINARY: CheckSpec = CheckSpec {
    id: "",
    label: "",
    description: "",
    group: Tables,
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

const fn drop_tables(safe: bool, description: &'static str) -> Option<FixSpec> {
    Some(FixSpec { id: "dropTables", command: Some("tables.delete"), safe, description })
}

const fn patch(safe: bool, description: &'static str) -> Option<FixSpec> {
    Some(FixSpec { id: "patchTable", command: Some("tables.recalculate"), safe, description })
}

const fn recompile_cff(description: &'static str) -> Option<FixSpec> {
    Some(FixSpec { id: "recompileCff", command: Some("tables.recompileCff"), safe: true, description })
}

/// The table checks, in the order of the inventory, then the outline bounds check.
pub(super) const CHECKS: &[CheckSpec] = &[
    CheckSpec {
        id: "check.tables.requiredTables",
        label: "Required tables",
        description: "A required table is missing: cmap, head, hhea, hmtx, maxp, name, OS/2, post, the outline table the file's version names, and STAT in a variable font.",
        severity: Fail,
        profile: Universal,
        codes: &["required-tables", "optional-tables"],
        code_severities: &[("optional-tables", Info)],
        run: Runner::Binary(t::required_tables),
        ..BINARY
    },
    CheckSpec {
        id: "check.tables.recommendedTrueTypeTables",
        label: "Recommended TrueType tables",
        description: "A TrueType font lacks gasp, kern, hdmx or VDMX. Never raised for CFF fonts.",
        severity: Info,
        profile: Strict,
        codes: &["missing-recommended-table"],
        preconditions: &[TrueTypeOutlines],
        run: Runner::Binary(t::recommended_truetype_tables),
        ..BINARY
    },
    CheckSpec {
        id: "check.tables.unwantedTables",
        label: "Tool-private tables",
        description: "The font carries tables that editors and hinting or layout tools keep for themselves, or layout features and language systems whose tags begin with zz.",
        severity: Fail,
        profile: Universal,
        fix: drop_tables(true, "Remove the tool-private tables."),
        codes: &["unwanted-table", "zz-feature", "zz-language-system"],
        run: Runner::Binary(t::unwanted_tables),
        ..BINARY
    },
    CheckSpec {
        id: "check.tables.unwantedAatTables",
        label: "Apple-only tables",
        description: "The font carries Apple layout tables that are not part of OpenType.",
        severity: Fail,
        profile: Opentype,
        fix: drop_tables(false, "Remove the Apple-only tables."),
        codes: &["unwanted-aat-table"],
        run: Runner::Binary(t::unwanted_aat_tables),
        ..BINARY
    },
    CheckSpec {
        id: "check.tables.unsupportedTables",
        label: "Tables outside the allow-list",
        description: "The font has a table that the project value `allowedTables` does not list.",
        severity: Fail,
        profile: Strict,
        codes: &["unsupported-table"],
        preconditions: &[ProjectValue("allowedTables")],
        run: Runner::Binary(t::unsupported_tables),
        ..BINARY
    },
    CheckSpec {
        id: "check.tables.dsig",
        label: "Digital signature table",
        description: "The font has a DSIG table, which nothing current needs.",
        severity: Warn,
        profile: Opentype,
        fix: drop_tables(true, "Remove the DSIG table."),
        codes: &["found-dsig"],
        run: Runner::Binary(t::dsig),
        ..BINARY
    },
    CheckSpec {
        id: "check.tables.postVersion",
        label: "post table format",
        description: "CFF fonts must use post format 3; TrueType fonts with format 3 lose their glyph names; formats 2.5 and 4 are not allowed.",
        severity: Fail,
        profile: Opentype,
        codes: &["cff-not-format-3", "format-3", "format-2-5", "format-4"],
        code_severities: &[("format-3", Warn)],
        preconditions: &[Table("post")],
        run: Runner::Binary(t::post_version),
        ..BINARY
    },
    CheckSpec {
        id: "check.tables.glyfIntegrity",
        label: "glyf and loca integrity",
        description: "loca has fewer entries than maxp.numGlyphs needs, goes backwards, points past the end of glyf, or leaves data after the last glyph.",
        severity: Fail,
        profile: Opentype,
        codes: &["loca-too-short", "offsets-decrease", "missing-data", "unreachable-data"],
        preconditions: &[TrueTypeOutlines],
        run: Runner::Binary(t::glyf_integrity),
        ..BINARY
    },
    CheckSpec {
        id: "check.tables.cmapFormat12",
        label: "cmap format 12",
        description: "A format 12 cmap subtable has no format 4 companion, maps nothing beyond the Basic Multilingual Plane, or leaves out characters the format 4 subtable maps.",
        severity: Fail,
        profile: Universal,
        fix: Some(FixSpec {
            id: "rebuildCmap",
            command: Some("tables.rebuildCmap"),
            safe: true,
            description: "Rebuild the cmap subtables from one mapping.",
        }),
        codes: &["no-cmap-4", "pointless-format-12", "unmapped-from-4"],
        code_severities: &[("unmapped-from-4", Warn)],
        preconditions: &[Table("cmap")],
        run: Runner::Binary(t::cmap_format_12),
        ..BINARY
    },
    CheckSpec {
        id: "check.tables.cmapSubtables",
        label: "Unexpected cmap subtables",
        description: "The cmap table has a subtable other than the usual Unicode, Windows and Mac Roman ones.",
        severity: Warn,
        profile: Strict,
        codes: &["unexpected-subtable", "unreadable-subtable"],
        preconditions: &[Table("cmap")],
        run: Runner::Binary(t::cmap_subtables_check),
        ..BINARY
    },
    CheckSpec {
        id: "check.tables.fsSelection",
        label: "Style bits",
        description: "The bold, italic and regular bits of OS/2.fsSelection do not match the style the names give, or disagree with head.macStyle.",
        severity: Fail,
        profile: Opentype,
        fix: Some(FixSpec {
            id: "syncStyleLinking",
            command: None,
            safe: false,
            description: "Derive fsSelection, macStyle and the subfamily name from the style-linking settings.",
        }),
        codes: &["bad-bold", "bad-italic", "bad-regular", "macstyle-bold", "macstyle-italic"],
        preconditions: &[Table("OS/2"), Table("head")],
        run: Runner::Binary(t::fs_selection),
        ..BINARY
    },
    CheckSpec {
        id: "check.tables.vendorId",
        label: "Vendor ID",
        description: "OS/2.achVendID differs from the project's vendor ID; without one, it is empty, a placeholder or not registered.",
        severity: Fail,
        profile: Opentype,
        codes: &["bad-vendor-id", "not-set", "placeholder", "unregistered"],
        code_severities: &[("not-set", Warn), ("placeholder", Warn), ("unregistered", Warn)],
        preconditions: &[Table("OS/2")],
        run: Runner::Binary(t::vendor_id),
        ..BINARY
    },
    CheckSpec {
        id: "check.tables.codePageRanges",
        label: "Code page ranges",
        description: "The OS/2 code page fields are empty or the table is missing; with `strict`, each code page and Unicode range bit is also compared with what the cmap covers.",
        severity: Fail,
        profile: Opentype,
        thresholds: &[t::STRICT_BITS],
        fix: patch(false, "Recompute the code page and Unicode range bits from the cmap."),
        codes: &["no-os2-table", "no-code-pages", "bad-code-page-bit", "bad-range-bit", "non-plane-0-without-format-12"],
        code_severities: &[("bad-code-page-bit", Warn), ("bad-range-bit", Warn), ("non-plane-0-without-format-12", Warn)],
        run: Runner::Binary(t::code_page_ranges),
        ..BINARY
    },
    CheckSpec {
        id: "check.tables.os2Version",
        label: "OS/2 version",
        description: "The OS/2 table is older than version 4.",
        severity: Warn,
        profile: Strict,
        fix: patch(true, "Upgrade the OS/2 table to version 4."),
        codes: &["old-version"],
        preconditions: &[Table("OS/2")],
        run: Runner::Binary(t::os2_version),
        ..BINARY
    },
    CheckSpec {
        id: "check.tables.weightClassRange",
        label: "Weight class",
        description: "usWeightClass is below 250 in a static CFF font, below 500 with the bold bit set, or not the value the weight name or the default weight calls for.",
        severity: Fail,
        profile: Strict,
        codes: &["cff-below-250", "bold-below-500", "bad-value", "no-value"],
        code_severities: &[("no-value", Info)],
        preconditions: &[Table("OS/2")],
        run: Runner::Binary(t::weight_class_range),
        ..BINARY
    },
    CheckSpec {
        id: "check.tables.fsType",
        label: "Embedding permissions",
        description: "OS/2.fsType is not the project's embedding setting (`fsType`).",
        severity: Fail,
        profile: Strict,
        fix: patch(true, "Set fsType to the project's embedding setting."),
        codes: &["unexpected-value"],
        preconditions: &[Table("OS/2"), ProjectValue("fsType")],
        run: Runner::Binary(t::fs_type),
        ..BINARY
    },
    CheckSpec {
        id: "check.tables.lowestRecPpem",
        label: "Smallest readable size",
        description: "head.lowestRecPPEM is below `minimum` pixels per em.",
        severity: Info,
        profile: Strict,
        thresholds: &[t::MIN_PPEM],
        codes: &["small-lowest-ppem"],
        preconditions: &[Table("head")],
        run: Runner::Binary(t::lowest_rec_ppem),
        ..BINARY
    },
    CheckSpec {
        id: "check.tables.cffCallDepth",
        label: "CFF subroutine depth",
        description: "A CFF or CFF2 charstring nests subroutine calls more than 10 deep, or a subroutine calls itself.",
        severity: Fail,
        profile: Opentype,
        codes: &["max-depth", "recursion"],
        preconditions: &[CffOutlines],
        run: Runner::Binary(t::cff_call_depth),
        ..BINARY
    },
    CheckSpec {
        id: "check.tables.cffAsciiStrings",
        label: "CFF strings",
        description: "The CFF font name or a top dictionary string has non-ASCII characters or cannot be found.",
        severity: Fail,
        profile: Opentype,
        codes: &["not-ascii", "cannot-decode"],
        preconditions: &[Table("CFF ")],
        run: Runner::Binary(t::cff_ascii_strings),
        ..BINARY
    },
    CheckSpec {
        id: "check.tables.cffDeprecatedOperators",
        label: "Deprecated CFF operators",
        description: "A CFF charstring builds an accented glyph with endchar (seac) or uses dotsection.",
        severity: Fail,
        profile: Opentype,
        fix: recompile_cff("Decompose seac glyphs and drop dotsection."),
        codes: &["endchar-seac", "dotsection"],
        code_severities: &[("dotsection", Warn)],
        preconditions: &[Table("CFF ")],
        run: Runner::Binary(t::cff_deprecated_operators),
        ..BINARY
    },
    CheckSpec {
        id: "check.tables.cffUniqueIds",
        label: "CFF unique IDs",
        description: "A name-keyed CFF font has UniqueID, XUID or UIDBase entries.",
        severity: Fail,
        profile: Strict,
        fix: recompile_cff("Remove the UniqueID, XUID and UIDBase entries."),
        codes: &["unique-id", "xuid", "uid-base"],
        preconditions: &[Table("CFF ")],
        run: Runner::Binary(t::cff_unique_ids),
        ..BINARY
    },
    CheckSpec {
        id: "check.tables.cffCharstringSize",
        label: "CFF charstring size",
        description: "A CFF charstring is longer than 32,767 bytes.",
        severity: Fail,
        profile: Strict,
        codes: &["charstring-too-long"],
        preconditions: &[CffOutlines],
        run: Runner::Binary(t::cff_charstring_size),
        ..BINARY
    },
    CheckSpec {
        id: "check.tables.baseTable",
        label: "BASE table",
        description: "The BASE table is missing or unreadable, its coordinate and tag counts differ, it uses unregistered baseline or script tags, offsets the default baseline, or places the ideographic baseline off the cap-height centre.",
        severity: Fail,
        profile: Strict,
        thresholds: &[t::BASE_TOLERANCE],
        codes: &[
            "missing-base",
            "unreadable",
            "coord-count-mismatch",
            "unregistered-baseline-tag",
            "unregistered-script-tag",
            "default-baseline-offset",
            "ideo-not-centred",
        ],
        code_severities: &[("default-baseline-offset", Warn), ("ideo-not-centred", Warn)],
        run: Runner::Binary(t::base_table),
        ..BINARY
    },
    CheckSpec {
        id: "check.tables.metaScriptLangTags",
        label: "Design and supported languages",
        description: "The meta table is missing or lacks its dlng or slng entry.",
        severity: Fail,
        profile: Strict,
        codes: &["lacks-meta-table", "missing-dlng-tag", "missing-slng-tag", "dlng-tag", "slng-tag"],
        code_severities: &[("lacks-meta-table", Warn), ("dlng-tag", Info), ("slng-tag", Info)],
        run: Runner::Binary(t::meta_script_lang_tags),
        ..BINARY
    },
    CheckSpec {
        id: "check.tables.statInStatics",
        label: "STAT in static fonts",
        description: "A static font's STAT table has more than one value for an axis.",
        severity: Fail,
        profile: Universal,
        codes: &["multiple-stat-entries"],
        preconditions: &[Static, Table("STAT")],
        run: Runner::Binary(t::stat_in_statics),
        ..BINARY
    },
    CheckSpec {
        id: "check.outlines.pointsOutsideBBox",
        label: "Points outside the bounding box",
        description: "A glyf glyph has a point outside its stored bounding box, or a coordinate beyond 32766 in magnitude.",
        group: Outlines,
        severity: Warn,
        profile: Opentype,
        fix: patch(true, "Recompute the glyph's stored bounding box."),
        capped: true,
        codes: &["points-out-of-bounds"],
        preconditions: &[TrueTypeOutlines],
        run: Runner::Binary(t::points_outside_bbox),
        ..BINARY
    },
];
