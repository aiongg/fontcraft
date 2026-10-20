//! FontCraft font source model (L1): pure data, no I/O, no UI.
//!
//! One [`Font`] holds font-wide information, metric definitions, its [`Master`]s and its
//! [`Glyph`]s. A glyph has a master layer per master and may have other layers; a layer holds
//! [`Shape`]s (paths and components), anchors and its advance width. The parts of a font are
//! shared with [`std::sync::Arc`], so a snapshot for undo costs a few pointers and an edit copies
//! only what it changes (see `fontcraft-engine`).
//!
//! Every struct that maps to an object in a source file keeps what the model has no field for in
//! its [`Extra`], tagged by format family, so that keys we do not understand yet survive a load and
//! a save. `docs/spec/30-source-formats.md` § Recommended model is the outline the model grows
//! into; `docs/plans/M1.md` says in which order.
#![forbid(unsafe_code)]

mod address;
pub mod attach;
pub mod axes;
pub mod cid;
pub mod corner;
mod extra;
pub mod fc_keys;
mod features;
mod font;
mod glyph;
mod glyph_info;
mod glyph_props;
pub mod guide_data;
pub mod hoi;
mod info;
mod kerning;
mod layer_info;
pub mod node_width;
mod order;
pub mod paint;
mod props_cache;
mod rare;
mod rename;
mod resolve;
mod shape_info;
pub mod smart;
pub mod ufo_classes;

pub use address::{LayerParts, LayerPartsMut, LayerSide};
pub use extra::{Extra, ExtraEntry, Family};
pub use features::{Features, Group, Snippet};
pub use font::{
    AlignmentZone, Axis, AxisLabel, AxisRange, FileStore, Font, FontInfo, GlyphMut, GlyphTable, Guide, Instance, LangName, Master, MetricDef,
    MetricValue, NameParts, NumberDef, Settings, Source, SourceFormat, StemDef, StyleClass, metric,
};
pub use fontcraft_glyphdata as glyphdata;
pub use fontcraft_plist as plist;
pub use glyph::{Anchor, Component, Glyph, Layer, Node, NodeKind, Path, RawShape, Shape, Transform};
pub use glyph_info::{ColorLabel, GlyphInfo, GlyphInfoBox, MetricsKeys, PartProperty};
pub use glyph_props::{GlyphProps, Prop, PropSource, data_props, glyph_props};
pub use hoi::{DEFAULT_HOI_TIME, Hoi, HoiAxis, HoiCenter, HoiCurve, HoiDirection, HoiTiming};
pub use info::{DEFAULT_LANGUAGE, LANGUAGES, Localized, Param, Params, Properties, Property, PropertyValue, UserData, language_id, param_flag};
pub use kerning::{KernContext, KernFirst, KernGroups, KernTable, Kerning, MasterKerning, glyphs_side, side_group, ufo_side};
pub use layer_info::{Annotation, AxisRule, Background, Hint, Image, LayerAttrs, LayerInfo, LayerInfoBox, PaletteEntry, SmartPole};
pub use order::{
    GLYPH_ORDER_PARAM, Placement, glyph_order, glyph_order_names, glyph_order_sections, insert_by_sort_key, insertion_index, is_section_header,
    keeps_alternates, place_by_sort_key, sort_key,
};
pub use paint::{Color, ColorStop, Extend, Gradient, GradientKind, MAX_GRADIENT_STOPS, Shadow};
pub use props_cache::PropsCache;
pub use rare::{Rare, RareDefault};
pub use rename::{GlyphReference, RenameReport, Renamed, rename_glyphs};
pub use resolve::{
    DONT_USE_PRODUCTION_NAMES, Metrics, ORIGIN_ANCHOR, Resolved, ResolvedAxis, ResolvedComponent, ResolvedGlyph, StyleLinking, Target, blue_values,
    is_ufo_family, location_class, strip_instance_info, user_class,
};
pub use shape_info::{
    Alignment, AnchorInfo, AnchorInfoBox, ComponentInfo, ComponentInfoBox, Dimension, NodeInfo, NodeInfoBox, Orientation, PathInfo, PathInfoBox,
    ShapeAttrs, SmartValue,
};

