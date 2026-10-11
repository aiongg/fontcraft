//! The contexts the runners of the glyph, source, binary and family scopes receive. The layer
//! scope keeps [`crate::AuditContext`] with its per-layer input.

use std::collections::BTreeMap;

use fontcraft_model::Glyph;
use read_fonts::FontRef;

use crate::AuditContext;
use crate::services::AuditServices;

/// One glyph with all its masters (its layers), for checks that compare masters.
pub struct GlyphCtx<'a> {
    pub audit: &'a AuditContext<'a>,
    pub glyph: &'a Glyph,
    pub services: &'a dyn AuditServices,
}

/// The whole source.
pub struct SourceCtx<'a> {
    pub audit: &'a AuditContext<'a>,
    pub services: &'a dyn AuditServices,
}

/// How a compiled font maps back to the source it was compiled from, so binary findings can name
/// source glyphs, masters and Font Info fields (`audit.scopeResolution`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SourceMap {
    /// The source instance compiled; `None` for the variable font.
    pub instance: Option<String>,
    /// Source glyph name per glyph id.
    pub glyphs: Vec<String>,
    /// Compiled field (`OS/2.usWinAscent`) → the source field or custom parameter that produces
    /// it (`master.customParameters.winAscent`).
    pub fields: BTreeMap<String, String>,
}

impl SourceMap {
    /// The source glyph of a glyph id.
    pub fn glyph(&self, glyph_id: u32) -> Option<&str> {
        self.glyphs.get(usize::try_from(glyph_id).ok()?).map(String::as_str)
    }

    /// The source field behind `table.field`.
    pub fn field(&self, key: &str) -> Option<&str> {
        self.fields.get(key).map(String::as_str)
    }
}

/// The file a compiled font was read from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FontFile {
    /// The path as the run was given it.
    pub path: String,
    /// The file's size in bytes, before any WOFF unwrapping.
    pub size: u64,
}

impl FontFile {
    /// The file name: the path after its last slash or backslash.
    pub fn name(&self) -> &str {
        self.path.rsplit(['/', '\\']).next().unwrap_or(&self.path)
    }

    /// The directory: the path before its last slash or backslash, or `.` when it has none.
    pub fn directory(&self) -> &str {
        match self.path.rfind(['/', '\\']) {
            Some(0) => "/",
            Some(i) => self.path.get(..i).unwrap_or("."),
            None => ".",
        }
    }
}

/// One compiled font.
pub struct BinaryCtx<'a> {
    /// The font's id in the result document (`f0`, `f1`…).
    pub id: &'a str,
    /// The file's bytes (the sfnt, after any WOFF unwrapping).
    pub bytes: &'a [u8],
    pub font: FontRef<'a>,
    /// Set when the font was compiled from a source in this run.
    pub source: Option<&'a SourceMap>,
    /// Set when the font was read from a file; `None` for a font compiled in memory.
    pub file: Option<FontFile>,
    pub services: &'a dyn AuditServices,
}

impl<'a> BinaryCtx<'a> {
    /// Reads face `face` of `bytes`. Fails with a message when read-fonts cannot read the table
    /// directory.
    pub fn new(id: &'a str, bytes: &'a [u8], face: u32, services: &'a dyn AuditServices) -> Result<Self, String> {
        let font = FontRef::from_index(bytes, face).map_err(|e| format!("cannot read the font: {e}"))?;
        Ok(Self { id, bytes, font, source: None, file: None, services })
    }

    /// Records the file the font was read from.
    pub fn with_file(mut self, path: impl Into<String>, size: u64) -> Self {
        self.file = Some(FontFile { path: path.into(), size });
        self
    }
}

/// Two or more compiled fonts of one family (at most `MAX_FAMILY_FONTS` of `fontcraft-tables`,
/// checked by the caller).
pub struct FamilyCtx<'a> {
    pub fonts: &'a [BinaryCtx<'a>],
    pub services: &'a dyn AuditServices,
}
