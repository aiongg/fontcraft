//! The `Filter` and `PreFilter` export parameter string (M13.4 fills the bodies).
//!
//! Form: `Name; key:value; bare value; …; include:A,B` or `exclude:A,B`, parts separated by `;`
//! with surrounding spaces trimmed. The first part names the filter: a built-in filter of
//! `crates/compile`, or an installed module by name or id. `include:` and `exclude:` are applied
//! by FontCraft, not passed to the module (a guess recorded on `filter.customParameter.thirdParty`);
//! `key:value` parts become parameters by name and bare values parameters by position, in the
//! manifest's declaration order. Rows: `filter.customParameter.thirdParty`,
//! `filter.menu.pluginFiltersListed` (the export half).

use serde_json::{Map, Value};

pub use crate::error::{MAX_FILTER_PARTS, MAX_FILTER_STRING_BYTES};
use crate::manifest::Manifest;
use crate::{Error, Result};

/// Which glyphs an export filter runs on.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum GlyphScope {
    /// Every glyph.
    #[default]
    All,
    /// Only these glyphs (`include:`).
    Include(Vec<String>),
    /// Every glyph but these (`exclude:`).
    Exclude(Vec<String>),
}

/// One parsed filter string.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FilterCall {
    /// The filter or module name, as written.
    pub name: String,
    /// `key:value` parts, in order.
    pub named: Vec<(String, String)>,
    /// Bare values, in order.
    pub positional: Vec<String>,
    pub scope: GlyphScope,
}

/// Parses a filter string of at most [`MAX_FILTER_STRING_BYTES`] and [`MAX_FILTER_PARTS`] parts.
/// An empty name, both `include:` and `exclude:`, or a part over the limits is [`Error::Syntax`].
pub fn parse_filter_string(text: &str) -> Result<FilterCall> {
    if text.len() > MAX_FILTER_STRING_BYTES {
        return Err(Error::Syntax(format!("filter string is {} bytes (limit {MAX_FILTER_STRING_BYTES})", text.len())));
    }
    // M13.4.
    Err(Error::Unsupported("module filter strings".into()))
}

impl FilterCall {
    /// The parameter object for `manifest`: named values by key, bare values by declaration
    /// order, each converted to the parameter's type. Then checked by
    /// [`Manifest::validate_params`].
    pub fn params_for(&self, manifest: &Manifest) -> Result<Map<String, Value>> {
        let _ = manifest;
        // M13.4.
        Err(Error::Unsupported("module filter parameters".into()))
    }

    /// Whether the filter runs on `glyph`.
    pub fn applies_to(&self, glyph: &str) -> bool {
        match &self.scope {
            GlyphScope::All => true,
            GlyphScope::Include(names) => names.iter().any(|n| n == glyph),
            GlyphScope::Exclude(names) => !names.iter().any(|n| n == glyph),
        }
    }
}
