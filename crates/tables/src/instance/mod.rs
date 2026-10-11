//! The instancer (M12.20, M12.21; rows `instance`, `font.instantiate`). A port of fontTools'
//! `varLib.instancer` (MIT): full instances ([`full`]), partial pinning and axis ranges
//! ([`partial`]) with its region solver ([`solver`]). Instances from sources (M12.22) are the
//! engine's job.

pub mod full;
pub mod partial;
pub mod solver;

use crate::sfnt::{SfntFont, Tag};
use crate::{Result, TablesError};

/// What happens to one axis, in user coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AxisLimit {
    /// Pin the axis at this value and drop it.
    Pin(f64),
    /// Limit the axis to a range; `default` moves the default when given.
    Range { min: f64, default: Option<f64>, max: f64 },
    /// Pin the axis at its default and drop it.
    Drop,
}

/// What [`instantiate`] does. Axes not named keep their full range.
#[derive(Clone, Debug, PartialEq)]
pub struct InstanceOptions {
    /// At most [`crate::limits::MAX_AXES`] entries; values finite.
    pub limits: Vec<(Tag, AxisLimit)>,
    /// Update `name` and `OS/2` for a static instance from the matching named instance or STAT.
    pub update_names: bool,
    /// Set the overlap flags on simple glyphs of a static instance.
    pub overlap_flags: bool,
    /// Turn `CFF2` into `CFF ` when every axis is pinned.
    pub downgrade_cff2: bool,
}

impl Default for InstanceOptions {
    fn default() -> Self {
        Self { limits: Vec::new(), update_names: false, overlap_flags: true, downgrade_cff2: true }
    }
}

/// The instance.
pub fn instantiate(font: &SfntFont, options: &InstanceOptions) -> Result<SfntFont> {
    // M12.20 (all axes pinned), M12.21 (the rest).
    let _ = (font, options);
    Err(TablesError::NotBuilt("instance"))
}
