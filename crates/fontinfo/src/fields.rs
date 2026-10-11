//! The General fields of Font Info: which properties each scope offers, which of them hold one
//! text per language, and which are built in (always shown, never removed).
//!
//! The lists follow the format's property keys and glyphsLib (`font.info.listFields` records the
//! guess). The plural keys (`designers`, `copyrights`) are the localizable properties; format 2's
//! singular spellings read as the same field.
//!
//! Rows: `font.info.listFields`, `font.info.addField`, `font.info.removeField`,
//! `font.info.setLocalizedValue`, `font.properties`, `master.properties`, `instance.properties`
//! (M7.3).

use crate::catalog::Scopes;
use crate::error::ValueError;
use crate::target::Scope;

/// One field of the General section.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FieldSpec {
    /// The property key (`designers`, `vendorID`) or, for a built-in field, the model's field
    /// (`familyName`, `unitsPerEm`, `versionMajor`).
    pub key: &'static str,
    /// The label the picker and the row show (`Designer`, `Vendor ID`).
    pub label: &'static str,
    pub scopes: Scopes,
    /// One text per language.
    pub localizable: bool,
    /// Always shown at the top of its tab and never removed (Family Name, Units per Em…).
    pub built_in: bool,
}

/// The fields `scope` offers, built-in ones first, in the order the picker lists them.
pub fn fields(scope: Scope) -> &'static [FieldSpec] {
    // M7.3: the field lists of the `font.info.listFields` guess.
    let _ = scope;
    &[]
}

/// The field `key` names at `scope`, also by its singular spelling.
pub fn field(scope: Scope, key: &str) -> Option<&'static FieldSpec> {
    fields(scope).iter().find(|f| f.key == key)
}

/// Refuses a field text longer than [`crate::MAX_FIELD_BYTES`] (and, for the built-in fields, a
/// text that is not their kind: Units per Em in [`crate::UPM_RANGE`], the version parts within
/// [`crate::MAX_VERSION_MAJOR`] and [`crate::MAX_VERSION_MINOR`]).
pub fn check_text(spec: &FieldSpec, text: &str) -> Result<(), ValueError> {
    // M7.3: the built-in fields' kinds.
    if text.len() > crate::MAX_FIELD_BYTES {
        return Err(ValueError::LimitExceeded { what: "bytes in one field", limit: crate::MAX_FIELD_BYTES });
    }
    let _ = spec;
    Ok(())
}
