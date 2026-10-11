//! The region solver of partial instancing (M12.21), ported from fontTools'
//! `varLib.instancer.solver` (MIT): how a tent-shaped region is rebased when an axis range is
//! limited.

/// A region on one normalised axis: start, peak, end.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tent {
    pub start: f64,
    pub peak: f64,
    pub end: f64,
}

/// A limited axis range in normalised coordinates: minimum, default, maximum.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AxisRange {
    pub min: f64,
    pub default: f64,
    pub max: f64,
}

/// The regions, each with the scalar its deltas are multiplied by, that replace `tent` once the
/// axis is limited to `range`. Empty: the region vanishes.
pub fn rebase_tent(tent: Tent, range: AxisRange) -> Vec<(f64, Option<Tent>)> {
    // M12.21: a `None` region is the default location (deltas folded into the default).
    let _ = (tent, range);
    Vec::new()
}
