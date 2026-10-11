//! Registry entries of the `check.files.*` checks (M12.57). Home profiles and fixes follow
//! `docs/spec/21-qa.md` § 8 (Files); `check.files.auditorVersion` is an M13 row.

use crate::Profile::{Strict, Universal};
use crate::Severity::{Fail, Info, Warn};
use crate::checks::files;
use crate::{CheckGroup, CheckScope, CheckSpec, FixSpec, Precondition, Runner};

/// The fields every file check shares; each entry sets the rest and takes these with `..FILES`.
const FILES: CheckSpec = CheckSpec {
    id: "",
    label: "",
    description: "",
    group: CheckGroup::Files,
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
    run: Runner::Binary(files::sfnt_version),
};

pub(super) const CHECKS: &[CheckSpec] = &[
    CheckSpec {
        id: "check.files.sfntVersion",
        label: "sfnt version",
        description: "The header's sfnt version matches the outlines: 0x00010000 for TrueType outlines, 'OTTO' for CFF and CFF2.",
        fix: Some(FixSpec {
            id: "patchTable",
            command: Some("tables.recalculate"),
            safe: true,
            description: "Rewrite the header's sfnt version to match the outline tables.",
        }),
        codes: &["wrong-sfnt-version-ttf", "wrong-sfnt-version-cff"],
        run: Runner::Binary(files::sfnt_version),
        ..FILES
    },
    CheckSpec {
        id: "check.files.fileSize",
        label: "File size",
        description: "The font file is larger than `warnSize` (a warning) or `failSize` (a failure).",
        thresholds: &[files::WARN_SIZE, files::FAIL_SIZE],
        codes: &["large-font", "massive-font"],
        code_severities: &[("large-font", Warn)],
        run: Runner::Binary(files::file_size),
        ..FILES
    },
    CheckSpec {
        id: "check.files.singleDirectory",
        label: "Family in one directory",
        description: "The files given as one family lie in more than one directory, so they may not belong together.",
        scope: CheckScope::Family,
        codes: &["single-directory"],
        run: Runner::Family(files::single_directory),
        ..FILES
    },
    CheckSpec {
        id: "check.files.sanitiser",
        label: "Table sanitiser",
        description: "A strict parse of the header, the table directory and every table, as a browser's sanitiser makes it, plus an external sanitiser's messages when one is installed and asked for.",
        codes: &[
            "bad-sfnt-version",
            "no-tables",
            "bad-search-fields",
            "unsorted-tables",
            "table-out-of-bounds",
            "misaligned-table",
            "overlapping-tables",
            "missing-table",
            "bad-table",
            "bad-glyph",
            "sanitiser-error",
            "sanitiser-warning",
        ],
        code_severities: &[("bad-search-fields", Warn), ("sanitiser-warning", Warn)],
        run: Runner::Binary(files::sanitiser),
        ..FILES
    },
    CheckSpec {
        id: "check.files.roundTrip",
        label: "Text dump round trip",
        description: "Dumping every table to the text format and reading the dump back fails.",
        codes: &["round-trip-failed"],
        run: Runner::Binary(files::round_trip),
        ..FILES
    },
    CheckSpec {
        id: "check.files.rasterises",
        label: "Glyphs draw",
        description: "Every glyph draws at `ppem` pixels per em, unhinted and, when the font has hints, hinted.",
        thresholds: &[files::RASTER_PPEM],
        codes: &["no-outline", "draw-failed", "hinting-failed", "hinted-draw-failed"],
        run: Runner::Binary(files::rasterises),
        ..FILES
    },
    CheckSpec {
        id: "check.files.externalValidator",
        label: "External validator",
        description: "The findings of the external validator at `validatorPath`, less its known false positives.",
        severity: Warn,
        profile: Strict,
        codes: &["validator-error", "validator-warning", "validator-info"],
        code_severities: &[("validator-info", Info)],
        preconditions: &[Precondition::ProjectValue("validatorPath")],
        run: Runner::Binary(files::external_validator),
        ..FILES
    },
    CheckSpec {
        id: "check.files.canonicalFileName",
        label: "Canonical file name",
        description: "The file name is built from the font's names: Family-Style.ext for a static font, Family[axis tags].ext for a variable one.",
        profile: Strict,
        codes: &["bad-filename"],
        run: Runner::Binary(files::canonical_file_name),
        ..FILES
    },
];
