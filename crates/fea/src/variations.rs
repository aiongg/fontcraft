//! What a variable-font compile needs to know beyond the code: the interface M9.12 implements.
//! Adapted inside this crate to fea-rs's `VariationInfo` (M8.2); M8 passes none.

use crate::{Metric, Tag};

/// One axis of the variable font, in user coordinates.
#[derive(Clone, Debug, PartialEq)]
pub struct Axis {
    pub tag: Tag,
    pub name: String,
    pub min: f64,
    pub default: f64,
    pub max: f64,
    pub hidden: bool,
    /// The axis mapping as (user, design) pairs in increasing order; empty when design
    /// coordinates equal user coordinates.
    pub mapping: Vec<(f64, f64)>,
}

/// A point in the design space: a normalised coordinate (-1 to 1) per axis tag. Axes not listed
/// are at their default (0).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Location(pub Vec<(Tag, f64)>);

/// The variable font's axes, masters and number values, and the step that turns per-location
/// values into a default plus deltas. Implementations must give finite numbers and keep within
/// [`crate::MAX_AXES`] and [`crate::MAX_LOCATIONS`]; the compile checks both and refuses more.
pub trait FeaVariations {
    /// The axes in `fvar` order.
    fn axes(&self) -> &[Axis];

    /// The masters' locations, normalised.
    fn master_locations(&self) -> &[Location];

    /// The number value `name` at `location` (a master's location), or `None` when the font
    /// has no number of that name. fea-rs asks for each master location in turn.
    fn number_value(&self, name: &str, location: &Location) -> Option<f64>;

    /// One value per location (the masters', or the locations a `(axis:coord) value` names)
    /// turned into the default location's value plus deltas, rounded to font units. An error is
    /// a message for the user (values that cannot be interpolated, a location outside the axes).
    fn resolve(&self, values: &[(Location, f64)]) -> Result<Metric, String>;
}
