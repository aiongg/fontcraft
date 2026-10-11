//! Side profiles of an outline and the measures auto-spacing and auto-kerning take from them
//! (M13.20 fills the bodies; M13.21, M13.22 and M13.23 use them). Plan: `docs/plans/M13.md` § 3
//! (auto-spacing and auto-kerning, decision D11).
//!
//! A profile samples, every `step` units over a horizontal band, the outline's horizontal
//! extreme on one side: the leftmost ink for the left side, the rightmost for the right. Rows of
//! the band with no ink have no sample. [`clip_depth`] limits each sample to `depth` inward from
//! the side's extreme, so deep counters (the inside of `c`, the gap of `T`) count only so far.
//! [`white_area`] is the area between a clipped profile and a sidebearing line; auto-spacing makes
//! it equal to a target scaled by band height. [`optical_gap`] is the mean horizontal distance
//! between the right profile of one glyph and the left profile of the next.
//!
//! Profiles are in the glyph's own coordinates; the caller places glyphs by passing offsets.

use fontcraft_geom::BezPath;

/// Smallest sampling step, in font units.
pub const MIN_PROFILE_STEP: f64 = 1.0;
/// Most samples in one profile; a band too tall for its step is refused.
pub const MAX_PROFILE_SAMPLES: usize = 4_096;

/// Which side of the glyph.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Side {
    Left,
    Right,
}

/// The vertical band sampled: from `bottom` to `top`, in font units (for example the baseline
/// minus overshoot to the x-height plus overshoot).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Band {
    pub bottom: f64,
    pub top: f64,
}

/// One side's profile.
#[derive(Clone, Debug, PartialEq)]
pub struct SideProfile {
    pub side: Side,
    /// The height of the first sample.
    pub bottom: f64,
    /// The distance between samples.
    pub step: f64,
    /// The extreme x at `bottom + i × step`; `None` where that row has no ink.
    pub samples: Vec<Option<f64>>,
}

impl SideProfile {
    /// The side's extreme over all samples (smallest x for the left side, largest for the right);
    /// `None` without ink.
    pub fn extreme(&self) -> Option<f64> {
        let xs = self.samples.iter().flatten().copied();
        match self.side {
            Side::Left => xs.reduce(f64::min),
            Side::Right => xs.reduce(f64::max),
        }
    }
}

/// Why profiles cannot be taken.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
pub enum ProfileError {
    #[error("the sampling step must be a number of at least {MIN_PROFILE_STEP} unit")]
    Step,
    #[error("the band must run upward between finite heights")]
    Band,
    #[error("the band needs more than {MAX_PROFILE_SAMPLES} samples at this step")]
    TooManySamples,
    #[error("not supported yet: {0}")]
    Unsupported(String),
}

/// The left and right profiles of `outline` (a glyph's resolved outline, components decomposed)
/// over `band`, sampled every `step` units.
pub fn profiles(outline: &BezPath, band: Band, step: f64) -> Result<(SideProfile, SideProfile), ProfileError> {
    if !(step.is_finite() && step >= MIN_PROFILE_STEP) {
        return Err(ProfileError::Step);
    }
    if !(band.bottom.is_finite() && band.top.is_finite() && band.bottom <= band.top) {
        return Err(ProfileError::Band);
    }
    let _ = outline;
    // M13.20.
    Err(ProfileError::Unsupported("side profiles".into()))
}

/// `profile` with every sample limited to `depth` units inward from the side's extreme.
pub fn clip_depth(profile: &SideProfile, depth: f64) -> SideProfile {
    let _ = depth;
    // M13.20.
    profile.clone()
}

/// The white area between `profile` and the vertical line at `x` (a sidebearing line), over the
/// rows that have ink, in square font units.
pub fn white_area(profile: &SideProfile, x: f64) -> f64 {
    let _ = (profile, x);
    // M13.20.
    0.0
}

/// The mean horizontal distance between `right` (the right profile of the first glyph) and
/// `left` (the left profile of the second glyph, shifted right by `offset`: the first glyph's
/// advance plus any kerning), over the rows where both have ink; `None` when no row has both.
/// Both profiles must share bottom and step.
pub fn optical_gap(right: &SideProfile, left: &SideProfile, offset: f64) -> Option<f64> {
    let _ = (right, left, offset);
    // M13.20.
    None
}
