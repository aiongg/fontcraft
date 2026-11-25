//! Optical bounds (M13.49): the `lfbd` and `rtbd` features, which move a glyph that starts or
//! ends a line so that its outline, not its sidebearing, meets the margin.
//!
//! Rows: `features.opticalBoundsFromGuides` and the master parameter `opticalOffset`.
//!
//! - Each master has a global pair of offsets, the `opticalOffset` parameter (`{left, right}`
//!   in font units): how much the left and right sidebearings of every glyph shrink at a line's
//!   start and end ([`OpticalOffset`]).
//! - A glyph layer overrides them with a guide named `ol` (left bound) or `or` (right bound)
//!   that is not horizontal: the bound is where the guide crosses the baseline
//!   ([`layer_bounds`]).
//! - [`bounds_features`] writes one rule per glyph with a non-zero bound: `pos g <-L 0 -L 0>;`
//!   in `lfbd`, `pos g <0 0 -R 0>;` in `rtbd`. For the variable font a bound that differs between
//!   masters is a variable value in user coordinates, as number tokens are written.
//! - [`add_missing`] leaves a feature out when the font's code already writes it.
//!
//! A glyph without outlines or components (a space) takes no global offset; a guide still gives
//! it a bound. A bound outside a value record's 16-bit range leaves the glyph out with a warning.

use std::fmt::Write as _;

use fontcraft_model::guide_follow::placed_x;
use fontcraft_model::plist::Value;
use fontcraft_model::{Features, Font, Guide, Layer, Master, Snippet};

use crate::auto::MAX_GENERATED_BYTES;
use crate::fea_variable::FeatureAxes;

/// The master parameter that holds the global offsets.
pub const OPTICAL_OFFSET: &str = "opticalOffset";
/// The name of the guide that marks a glyph's left bound.
pub const LEFT_GUIDE: &str = "ol";
/// The name of the guide that marks a glyph's right bound.
pub const RIGHT_GUIDE: &str = "or";
/// The feature that holds the left bounds.
pub const LEFT_FEATURE: &str = "lfbd";
/// The feature that holds the right bounds.
pub const RIGHT_FEATURE: &str = "rtbd";
/// Largest bound written, in either direction: a value record holds 16-bit values.
pub const MAX_BOUND: f64 = 32_767.0;

/// The least sine of a guide's angle for the guide to mark a bound: guides closer than 30° to
/// horizontal are not read.
const MIN_GUIDE_SINE: f64 = 0.5;

/// A master's global offsets: how far the left and right sidebearings shrink, in font units.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct OpticalOffset {
    pub left: f64,
    pub right: f64,
}

impl OpticalOffset {
    /// The master's `opticalOffset` parameter, a dictionary with `left` and `right`; a missing,
    /// non-numeric or non-finite side is 0.
    pub fn of_master(master: &Master) -> Self {
        let Some(dict) = master.params.get(OPTICAL_OFFSET).and_then(Value::as_dict) else { return Self::default() };
        let side = |key: &str| dict.get(key).and_then(number).unwrap_or(0.0);
        Self { left: side("left"), right: side("right") }
    }
}

/// A number, or a string that holds one (format version 2 writes parameter numbers as strings).
fn number(value: &Value) -> Option<f64> {
    let n = match value {
        Value::String(s) => s.as_str().trim().parse::<f64>().ok()?,
        _ => value.as_num()?.as_f64(),
    };
    n.is_finite().then_some(n)
}

/// Where the first guide of `layer` named `name` that is not horizontal crosses the baseline.
pub fn guide_position(layer: &Layer, name: &str) -> Option<f64> {
    layer.info.guides.iter().filter(|g| g.name.as_deref() == Some(name)).find_map(|g| baseline_crossing(g, layer.width))
}

fn baseline_crossing(guide: &Guide, width: f64) -> Option<f64> {
    let (sin, cos) = guide.angle.to_radians().sin_cos();
    if !sin.is_finite() || sin.abs() < MIN_GUIDE_SINE {
        return None;
    }
    let x = placed_x(guide, width) - guide.y * cos / sin;
    x.is_finite().then_some(x)
}

/// How far the left and right sidebearings of `layer` shrink: the distance from the origin to
/// the `ol` guide and from the `or` guide to the advance width, each side falling back to
/// `offset` without its guide.
pub fn layer_bounds(layer: &Layer, offset: OpticalOffset) -> (f64, f64) {
    let left = guide_position(layer, LEFT_GUIDE).unwrap_or(offset.left);
    let right = guide_position(layer, RIGHT_GUIDE).map_or(offset.right, |x| layer.width - x);
    (left, right)
}

/// What the bounds are written for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoundsTarget<'a> {
    /// A static font: the bounds of this master's layers.
    Master(&'a str),
    /// The variable font: the bounds of every master.
    Variable,
}

