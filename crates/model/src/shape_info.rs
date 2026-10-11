//! What paths, nodes, components and anchors hold beyond their geometry: locks, orientations,
//! names, identifiers, drawing attributes, a component's alignment and smart-component values,
//! and user data.
//!
//! Most shapes have none of it, so each holds it as a [`Rare`]: a node, the most numerous object
//! in a font, stays 32 bytes, and its name, identifier, user data and unknown keys sit behind one
//! pointer ([`NodeInfo`]).

use std::sync::LazyLock;

use serde::{Deserialize, Serialize};

use crate::paint::{Color, Gradient, MAX_GRADIENT_STOPS, Shadow};
use crate::rare::{Rare, RareDefault};
use crate::{ColorLabel, Extra, UserData};

/// Which edge of the layer an element's horizontal position is measured from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Orientation {
    #[default]
    Left,
    Right,
    Center,
}

impl Orientation {
    pub fn is_left(&self) -> bool {
        *self == Orientation::Left
    }
}

/// A node's name, identifier and user data, and what the model has no field for.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct NodeInfo {
    /// The node's name (a UFO point's `name`; hint sets refer to it).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// A UFO point's identifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identifier: Option<String>,
    #[serde(default, skip_serializing_if = "UserData::is_empty")]
    pub user_data: UserData,
    #[serde(default, skip_serializing_if = "Extra::is_empty")]
    pub extra: Extra,
}

impl RareDefault for NodeInfo {
    fn default_ref() -> &'static Self {
        static EMPTY: LazyLock<NodeInfo> = LazyLock::new(NodeInfo::default);
        &EMPTY
    }
}

/// A node's rare data, behind one pointer.
pub type NodeInfoBox = Rare<NodeInfo>;

/// A stroke size or position: a number, or an expression the reference resolves (version 4), or
/// a number the file writes as text.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Dimension {
    Number(f64),
    Expression(String),
}

/// How a path or component is drawn in the editor and in colour fonts. The values are as the
/// source's format version states them: `stroke_pos` and `mask` count differently in `.glyphs`
/// versions 3 and 4 (see `docs/spec/30-source-formats.md` § 2.7).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ShapeAttrs {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke_width: Option<Dimension>,
    /// The vertical stroke size; the width when `None`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke_height: Option<Dimension>,
    /// Where the stroke sits on the path. Today the source's format version states it
    /// (`.glyphs` version 3: 0 outside, 0.5 centred, 1 inside; version 4: −1 inside, 0 centred, 1
    /// outside). M11.5 stores it in the version 4 convention and converts version 3 files when
    /// they are read and written.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke_pos: Option<Dimension>,
    /// 0 butt, 1 round, 2 square, 3 round inset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_cap_start: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_cap_end: Option<u8>,
    /// 0 miter, 1 round, 2 bevel, 3 round full.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line_join: Option<u8>,
    /// A stroked path is also filled.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub fill: bool,
    /// The fill: RGBA from 0 to 1, or a palette entry with an alpha.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill_color: Option<Color>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stroke_color: Option<Color>,
    /// A gradient, which replaces the fill colour when present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gradient: Option<Gradient>,
    /// A drop shadow (image export only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shadow: Option<Shadow>,
    /// An inner shadow (image export only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shadow_in: Option<Shadow>,
    /// The shape's colour label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<ColorLabel>,
    /// How the shape masks the shapes before it (version 3: 1; version 4: 1 subtract, 2 intersect).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mask: Option<u8>,
    /// The shape's paths are reversed when drawn and exported.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub reverse_paths: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub hidden: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opacity: Option<f64>,
    /// Attributes kept as stored values: compositing, shape-group membership, keys the model
    /// does not know, and (until M11.5) the gradients, shadows and palette colours the readers
    /// do not yet type.
    #[serde(default, skip_serializing_if = "Extra::is_empty")]
    pub extra: Extra,
}

impl ShapeAttrs {
    pub fn is_empty(&self) -> bool {
        *self == ShapeAttrs::default()
    }

    /// Every number, for range checks.
    pub fn numbers(&self) -> impl Iterator<Item = f64> + '_ {
        let dim = |d: &Option<Dimension>| match d {
            Some(Dimension::Number(f)) => Some(*f),
            _ => None,
        };
        [dim(&self.stroke_width), dim(&self.stroke_height), dim(&self.stroke_pos), self.opacity]
            .into_iter()
            .flatten()
            .chain(self.fill_color.iter().flat_map(Color::numbers))
            .chain(self.stroke_color.iter().flat_map(Color::numbers))
            .chain(self.gradient.iter().flat_map(Gradient::numbers))
            .chain(self.shadow.iter().chain(self.shadow_in.iter()).flat_map(Shadow::numbers))
    }
}

