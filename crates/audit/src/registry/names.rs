//! Registry entries of the `check.names.*` checks (M12.50). Home profiles and fixes follow
//! `docs/spec/21-qa.md` § 8 (Name table); `check.names.nameCollision` is an M13 row.

use crate::Profile::{Opentype, Strict, Universal};
use crate::Severity::{Fail, Info, Warn};
use crate::checks::names;
use crate::{CheckGroup, CheckScope, CheckSpec, FixSpec, Precondition, Runner};

const NAME_TABLE: &[Precondition] = &[Precondition::Table("name")];

/// The fields every name check shares; each entry sets the rest and takes these with `..NAMES`.
const NAMES: CheckSpec = CheckSpec {
    id: "",
    label: "",
    description: "",
    group: CheckGroup::Names,
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
    preconditions: NAME_TABLE,
    run: Runner::Binary(names::required_records),
};

const CLEAN_NAME_RECORDS: Option<FixSpec> = Some(FixSpec {
    id: "cleanNameRecords",
    command: Some("tables.cleanNameRecords"),
    safe: true,
    description: "Trim and collapse white space, drop empty and Macintosh records, and make the PostScript names agree.",
});

pub(super) const CHECKS: &[CheckSpec] = &[
    CheckSpec {
        id: "check.names.requiredRecords",
        label: "Required name records",
        description: "The Windows English family, subfamily, full and PostScript names exist and are not empty; version, copyright, trademark, manufacturer and licence URL should exist.",
        codes: &["missing", "empty", "missing-recommended"],
        code_severities: &[("missing-recommended", Warn)],
        run: Runner::Binary(names::required_records),
        ..NAMES
    },
    CheckSpec {
        id: "check.names.emptyRecords",
        label: "Empty name records",
        description: "A name record holds no text or only white space.",
        profile: Opentype,
        fix: CLEAN_NAME_RECORDS,
        codes: &["empty-record"],
        run: Runner::Binary(names::empty_records),
        ..NAMES
    },
    CheckSpec {
        id: "check.names.whitespace",
        label: "White space in names",
        description: "A name record starts or ends with white space, or a menu name holds two spaces in a row.",
        fix: CLEAN_NAME_RECORDS,
        codes: &["trailing-space", "leading-space", "double-space"],
        run: Runner::Binary(names::whitespace),
        ..NAMES
    },
    CheckSpec {
        id: "check.names.lineBreaks",
        label: "Line breaks in names",
        description: "A name record contains a line break.",
        profile: Strict,
        fix: CLEAN_NAME_RECORDS,
        codes: &["line-break"],
        run: Runner::Binary(names::line_breaks),
        ..NAMES
    },
    CheckSpec {
        id: "check.names.postScriptNameCharacters",
        label: "PostScript name characters",
        description: "The PostScript name uses only A-Z, a-z, 0-9 and hyphen; the CID name and the variations prefix use their allowed characters.",
        profile: Opentype,
        codes: &["bad-psname", "bad-string"],
        run: Runner::Binary(names::postscript_name_characters),
        ..NAMES
    },
    CheckSpec {
        id: "check.names.postScriptNameConsistency",
        label: "PostScript name consistency",
        description: "Every PostScript name record holds the same name, and it equals the CFF font name.",
        profile: Opentype,
        fix: CLEAN_NAME_RECORDS,
        codes: &["inconsistency", "cff-name-error", "ps-cff-name-mismatch"],
        run: Runner::Binary(names::postscript_name_consistency),
        ..NAMES
    },
    CheckSpec {
        id: "check.names.fullNameStartsWithFamily",
        label: "Full name starts with family",
        description: "The full name begins with the family name of the same platform and language.",
        profile: Opentype,
        codes: &["mismatch-font-names", "missing-font-names", "cff-full-name"],
        code_severities: &[("cff-full-name", Info)],
        run: Runner::Binary(names::full_name_starts_with_family),
        ..NAMES
    },
    CheckSpec {
        id: "check.names.recommendedLengths",
        label: "Recommended name lengths",
        description: "Full and PostScript names stay within 63 characters, family and style names within 31.",
        severity: Info,
        profile: Opentype,
        codes: &["too-long"],
        run: Runner::Binary(names::recommended_lengths),
        ..NAMES
    },
    CheckSpec {
        id: "check.names.familyAndStyleLength",
        label: "Family and style length",
        description: "The family name, the family name with a style, or a named instance is longer than 31 characters.",
        codes: &["nameid1-too-long", "long-name", "familyname-plus-stat-entries-too-long", "fvar-instance-too-long"],
        run: Runner::Binary(names::family_and_style_length),
        ..NAMES
    },
    CheckSpec {
        id: "check.names.menuNameLimits",
        label: "Menu name limits",
        description: "A menu name (IDs 1, 2, 4, 16, 17, 18) is longer than 63 characters.",
        profile: Strict,
        codes: &["too-long"],
        run: Runner::Binary(names::menu_name_limits),
        ..NAMES
    },
    CheckSpec {
        id: "check.names.versionString",
        label: "Version string",
        description: "The version string can be read and matches head.fontRevision.",
        profile: Opentype,
        fix: Some(FixSpec {
            id: "syncVersion",
            command: Some("font.info.syncVersion"),
            safe: false,
            description: "Write the version string from the font revision.",
        }),
        codes: &["missing", "parse", "mismatch", "near-mismatch", "development", "version-format", "version-decimals", "version-not-above-one"],
        code_severities: &[
            ("near-mismatch", Warn),
            ("development", Info),
            ("version-format", Info),
            ("version-decimals", Info),
            ("version-not-above-one", Info),
        ],
        preconditions: &[Precondition::Table("name"), Precondition::Table("head")],
        run: Runner::Binary(names::version_string),
        ..NAMES
    },
    CheckSpec {
        id: "check.names.italicNames",
        label: "Italic names",
        description: "The names of an italic font follow the italic naming rules.",
        codes: &["bad-familyname", "bad-subfamilyname", "bad-typographicfamilyname", "bad-typographicsubfamilyname"],
        run: Runner::Binary(names::italic_names),
        ..NAMES
    },
    CheckSpec {
        id: "check.names.subfamilyMatchesStyleBits",
        label: "Subfamily matches style bits",
        description: "The Windows subfamily name is the one the bold and italic bits call for.",
        fix: Some(FixSpec {
            id: "syncStyleLinking",
            command: Some("font.instance.syncStyleLinking"),
            safe: false,
            description: "Derive fsSelection, macStyle and the subfamily name from the style-linking settings.",
        }),
        codes: &["mismatch"],
        run: Runner::Binary(names::subfamily_matches_style_bits),
        ..NAMES
    },
    CheckSpec {
        id: "check.names.noMacRecords",
        label: "No Macintosh name records",
        description: "The name table has Macintosh-platform records.",
        fix: CLEAN_NAME_RECORDS,
        codes: &["mac-names"],
        run: Runner::Binary(names::no_mac_records),
        ..NAMES
    },
    CheckSpec {
        id: "check.names.macRecordsComplete",
        label: "Macintosh name records complete",
        description: "A font that keeps Macintosh name records has all of them.",
        profile: Strict,
        codes: &["missing-mac-record", "missing-optional-mac-record"],
        code_severities: &[("missing-optional-mac-record", Warn)],
        preconditions: &[Precondition::Table("name"), Precondition::ProjectValue("keepMacNames")],
        run: Runner::Binary(names::mac_records_complete),
        ..NAMES
    },
    CheckSpec {
        id: "check.names.copyrightInDescription",
        label: "Copyright in description",
        description: "The description contains copyright wording, or is very long.",
        thresholds: &[names::MAX_DESCRIPTION_LENGTH],
        codes: &["copyright-on-description", "too-long"],
        code_severities: &[("too-long", Warn)],
        run: Runner::Binary(names::copyright_in_description),
        ..NAMES
    },
    CheckSpec {
        id: "check.names.legalAndCredits",
        label: "Legal and credits",
        description: "Copyright, trademark, designer note and foundry URL are present, the copyright holds no patent number, and the URL and vendor ID match the project's values when they are set.",
        profile: Strict,
        codes: &[
            "missing-copyright",
            "missing-trademark",
            "missing-designer-note",
            "missing-vendor-url",
            "patent-number",
            "url-mismatch",
            "vendor-id-mismatch",
        ],
        code_severities: &[("missing-copyright", Warn), ("missing-trademark", Warn)],
        run: Runner::Binary(names::legal_and_credits),
        ..NAMES
    },
    CheckSpec {
        id: "check.names.familyNameStartsWithDigit",
        label: "Family name starts with digit",
        description: "The family name begins with a digit.",
        profile: Strict,
        codes: &["begins-with-digit"],
        run: Runner::Binary(names::family_name_starts_with_digit),
        ..NAMES
    },
];
