//! Reading and writing the interpolation parameters in the model's parameter lists (M9.2).
//!
//! Rows: parameters `Axis Location`, `Axis Mappings`, `Virtual Master`, `Disable Masters`,
//! `Variable Font Origin`, `Variation Font Origin`, `Ignore out of bounds instances`,
//! `Enforce Compatibility Check`.
//!
//! Contract: M9's commands write these parameters through the setters here, so the values they
//! write have the shape the reference's format gives them; M7's generic custom-parameter
//! commands edit the same values. Readers refuse a value they cannot read with
//! [`InterpError::BadParameter`]; the setters return a new parameter list and leave the input
//! unchanged. An empty value removes the parameter.

use fontcraft_model::{Font, Instance, Params};

use crate::InterpError;

pub use fontcraft_model::axes::{AXIS_LOCATION, AXIS_MAPPINGS, ORIGIN, VIRTUAL_MASTER};

/// The variable font setting parameter that lists the masters left out of that font, by name.
pub const DISABLE_MASTERS: &str = "Disable Masters";
/// The older name of [`ORIGIN`], which names the master by name.
pub const ORIGIN_BY_NAME: &str = "Variation Font Origin";
/// The font parameter that leaves instances outside the axis ranges out of the export.
pub const IGNORE_OUT_OF_BOUNDS: &str = "Ignore out of bounds instances";
/// The font parameter that widens the compatibility check to every master (on) or hides the
/// indicators (off).
pub const ENFORCE_COMPATIBILITY_CHECK: &str = "Enforce Compatibility Check";

/// One entry of an Axis Location parameter: an axis, by name, and the external coordinate there.
#[derive(Clone, Debug, PartialEq)]
pub struct AxisLocationEntry {
    pub axis: String,
    pub location: f64,
}

/// A Virtual Master parameter: one coordinate per axis, by axis name.
#[derive(Clone, Debug, PartialEq)]
pub struct VirtualMasterEntry {
    pub coordinates: Vec<(String, f64)>,
}

/// The Axis Location of a master or instance; `None` when the parameter is absent.
pub fn axis_location(params: &Params) -> Result<Option<Vec<AxisLocationEntry>>, InterpError> {
    // M9.2
    let _ = params;
    Ok(None)
}

/// `params` with its Axis Location set to `entries` (removed when empty).
pub fn set_axis_location(params: &Params, entries: &[AxisLocationEntry]) -> Result<Params, InterpError> {
    // M9.2
    let _ = entries;
    Ok(params.clone())
}

/// The font's Axis Mappings, as (axis tag, `(external, internal)` points sorted by external
/// value), in the parameter's order.
pub fn axis_mappings(font: &Font) -> Result<Vec<(String, Vec<(f64, f64)>)>, InterpError> {
    // M9.2
    let _ = font;
    Ok(Vec::new())
}

/// `params` (a font's) with axis `tag`'s mapping set to `points`; other axes are kept. Empty
/// `points` remove the axis's entry, and the parameter when no entry is left.
pub fn set_axis_mappings(params: &Params, tag: &str, points: &[(f64, f64)]) -> Result<Params, InterpError> {
    // M9.2
    let _ = (tag, points);
    Ok(params.clone())
}

/// The font's Virtual Master parameters, in order.
pub fn virtual_masters(font: &Font) -> Result<Vec<VirtualMasterEntry>, InterpError> {
    // M9.2
    let _ = font;
    Ok(Vec::new())
}

/// `params` (a font's) with the Virtual Master parameters replaced by `entries`.
pub fn set_virtual_masters(params: &Params, entries: &[VirtualMasterEntry]) -> Result<Params, InterpError> {
    // M9.2
    let _ = entries;
    Ok(params.clone())
}

/// The names of the masters a variable font setting disables.
pub fn disabled_masters(setting: &Instance) -> Vec<String> {
    // M9.2
    let _ = setting;
    Vec::new()
}

/// `params` (a variable font setting's) with Disable Masters set to `names` (removed when empty).
pub fn set_disabled_masters(params: &Params, names: &[String]) -> Params {
    // M9.2
    let _ = names;
    params.clone()
}

/// The Enforce Compatibility Check setting: `Some(true)` widens the check to every master,
/// `Some(false)` hides the indicators, `None` is the default scope.
pub fn enforce_compatibility(font: &Font) -> Option<bool> {
    // M9.3
    let _ = font;
    None
}
