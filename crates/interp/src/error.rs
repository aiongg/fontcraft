//! The crate's error type and its named limits (`docs/plans/M9.md` § 3 › Limits).
//!
//! Every count read from a font or a command parameter is checked against one of these limits
//! before the work it bounds, and every coordinate is checked to be finite and within
//! [`MAX_COORDINATE`] before any model call.

use fontcraft_geom::variation::VariationError;
use fontcraft_model::ModelError;

use crate::compat::Mismatch;

/// Most axes a font may have; also the most coordinates and alternate-range entries per layer.
pub const MAX_AXES: usize = fontcraft_model::MAX_AXIS_VALUES;
/// Most sources one interpolation takes: masters, virtual masters and intermediate layers
/// together.
pub const MAX_SOURCES: usize = fontcraft_geom::variation::MAX_MASTERS;
/// Most virtual masters a font may have.
pub const MAX_VIRTUAL_MASTERS: usize = 64;
/// Most instances a font may have, static instances and variable font settings together.
pub const MAX_INSTANCES: usize = 4_096;
/// Most distinct alternate range sets one glyph may have.
pub const MAX_RANGE_SETS: usize = 64;
/// Most records the overlay of alternate ranges may produce.
pub const MAX_OVERLAY_RECORDS: usize = 4_096;
/// Most steps (box intersections and copies) the overlay may take. Overlapping boxes can make the
/// overlay grow exponentially; past this budget it stops with [`InterpError::LimitExceeded`].
pub const MAX_OVERLAY_WORK: usize = 10_000_000;
/// Most points one axis mapping may have.
pub const MAX_MAPPING_POINTS: usize = fontcraft_model::axes::MAX_MAPPING_POINTS;
/// Largest absolute coordinate, internal or external, accepted anywhere.
pub const MAX_COORDINATE: f64 = fontcraft_model::MAX_COORDINATE;
/// Most variation models one [`crate::ModelCache`] keeps; past it the cache starts again.
pub const MAX_CACHED_MODELS: usize = 4_096;
/// Most numbers (coordinates, transform values, anchors, the width) one interpolated layer may
/// have.
pub const MAX_LAYER_NUMBERS: usize = fontcraft_model::MAX_OUTLINE_ELEMENTS;
/// Most glyphs an instance font interpolates.
pub const MAX_GLYPHS: usize = fontcraft_model::MAX_GLYPHS;
/// Most kerning pairs an instance font interpolates per direction.
pub const MAX_KERN_PAIRS: usize = 1_000_000;

/// Most documents one Generate Instances run may create.
pub const MAX_GENERATED_INSTANCES: usize = 256;
/// Compatible layers whose corresponding handles turn by more than this many degrees get an
/// [`crate::compat::AngleWarning`].
pub const ANGLE_WARNING_DEGREES: f64 = 20.0;

#[derive(Debug, PartialEq, thiserror::Error)]
pub enum InterpError {
    /// The layers of a glyph cannot interpolate; the report names the first difference of each
    /// pair of layers.
    #[error("`{glyph}` has layers that are not compatible{}", .mismatches.first().map(|m| format!(": {m}")).unwrap_or_default())]
    Incompatible { glyph: String, mismatches: Vec<Mismatch> },
    /// A location outside the axis ranges where extrapolation is not allowed (variable fonts, or
    /// extrapolation turned off).
    #[error("{what} lies outside the designspace")]
    OutsideDesignspace { what: String },
    #[error("there is no axis `{0}`")]
    UnknownAxis(String),
    #[error("there is no master `{0}`")]
    UnknownMaster(String),
    #[error("there is no instance number {0}")]
    UnknownInstance(usize),
    /// An instance index that is not a variable font setting.
    #[error("instance number {0} is not a variable font setting")]
    NotAVariableSetting(usize),
    #[error("there is no glyph called `{0}`")]
    UnknownGlyph(String),
    #[error("there is no layer `{layer}` in `{glyph}`")]
    UnknownLayer { glyph: String, layer: String },
    /// An axis tag that is not four printable ASCII characters.
    #[error("`{0}` is not an axis tag: a tag is four printable ASCII characters")]
    BadAxisTag(String),
    #[error("two axes have the tag `{0}`")]
    DuplicateAxisTag(String),
    /// A coordinate or value that is not finite or is beyond [`MAX_COORDINATE`].
    #[error("{what} is not a usable number")]
    BadNumber { what: String },
    /// An axis mapping whose points are not finite and increasing on both coordinates.
    /// An alternate layer's range whose minimum lies above its maximum.
    #[error("an alternate range on axis {axis} runs from {min} down to {max}; its minimum must not be above its maximum")]
    ReversedRange { axis: usize, min: f64, max: f64 },
    #[error("the mapping of axis `{axis}` is not usable: {reason}")]
    BadMapping { axis: String, reason: String },
    /// A custom parameter whose value cannot be read.
    #[error("the `{name}` parameter cannot be read: {reason}")]
    BadParameter { name: String, reason: String },
    /// The masters do not allow a variable font: no master at the origin of an axis, or the
    /// origin named by the font is not a master.
    #[error("the masters cannot make a variable font: {0}")]
    BadMasterSetup(String),
    #[error("more than {limit} {what}")]
    LimitExceeded { what: &'static str, limit: usize },
    #[error(transparent)]
    Model(#[from] ModelError),
    #[error("{0}")]
    Variation(#[from] VariationError),
    /// A function of this crate whose M9 package has not been written yet.
    #[error("{0} is not available yet")]
    NotYetImplemented(&'static str),
}
