//! What the code editor of the Features tab asks about the code (M8.15): syntax colouring,
//! completion by position, and the fixes offered for an error.
//!
//! Rows: `fontInfo.features.syntaxColouring` (the token classes), `.autocomplete` (the data),
//! `font.features.complete`, `font.features.applyFix`.
//!
//! The code is untrusted and at most [`crate::MAX_ENTRY_BYTES`]; every function works in one
//! pass over it and returns at most a bounded number of items.

use std::ops::Range;

use crate::source_map::EntryDiagnostic;

/// Most completions one call returns.
pub const MAX_COMPLETIONS: usize = 1_000;
/// Most fixes offered for one diagnostic.
pub const MAX_FIXES: usize = 16;

/// What a stretch of code is, for colouring.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum HighlightKind {
    Keyword,
    /// A feature, script or language tag.
    Tag,
    GlyphName,
    /// `@Name`.
    ClassName,
    LookupName,
    Number,
    String,
    Comment,
    /// `$name`, `${…}`, `$[…]`.
    Token,
    Punctuation,
}

/// One coloured stretch.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Highlight {
    pub range: Range<usize>,
    pub kind: HighlightKind,
}

/// The stretches of `code` to colour, in order, not overlapping. Text not covered is plain.
pub fn highlight(code: &str) -> Vec<Highlight> {
    // M8.15
    let _ = code;
    Vec::new()
}

/// What a completion inserts.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CompletionKind {
    Glyph,
    Class,
    Lookup,
    Keyword,
    Tag,
    Number,
}

/// One completion: `label` replaces `replace` (the partial word before the cursor).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Completion {
    pub label: String,
    pub kind: CompletionKind,
    pub replace: Range<usize>,
}

/// The names completions are drawn from: the font's glyphs, the classes and lookups of all
/// entries, the number values.
#[derive(Clone, Copy, Debug, Default)]
pub struct CompletionNames<'a> {
    pub glyphs: &'a [String],
    pub classes: &'a [String],
    pub lookups: &'a [String],
    pub numbers: &'a [String],
}

/// The completions at byte offset `cursor` of `code`: glyphs after `sub` and `pos`, lookups
/// after `lookup`, classes after `@`, numbers after `$`; narrowed by the partial word before the
/// cursor; at most [`MAX_COMPLETIONS`].
pub fn complete(code: &str, cursor: usize, names: &CompletionNames) -> Vec<Completion> {
    // M8.15
    let _ = (code, cursor, names);
    Vec::new()
}

/// A fix: edits to the entry's code, as (byte range of the code, replacement), not overlapping,
/// in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fix {
    pub label: String,
    pub edits: Vec<(Range<usize>, String)>,
}

/// The fixes offered for `diagnostic` in `code` (the code of the entry it points at); empty
/// when there are none. Each gives code that compiles past that error.
pub fn fixes(code: &str, diagnostic: &EntryDiagnostic) -> Vec<Fix> {
    // M8.15
    let _ = (code, diagnostic);
    Vec::new()
}
