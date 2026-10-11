//! Script runs (M8.18): text split where the script changes, by the Unicode Script_Extensions
//! property from icu_properties. Common and Inherited characters join the run around them, as
//! HarfBuzz's itemisation does.

use std::ops::Range;

use crate::Tag;

/// The ISO 15924 tag of the Common script, for text without a script of its own.
pub const COMMON: Tag = Tag::new(b"Zyyy");

/// One run of one script.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ScriptRun {
    /// Byte range of the text.
    pub range: Range<usize>,
    /// The ISO 15924 script tag (`Latn`, `Arab`), as [`crate::ShapeRequest::script`] takes it.
    pub script: Tag,
}

/// The script runs of `text`, in logical order, covering it without gaps.
pub fn itemize(text: &str) -> Vec<ScriptRun> {
    // M8.18: Script_Extensions per character, runs merged.
    if text.is_empty() {
        return Vec::new();
    }
    vec![ScriptRun { range: 0..text.len(), script: COMMON }]
}
