//! The model's metrics as the shaper reads them in Text Preview: advances, vertical advances and
//! vertical origins come live from the layers of the master whose metrics the shown master uses,
//! so metric edits show without building the preview font again. They match what
//! [`crate::layout()`] places by: a glyph without a vertical width advances by the ascender minus
//! the descender, and one without a vertical origin hangs from the ascender.

use fontcraft_model::{Font, Layer, metric};
use fontcraft_shaping::VerticalMetrics;
use fontcraft_spacing::metrics_source;

use crate::shape::PreviewShaper;

pub(crate) struct Live<'a> {
    font: &'a Font,
    shaper: &'a PreviewShaper,
    master: &'a str,
    /// The master whose metrics `master` shows.
    source: &'a str,
    ascender: f64,
    descender: f64,
}

fn finite_or_zero(v: f64) -> f64 {
    if v.is_finite() { v } else { 0.0 }
}

impl<'a> Live<'a> {
    pub(crate) fn new(font: &'a Font, master: &'a str, shaper: &'a PreviewShaper) -> Self {
        let upm = f64::from(font.units_per_em);
        let source = metrics_source(font, master).or_else(|| font.masters.iter().find(|m| m.id == master));
        let (ascender, descender) = source.map_or((upm * 0.8, -upm * 0.2), |m| {
            (font.metric_pos(m, metric::ASCENDER, upm * 0.8), font.metric_pos(m, metric::DESCENDER, -upm * 0.2))
        });
        let source = source.map_or(master, |m| m.id.as_str());
        Self { font, shaper, master, source, ascender, descender }
    }

    fn layer(&self, gid: u16) -> Option<&'a Layer> {
        let glyph = self.font.glyph(self.shaper.glyph_name(gid)?)?;
        glyph.layer(self.source).or_else(|| glyph.layer(self.master))
    }

    /// The glyph's width.
    pub(crate) fn advance(&self, gid: u16) -> Option<i32> {
        let layer = self.layer(gid)?;
        layer.width.is_finite().then(|| layer.width.round() as i32)
    }

    /// The glyph's vertical width and the height of its vertical origin.
    pub(crate) fn vertical(&self, gid: u16) -> Option<VerticalMetrics> {
        let layer = self.layer(gid)?;
        let origin = finite_or_zero(layer.info.vert_origin.unwrap_or(0.0));
        let width = finite_or_zero(layer.info.vert_width.unwrap_or(self.ascender - self.descender));
        // `as` saturates; the shaper clamps further.
        Some(VerticalMetrics { advance: width.round() as i32, origin_y: (self.ascender - origin).round() as i32 })
    }
}
