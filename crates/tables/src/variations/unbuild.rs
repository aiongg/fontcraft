//! The `avar` table as a designspace snippet (M12.76; row `dump`, view `avar=designspace`).
//! Oracle: fontTools `varLib.avar.unbuild`.
//!
//! Ported from fontTools 4.66 `varLib/avar/unbuild.py` (MIT, Copyright (c) 2017 Just van Rossum,
//! Behdad Esfahbod and the fontTools authors; see `NOTICE`): `mappings_from_avar`,
//! `_pruneLocations` and `unbuild`.
//!
//! - Each axis prints its `fvar` range and, unless its segment map is empty or the identity, the
//!   map in user coordinates, sorted by input.
//! - With `avar` version 2, the input locations are the peaks and corners of the variation
//!   store's regions; the ones the variation model does not need are pruned, and each kept
//!   location prints the axes the store moves (unrounded deltas) as a cross-axis mapping. The
//!   mapping at the default location is left out when it moves nothing.
//! - Numbers print as integers when whole, otherwise as Python writes floats.

use std::collections::BTreeSet;
use std::fmt::Write as _;

use fontcraft_geom::variation::{VariationError, VariationModel};
use read_fonts::tables::fvar::Fvar;
use read_fonts::tables::variations::DeltaSetIndex;
use read_fonts::{FontData, FontRead};

use super::{FvarAxis, ONE, denormalize_value, f2, fvar_axes, read_avar, store_delta};
use crate::sfnt::{SfntFont, tag_str};
use crate::{Result, TablesError};

/// Most input locations the version 2 mappings consider (the variation model's master limit).
const MAX_INPUT_LOCATIONS: usize = fontcraft_geom::variation::MAX_MASTERS;

/// A normalised location: `(axis index, value)` pairs in axis order, zeros left out.
type Location = Vec<(usize, f64)>;

/// The font's `avar` table (or its `fvar` axes alone) as designspace XML.
pub fn avar_designspace(font: &SfntFont) -> Result<String> {
    let axes = fvar_axes(font)?;
    let names = axis_names(font, &axes)?;
    let (segments, mappings) = mappings(font, &axes)?;
    let mut out = String::new();
    let mut line = |s: &str| {
        out.push_str(s);
        out.push('\n');
    };
    line("<?xml version='1.0' encoding='UTF-8'?>");
    line("<designspace format=\"5.1\">");
    line("  <axes>");
    for (i, axis) in axes.iter().enumerate() {
        let name = names.get(i).map_or("", String::as_str);
        let map = segments.iter().find(|(a, _)| *a == i).map(|(_, m)| m);
        let closing = if map.is_some() { ">" } else { "/>" };
        line(&format!(
            "    <axis tag=\"{}\" name=\"{name}\" minimum=\"{}\" maximum=\"{}\" default=\"{}\"{closing}",
            escape_attr(&tag_str(axis.tag)),
            num(axis.minimum),
            num(axis.maximum),
            num(axis.default)
        ));
        if let Some(map) = map {
            for (k, v) in map {
                line(&format!("      <map input=\"{}\" output=\"{}\"/>", num(*k), num(*v)));
            }
            line("    </axis>");
        }
    }
    if !mappings.is_empty() {
        line("    <mappings>");
        for (input, output) in &mappings {
            line("      <mapping>");
            for (element, location) in [("input", input), ("output", output)] {
                line(&format!("        <{element}>"));
                // fontTools sorts the dimensions by axis tag.
                let mut dims: Vec<(String, usize, f64)> =
                    location.iter().map(|&(i, v)| (axes.get(i).map(|a| tag_str(a.tag)).unwrap_or_default(), i, v)).collect();
                dims.sort_by(|a, b| a.0.cmp(&b.0));
                for (_, i, v) in dims {
                    let name = names.get(i).map_or("", String::as_str);
                    line(&format!("          <dimension name=\"{name}\" xvalue=\"{}\"/>", num(v)));
                }
                line(&format!("        </{element}>"));
            }
            line("      </mapping>");
        }
        line("    </mappings>");
    }
    line("  </axes>");
    line("</designspace>");
    Ok(out)
}

/// Each axis's name as `getDebugName` gives it, or its tag; escaped for an attribute.
fn axis_names(font: &SfntFont, axes: &[FvarAxis]) -> Result<Vec<String>> {
    let data = font.get(*b"fvar").ok_or_else(|| TablesError::MissingTable("fvar".into()))?;
    let fvar = Fvar::read(FontData::new(data)).map_err(|e| TablesError::invalid(format!("the fvar table cannot be read: {e}")))?;
    let records = fvar.axes().map_err(|e| TablesError::invalid(format!("the fvar axes cannot be read: {e}")))?;
    let name_table = font.get(*b"name");
    Ok(axes
        .iter()
        .zip(records)
        .map(|(axis, record)| {
            let name = name_table
                .and_then(|data| crate::ttx::otdata::debug_name(data, i64::from(record.axis_name_id().to_u16())))
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| tag_str(axis.tag));
            escape_attr(&name)
        })
        .collect())
}

