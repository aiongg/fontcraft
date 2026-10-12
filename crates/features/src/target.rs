//! What feature code is compiled for, and the values its tokens read there (M8.5).
//!
//! Rows: `token.numberValue.simple`, `token.glyphProperty.layerChoice`,
//! `fontInfo.numbers.tokensInterpolate` (masters).
//!
//! [`MasterValues`] reads the master layer of each glyph: its width, its sidebearings as the
//! spacing crate measures them (slanted back in an italic master), its anchors. The exported
//! glyphs are those whose export flag is on, in the font's order.

use std::cell::RefCell;

use fontcraft_glyphquery::ClassResolver;
use fontcraft_glyphquery::predicate::Predicate;
use fontcraft_model::glyphdata::GlyphData;
use fontcraft_model::{Font, Layer, Master};
use fontcraft_spacing::Field;

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

/// Where [`MasterValues`] reads anchor positions from: `(glyph, anchor)` to the position on the
/// master. The compiler passes propagated anchors this way, since this crate does not depend on
/// `fontcraft-components`.
pub type AnchorSource<'a> = Box<dyn Fn(&str, &str) -> Option<(f64, f64)> + 'a>;

/// [`TargetValues`] for one master: its number values, its layers' metrics through the spacing
/// crate's sidebearings, and its anchors: those of an [`AnchorSource`] when one is given
/// ([`MasterValues::with_anchors`]), else the layer's stored anchors.
pub struct MasterValues<'a> {
    target: Target,
    font: &'a Font,
    data: &'a GlyphData,
    master: &'a Master,
    glyphs: Vec<String>,
    anchors: Option<AnchorSource<'a>>,
    /// The feature classes `class(…)` reads, resolved once for every predicate token, with
    /// one work cap for all of them.
    classes: RefCell<ClassResolver<'a>>,
}

impl<'a> MasterValues<'a> {
    /// Refused: a master the font does not have.
    pub fn new(font: &'a Font, data: &'a GlyphData, master: &str) -> Result<Self, FeatureError> {
        let Some(found) = font.masters.iter().find(|m| m.id == master) else {
            return Err(FeatureError::Unsupported(format!("the font has no master `{master}`")));
        };
        let glyphs = font.glyphs.as_arcs().iter().filter(|g| g.export).map(|g| g.name.clone()).collect();
        let classes = RefCell::new(ClassResolver::new(font, data, Some(found.id.as_str())));
        Ok(Self { target: Target::Master(master.to_string()), font, data, master: found, glyphs, anchors: None, classes })
    }

    /// Reads anchors from `source` (propagated anchors) instead of the layers' stored ones.
    pub fn with_anchors(mut self, source: AnchorSource<'a>) -> Self {
        self.anchors = Some(source);
        self
    }

    pub fn font(&self) -> &'a Font {
        self.font
    }

    pub fn glyph_data(&self) -> &'a GlyphData {
        self.data
    }

    /// The master layer of `glyph`.
    fn layer(&self, glyph: &str) -> Option<&'a Layer> {
        self.font.glyph(glyph)?.layer(&self.master.id)
    }
}

impl TargetValues for MasterValues<'_> {
    fn target(&self) -> &Target {
        &self.target
    }

    fn number_value(&self, name: &str) -> Option<f64> {
        let index = self.font.numbers.iter().position(|n| n.name == name)?;
        // A master that stores fewer values than the font has names reads zero for the others.
        let value = self.master.number_values.get(index).copied().unwrap_or(0.0);
        Some(if value.is_finite() { value } else { 0.0 })
    }

    fn glyph_metric(&self, glyph: &str, metric: GlyphMetric) -> Option<f64> {
        let layer = self.layer(glyph)?;
        let field = match metric {
            GlyphMetric::Width => Field::Width,
            GlyphMetric::Lsb => Field::Left,
            GlyphMetric::Rsb => Field::Right,
            GlyphMetric::Tsb => Field::Top,
            GlyphMetric::Bsb => Field::Bottom,
        };
        fontcraft_spacing::metric(self.font, layer, field).ok().flatten().filter(|v| v.is_finite())
    }

    fn anchor(&self, glyph: &str, anchor: &str) -> Option<(f64, f64)> {
        if let Some(source) = &self.anchors {
            return source(glyph, anchor).filter(|(x, y)| x.is_finite() && y.is_finite());
        }
        // A name stored twice reads its last position, as the mark writer does.
        let a = self.layer(glyph)?.anchors.iter().rev().find(|a| a.name == anchor)?;
        (a.x.is_finite() && a.y.is_finite()).then_some((a.x, a.y))
    }

    fn glyphs(&self) -> &[String] {
        &self.glyphs
    }

    /// Layer attributes read this master; `class(…)` reads the font's enabled feature classes;
    /// non-exporting glyphs are left out.
    fn select(&self, predicate: &Predicate) -> Result<Vec<String>, String> {
        let master = Some(self.master.id.as_str());
        let wanted = predicate.classes();
        let classes = {
            let mut resolver = self.classes.try_borrow_mut().map_err(|_| "the feature classes are already being read".to_string())?;
            resolver.classes(Some(wanted.as_slice())).map_err(|e| e.to_string())?
        };
        let env = fontcraft_glyphquery::Env::new(self.font, self.data, master).map_err(|e| e.to_string())?.with_classes(classes);
        let names = fontcraft_glyphquery::predicate::evaluate(predicate, &env).map_err(|e| e.to_string())?;
        Ok(names.into_iter().filter(|n| self.font.glyph(n).is_some_and(|g| g.export)).collect())
    }
}
