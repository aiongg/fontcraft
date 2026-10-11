//! Variation models: HVAR from phantom points, model description, plots and `avar` planning
//! (M12.34; rows `variations build-hvar`, `model`, `plot`, `plan-avar` and their commands).
//! Oracles: fontTools `varLib.hvar`, `varLib.models`, `varLib.plot`, `varLib.avar.plan`.

use crate::sfnt::{SfntFont, Tag};
use crate::{Result, TablesError};

/// The font with an `HVAR` table built from the `gvar` phantom points.
pub fn build_hvar(font: &SfntFont) -> Result<SfntFont> {
    // M12.34
    let _ = font;
    Err(TablesError::NotBuilt("variations build-hvar"))
}

/// One master's region in a variation model.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ModelSupport {
    /// The master location, normalised, by axis tag.
    pub location: Vec<(Tag, f64)>,
    /// Per axis, `(start, peak, end)`.
    pub region: Vec<(Tag, [f64; 3])>,
}

/// The supports of the model for `locations` (normalised), in fontTools' master order.
pub fn describe_model(locations: &[Vec<(Tag, f64)>]) -> Result<Vec<ModelSupport>> {
    // M12.34
    let _ = locations;
    Err(TablesError::NotBuilt("variations model"))
}

/// An SVG plot of the model's supports.
pub fn plot(locations: &[Vec<(Tag, f64)>]) -> Result<String> {
    // M12.34
    let _ = locations;
    Err(TablesError::NotBuilt("variations plot"))
}

/// One axis's `avar` segment map: the axis tag and its `(from, to)` points.
pub type AvarSegments = (Tag, Vec<(f64, f64)>);

/// The `avar` segment maps fontTools' planner proposes for the weight, width, slant and optical
/// size axes of `font`, per axis tag.
pub fn plan_avar(font: &SfntFont) -> Result<Vec<AvarSegments>> {
    // M12.34
    let _ = font;
    Err(TablesError::NotBuilt("variations plan-avar"))
}
