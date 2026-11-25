//! The `cvar` table (M10.19).
//!
//! Rows: the `cvar` table row, `hinting.tt.variable.cvar`.
//!
//! A variable TrueType font writes the default master's `cvt `. Each other master's control
//! values are worked out the way [`crate::tt_hints::tt_hinting`] works out the default's: our
//! zones and stems ([`fontcraft_hinting::control_values`], plus the autohinter's entries when
//! Autohint is on), or the master's cvt Table. `cvar` then holds, per master region, the
//! differences from the default, so zone positions and stem widths follow the designspace.
//!
//! As in fontTools' `_merge_TTHinting` (`varLib/__init__.py`, MIT; fontmake runs it), deltas are
//! rounded half to even one region at a time, each later region taken from the rounded earlier
//! ones, and a region whose deltas are all zero is left out; write-fonts chooses between all
//! entries and a sparse point list per tuple. A master with Dehint on has no `cvt ` and adds no
//! region, the way fontTools treats a master without one as sparse; the regions come from the
//! other masters. Masters at the default's location, or at one an earlier master took, add no
//! region either, with a warning when their values differ from the earlier master's (fontTools
//! refuses two masters at one location). Intermediate (brace) layers have no control values of
//! their own and add none.
//!
//! No `cvar` is written, with a warning, when the masters' values do not line up: a master with a
//! different number of entries, zones or stems in a different order, a different source (cvt
//! Table on one master, our zones on another), or settings that cannot be read. fontTools drops
//! the font's hinting in that case; FontCraft keeps the default master's (`docs/spec/40-compile.md`).

use fontcraft_geom::variation::VariationModel;
use fontcraft_hinting::settings::family_tt_zones;
use fontcraft_hinting::{HintSettings, TtZone, autohint_prep, control_values, prepare_settings};
use fontcraft_model::Font;
use write_fonts::tables::cvar::{Cvar, CvtDeltas};

use super::gvar;
use crate::Options;
use crate::tt_hints::{AutoSetup, autohinted, imported};

/// Most control values `cvar` varies: every tuple then stays within the 16-bit size of its data.
pub const MAX_CVAR_ENTRIES: usize = 30_000;

/// The masters a variable font's `cvar` varies with, as the variable compile hands them to the
/// table stage.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CvarMasters {
    pub axis_count: u16,
    /// The default master's id.
    pub origin: String,
    /// Each other master's id and normalized location.
    pub masters: Vec<(String, Vec<f64>)>,
}

/// One master's control values and what they are made of, so two masters' values are only
/// compared entry by entry when they mean the same.
#[derive(Clone, Debug, PartialEq)]
struct MasterValues {
    values: Vec<i16>,
    /// The source and the zone and stem names the entries follow.
    layout: Vec<String>,
}

/// The control values of master `master`, chosen as [`crate::tt_hints::tt_hinting`] chooses the
/// default master's, or `None` when Dehint leaves the master without TrueType hinting.
/// `win_metrics` are the default master's, so the zones Windows compatibility adds do not vary.
fn master_values(source: &Font, master: &str, options: &Options, win_metrics: (f64, f64)) -> Result<Option<MasterValues>, String> {
    let mut settings = HintSettings::for_master(source, master).map_err(|e| e.to_string())?;
    let names = |s: &HintSettings, family: &[TtZone]| -> Vec<String> {
        let mut out: Vec<String> = s.tt_zones.iter().map(|z| format!("zone {}", z.name)).collect();
        out.extend(s.tt_stems.iter().map(|t| format!("stem {}", t.name)));
        out.extend(family.iter().map(|z| format!("family zone {}", z.name)));
        out
    };
    if autohinted(&settings, options) {
        let Some(auto) = AutoSetup::new(&settings, &mut Vec::new()) else {
            return Ok(None);
        };
        let measured = prepare_settings(source, &mut settings, &auto.options);
        if auto.options.windows_compatibility {
            let (ascent, descent) = win_metrics;
            for (name, position, size) in [("winAscent", ascent, 1.0), ("winDescent", -descent, -1.0)] {
                if position.is_finite() && !settings.tt_zones.iter().any(|z| z.name == name) {
                    settings.tt_zones.push(TtZone { name: name.into(), position, size, ..Default::default() });
                }
            }
        }
        let family = family_tt_zones(source).map_err(|e| e.to_string())?;
        let (layout, mut values) = control_values(&settings, &family).map_err(|e| e.to_string())?;
        let (extra, _) = autohint_prep(&auto.options, &settings, &layout, &measured).map_err(|e| e.to_string())?;
        values.extend(extra);
        let mut layout = names(&settings, &family);
        layout.insert(0, "autohinter".into());
        return Ok(Some(MasterValues { values, layout }));
    }
    if imported(&settings) {
        return Ok(Some(MasterValues { values: settings.cvt.unwrap_or_default(), layout: vec!["cvt Table".into()] }));
    }
    let family = family_tt_zones(source).map_err(|e| e.to_string())?;
    let (_, values) = control_values(&settings, &family).map_err(|e| e.to_string())?;
    Ok(Some(MasterValues { values, layout: names(&settings, &family) }))
}

