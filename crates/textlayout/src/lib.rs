//! FontCraft text layout (L2): the text of an Edit View tab or a preview as a list of items, and
//! the layout of those items. Pure functions over a [`fontcraft_model::Font`]; no I/O. Edit View,
//! the previews, Text Preview, `ui.render` and hit testing use it. Behaviour notes:
//! `docs/spec/03b-edit-view-display.md` › Text mode and `docs/spec/08-spacing-kerning.md` § 3.2;
//! rows `text.*`, `kerning.mode*` and `preview.singleLine`.
//!
//! # Text ([`text`])
//! An [`Item`] is a glyph (with the character it was typed as, if any), a character no glyph
//! encodes, a placeholder or a line break. [`parse`] reads a display string: characters map to
//! glyphs by Unicode value, `/name` names a glyph (a space after the name ends it and is
//! dropped), `/Placeholder` is a placeholder, a line break is a line break.
//! [`Text::to_display_string`] writes it back; [`Text::to_plain_text`] gives the characters. A
//! [`Text`] holds the items, the cursor, the selection and the current glyph (the glyph after the
//! cursor, which placeholders show), and edits within its item limit ([`MAX_ITEMS`] for a tab,
//! up to [`MAX_PREVIEW_ITEMS`] for Text Preview). A failed edit changes nothing.
//!
//! # Layout ([`mod@layout`])
//! [`layout()`] places every item: advances from the master whose metrics the master shows,
//! kerning by [`KerningMode`] from [`fontcraft_spacing::kerning::KernIndex`], line breaks, and
//! wrapping at the line width (after the last space of the line, else before the glyph that
//! overflows). Left to right runs from x = 0 rightwards; right to left ends each line at x = 0
//! and runs leftwards in logical order, so the visual order is reversed; top to bottom uses
//! vertical widths and origins, with columns from right to left. [`Layout::hit`] turns a point
//! into a cursor index, [`Layout::caret`] a cursor index into a point.
//!
//! # Shaping ([`shape`], [`cursive`])
//! With [`Options::shaping`] (Edit View), the items first go through the compiled preview font:
//! the substitution features active in the tab replace their glyphs (a ligature shows on its first
//! component, the others take no room; a glyph that becomes several shows them all on its item),
//! while advances and kerning stay live from the model. With [`Options::cursive`], right-to-left
//! text attaches each glyph's `entry` anchor to the previous glyph's `exit` anchor instead of
//! kerning them.
//!
//! # Mixed directions ([`bidi`])
//! With [`Options::bidi`] (Text Preview), horizontal text is ordered by the Unicode
//! Bidirectional Algorithm (UAX #9): a Latin word in right-to-left text keeps its letters in
//! order, and "12 34" there reads "34 12". [`Embedding`]s give ranges their own base direction.
//! Kerning applies between neighbours in the same run, from the table of that run's direction.
//! [`Layout::levels`] keeps the levels. Hit testing assumes one direction per line.
//!
//! # Text Preview shaping ([`line_shape`])
//! [`shape_line`] shapes one line of such a layout through the full preview font: each run of
//! one level in its own direction, with the script's default features and the compiled
//! positioning, advances live from the model (`live.rs`). A vertical layout's columns are shaped
//! top to bottom with [`vertical_features`], vertical advances and origins live from the model.
//!
//! # Line breaking ([`linebreak`], [`measure`])
//! With [`Options::break_opportunities`] (Text Preview), lines break at the opportunities of the
//! Unicode Line Breaking Algorithm (UAX #14): after spaces and hyphens, between ideographs, never
//! before closing punctuation; a word wider than the line breaks between two clusters. With
//! [`Options::measure`], the items are first measured through the full preview font, so kerning
//! and substitutions decide where lines end. Levels come from the whole paragraph, lines break,
//! then each line is reordered (UAX #9).
//!
//! The public API:
//! ```text
//! MAX_ITEMS = 10_000;  MAX_PREVIEW_ITEMS = 1_000_000;  PLACEHOLDER = "Placeholder"
//! Item::{Glyph { name: Arc<str>, typed: Option<char> }, Missing(char), Placeholder, LineBreak};  Item::glyph(name: &str) -> Item
//! Charmap::new(font: &Font) -> Charmap;  Charmap::glyph(&self, c: char) -> Option<&Arc<str>>;  Charmap::item(&self, c: char) -> Item
//! parse(font: &Font, s: &str, limit: usize) -> Result<Vec<Item>, TextError>
//! parse_with(map: &Charmap, s: &str, limit: usize) -> Result<Vec<Item>, TextError>
//! parse_prefix(map: &Charmap, s: &str, limit: usize) -> Result<(Vec<Item>, bool more), TextError>
//! Text::new() -> Text;  Text::with_limit(limit: usize) -> Result<Text, TextError>
//! Text::from_display_string(font: &Font, s: &str) -> Result<Text, TextError>
//! Text::items(&self) -> &[Item];  len;  is_empty;  limit;  cursor(&self) -> usize;  selection(&self) -> Range<usize>
//! Text::set_cursor(&mut self, index: usize);  Text::select(&mut self, anchor: usize, cursor: usize)
//! Text::replace(&mut self, range: Range<usize>, items: Vec<Item>) -> Result<(), TextError>
//! Text::insert(&mut self, index: usize, items: Vec<Item>) -> Result<(), TextError>
//! Text::delete(&mut self, range: Range<usize>) -> Result<(), TextError>
//! Text::layer(&self, index: usize) -> Option<&str>;  Text::layer_of(&self, index: usize, name: &str) -> Option<&str>
//! Text::set_layer(&mut self, index: usize, layer: Option<&str>) -> bool;  Text::clear_layers(&mut self)
//! Text::current_glyph(&self) -> Option<&str>;  Text::set_current_glyph(&mut self, name: Option<&str>)
//! Text::to_display_string(&self, font: &Font) -> String;  Text::to_plain_text(&self, font: &Font) -> String
//! KerningMode::{None, Kerning, Locked}: name, from_name, shows_kerning
//! Options<'a> { direction: Direction, kerning: KerningMode, line_width: f64, line_height: Option<f64>,
//!               single_line: bool, missing_advance: Option<f64>, index: Option<&'a KernIndex>,
//!               layers: Option<&'a BTreeMap<String, String>> (glyph → layer id),
//!               bidi: bool, embeddings: &'a [Embedding], shaping: Option<Shaping<'a>>, cursive: bool }  (Default)
//! PreviewShaper::new(bytes: Arc<[u8]>, glyph_order: &[String], private_use: &[(u32, u16)], features: &[Tag]) -> Result<PreviewShaper, ShapeError>
//! PreviewShaper::features(&self) -> &[Tag];  glyph_name(&self, gid: u16) -> Option<&Arc<str>>;  has_glyph(&self, name: &str) -> bool
//! PreviewShaper::is_shaped(&self, font: &Font, text: &Text, direction: Direction, active: &[Tag], localized: Option<(Tag, Tag)>) -> bool
//! PreviewShaper::shape_ahead(&self, font: &Font, text: &Text, direction: Direction, active: &[Tag], localized: Option<(Tag, Tag)>)
//! Shaping<'a> { shaper: &'a PreviewShaper, active: &'a [Tag], localized: Option<(Tag script, Tag language)> }
//! Embedding { items: Range<usize>, rtl: bool };  bidi::MAX_EMBEDDINGS = 1_000
//! bidi::is_strong_rtl(c: char) -> bool
//! Options { …, break_opportunities: bool, measure: Option<Measure<'a>> }
//! Measure<'a> { shaper: &'a PreviewShaper, cache: &'a WidthCache };  WidthCache::new();  len;  is_empty
//! measure::{MAX_SEGMENT_CHARS = 128, MAX_CACHED_SEGMENTS = 65_536}
//! linebreak::opportunities(chars: impl IntoIterator<Item = char>) -> Vec<Break>
//! Break::{Never, Prohibited, Allowed, Mandatory}  (entry k: after character k)
//! shape_line(font: &Font, master: &str, text: &Text, layout: &Layout, line: usize, shaper: &PreviewShaper) -> Option<ShapedLine>
//! vertical_features(shaper: &PreviewShaper) -> Vec<FeatureSetting>
//! ShapedLine { placed: Vec<Placed>, length: f64, error: Option<String> }
//! layout(font: &Font, master: &str, text: &Text, options: &Options) -> Result<Layout, LayoutError>
//! line_height(font: &Font, master: &Master) -> f64
//! Layout { placed: Vec<Placed>, lines: Vec<Line>, direction, line_height: f64, ascender: f64, shaping_error: Option<String>,
//!          levels: Option<Vec<u8>> }
//! Placed { glyph: Option<Arc<str>>, x: f64, y: f64, advance: f64, offset: f64, kern: f64, line: usize,
//!          extra: Vec<(Arc<str>, f64, f64)>, cursive: bool }
//! Line { items: Range<usize>, baseline: f64, length: f64 }
//! Layout::hit(&self, x: f64, y: f64) -> usize;  Layout::caret(&self, index: usize) -> (f64, f64)
//! ```
#![forbid(unsafe_code)]

pub mod bidi;
pub mod cursive;
pub mod layout;
pub mod line_shape;
pub mod linebreak;
mod live;
pub mod measure;
pub mod shape;
pub mod text;

#[cfg(test)]
mod tests;

pub use bidi::Embedding;
pub use fontcraft_shaping as shaping;
pub use fontcraft_spacing::kerning::Direction;
pub use layout::{
    DEFAULT_LINE_WIDTH, KerningMode, LENGTH_RANGE, LINE_HEIGHT_PARAM, Layout, LayoutError, Line, MISSING_ADVANCE_RANGE, Options, Placed, layout,
    line_height,
};
pub use line_shape::{ShapedLine, shape_line, vertical_features};
pub use linebreak::Break;
pub use measure::{Measure, WidthCache};
pub use shape::{PreviewShaper, Shaping, tag};
pub use text::{Charmap, Item, PLACEHOLDER, Text, TextError, parse, parse_prefix, parse_with};

/// Items in an Edit View tab.
pub const MAX_ITEMS: usize = 10_000;
/// Items in a Text Preview text (one per character).
pub const MAX_PREVIEW_ITEMS: usize = 1_000_000;