/// Smallest and largest units-per-em the OpenType `head` table allows.
pub const UPM_RANGE: std::ops::RangeInclusive<u16> = 16..=16384;
/// Most glyphs a font may hold (glyph ids are 16-bit in the tables we write).
pub const MAX_GLYPHS: usize = 65_535;
/// How deep components may nest before a glyph counts as cyclic.
pub const MAX_COMPONENT_DEPTH: usize = 32;
/// Largest coordinate or advance, in font units, the model accepts. Far beyond any real font
/// (OpenType stores 16-bit coordinates); it exists so curve work on hostile input stays bounded.
pub const MAX_COORDINATE: f64 = 1_000_000.0;
/// Most path elements one resolved outline may have. Components can multiply an outline
/// exponentially (a glyph made of two copies of a glyph made of two copies…); this bounds the work.
pub const MAX_OUTLINE_ELEMENTS: usize = 500_000;
/// Longest family, master or glyph name, in bytes.
pub const MAX_NAME_LEN: usize = 255;
/// Most code points one glyph may be mapped from.
pub const MAX_UNICODES: usize = 1024;
/// Most tags one glyph may have.
pub const MAX_GLYPH_TAGS: usize = 256;
/// Most properties one smart glyph may have.
pub const MAX_PART_PROPERTIES: usize = 64;
/// Most guides one layer (or its background) may have.
pub const MAX_LAYER_GUIDES: usize = 4096;
/// Most hints one layer (or its background) may have.
pub const MAX_LAYER_HINTS: usize = 4096;
/// Most annotations one layer (or its background) may have.
pub const MAX_ANNOTATIONS: usize = 1024;
/// Most axis values one layer's coordinates or axis rules may list.
pub const MAX_AXIS_VALUES: usize = 64;

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum ModelError {
    #[error("glyph name is empty")]
    EmptyGlyphName,
    #[error("glyph `{0}` already exists")]
    DuplicateGlyph(String),
    #[error("no glyph named `{0}`")]
    NoSuchGlyph(String),
    #[error("no master `{0}`")]
    NoSuchMaster(String),
    #[error("the font has no master")]
    NoMaster,
    #[error("units per em must be between 16 and 16384, got {0}")]
    BadUnitsPerEm(u32),
    #[error("glyph `{glyph}`: {message}")]
    BadOutline { glyph: String, message: String },
    #[error("the font would hold more than {MAX_GLYPHS} glyphs")]
    TooManyGlyphs,
    #[error("glyph `{0}`: components nest too deeply or refer to themselves")]
    ComponentCycle(String),
    #[error("glyph `{0}`: the outline with its components resolved has more than {MAX_OUTLINE_ELEMENTS} segments")]
    OutlineTooLarge(String),
    #[error("the {what} name {problem}")]
    BadName { what: &'static str, problem: String },
    #[error("two masters share the id `{0}`")]
    DuplicateMaster(String),
    #[error("master `{0}` has a metric that is not a number or is beyond ±{MAX_COORDINATE}")]
    BadMetric(String),
    #[error("glyph `{glyph}` has two layers with the id `{master}`")]
    DuplicateLayer { glyph: String, master: String },
    #[error("glyph `{glyph}`: {value:#X} is not a Unicode code point")]
    BadUnicode { glyph: String, value: u32 },
    #[error("glyph `{glyph}` is mapped from more than {MAX_UNICODES} code points")]
    TooManyUnicodes { glyph: String },
    #[error("glyph `{glyph}`: {message}")]
    BadGlyphInfo { glyph: String, message: String },
    #[error("glyph `{glyph}` layer `{layer}`: {message}")]
    BadLayerInfo { glyph: String, layer: String, message: String },
    #[error("there is no instance number {0}")]
    NoSuchInstance(usize),
    #[error("instance `{0}` describes a variable font, not a static one")]
    NotStatic(String),
    #[error("instance `{0}` lies between masters; it is interpolated before it is resolved")]
    BetweenMasters(String),
}

/// A name is usable when it is not empty, not longer than [`MAX_NAME_LEN`] bytes and has no
/// control characters.
pub(crate) fn check_name(what: &'static str, name: &str) -> Result<(), ModelError> {
    let problem = if name.trim().is_empty() {
        "is empty"
    } else if name.len() > MAX_NAME_LEN {
        "is longer than 255 bytes"
    } else if name.chars().any(char::is_control) {
        "contains a control character"
    } else {
        return Ok(());
    };
    Err(ModelError::BadName { what, problem: problem.to_string() })
}

pub(crate) fn in_range(v: f64) -> bool {
    v.is_finite() && v.abs() <= MAX_COORDINATE
}

#[cfg(test)]
mod tests;