/// The segment maps that change something, by axis index, in user coordinates and sorted by
/// input; and the version 2 cross-axis mappings, in user coordinates.
#[allow(clippy::type_complexity)]
fn mappings(font: &SfntFont, axes: &[FvarAxis]) -> Result<(Vec<(usize, Vec<(f64, f64)>)>, Vec<(Location, Location)>)> {
    let Some(avar) = read_avar(font, axes)? else { return Ok((Vec::new(), Vec::new())) };
    let identity = [(-1.0, -1.0), (0.0, 0.0), (1.0, 1.0)];
    let mut segments = Vec::new();
    for (i, seg) in avar.segments.iter().enumerate() {
        if seg.is_empty() || (seg.len() == identity.len() && identity.iter().all(|p| seg.contains(p))) {
            continue;
        }
        let Some(axis) = axes.get(i) else { continue };
        let mut map: Vec<(f64, f64)> = Vec::with_capacity(seg.len());
        for &(k, v) in seg {
            let (k, v) = (denormalize_value(k, axis), denormalize_value(v, axis));
            // A dictionary keeps the last value of a repeated key.
            match map.iter_mut().find(|(mk, _)| *mk == k) {
                Some(slot) => slot.1 = v,
                None => map.push((k, v)),
            }
        }
        map.sort_by(|a, b| a.0.total_cmp(&b.0));
        segments.push((i, map));
    }
    let mut mappings = Vec::new();
    let bad = |e: read_fonts::ReadError| TablesError::invalid(format!("the avar version 2 data cannot be read: {e}"));
    let store = if avar.major >= 2 { avar.table.var_store().transpose().map_err(bad)? } else { None };
    if let Some(store) = store {
        let regions = store.variation_region_list().map_err(bad)?;
        let region_list = regions.variation_regions();
        let mut poles: Vec<Location> = vec![Vec::new()];
        let mut inputs: Vec<Location> = vec![Vec::new()];
        for data in store.item_variation_data().iter() {
            let Some(data) = data else { continue };
            let data = data.map_err(bad)?;
            for index in data.region_indexes() {
                let region = region_list.get(usize::from(index.get())).map_err(bad)?;
                let mut peak: Location = Vec::new();
                let mut corners: Vec<Vec<(usize, f64)>> = Vec::new();
                for (a, axis) in region.region_axes().iter().enumerate() {
                    if axis.peak_coord().to_bits() == 0 {
                        continue;
                    }
                    if a >= axes.len() {
                        return Err(TablesError::invalid("an avar variation region has more axes than the fvar table"));
                    }
                    peak.push((a, f2(axis.peak_coord())));
                    let mut corner = Vec::with_capacity(2);
                    for c in [axis.start_coord(), axis.end_coord()] {
                        if c.to_bits() != 0 {
                            corner.push((a, f2(c)));
                        }
                    }
                    corners.push(corner);
                }
                if !poles.contains(&peak) {
                    poles.push(peak.clone());
                }
                add_input(&mut inputs, peak)?;
                for corner in product(&corners)? {
                    add_input(&mut inputs, corner)?;
                }
            }
        }
        inputs.sort_by(|a, b| {
            a.len().cmp(&b.len()).then_with(|| a.iter().map(|p| p.0).cmp(b.iter().map(|p| p.0))).then_with(|| {
                a.iter()
                    .map(|p| p.1)
                    .zip(b.iter().map(|p| p.1))
                    .map(|(x, y)| x.total_cmp(&y))
                    .find(|o| o.is_ne())
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
        });
        let kept = prune(&inputs, &poles, axes.len())?;
        let index_map = avar.table.axis_index_map().transpose().map_err(bad)?;
        for location in kept {
            let mut dense = vec![0.0; axes.len()];
            for &(a, v) in &location {
                if let Some(slot) = dense.get_mut(a) {
                    *slot = v;
                }
            }
            let mut output = Vec::new();
            for (a, &v) in dense.iter().enumerate() {
                let index = match &index_map {
                    Some(map) => map.get(u32::try_from(a).unwrap_or(u32::MAX)).map_err(bad)?,
                    None => DeltaSetIndex { outer: 0, inner: u16::try_from(a).unwrap_or(u16::MAX) },
                };
                let delta = store_delta(&store, index, &dense)?;
                if delta != 0.0 {
                    output.push((a, v + delta / ONE));
                }
            }
            mappings.push((location, output));
        }
        if mappings.first().is_some_and(|(input, output)| input.is_empty() && output.is_empty()) {
            mappings.remove(0);
        }
    }
    let user = |loc: &Location| -> Location { loc.iter().map(|&(a, v)| (a, axes.get(a).map_or(v, |axis| denormalize_value(v, axis)))).collect() };
    let mappings = mappings.iter().map(|(i, o)| (user(i), user(o))).collect();
    Ok((segments, mappings))
}

/// Adds `location` to the input set unless it is there, within [`MAX_INPUT_LOCATIONS`].
fn add_input(inputs: &mut Vec<Location>, location: Location) -> Result<()> {
    if inputs.contains(&location) {
        return Ok(());
    }
    if inputs.len() >= MAX_INPUT_LOCATIONS {
        return Err(TablesError::TooLarge { what: "the input locations of the avar version 2 mappings", limit: MAX_INPUT_LOCATIONS });
    }
    inputs.push(location);
    Ok(())
}

/// Every location that takes one corner value per axis (`itertools.product`), at most
/// [`MAX_INPUT_LOCATIONS`] of them.
fn product(corners: &[Vec<(usize, f64)>]) -> Result<Vec<Location>> {
    let mut count: usize = 1;
    for c in corners {
        count = count
            .checked_mul(c.len())
            .filter(|n| *n <= MAX_INPUT_LOCATIONS)
            .ok_or(TablesError::TooLarge { what: "the input locations of the avar version 2 mappings", limit: MAX_INPUT_LOCATIONS })?;
    }
    let mut out: Vec<Location> = vec![Vec::new()];
    for c in corners {
        let mut next = Vec::with_capacity(out.len().saturating_mul(c.len()));
        for partial in &out {
            for &p in c {
                let mut l = partial.clone();
                l.push(p);
                next.push(l);
            }
        }
        out = next;
    }
    Ok(out)
}

/// `_pruneLocations`: the poles, and for each support edge of a pole that is not −1, 0 or 1 a
/// location pinning it, found among the pins so far or the earlier masters of the model.
fn prune(inputs: &[Location], poles: &[Location], axis_count: usize) -> Result<Vec<Location>> {
    let dense: Vec<Vec<f64>> = inputs
        .iter()
        .map(|loc| {
            let mut d = vec![0.0; axis_count];
            for &(a, v) in loc {
                if let Some(slot) = d.get_mut(a) {
                    *slot = v;
                }
            }
            d
        })
        .collect();
    let model = VariationModel::new(&dense, false).map_err(|e| match e {
        VariationError::Size => TablesError::TooLarge { what: "the input locations of the avar version 2 mappings", limit: MAX_INPUT_LOCATIONS },
        other => TablesError::invalid(format!("the avar version 2 regions make no variation model: {other}")),
    })?;
    // Each support in the model's order, as (axis, start, peak, end) for the axes away from 0.
    let supports: Vec<Vec<(usize, f64, f64, f64)>> = model
        .regions()
        .iter()
        .map(|r| r.0.iter().enumerate().filter(|(_, a)| a.peak != 0.0).map(|(i, a)| (i, a.start, a.peak, a.end)).collect())
        .collect();
    let mut pins: Vec<Location> = Vec::new();
    for pole in poles {
        if !pins.contains(pole) {
            pins.push(pole.clone());
        }
    }
    for pole in poles {
        let Some(i) = supports.iter().position(|s| peak_of(s) == *pole) else {
            return Err(TablesError::invalid("an avar version 2 region matches no master of the model"));
        };
        let Some(support) = supports.get(i) else { continue };
        let support_axes: BTreeSet<usize> = support.iter().map(|s| s.0).collect();
        for &(axis, start, _, end) in support {
            for v in [start, end] {
                if v == -1.0 || v == 0.0 || v == 1.0 {
                    continue;
                }
                let pinned = pins.iter().any(|pin| {
                    pin.iter().map(|p| p.0).collect::<BTreeSet<usize>>() == support_axes && pin.iter().any(|&(a, pv)| a == axis && pv == v)
                });
                if pinned {
                    continue;
                }
                let candidate = supports.iter().take(i).rev().find(|c| {
                    c.iter().map(|s| s.0).collect::<BTreeSet<usize>>() == support_axes && c.iter().any(|&(a, _, p, _)| a == axis && p == v)
                });
                match candidate {
                    Some(c) => {
                        let pin = peak_of(c);
                        if !pins.contains(&pin) {
                            pins.push(pin);
                        }
                    }
                    None => return Err(TablesError::invalid("the avar version 2 regions leave a support edge with no master to pin it")),
                }
            }
        }
    }
    Ok(pins)
}

/// The peak location of a support: `(axis, peak)` for its axes.
fn peak_of(support: &[(usize, f64, f64, f64)]) -> Location {
    support.iter().map(|&(a, _, p, _)| (a, p)).collect()
}

/// A number as `unbuild` prints it: an integer when whole, otherwise as Python writes a float.
fn num(v: f64) -> String {
    if v.is_finite() && v.fract() == 0.0 && v.abs() < 1e15 {
        let mut s = String::new();
        let _ = write!(s, "{}", v as i64);
        s
    } else {
        crate::ttx::py_float(v)
    }
}

/// fontTools' `escapeattr`.
fn escape_attr(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\r' => out.push_str("&#13;"),
            '"' => out.push_str("&quot;"),
            c => out.push(c),
        }
    }
    out
}
