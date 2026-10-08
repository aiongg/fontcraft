//! What a layer holds beyond its id, master, width, shapes and anchors: its name, attributes,
//! vertical metrics, metrics keys, colour, visibility, guides, hints, annotations, image,
//! background and user data.
//!
//! Most layers have none of it, so a [`crate::Layer`] holds it as a [`Rare`]`<LayerInfo>`.
//! The background is a [`Background`]: shapes, anchors, guides, hints, annotations and an image,
//! with the layer's own shape types. It has no background of its own, so it cannot nest.

use std::sync::LazyLock;

use fontcraft_plist::Value;
use serde::{Deserialize, Serialize};

use crate::rare::{Rare, RareDefault};
use crate::{Anchor, ColorLabel, Extra, Family, Guide, MetricsKeys, Shape, Transform, UserData};

/// A layer's data beyond its id, master, width, shapes and anchors.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LayerInfo {
    /// The name the file stores. Master layers and other special layers usually store none; the
    /// editor shows a name derived from their role.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// What kind of layer this is beyond master and backup: intermediate, alternate, colour,
    /// smart-glyph pole.
    #[serde(default, skip_serializing_if = "LayerAttrs::is_empty")]
    pub attrs: LayerAttrs,
    /// The vertical advance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vert_width: Option<f64>,
    /// The vertical origin, measured down from the ascender (the reference's convention).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vert_origin: Option<f64>,
    /// Metrics keys that override the glyph's for this layer.
    #[serde(default, skip_serializing_if = "MetricsKeys::is_empty")]
    pub metrics_keys: MetricsKeys,
    /// The layer's colour label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<ColorLabel>,
    /// Shown when the editor shows all layers.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub visible: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub guides: Vec<Guide>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hints: Vec<Hint>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub annotations: Vec<Annotation>,
    /// The placed image (the reference's background image).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<Image>,
    /// The background drawing. `Some` with nothing in it is a background the file states empty.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<Background>,
    #[serde(default, skip_serializing_if = "UserData::is_empty")]
    pub user_data: UserData,
}

impl RareDefault for LayerInfo {
    fn default_ref() -> &'static Self {
        static EMPTY: LazyLock<LayerInfo> = LazyLock::new(LayerInfo::default);
        &EMPTY
    }
}

/// A layer's data, behind one pointer.
pub type LayerInfoBox = Rare<LayerInfo>;

/// The attributes that make a layer special. They combine: an intermediate layer can also be a
/// colour layer.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LayerAttrs {
    /// An intermediate layer's location, axis by axis in the font's axis order. `None` stands for
    /// the master's own value (version 4 writes `"-"`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub coordinates: Vec<Option<f64>>,
    /// An alternate layer's ranges, axis by axis: where on each axis it replaces the master
    /// layer.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub axis_rules: Vec<AxisRule>,
    /// A colour palette layer's entry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color_palette: Option<PaletteEntry>,
    /// A full-colour layer: its shapes carry their own fills.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub full_color: bool,
    /// A bitmap colour layer's image size (`sbix`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sbix_size: Option<u16>,
    /// An SVG colour layer.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub svg: bool,
    /// A smart glyph's layer: which end of each property's range it draws.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub smart_poles: Vec<SmartPole>,
    /// Attributes the model has no field for.
    #[serde(default, skip_serializing_if = "Extra::is_empty")]
    pub extra: Extra,
}

impl LayerAttrs {
    pub fn is_empty(&self) -> bool {
        *self == LayerAttrs::default()
    }
}

/// An alternate layer's range on one axis, in design coordinates. A missing end is open.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AxisRule {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
}

/// A colour palette layer's colour.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PaletteEntry {
    /// An entry of the font's palettes.
    Index(u16),
    /// The text colour (index 65535 in `COLR`).
    Foreground,
}

/// Which end of a smart-glyph property's range a layer draws.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SmartPole {
    pub property: String,
    /// `true` for the top value, `false` for the bottom value.
    pub top: bool,
}

/// A hint, kept as the source stores it: node references by index path, interpreted later
/// (M10). In the native file the value is property-list text in its family's syntax.
#[derive(Clone, Debug, PartialEq)]
pub struct Hint {
    pub family: Family,
    pub value: Value,
}

impl Hint {
    /// The hint's kind (`Stem`, `TTStem`, `Corner`…), when the stored value names one.
    pub fn kind(&self) -> Option<&str> {
        self.value.as_dict()?.get("type")?.as_str()
    }
}

#[derive(Serialize, Deserialize)]
struct HintRepr {
    family: Family,
    value: String,
}

impl Serialize for Hint {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        HintRepr { family: self.family, value: crate::extra::value_text(self.family, &self.value) }.serialize(s)
    }
}

impl<'de> Deserialize<'de> for Hint {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let r = HintRepr::deserialize(d)?;
        let value = crate::extra::parse_value_text(r.family, &r.value).map_err(|e| <D::Error as serde::de::Error>::custom(format!("hint: {e}")))?;
        Ok(Hint { family: r.family, value })
    }
}

