//! The reverse component index (M6.1): for each glyph, the glyphs that use it as a component,
//! directly and through other composites; a dependency order; the cycles.
//!
//! Rows: `components.liveUpdateFromBaseGlyph` (which glyphs follow an edit),
//! `components.nestingAllowed`, `components.placeholderCircularReference` (detection).
//!
//! Contract:
//! - Built from a whole [`Font`]; the components of every layer count, master layers and others.
//!   A component whose base glyph does not exist is listed in
//!   [`ComponentIndex::missing_bases`] and adds no edge.
//! - [`ComponentIndex::dependency_order`] lists every glyph that is not in a cycle, each after
//!   all its bases; glyphs without components keep their order in the font. Glyphs in a cycle,
//!   and the glyphs that use them, are left out of it and named in [`ComponentIndex::cycles`].
//! - The index is a snapshot. [`ComponentIndex::is_current`] tells whether a font still has the
//!   same glyphs (by `Arc` pointer); the engine rebuilds the index when it does not.
//! - Work is linear in glyphs plus components; nothing recurses without a depth limit.

use std::collections::HashMap;
use std::sync::Arc;

use fontcraft_model::{Font, Glyph};

/// See the module doc.
#[derive(Clone, Debug, Default)]
pub struct ComponentIndex {
    /// The glyph pointers the index was built from, in font order.
    glyphs: Vec<Arc<Glyph>>,
    /// Base glyph → the glyphs with a component of it, each once, in font order.
    users: HashMap<String, Vec<String>>,
    order: Vec<String>,
    cycles: Vec<Vec<String>>,
    /// (glyph, base name) for each component whose base glyph is missing.
    missing: Vec<(String, String)>,
}

impl ComponentIndex {
    /// The index of `font`.
    pub fn new(font: &Font) -> Self {
        // M6.1: build the reverse edges, the dependency order and the cycles.
        let _ = font;
        Self::default()
    }

    /// Whether `font` has exactly the glyph pointers this index was built from.
    pub fn is_current(&self, font: &Font) -> bool {
        // M6.1: compare the glyph `Arc`s.
        let _ = (font, &self.glyphs);
        false
    }

    /// No glyph uses another.
    pub fn is_empty(&self) -> bool {
        self.users.is_empty()
    }

    /// The glyphs with a component of `glyph`, in font order.
    pub fn direct_users(&self, glyph: &str) -> &[String] {
        self.users.get(glyph).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Every glyph that uses `glyph`, directly or through other composites, each once, in
    /// dependency order.
    pub fn users(&self, glyph: &str) -> Vec<String> {
        // M6.1: walk `users` breadth-first, then sort by `order`.
        let _ = glyph;
        Vec::new()
    }

    /// The transitive users of all of `glyphs` together, each once, in dependency order. The
    /// glyphs themselves are not included unless one uses another.
    pub fn dependents<'a>(&self, glyphs: impl IntoIterator<Item = &'a str>) -> Vec<String> {
        // M6.1
        let _ = glyphs.into_iter().count();
        Vec::new()
    }

    /// Every glyph not in or above a cycle, each after its bases.
    pub fn dependency_order(&self) -> &[String] {
        &self.order
    }

    /// Each cycle as the glyph names along it, starting from the one earliest in the font.
    pub fn cycles(&self) -> &[Vec<String>] {
        &self.cycles
    }

    /// Whether `glyph` is on a cycle or uses a glyph that is.
    pub fn in_cycle(&self, glyph: &str) -> bool {
        // M6.1
        let _ = glyph;
        false
    }

    /// (glyph, missing base name) for each component whose base glyph does not exist.
    pub fn missing_bases(&self) -> &[(String, String)] {
        &self.missing
    }
}