/// The `lfbd` and `rtbd` feature entries for `glyphs`, each a pair (name in the compiled font,
/// source glyph name) in glyph order. A feature without a non-zero bound has no entry; no
/// entry at all when no master has offsets and no glyph has an `ol` or `or` guide.
pub fn bounds_features(font: &Font, glyphs: &[(&str, &str)], target: BoundsTarget, warnings: &mut Vec<String>) -> Vec<Snippet> {
    let masters: Vec<&Master> = match target {
        BoundsTarget::Master(id) => font.masters.iter().filter(|m| m.id == id).take(1).collect(),
        BoundsTarget::Variable => font.masters.iter().collect(),
    };
    if masters.is_empty() {
        return Vec::new();
    }
    let offsets: Vec<OpticalOffset> = masters.iter().map(|m| OpticalOffset::of_master(m)).collect();
    // Variable values are written against every master of the font, in its order, which is
    // `masters` for the variable target.
    let axes = match target {
        BoundsTarget::Variable if !font.axes.is_empty() => Some(FeatureAxes::from_font(font)),
        _ => None,
    };
    let mut left = String::new();
    let mut right = String::new();
    for (name, source) in glyphs {
        let Some(glyph) = font.glyph(source) else { continue };
        let mut lefts = Vec::with_capacity(masters.len());
        let mut rights = Vec::with_capacity(masters.len());
        let mut bounded = false;
        for (master, offset) in masters.iter().zip(&offsets) {
            match glyph.layer(&master.id) {
                Some(layer) => {
                    let guided = guide_position(layer, LEFT_GUIDE).is_some() || guide_position(layer, RIGHT_GUIDE).is_some();
                    bounded |= guided || !layer.shapes.is_empty();
                    let (l, r) = layer_bounds(layer, *offset);
                    lefts.push(l);
                    rights.push(r);
                }
                None => {
                    lefts.push(offset.left);
                    rights.push(offset.right);
                }
            }
        }
        if !bounded {
            continue;
        }
        let (Some(lefts), Some(rights)) = (rounded(&lefts), rounded(&rights)) else {
            warnings.push(format!("the optical bounds of {name} were left out: one is beyond {MAX_BOUND} units"));
            continue;
        };
        let written = value_text(&lefts, axes.as_ref()).and_then(|l| Ok((l, value_text(&rights, axes.as_ref())?)));
        match written {
            Ok((l, r)) => {
                if let Some(l) = l {
                    let _ = writeln!(left, "pos {name} <{l} 0 {l} 0>;");
                }
                if let Some(r) = r {
                    let _ = writeln!(right, "pos {name} <0 0 {r} 0>;");
                }
            }
            Err(e) => warnings.push(format!("the optical bounds of {name} were left out: {e}")),
        }
        if left.len().max(right.len()) > MAX_GENERATED_BYTES {
            warnings.push(format!("the optical bounds were left out: their code would pass {MAX_GENERATED_BYTES} bytes"));
            return Vec::new();
        }
    }
    [(LEFT_FEATURE, left), (RIGHT_FEATURE, right)]
        .into_iter()
        .filter(|(_, code)| !code.is_empty())
        .map(|(tag, code)| Snippet { name: tag.to_string(), code, automatic: true, ..Default::default() })
        .collect()
}

/// `values` rounded half up and negated (a bound shrinks the sidebearing); `None` when one is
/// beyond [`MAX_BOUND`].
fn rounded(values: &[f64]) -> Option<Vec<i32>> {
    values
        .iter()
        .map(|v| {
            let r = (v + 0.5).floor();
            // The range check makes the cast exact.
            #[allow(clippy::cast_possible_truncation)]
            (r.abs() <= MAX_BOUND).then(|| -(r as i32))
        })
        .collect()
}

/// The value written for `values` (one per master): `None` when every one is 0, a number when
/// they agree, otherwise a variable value.
fn value_text(values: &[i32], axes: Option<&FeatureAxes>) -> Result<Option<String>, String> {
    let Some(&first) = values.first() else { return Ok(None) };
    if values.iter().all(|&v| v == first) {
        return Ok((first != 0).then(|| first.to_string()));
    }
    match axes {
        Some(axes) => axes.master_scalar(values),
        None => Ok((first != 0).then(|| first.to_string())),
    }
}

/// Adds the `generated` entries to `entries`, except a feature the code already writes: an
/// entry with its tag, enabled or not, or a `feature <tag>` block in the code of any entry (a
/// UFO's feature file is read as one prefix).
pub fn add_missing(entries: &mut Features, generated: Vec<Snippet>) {
    for entry in generated {
        let block = format!("feature {}", entry.name);
        let written = entries.features.iter().any(|f| f.name == entry.name)
            || entries.prefixes.iter().chain(entries.features.iter()).any(|e| e.code.contains(&block));
        if !written {
            entries.features.push(entry);
        }
    }
}

#[cfg(test)]
#[path = "optical_tests.rs"]
mod tests;
