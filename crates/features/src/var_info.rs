//! The variable font as the feature compiler sees it (M9.12): [`fontcraft_fea::FeaVariations`]
//! over the font's axes (external ranges and their maps), the variable layout's masters and the
//! masters' number values.
//!
//! Rows: the `fea.var.*` rows of `spec/inventory/12b-feature-language.toml`,
//! `token.numberValue.variable`.
//!
//! Values over the masters' locations resolve through the layout's own model, so the code's
//! variable values and the generated kerning and marks get the same rounding and share one
//! variation store. A value over other locations (a scalar that names fewer or more locations)
//! gets a model of its own, cached by its set of locations.

use std::sync::Mutex;

use fontcraft_fea::{Axis, FeaVariations, Location, Metric};
use fontcraft_model::Font;

use crate::FeatureError;
use crate::fea_variable::FeatureAxes;
use crate::var_layout::{VarLayout, VarMaster};

/// Most models built for values over other locations than the masters'; past it, each such
/// value builds its model again.
pub const MAX_CACHED_MODELS: usize = 64;

/// [`FeaVariations`] for a variable font compile.
pub struct VariableFeatures {
    axes: Vec<Axis>,
    locations: Vec<Location>,
    /// Per master in [`Self::locations`] order: the number values by name.
    numbers: Vec<Vec<(String, f64)>>,
    layout: VarLayout,
    models: Mutex<Vec<(Vec<Vec<f64>>, VarLayout)>>,
}

impl VariableFeatures {
    /// The variable font of `font` whose masters and axes `layout` holds. Refused: an axis of the
    /// layout the font does not have, a master the font does not have.
    pub fn new(font: &Font, layout: &VarLayout) -> Result<Self, FeatureError> {
        let bad = |m: String| FeatureError::Unsupported(format!("the variable feature code cannot be compiled: {m}"));
        let feature_axes = FeatureAxes::from_font(font);
        let axes = layout
            .axes()
            .iter()
            .map(|tag| {
                let text = tag.to_string();
                let a = feature_axes.axes.iter().find(|a| a.tag == text.trim_end()).ok_or_else(|| bad(format!("the font has no axis `{text}`")))?;
                Ok(Axis {
                    tag: *tag,
                    name: a.name.clone(),
                    min: a.external.minimum,
                    default: a.external.default,
                    max: a.external.maximum,
                    hidden: false,
                    mapping: a.map.clone(),
                })
            })
            .collect::<Result<Vec<_>, FeatureError>>()?;
        let mut locations = Vec::with_capacity(layout.masters().len());
        let mut numbers = Vec::with_capacity(layout.masters().len());
        for m in layout.masters() {
            let master = font.masters.iter().find(|x| x.id == m.id).ok_or_else(|| bad(format!("the font has no master `{}`", m.id)))?;
            locations.push(Location(layout.axes().iter().copied().zip(m.location.iter().copied()).filter(|(_, v)| *v != 0.0).collect()));
            // A master that stores fewer values than the font has names reads zero for the
            // others, as the static targets read them.
            numbers.push(
                font.numbers
                    .iter()
                    .enumerate()
                    .map(|(i, n)| (n.name.clone(), master.number_values.get(i).copied().filter(|v| v.is_finite()).unwrap_or(0.0)))
                    .collect(),
            );
        }
        Ok(Self { axes, locations, numbers, layout: layout.clone(), models: Mutex::new(Vec::new()) })
    }

    /// `location` as one coordinate per axis, in `fvar` order.
    fn coordinates(&self, location: &Location) -> Vec<f64> {
        self.layout.axes().iter().map(|t| location.0.iter().find(|(x, _)| x == t).map_or(0.0, |(_, v)| *v)).collect()
    }

    /// The value `values` give at each of `layout`'s masters, or `None` when one is missing.
    fn resolve_with(layout: &VarLayout, points: &[(Vec<f64>, f64)]) -> Result<Metric, String> {
        let column: Option<Vec<Vec<f64>>> =
            layout.masters().iter().map(|m| points.iter().find(|(l, _)| *l == m.location).map(|(_, v)| vec![*v])).collect();
        let column = column.ok_or("a location of the value is not a master's")?;
        let resolved = layout.resolve(&column).map_err(|e| e.to_string())?;
        resolved.first().map(|v| layout.metric(v)).ok_or_else(|| "the value has no locations".to_string())
    }
}

impl FeaVariations for VariableFeatures {
    fn axes(&self) -> &[Axis] {
        &self.axes
    }

    fn master_locations(&self) -> &[Location] {
        &self.locations
    }

    fn number_value(&self, name: &str, location: &Location) -> Option<f64> {
        let index = self.locations.iter().position(|l| l == location)?;
        self.numbers.get(index)?.iter().find(|(n, _)| n == name).map(|(_, v)| *v)
    }

    fn resolve(&self, values: &[(Location, f64)]) -> Result<Metric, String> {
        let mut points: Vec<(Vec<f64>, f64)> = Vec::with_capacity(values.len());
        for (location, value) in values {
            let coords = self.coordinates(location);
            if !value.is_finite() || coords.iter().any(|c| !c.is_finite() || !(-1.0..=1.0).contains(c)) {
                return Err("a variable value has a location outside the axes".into());
            }
            if !points.iter().any(|(l, _)| *l == coords) {
                points.push((coords, *value));
            }
        }
        let masters = self.layout.masters();
        if points.len() == masters.len() && masters.iter().all(|m| points.iter().any(|(l, _)| *l == m.location)) {
            return Self::resolve_with(&self.layout, &points);
        }
        // A model over the value's own locations, the origin first.
        let Some(origin) = points.iter().position(|(l, _)| l.iter().all(|c| *c == 0.0)) else {
            return Err("a variable value needs a value at the default location".into());
        };
        points.swap(0, origin);
        let key: Vec<Vec<f64>> = points.iter().map(|(l, _)| l.clone()).collect();
        let mut sorted_key = key.clone();
        sorted_key.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let cached = self.models.lock().ok().and_then(|models| models.iter().find(|(k, _)| *k == sorted_key).map(|(_, l)| l.clone()));
        let layout = match cached {
            Some(layout) => layout,
            None => {
                let masters = key.iter().enumerate().map(|(i, l)| VarMaster { id: format!("location {i}"), location: l.clone() }).collect();
                let layout = VarLayout::new(self.layout.axes().to_vec(), masters).map_err(|e| e.to_string())?;
                if let Ok(mut models) = self.models.lock()
                    && models.len() < MAX_CACHED_MODELS
                {
                    models.push((sorted_key, layout.clone()));
                }
                layout
            }
        };
        Self::resolve_with(&layout, &points)
    }
}
