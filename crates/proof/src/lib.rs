//! FontCraft proof sheets (L3; rows `proof`, `font.proof`, format `proof-html`,
//! `proofing.webfontPreview`).
//!
//! [`proof`] lays a compiled font's glyphs out on pages and writes them as SVG (one document per
//! page), as one HTML page, or as PDF. Outlines come from skrifa at the requested location.
//!
//! - `page`: the sheets as pages of drawing items (glyph grid, one glyph per page with points and
//!   metrics, waterfall), independent of the file format.
//! - `svg`: one SVG document per page; glyphs as paths, waterfall text in the embedded font.
//! - `pdf`: one PDF document with every page (through `pdf-writer`); glyphs as paths, waterfall
//!   text as outlines placed by `cmap`, advances and pair kerning (`kern`).
//! - `html`: the browser specimen with the font embedded (WOFF2 when the input is WOFF2), sample
//!   text, waterfall, character set, feature switches and axis stops; it carries no script.
//!
//! [`select_glyphs`] turns a selection with ranges, exclusions and a sort order into the glyph
//! names [`ProofOptions::glyphs`] takes; [`grid_size_for`] turns a glyphs-per-page count into a
//! glyph size.
//!
//! Limits: at most [`MAX_PAGES`] pages and [`MAX_SHEET_GLYPHS`] glyphs per sheet set; outline
//! segments, sizes, waterfall text and lines are capped in the modules.
#![forbid(unsafe_code)]

mod font;
mod html;
mod kern;
mod page;
mod pdf;
mod select;
mod svg;
mod text;

pub use page::grid_size_for;
pub use select::{GlyphSort, Selection, select_glyphs};

/// Most pages one proof writes.
pub const MAX_PAGES: usize = 10_000;
/// Most glyphs one sheet set shows.
pub const MAX_SHEET_GLYPHS: usize = 65_535;

/// The file format of a proof.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ProofFormat {
    /// One SVG document per page.
    #[default]
    Svg,
    /// One HTML page.
    Html,
    /// One PDF document.
    Pdf,
}

/// What a sheet shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Sheet {
    /// Every selected glyph in a grid, with name and code point.
    GlyphGrid,
    /// Lines of sample text at each size.
    Waterfall { text: String },
    /// Each selected glyph alone on a page, with metrics lines.
    GlyphPerPage,
}

/// What [`proof`] makes.
#[derive(Clone, Debug, PartialEq)]
pub struct ProofOptions {
    pub format: ProofFormat,
    pub sheets: Vec<Sheet>,
    /// Glyphs to show, by name, in the order shown; empty shows every glyph in glyph order.
    pub glyphs: Vec<String>,
    /// Page size in points, `[width, height]`; US Letter by default, as the reference proofs.
    pub page_size: [f64; 2],
    /// Point sizes for waterfalls and the grid cell height.
    pub sizes: Vec<f64>,
    /// The location of a variable font, by axis tag in user coordinates.
    pub location: Vec<([u8; 4], f64)>,
}

impl Default for ProofOptions {
    fn default() -> Self {
        Self {
            format: ProofFormat::Svg,
            sheets: vec![Sheet::GlyphGrid],
            glyphs: Vec::new(),
            page_size: [612.0, 792.0],
            sizes: vec![72.0],
            location: Vec::new(),
        }
    }
}

/// The written proof: one file for HTML and PDF, one per page for SVG.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Proof {
    pub files: Vec<Vec<u8>>,
    pub pages: usize,
}

/// Why a proof was not made.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProofError {
    /// The format's package has not landed yet.
    #[error("{0} proofs are not built yet")]
    NotBuilt(&'static str),
    /// The options ask for more than the limits allow, or name glyphs the font lacks.
    #[error("{0}")]
    BadOptions(String),
    /// The font could not be read.
    #[error(transparent)]
    Tables(#[from] fontcraft_tables::TablesError),
}

/// Makes the proof of the compiled font `bytes` (face `face` of a collection).
pub fn proof(bytes: &[u8], face: u32, options: &ProofOptions) -> Result<Proof, ProofError> {
    let font::FontBytes { sfnt, embed, container } = font::load(bytes, face)?;
    let font = font::font_ref(&sfnt)?;
    let location = font::location(&font, &options.location)?;
    let face = font::Face::new(font, &location)?;
    let gids = face.resolve(&options.glyphs)?;
    match options.format {
        ProofFormat::Html => Ok(Proof { files: vec![html::write(&face, &gids, options, &embed, container)?], pages: 1 }),
        ProofFormat::Svg => {
            let doc = page::layout(&face, &gids, options, Some(embedded(embed, container, options)))?;
            let files = svg::write(&doc);
            Ok(Proof { pages: files.len(), files })
        }
        // The PDF draws waterfall text as outlines, so it embeds no font.
        ProofFormat::Pdf => {
            let doc = page::layout(&face, &gids, options, None)?;
            Ok(Proof { pages: doc.pages.len(), files: vec![pdf::write(&doc, &face)?] })
        }
    }
}

fn embedded(bytes: Vec<u8>, container: font::Container, options: &ProofOptions) -> page::Embedded {
    let variation = options.location.iter().map(|(tag, v)| (String::from_utf8_lossy(tag).into_owned(), *v)).collect();
    page::Embedded { bytes, container, variation }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn junk_is_an_error_in_every_format() {
        for format in [ProofFormat::Svg, ProofFormat::Html, ProofFormat::Pdf] {
            let options = ProofOptions { format, ..ProofOptions::default() };
            assert!(proof(b"not a font at all", 0, &options).is_err());
        }
    }
}
