//! FontCraft proof sheets (L3; rows `proof`, `font.proof`, format `proof-html`,
//! `proofing.webfontPreview`).
//!
//! [`proof`] lays a compiled font's glyphs out on pages and writes them as SVG (one document per
//! page), as one HTML page, or as PDF. Drawing goes through skrifa and `fontcraft-render`. The
//! HTML page embeds the font as WOFF2 and shows sample text, the character set, feature toggles
//! and axis sliders. SVG and HTML are M12.35; PDF (through `pdf-writer`) is M12.36.
//!
//! Limits: at most [`MAX_PAGES`] pages and [`MAX_SHEET_GLYPHS`] glyphs per sheet set.
#![forbid(unsafe_code)]

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
    /// Glyphs to show, by name; empty shows every glyph in glyph order.
    pub glyphs: Vec<String>,
    /// Page size in points, `[width, height]`.
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
            page_size: [595.0, 842.0],
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
    // M12.35 (SVG, HTML), M12.36 (PDF)
    let _ = (bytes, face);
    Err(ProofError::NotBuilt(match options.format {
        ProofFormat::Svg => "SVG",
        ProofFormat::Html => "HTML",
        ProofFormat::Pdf => "PDF",
    }))
}

#[cfg(test)]
mod tests {
    #[test]
    fn unbuilt_formats_refuse_with_a_message() {
        let e = super::proof(&[], 0, &super::ProofOptions::default()).unwrap_err();
        assert!(e.to_string().contains("not built"));
    }
}
