//! FontCraft font information (L2): what Font Info edits and what the export-time parameters
//! mean, as pure functions over a [`fontcraft_model::Font`]. No I/O. Plan: `docs/plans/M7.md`
//! § 3 and § 4 (M7.2 to M7.13). Behaviour notes: `docs/spec/06-font-info.md` and
//! `docs/spec/18-custom-parameters.md`; rows in `spec/inventory/06-font-info.toml` and
//! `spec/inventory/18-custom-parameters.toml`.
//!
//! | Module | Package | What it gives |
//! |---|---|---|
//! | [`catalog`] | M7.2 | every known custom parameter: name, aliases, value kind, scopes, effect, units flag |
//! | [`value`] | M7.2 | value checks per kind and per parameter syntax, JSON ↔ property-list values, typed readers |
//! | [`fields`] | M7.3 | the General fields (properties) each scope offers |
//! | [`target`] | M7.3 | scopes, and masters and instances addressed by id, name or index |
//! | [`rows`] | M7.3 | Font Info rows and their clipboard text (JSON and property list) |
//! | [`tokens`] | M7.13 | `{{{token}}}` text tokens and their expansion |
//! | [`scale`] | M7.4 | a font scaled to a new units-per-em |
//! | [`glyphset`] | M7.9 | the line grammar of Remove, Keep and Export Glyphs; Rename and Reencode lists |
//!
//! Who calls what: the engine's `font.customParameters.*`, `font.info.*`, `font.master.*` and
//! `font.instance.*` commands check and address through [`catalog`], [`value`], [`fields`] and
//! [`target`]; the engine's export steps (`engine/src/export_steps.rs`) run [`scale`]; the
//! compiler's export parameters (`compile/src/export_params.rs`) read [`glyphset`], and its name
//! table expands [`tokens`]. The Font Info window draws one value editor per [`catalog::ParamKind`].
//!
//! The limits of the plan's § 3 are the constants below; every function returns an error or a
//! reduced result instead of panicking when an input passes one.
#![forbid(unsafe_code)]

pub mod catalog;
mod catalog_data;
pub mod error;
pub mod fields;
pub mod glyphset;
pub mod rows;
pub mod scale;
pub mod target;
pub mod tokens;
pub mod value;

pub use catalog::{Effect, ParamKind, ParamSpec, Scopes};
pub use error::{PatternError, RowsError, ScaleError, TargetError, ValueError};
pub use target::{Scope, TargetRef};

// ---------- limits (`docs/plans/M7.md` § 3 › Limits) ----------

/// Most custom parameters on one font, master or instance.
pub const MAX_PARAMS_PER_SCOPE: usize = 4_096;
/// Most properties (General fields) on one font, master or instance.
pub const MAX_PROPERTIES_PER_SCOPE: usize = 1_024;
/// Most languages of one localized field.
pub const MAX_LANGUAGES: usize = 256;
/// Longest text of one field, in bytes (the compiler still cuts name strings shorter).
pub const MAX_FIELD_BYTES: usize = 65_535;
/// Longest text one parameter value holds, all strings together, in bytes.
pub const MAX_VALUE_BYTES: usize = 4 << 20;
/// Most items of one list or dictionary in a parameter value.
pub const MAX_VALUE_ITEMS: usize = 65_535;
/// Deepest nesting of a parameter value.
pub const MAX_VALUE_DEPTH: usize = fontcraft_plist::MAX_DEPTH;

/// Most masters of a font.
pub const MAX_MASTERS: usize = 256;
/// Most export entries (instances) of a font.
pub const MAX_INSTANCES: usize = 4_096;
/// Most instances of a project.
pub const MAX_PROJECT_INSTANCES: usize = 4_096;
/// Most metric definitions of a font.
pub const MAX_METRIC_DEFS: usize = 256;
/// Most stem definitions of a font.
pub const MAX_STEM_DEFS: usize = 256;

/// Units per em a font may have (the `head` table's range).
pub const UPM_RANGE: std::ops::RangeInclusive<u16> = fontcraft_model::UPM_RANGE;
/// Largest magnitude a scaled coordinate may reach; a scaling past it is refused.
pub const MAX_SCALED_COORDINATE: f64 = 32_768.0;
/// Weight Class of an instance (OS/2 `usWeightClass`).
pub const WEIGHT_CLASS_RANGE: std::ops::RangeInclusive<u16> = 1..=1000;
/// Width Class of an instance (OS/2 `usWidthClass`).
pub const WIDTH_CLASS_RANGE: std::ops::RangeInclusive<u16> = 1..=9;
/// Largest major version (as `font.info.set` takes it).
pub const MAX_VERSION_MAJOR: u16 = 32_767;
/// Largest minor version.
pub const MAX_VERSION_MINOR: u16 = 999;

/// Most lines of one glyph-set parameter (Remove, Keep, Export, Rename, Reencode Glyphs).
pub const MAX_GLYPHSET_LINES: usize = 65_535;
/// Longest line of a glyph-set parameter, in bytes.
pub const MAX_GLYPHSET_LINE_BYTES: usize = 1_024;
/// Most `*` in one glyph-name pattern; patterns are matched without backtracking.
pub const MAX_PATTERN_WILDCARDS: usize = 16;

/// Most tokens expanded in one field; later ones are kept as written.
pub const MAX_TOKENS: usize = 1_024;
/// Longest expanded text, in bytes; a token whose value would pass it is kept as written.
pub const MAX_EXPANDED_BYTES: usize = 65_535;

/// Most Filter and PreFilter strings run for one export, both together.
pub const MAX_FILTER_STRINGS: usize = 64;
/// Most Import Font parameters read for one font.
pub const MAX_IMPORT_FONT: usize = 16;
/// Most Import Master parameters read for one font.
pub const MAX_IMPORT_MASTER: usize = 16;
/// Most path segments of an Export Folder value.
pub const MAX_EXPORT_FOLDER_SEGMENTS: usize = 32;
/// Longest Export Folder value, in bytes.
pub const MAX_EXPORT_FOLDER_BYTES: usize = 1_024;

/// Most pairs in one subtable of the legacy `kern` table (its 16-bit search fields).
pub const MAX_KERN_SUBTABLE_PAIRS: usize = 10_920;
/// Most pairs in the legacy `kern` table; the rest are dropped with a warning.
pub const MAX_KERN_PAIRS: usize = 65_535;

/// Most rows one copy or paste of Font Info rows holds.
pub const MAX_ROWS: usize = 4_096;
/// Longest clipboard text of Font Info rows, in bytes.
pub const MAX_ROWS_BYTES: usize = 4 << 20;

#[cfg(test)]
mod tests {
    use crate::{Scope, Scopes, catalog};

    #[test]
    fn empty_scopes_allow_every_scope() {
        assert!([Scope::Font, Scope::Master, Scope::Instance].iter().all(|s| Scopes::ANY.allows(*s)));
        assert!(!Scopes::MASTER.allows(Scope::Font));
        assert!(catalog::lookup("").is_none());
    }
}
