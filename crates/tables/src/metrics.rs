//! The metrics report (M12.16; rows `metrics`, `font.reportMetrics`). Oracles: `tx -mtx`,
//! `tx -afm`, fontTools `pens.statisticsPen`.

use crate::sfnt::SfntFont;
use crate::{Result, TablesError};

/// What [`report`] measures.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MetricsOptions {
    /// Glyphs to measure; empty measures every glyph.
    pub glyphs: Vec<String>,
    /// Include the kerning pairs.
    pub kerning: bool,
    /// Include area and moments.
    pub statistics: bool,
}

/// One glyph's metrics in font units.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GlyphMetrics {
    pub name: String,
    pub advance: f64,
    /// `[x_min, y_min, x_max, y_max]`; `None` for an empty glyph.
    pub bounds: Option<[f64; 4]>,
    pub left_side_bearing: f64,
    pub right_side_bearing: f64,
    pub area: Option<f64>,
    /// Centre of mass, `[x, y]`.
    pub centroid: Option<[f64; 2]>,
}

/// One kerning pair, by glyph name.
#[derive(Clone, Debug, PartialEq)]
pub struct KernPair {
    pub left: String,
    pub right: String,
    pub value: f64,
}

/// The report.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MetricsReport {
    pub units_per_em: u16,
    pub glyphs: Vec<GlyphMetrics>,
    pub kerning: Vec<KernPair>,
}

/// Measures the font.
pub fn report(font: &SfntFont, options: &MetricsOptions) -> Result<MetricsReport> {
    // M12.16
    let _ = (font, options);
    Err(TablesError::NotBuilt("metrics"))
}
