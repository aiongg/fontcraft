//! Interpolated layers and static instance fonts (M9.4).
//!
//! Rows: `masters.metricsInterpolate`, `masters.originAutomaticForStatic`,
//! `instances.multiAxisCoordinates`, `instances.parametersStaticOnly`,
//! `alternateLayer.staticExport`, `fontInfo.customParameters.masterInterpolation`,
//! `fontInfo.stems.interpolateByName`, `fontInfo.instance.extrapolation`, `fontInfo.fontType`,
//! `instance.manualInterpolation`, `instance.instanceInterpolations`,
//! `compile.instanceInterpolation`, `compile.instanceLayoutInterpolation` (kerning),
//! `compile.kernGroupReconciliation`, `intermediateLayer.sparseInExport` (static half),
//! parameters `Local Interpolation`, `InterpolationWeightY`.
//!
//! Contract:
//! - [`interpolate_layer`] interpolates paths, component transforms, anchors, the width and
//!   smart-component values at an internal location, through the glyph's own sub-model (its
//!   sources, without missing masters), switching alternate layers by inclusive ranges. The
//!   sources must be compatible ([`crate::compat`]); otherwise [`InterpError::Incompatible`].
//! - [`instance_font`] returns a copy of the font with a single master at the instance's location:
//!   every glyph's layer, the metrics, stems by name, number values, kerning (groups reconciled
//!   across masters) and the parameters present on every master, interpolated. Manual master
//!   weights, Local Interpolation and InterpolationWeightY apply. `compile` resolves that master
//!   as it resolves any other; `Font::resolve` keeps refusing instances between masters, so the
//!   model does not depend on this crate.
//! - Static instances outside the axis ranges extrapolate (`docs/plans/M9.md` § 7 question 6,
//!   decided: the handbook's rule) when [`InterpOptions::extrapolate`] is on; off, they are
//!   refused with [`InterpError::OutsideDesignspace`].
//! - A failure changes nothing and leaves the input untouched.

use std::collections::HashMap;
use std::sync::Arc;

use fontcraft_geom::variation::VariationModel;
use fontcraft_model::{Font, Layer};

use crate::InterpError;
use crate::designspace::{Designspace, Location, NormLocation};
use crate::sources::GlyphSources;

/// Which variation model static instances use (the Font Type setting; a guess on
/// `fontInfo.fontType`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ModelKind {
    /// The model anchored at the origin master, as variable fonts use it.
    #[default]
    Default,
    /// The variable font's model, with intermediate regions exactly as the font file holds them.
    Variable,
}

/// How an instance is interpolated.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct InterpOptions {
    /// Locations beyond the masters extrapolate; off, they are refused.
    pub extrapolate: bool,
    pub model: ModelKind,
    /// The InterpolationWeightY parameter: a separate weight-axis value for vertical coordinates.
    pub weight_y: Option<f64>,
}

impl Default for InterpOptions {
    fn default() -> Self {
        Self { extrapolate: true, model: ModelKind::Default, weight_y: None }
    }
}

/// The options the instance at `instance` asks for through its parameters and the font's.
pub fn options_for(font: &Font, instance: usize) -> Result<InterpOptions, InterpError> {
    // M9.4
    let _ = (font, instance);
    Ok(InterpOptions::default())
}

/// The weight of each master (by id, in the designspace's order) at `location`; with an
/// instance's manual interpolation, its stored weights.
pub fn master_weights(designspace: &Designspace, location: &Location, options: &InterpOptions) -> Result<Vec<(String, f64)>, InterpError> {
    // M9.4
    let _ = (designspace, location, options);
    Err(InterpError::NotYetImplemented("master weights"))
}

/// The layer of `sources`' glyph at `location`, interpolated; its id and master are the origin
/// master's.
pub fn interpolate_layer(font: &Font, sources: &GlyphSources, location: &Location, options: &InterpOptions) -> Result<Layer, InterpError> {
    // M9.4
    let _ = (font, sources, location, options);
    Err(InterpError::NotYetImplemented("interpolating a layer"))
}

/// `font` reduced to one master at the location of the instance at `instance` (an index into
/// `Font::instances`), everything master-bound interpolated.
pub fn instance_font(font: &Font, instance: usize, options: &InterpOptions) -> Result<Font, InterpError> {
    // M9.4
    let _ = (font, instance, options);
    Err(InterpError::NotYetImplemented("static instances between masters"))
}

/// Variation models of a designspace's glyphs, cached by their sources' normalized locations, so
/// glyphs with the same sources share one model.
#[derive(Clone, Debug, Default)]
pub struct ModelCache {
    /// Keyed by the locations' bits, so equal locations share an entry.
    models: HashMap<Vec<u64>, Arc<VariationModel>>,
}

impl ModelCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// The model of sources at `locations`, built once.
    pub fn get(&mut self, locations: &[NormLocation], extrapolate: bool) -> Result<Arc<VariationModel>, InterpError> {
        // M9.4: key by locations and the extrapolation flag; cap the entries.
        let _ = (locations, extrapolate, &mut self.models);
        Err(InterpError::NotYetImplemented("variation models"))
    }

    pub fn clear(&mut self) {
        self.models.clear();
    }
}
