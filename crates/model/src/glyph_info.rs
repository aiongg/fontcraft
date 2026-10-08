//! A glyph's metadata: production name, classification, metrics keys, colour label, tags, note,
//! lock, last change, sort names, smart-glyph properties and user data.
//!
//! Most glyphs have none of it, so a [`crate::Glyph`] holds it as a [`Rare`]`<GlyphInfo>`. The
//! classification fields override what the reference's glyph data says about the glyph's name;
//! `None` means "as the glyph data says".

use std::sync::LazyLock;

use serde::{Deserialize, Serialize};

use crate::rare::{Rare, RareDefault};
use crate::{Extra, UserData};

/// What a glyph knows beyond its name, code points, export flag, kerning groups and layers.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct GlyphInfo {
    /// The glyph's name in compiled fonts, when it is not the one the glyph data gives.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub production: Option<String>,
    /// `Letter`, `Mark`, `Number`, `Symbol`…
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// `Uppercase`, `Nonspacing`, `Ligature`…
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sub_category: Option<String>,
    /// `latin`, `cyrillic`…
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub script: Option<String>,
    /// `noCase`, `upper`, `lower`, `smallCaps`, `minor` or `other`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub case: Option<String>,
    /// The writing direction: `BIDI`, `LTR`, `RTL`, `VTR` or `VTL`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub direction: Option<String>,
    /// The glyph's class for the OpenType `GDEF` table, as a UFO's `public.openTypeCategories`
    /// names it: `unassigned`, `base`, `mark`, `ligature` or `component`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opentype_category: Option<String>,
    #[serde(default, skip_serializing_if = "MetricsKeys::is_empty")]
    pub metrics_keys: MetricsKeys,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<ColorLabel>,
    /// In the order the source lists them (the reference sorts them).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub note: String,
    /// The glyph cannot be edited until it is unlocked.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub locked: bool,
    /// When the glyph was last changed, as the source writes it (`2026-10-08 12:00:00 +0000`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_change: Option<String>,
    /// The name the glyph is sorted by, in place of the glyph data's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort_name: Option<String>,
    /// The same, when alternates are kept next to their base glyph.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sort_name_keep: Option<String>,
    /// A smart glyph's properties (the reference's `partsSettings`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parts: Vec<PartProperty>,
    #[serde(default, skip_serializing_if = "UserData::is_empty")]
    pub user_data: UserData,
}

impl RareDefault for GlyphInfo {
    fn default_ref() -> &'static Self {
        static EMPTY: LazyLock<GlyphInfo> = LazyLock::new(GlyphInfo::default);
        &EMPTY
    }
}

/// The glyph's metadata, behind one pointer.
pub type GlyphInfoBox = Rare<GlyphInfo>;

/// Metrics keys: a glyph name or a formula (`=H`, `=|`, `=O-4`) a side bearing or width is
/// taken from.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MetricsKeys {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub left: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub right: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bottom: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vert_width: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vert_origin: Option<String>,
}

impl MetricsKeys {
    pub fn is_empty(&self) -> bool {
        *self == MetricsKeys::default()
    }

    /// Every key with its name (`left`, `right`, `width`, `top`, `bottom`, `vertWidth`,
    /// `vertOrigin`), in that order.
    pub fn all(&self) -> [(&'static str, &Option<String>); 7] {
        [
            ("left", &self.left),
            ("right", &self.right),
            ("width", &self.width),
            ("top", &self.top),
            ("bottom", &self.bottom),
            ("vertWidth", &self.vert_width),
            ("vertOrigin", &self.vert_origin),
        ]
    }
}

/// A colour label.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ColorLabel {
    /// One of the reference's numbered label colours (0 red to 11 dark grey).
    Index(u8),
    /// Red, green, blue and alpha, each from 0 to 1, in sRGB.
    Rgba([f64; 4]),
}

impl ColorLabel {
    /// Whether every component is a number from 0 to 1.
    pub fn is_valid(&self) -> bool {
        match self {
            ColorLabel::Index(_) => true,
            ColorLabel::Rgba(c) => c.iter().all(|v| (0.0..=1.0).contains(v)),
        }
    }
}

/// One property of a smart glyph: a name and the range its value moves in.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PartProperty {
    pub name: String,
    pub bottom_value: f64,
    pub top_value: f64,
    /// Names for the two ends, which format version 2 and 3 files may hold (the reference does
    /// not use them).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bottom_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_name: Option<String>,
    #[serde(default, skip_serializing_if = "Extra::is_empty")]
    pub extra: Extra,
}

/// The limits and value ranges a glyph's metadata must keep: at most [`crate::MAX_GLYPH_TAGS`]
/// tags and [`crate::MAX_PART_PROPERTIES`] smart-glyph properties, finite property ranges, a
/// colour's components from 0 to 1.
pub(crate) fn check_glyph_info(g: &crate::Glyph) -> Result<(), crate::ModelError> {
    let info = &*g.info;
    let bad = |message: String| crate::ModelError::BadGlyphInfo { glyph: g.name.clone(), message };
    if info.tags.len() > crate::MAX_GLYPH_TAGS {
        return Err(bad(format!("more than {} tags", crate::MAX_GLYPH_TAGS)));
    }
    if info.parts.len() > crate::MAX_PART_PROPERTIES {
        return Err(bad(format!("more than {} smart-glyph properties", crate::MAX_PART_PROPERTIES)));
    }
    if info.parts.iter().any(|p| !(crate::in_range(p.bottom_value) && crate::in_range(p.top_value))) {
        return Err(bad("a smart-glyph property's range is not a number or is out of range".into()));
    }
    if info.color.is_some_and(|c| !c.is_valid()) {
        return Err(bad("a colour component is not a number from 0 to 1".into()));
    }
    Ok(())
}
