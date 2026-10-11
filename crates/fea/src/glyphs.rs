//! The glyph order the code's glyph names resolve against.

use std::collections::HashMap;

use crate::{Diagnostic, GlyphId16, MAX_GLYPHS, Severity};

/// The compiled font's glyphs by name, in glyph-id order: a name's index is its glyph id
/// (`.notdef` first). Names are the source's glyph names, which the feature code uses; the
/// compiler maps them to production names afterwards.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GlyphOrder {
    names: Vec<String>,
    ids: HashMap<String, u16>,
}

impl GlyphOrder {
    /// Refused: more than [`MAX_GLYPHS`] names, or a name given twice.
    pub fn new(names: Vec<String>) -> Result<Self, Diagnostic> {
        if names.len() > MAX_GLYPHS {
            return Err(Diagnostic::request(Severity::Error, format!("the font has more than {MAX_GLYPHS} glyphs")));
        }
        let mut ids = HashMap::with_capacity(names.len());
        for (i, name) in names.iter().enumerate() {
            let id = u16::try_from(i).map_err(|_| Diagnostic::request(Severity::Error, format!("the font has more than {MAX_GLYPHS} glyphs")))?;
            if ids.insert(name.clone(), id).is_some() {
                return Err(Diagnostic::request(Severity::Error, format!("the glyph order names `{name}` twice")));
            }
        }
        Ok(Self { names, ids })
    }

    pub fn id(&self, name: &str) -> Option<GlyphId16> {
        self.ids.get(name).map(|&id| GlyphId16::new(id))
    }

    pub fn name(&self, id: GlyphId16) -> Option<&str> {
        self.names.get(usize::from(id.to_u16())).map(String::as_str)
    }

    pub fn names(&self) -> &[String] {
        &self.names
    }

    pub fn len(&self) -> usize {
        self.names.len()
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }
}
