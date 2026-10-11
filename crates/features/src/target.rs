//! What feature code is compiled for, and the values its tokens read there (M8.5).
//!
//! Rows: `token.numberValue.simple`, `token.glyphProperty.layerChoice`,
//! `fontInfo.numbers.tokensInterpolate` (masters).

use fontcraft_glyphquery::predicate::Predicate;
use fontcraft_model::Font;
use fontcraft_model::glyphdata::GlyphData;

use crate::FeatureError;

/// The build feature code is assembled, expanded and compiled for. Not the same as
/// `fontcraft_model::Target`, which names what the outline compiler resolves: feature code also
/// has a variable-font target.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Target {
    /// The master with this id: a static export of it, or the Edit View preview.
    Master(String),
    /// The instance at this index of `Font::instances` (M9: interpolated values).
    Instance(usize),
    /// The variable font (M9: values per master location, `features::variable::lower`).
    Variable,
}

impl Target {
    pub fn is_variable(&self) -> bool {
        matches!(self, Target::Variable)
    }
}

/// A glyph metric a property token reads (`${glyph:width}`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GlyphMetric {
    Width,
    Lsb,
    Rsb,
    Tsb,
    Bsb,
}

impl GlyphMetric {
    /// The property name as tokens spell it (`width`, `LSB`); matched exactly.
    pub fn from_name(name: &str) -> Option<GlyphMetric> {
        match name {
            "width" => Some(GlyphMetric::Width),
            "LSB" => Some(GlyphMetric::Lsb),
            "RSB" => Some(GlyphMetric::Rsb),
            "TSB" => Some(GlyphMetric::Tsb),
            "BSB" => Some(GlyphMetric::Bsb),
            _ => None,
        }
    }
}

/// The values tokens are evaluated against for one target. M8 implements it for a master
/// ([`MasterValues`]); M9 for instances and the variable font. Every number is finite.
pub trait TargetValues {
    fn target(&self) -> &Target;

    /// The number value `name` (Font Info › Masters › Number Values), or `None` when the font
    /// has no number of that name.
    fn number_value(&self, name: &str) -> Option<f64>;

    /// A metric of `glyph`'s layer for the target, or `None` for a glyph or layer that does not
    /// exist (or a vertical metric of a font without vertical metrics).
    fn glyph_metric(&self, glyph: &str, metric: GlyphMetric) -> Option<f64>;

    /// The position of anchor `anchor` of `glyph` for the target (after propagation), or `None`.
    fn anchor(&self, glyph: &str, anchor: &str) -> Option<(f64, f64)>;

    /// The glyphs the target exports, in the font's order.
    fn glyphs(&self) -> &[String];

    /// The glyphs `predicate` selects for the target, in the font's order (`$[…]` tokens). An
    /// error is a message for the user.
    fn select(&self, predicate: &Predicate) -> Result<Vec<String>, String>;
}

/// [`TargetValues`] for one master: its number values, its layers' metrics through the spacing
/// crate's sidebearings, its propagated anchors.
pub struct MasterValues<'a> {
    target: Target,
    font: &'a Font,
    data: &'a GlyphData,
    glyphs: Vec<String>,
}

impl<'a> MasterValues<'a> {
    /// Refused: a master the font does not have.
    pub fn new(font: &'a Font, data: &'a GlyphData, master: &str) -> Result<Self, FeatureError> {
        if !font.masters.iter().any(|m| m.id == master) {
            return Err(FeatureError::Unsupported(format!("the font has no master `{master}`")));
        }
        // M8.5: the exporting glyphs in order.
        Ok(Self { target: Target::Master(master.to_string()), font, data, glyphs: Vec::new() })
    }

    pub fn font(&self) -> &'a Font {
        self.font
    }

    pub fn glyph_data(&self) -> &'a GlyphData {
        self.data
    }
}

impl TargetValues for MasterValues<'_> {
    fn target(&self) -> &Target {
        &self.target
    }

    fn number_value(&self, name: &str) -> Option<f64> {
        // M8.5
        let _ = name;
        None
    }

    fn glyph_metric(&self, glyph: &str, metric: GlyphMetric) -> Option<f64> {
        // M8.5
        let _ = (glyph, metric);
        None
    }

    fn anchor(&self, glyph: &str, anchor: &str) -> Option<(f64, f64)> {
        // M8.5
        let _ = (glyph, anchor);
        None
    }

    fn glyphs(&self) -> &[String] {
        &self.glyphs
    }

    fn select(&self, predicate: &Predicate) -> Result<Vec<String>, String> {
        // M8.6: `fontcraft_glyphquery::predicate::evaluate` on this master, with the font's classes.
        let _ = predicate;
        Err("glyph predicate tokens are not available yet".into())
    }
}