/// A path's lock, drawing attributes and identifier.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PathInfo {
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub locked: bool,
    #[serde(default, skip_serializing_if = "ShapeAttrs::is_empty")]
    pub attrs: ShapeAttrs,
    /// A UFO contour's identifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identifier: Option<String>,
}

impl RareDefault for PathInfo {
    fn default_ref() -> &'static Self {
        static EMPTY: LazyLock<PathInfo> = LazyLock::new(PathInfo::default);
        &EMPTY
    }
}

pub type PathInfoBox = Rare<PathInfo>;

/// Whether the editor places a component automatically.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Alignment {
    /// By the usual rules (the stored position is the result).
    #[default]
    Default,
    /// Never: the stored position is the designer's.
    Disabled,
    /// Always, whatever the rules say.
    Forced,
    /// Only horizontally; the vertical position is the designer's.
    Horizontal,
}

/// A smart component's value for one property of its glyph.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SmartValue {
    pub property: String,
    pub value: f64,
}

/// A component's placement rules, smart-component values, lock, drawing attributes, identifier
/// and user data.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ComponentInfo {
    #[serde(default, skip_serializing_if = "is_default_alignment")]
    pub alignment: Alignment,
    /// The anchor of the base glyph it attaches to, when there are several candidates.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub anchor: Option<String>,
    /// Smart-component values, in the source's order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub smart_values: Vec<SmartValue>,
    /// The master whose layer of the base glyph is used, when it is not the layer's own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub master_id: Option<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub locked: bool,
    #[serde(default, skip_serializing_if = "Orientation::is_left")]
    pub orientation: Orientation,
    #[serde(default, skip_serializing_if = "ShapeAttrs::is_empty")]
    pub attrs: ShapeAttrs,
    /// A UFO component's identifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identifier: Option<String>,
    #[serde(default, skip_serializing_if = "UserData::is_empty")]
    pub user_data: UserData,
}

fn is_default_alignment(a: &Alignment) -> bool {
    *a == Alignment::Default
}

impl RareDefault for ComponentInfo {
    fn default_ref() -> &'static Self {
        static EMPTY: LazyLock<ComponentInfo> = LazyLock::new(ComponentInfo::default);
        &EMPTY
    }
}

pub type ComponentInfoBox = Rare<ComponentInfo>;

/// An anchor's lock, orientation, colour, identifier and user data.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct AnchorInfo {
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub locked: bool,
    #[serde(default, skip_serializing_if = "Orientation::is_left")]
    pub orientation: Orientation,
    /// A UFO anchor's colour, as the UFO writes it (`"r,g,b,a"`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// A UFO anchor's identifier.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identifier: Option<String>,
    #[serde(default, skip_serializing_if = "UserData::is_empty")]
    pub user_data: UserData,
}

impl RareDefault for AnchorInfo {
    fn default_ref() -> &'static Self {
        static EMPTY: LazyLock<AnchorInfo> = LazyLock::new(AnchorInfo::default);
        &EMPTY
    }
}

pub type AnchorInfoBox = Rare<AnchorInfo>;

/// The limits a shape's data must keep: every number finite, colour components from 0 to 1, and
/// (for a component) at most [`crate::MAX_PART_PROPERTIES`] smart values.
pub(crate) fn check_attrs(a: &ShapeAttrs) -> Result<(), String> {
    if !a.numbers().all(f64::is_finite) {
        return Err("a drawing attribute is not a number".into());
    }
    let colors = a.fill_color.iter().chain(&a.stroke_color).chain(a.shadow.iter().chain(&a.shadow_in).map(|s| &s.color));
    let stops = a.gradient.iter().flat_map(|g| g.stops.iter().map(|s| &s.color));
    if colors.chain(stops).any(|c| !c.is_valid()) || a.color.is_some_and(|c| !c.is_valid()) {
        return Err("a colour component is not a number from 0 to 1".into());
    }
    if a.gradient.as_ref().is_some_and(|g| g.stops.len() > MAX_GRADIENT_STOPS) {
        return Err(format!("a gradient has more than {MAX_GRADIENT_STOPS} stops"));
    }
    Ok(())
}

pub(crate) fn check_component_info(c: &ComponentInfo) -> Result<(), String> {
    if c.smart_values.len() > crate::MAX_PART_PROPERTIES {
        return Err(format!("more than {} smart-component values", crate::MAX_PART_PROPERTIES));
    }
    if c.smart_values.iter().any(|v| !v.value.is_finite()) {
        return Err("a smart-component value is not a number".into());
    }
    check_attrs(&c.attrs)
}
