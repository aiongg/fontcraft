//! The instance parameters that change the feature code of an export (M8.3).
//!
//! Rows: `fontInfo.features.exportSpecific`, `Add Class`, `Add Feature`, `Add Prefix`,
//! `Remove Classes`, `Remove Features`, `Remove Prefixes`, `Replace Class`, `Replace Feature`,
//! `Replace Prefix`.

use fontcraft_model::{Features, Params};

use crate::FeatureError;

pub const ADD_CLASS: &str = "Add Class";
pub const ADD_FEATURE: &str = "Add Feature";
pub const ADD_PREFIX: &str = "Add Prefix";
pub const REMOVE_CLASSES: &str = "Remove Classes";
pub const REMOVE_FEATURES: &str = "Remove Features";
pub const REMOVE_PREFIXES: &str = "Remove Prefixes";
pub const REPLACE_CLASS: &str = "Replace Class";
pub const REPLACE_FEATURE: &str = "Replace Feature";
pub const REPLACE_PREFIX: &str = "Replace Prefix";

/// `entries` with the instance parameters of `params` applied, in parameter order. A malformed
/// parameter value is an error naming the parameter; the entries are untouched then.
pub fn apply_instance_params(entries: &Features, params: &Params) -> Result<Features, FeatureError> {
    // M8.3
    let _ = params;
    Ok(entries.clone())
}