/// A note drawn in the editor: text, an arrow, a circle, a plus or a minus sign.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Annotation {
    /// `Text`, `Arrow`, `Circle`, `Plus` or `Minus`.
    pub kind: String,
    #[serde(default)]
    pub x: f64,
    #[serde(default)]
    pub y: f64,
    /// Degrees, as the source writes them.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub angle: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// A text box's or a circle's width.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub width: f64,
    #[serde(default, skip_serializing_if = "Extra::is_empty")]
    pub extra: Extra,
}

fn is_zero(v: &f64) -> bool {
    *v == 0.0
}

/// An image placed in a layer.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Image {
    /// The image file, relative to the source (a UFO's `images/` folder holds it by name).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// Where the image is drawn: decomposed as format versions 3 and 4 write it, or a matrix as
    /// version 2 and UFO write it.
    #[serde(default)]
    pub transform: Transform,
    /// The part of the image shown: x, y, width and height in image pixels.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub crop: Option<[f64; 4]>,
    /// Opacity in the editor, in percent (the reference's default is 50).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub alpha: Option<f64>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub locked: bool,
    #[serde(default, skip_serializing_if = "Extra::is_empty")]
    pub extra: Extra,
}

/// A layer's background: a drawing behind the outline, with the layer's shape types.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Background {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub shapes: Vec<Shape>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub anchors: Vec<Anchor>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub guides: Vec<Guide>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub hints: Vec<Hint>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub annotations: Vec<Annotation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<Image>,
    #[serde(default, skip_serializing_if = "Extra::is_empty")]
    pub extra: Extra,
}

impl Background {
    pub fn is_empty(&self) -> bool {
        *self == Background::default()
    }
}

/// The limits a layer's data must keep: at most [`crate::MAX_LAYER_GUIDES`] guides,
/// [`crate::MAX_LAYER_HINTS`] hints, [`crate::MAX_ANNOTATIONS`] annotations,
/// [`crate::MAX_AXIS_VALUES`] coordinates or axis rules and [`crate::MAX_PART_PROPERTIES`] smart
/// poles; every number finite and positions within the coordinate limit; a colour's components
/// from 0 to 1. The background's shapes are checked by the caller as the layer's are.
pub(crate) fn check_layer_info(info: &LayerInfo) -> Result<(), String> {
    let a = &info.attrs;
    if a.coordinates.len() > crate::MAX_AXIS_VALUES || a.axis_rules.len() > crate::MAX_AXIS_VALUES {
        return Err(format!("more than {} layer coordinates or axis rules", crate::MAX_AXIS_VALUES));
    }
    if a.smart_poles.len() > crate::MAX_PART_PROPERTIES {
        return Err(format!("more than {} smart-glyph poles", crate::MAX_PART_PROPERTIES));
    }
    let finite = |v: &Option<f64>| v.is_none_or(crate::in_range);
    if !a.coordinates.iter().all(finite) || !a.axis_rules.iter().all(|r| finite(&r.min) && finite(&r.max)) {
        return Err("a layer coordinate or axis rule is not a number or is out of range".into());
    }
    if !finite(&info.vert_width) || !finite(&info.vert_origin) {
        return Err("a vertical metric is not a number or is out of range".into());
    }
    if info.color.is_some_and(|c| !c.is_valid()) {
        return Err("a layer colour component is not a number from 0 to 1".into());
    }
    check_content(&info.guides, &info.hints, &info.annotations, info.image.as_ref())?;
    if let Some(b) = &info.background {
        check_content(&b.guides, &b.hints, &b.annotations, b.image.as_ref())?;
        if b.anchors.iter().any(|a| !(crate::in_range(a.x) && crate::in_range(a.y))) {
            return Err("a background anchor position is not a number or is out of range".into());
        }
    }
    Ok(())
}

fn check_content(guides: &[Guide], hints: &[Hint], annotations: &[Annotation], image: Option<&Image>) -> Result<(), String> {
    if guides.len() > crate::MAX_LAYER_GUIDES || hints.len() > crate::MAX_LAYER_HINTS || annotations.len() > crate::MAX_ANNOTATIONS {
        return Err(format!(
            "more than {} guides, {} hints or {} annotations in one layer",
            crate::MAX_LAYER_GUIDES,
            crate::MAX_LAYER_HINTS,
            crate::MAX_ANNOTATIONS
        ));
    }
    if guides.iter().any(|g| !(crate::in_range(g.x) && crate::in_range(g.y) && g.angle.is_finite())) {
        return Err("a guide's position or angle is not a number or is out of range".into());
    }
    if annotations.iter().any(|a| !(crate::in_range(a.x) && crate::in_range(a.y) && a.angle.is_finite() && a.width.is_finite())) {
        return Err("an annotation's position, angle or width is not a number or is out of range".into());
    }
    if let Some(i) = image {
        let numbers = i.transform.numbers().into_iter().chain(i.crop.into_iter().flatten()).chain(i.alpha);
        if !numbers.into_iter().all(f64::is_finite) {
            return Err("an image's placement, crop or alpha is not a number".into());
        }
    }
    Ok(())
}
