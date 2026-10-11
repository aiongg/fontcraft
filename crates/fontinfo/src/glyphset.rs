//! The values of the glyph-set parameters: Remove Glyphs, Keep Glyphs and Export Glyphs (one entry
//! per line: a glyph name, a name pattern with `*`, or a `key=value` query), Rename Glyphs
//! (`old=new`) and Reencode Glyphs (`glyph=HEX[,HEX…]`). The grammar and the query keys are in
//! `docs/spec/18-custom-parameters.md` › Remove Glyphs and Keep Glyphs.
//!
//! Patterns are matched without backtracking, with at most [`crate::MAX_PATTERN_WILDCARDS`] `*`.
//! A value holds at most [`crate::MAX_GLYPHSET_LINES`] lines of
//! [`crate::MAX_GLYPHSET_LINE_BYTES`] each. Blank lines are skipped.
//!
//! The compiler (`compile/src/export_params.rs`) matches resolved glyphs; the engine may match
//! source glyphs. Both describe a glyph through [`GlyphFacts`].
//!
//! Rows: `Export Glyphs`, `Keep Glyphs`, `Remove Glyphs`, `Rename Glyphs`, `Reencode Glyphs` (M7.9).

use crate::error::PatternError;

/// Which metrics key a query reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MetricsSide {
    Left,
    Right,
    Width,
    Top,
    Bottom,
    VertWidth,
}

/// Which kerning group a query reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KernSide {
    Left,
    Right,
    Top,
    Bottom,
}

/// What a query may ask of a glyph, one method per query key.
pub trait GlyphFacts {
    fn name(&self) -> &str;
    /// The code points, first one first (`unicode=` matches the first).
    fn code_points(&self) -> &[u32];
    fn note(&self) -> Option<&str>;
    /// The script as Glyph Info shows it (`latin`, `greek`).
    fn script(&self) -> Option<&str>;
    fn category(&self) -> Option<&str>;
    fn sub_category(&self) -> Option<&str>;
    fn production_name(&self) -> Option<&str>;
    fn metrics_key(&self, side: MetricsSide) -> Option<&str>;
    fn kerning_group(&self, side: KernSide) -> Option<&str>;
    /// Every layer, master layers included.
    fn layer_count(&self) -> usize;
    /// Whether the glyph interpolates across all masters.
    fn masters_compatible(&self) -> bool;
    fn exports(&self) -> bool;
    /// Whether some colour layer uses palette entry `index`.
    fn uses_color_index(&self, index: i64) -> bool;
    /// Whether the glyph has colour data of any kind (palette layers, bitmap colour, full colour).
    fn is_any_color_glyph(&self) -> bool;
    /// Whether the glyph has a bitmap colour layer.
    fn is_apple_color_glyph(&self) -> bool;
    /// Whether the glyph has an alternate (bracket) or intermediate (brace) layer.
    fn has_special_layers(&self) -> bool;
}

/// One entry of Remove, Keep or Export Glyphs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GlyphPattern {
    // M7.9: the parsed entry (name, wildcard parts, or query key and value).
    line: String,
}

impl GlyphPattern {
    /// Reads one entry (already trimmed, not blank). `line` in errors is 1.
    pub fn parse(line: &str) -> Result<Self, PatternError> {
        if line.len() > crate::MAX_GLYPHSET_LINE_BYTES {
            return Err(PatternError::LimitExceeded { what: "bytes in one line", limit: crate::MAX_GLYPHSET_LINE_BYTES });
        }
        // M7.9: names, `*` patterns and queries.
        Err(PatternError::NotYetImplemented("glyph-set patterns"))
    }

    /// Whether the glyph matches.
    pub fn matches(&self, glyph: &dyn GlyphFacts) -> bool {
        // M7.9: matching without backtracking.
        let _ = (glyph, &self.line);
        false
    }
}

/// A whole Remove, Keep or Export Glyphs value.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GlyphList {
    pub patterns: Vec<GlyphPattern>,
}

impl GlyphList {
    /// Reads every line (a list value's items, or the lines of a text). Errors name the line.
    pub fn parse(lines: &[&str]) -> Result<Self, PatternError> {
        if lines.len() > crate::MAX_GLYPHSET_LINES {
            return Err(PatternError::LimitExceeded { what: "lines", limit: crate::MAX_GLYPHSET_LINES });
        }
        let mut patterns = Vec::new();
        for (i, line) in lines.iter().enumerate() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            patterns.push(GlyphPattern::parse(line).map_err(|e| renumber(e, i + 1))?);
        }
        Ok(Self { patterns })
    }

    /// Whether any entry matches the glyph.
    pub fn matches(&self, glyph: &dyn GlyphFacts) -> bool {
        self.patterns.iter().any(|p| p.matches(glyph))
    }
}

/// A Rename Glyphs value: (old, new) pairs in order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RenameList {
    pub pairs: Vec<(String, String)>,
}

impl RenameList {
    /// Reads `old=new` lines; blank lines are skipped.
    pub fn parse(lines: &[&str]) -> Result<Self, PatternError> {
        if lines.len() > crate::MAX_GLYPHSET_LINES {
            return Err(PatternError::LimitExceeded { what: "lines", limit: crate::MAX_GLYPHSET_LINES });
        }
        // M7.9: the line grammar.
        Err(PatternError::NotYetImplemented("Rename Glyphs"))
    }
}

/// A Reencode Glyphs value: (glyph, code points) in order; no code points clears the glyph's.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ReencodeList {
    pub entries: Vec<(String, Vec<u32>)>,
}

impl ReencodeList {
    /// Reads `glyph=HEX[,HEX…]` lines; blank lines are skipped.
    pub fn parse(lines: &[&str]) -> Result<Self, PatternError> {
        if lines.len() > crate::MAX_GLYPHSET_LINES {
            return Err(PatternError::LimitExceeded { what: "lines", limit: crate::MAX_GLYPHSET_LINES });
        }
        // M7.9: the line grammar, code points within Unicode.
        Err(PatternError::NotYetImplemented("Reencode Glyphs"))
    }
}

/// An error from one line, with its line number in the whole value.
fn renumber(e: PatternError, line: usize) -> PatternError {
    match e {
        PatternError::Syntax { message, .. } => PatternError::Syntax { line, message },
        PatternError::TooManyWildcards { limit, .. } => PatternError::TooManyWildcards { line, limit },
        other => other,
    }
}