/// The `cvar` table of a variable font whose `cvt ` is `written`, or `None` when no master's
/// values differ from the default's or they do not line up (with a warning).
pub(crate) fn font_cvar(
    source: &Font,
    masters: &CvarMasters,
    written: &[i16],
    options: &Options,
    win_metrics: (f64, f64),
    warnings: &mut Vec<String>,
) -> Option<Vec<u8>> {
    if written.is_empty() || masters.masters.is_empty() {
        return None;
    }
    let mut left_out = |reason: String| {
        warnings.push(format!(
            "no `cvar` table was written ({reason}), so TrueType zones and stems keep the default master's values across the variable font"
        ));
        None
    };
    let origin = match master_values(source, &masters.origin, options, win_metrics) {
        Ok(Some(v)) => v,
        Ok(None) => return left_out("the default master has Dehint on".into()),
        Err(e) => return left_out(format!("the default master's control values cannot be read: {e}")),
    };
    if origin.values != written {
        return left_out("the default master's control values are not the ones written".into());
    }
    let mut sources: Vec<(Vec<f64>, Vec<i16>)> = Vec::with_capacity(masters.masters.len());
    for (id, location) in &masters.masters {
        let name = source.master(id).map_or(id.as_str(), |m| m.name.as_str());
        let values = match master_values(source, id, options, win_metrics) {
            Ok(Some(v)) => v,
            // Sparse, as in fontTools: the other masters still vary the values.
            Ok(None) => continue,
            Err(e) => return left_out(format!("the control values of master `{name}` cannot be read: {e}")),
        };
        if values.layout != origin.layout || values.values.len() != origin.values.len() {
            return left_out(format!("the control values of master `{name}` do not line up with the default master's"));
        }
        sources.push((location.clone(), values.values));
    }
    let mut notes = Vec::new();
    let table = match build_cvar(masters.axis_count, written, &sources, &mut notes) {
        Ok(table) => table,
        Err(e) => left_out(e),
    };
    warnings.extend(notes);
    table
}

/// The `cvar` table for `default` (the `cvt ` written) and each master's values at its
/// normalized location, or `None` when no master differs. A master at a location already taken
/// is left out, with a warning in `warnings` when its values differ.
pub fn build_cvar(axis_count: u16, default: &[i16], masters: &[(Vec<f64>, Vec<i16>)], warnings: &mut Vec<String>) -> Result<Option<Vec<u8>>, String> {
    if default.len() > MAX_CVAR_ENTRIES {
        return Err(format!("{} control values; `cvar` varies at most {MAX_CVAR_ENTRIES}", default.len()));
    }
    let origin = vec![0.0; usize::from(axis_count)];
    let mut locations = vec![origin];
    let mut values: Vec<Vec<f64>> = vec![default.iter().map(|v| f64::from(*v)).collect()];
    for (location, cvt) in masters {
        if location.len() != usize::from(axis_count) {
            return Err(format!("a master location has {} axes; the font has {axis_count}", location.len()));
        }
        if cvt.len() != default.len() {
            return Err(format!("a master has {} control values; the default master has {}", cvt.len(), default.len()));
        }
        if let Some(i) = locations.iter().position(|l| l == location) {
            let earlier = values.get(i).is_some_and(|e| e.iter().zip(cvt).all(|(a, b)| *a == f64::from(*b)));
            if !earlier {
                let whose = if i == 0 { "the default master's" } else { "an earlier master's" };
                warnings.push(format!(
                    "a master at normalized location {location:?} shares {whose} location but has different control values; `cvar` ignores its values"
                ));
            }
            continue;
        }
        // Checked before the model is built: one region per master.
        if locations.len() > gvar::MAX_GLYPH_TUPLES {
            return Err(format!("more than {} master regions", gvar::MAX_GLYPH_TUPLES));
        }
        locations.push(location.clone());
        values.push(cvt.iter().map(|v| f64::from(*v)).collect());
    }
    if locations.len() < 2 {
        return Ok(None);
    }
    let model = VariationModel::new(&locations, false).map_err(|e| e.to_string())?;
    let deltas = model.deltas_with(&values, f64::round_ties_even).map_err(|e| e.to_string())?;
    let mut variations = Vec::new();
    for (region, d) in model.regions().into_iter().zip(deltas).skip(1) {
        if d.iter().all(|v| *v == 0.0) {
            continue;
        }
        if let Some(v) = d.iter().find(|v| !gvar::fits_i16(**v)) {
            return Err(format!("a control value delta of {v} units does not fit the table"));
        }
        let tents = gvar::tents(&region, axis_count)?;
        variations.push(CvtDeltas::new(tents, d.iter().map(|v| *v as i32).collect()));
    }
    if variations.is_empty() {
        return Ok(None);
    }
    let cvar = Cvar::new(variations, axis_count).map_err(|e| e.to_string())?;
    write_fonts::dump_table(&cvar).map(Some).map_err(|e| e.to_string())
}

#[cfg(test)]
#[path = "cvar_tests.rs"]
mod tests;
