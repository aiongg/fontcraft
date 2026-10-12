//! Variation utilities: coordinate mapping and `avar` building (M12.33; rows `variations map`,
//! `variations build-avar`, `variations.mapCoordinates`, `variations.buildAvar`). Oracles:
//! fontTools `varLib.avar.map` and `varLib.avar`.
//!
//! Ported from fontTools 4.66 (MIT; see `NOTICE`): `varLib.avar.map.map` and `_denormalize`,
//! `ttLib.tables._a_v_a_r.renormalizeLocation`, `varLib.models.normalizeValue`,
//! `piecewiseLinearMap` and `supportScalar`, `varLib.varStore.VarStoreInstancer`,
//! `varLib._add_avar` and `designspaceLib`'s `get_validated_map`, so the numbers match the ones
//! fontTools computes:
//!
//! - avar version 1 maps unquantised values; version 2 quantises the mapped location to F2Dot14,
//!   adds the rounded deltas of its variation store (scalars taken at the unquantised location)
//!   and clamps to −1…1.
//! - Without `avar` version 2, axes the caller did not name stay at 0 and are not mapped.
//! - Building takes each axis's user-to-design map, normalises its inputs by the `fvar` range
//!   and its outputs by that range mapped to design space, and writes no `avar` at all when
//!   every map is the identity and there is no cross-axis mapping.

mod designspace;
#[cfg(test)]
mod tests;

use read_fonts::tables::avar::Avar as ReadAvar;
use read_fonts::tables::fvar::Fvar;
use read_fonts::tables::variations::{DeltaSetIndex, ItemVariationStore};
use read_fonts::{FontData, FontRead};
use write_fonts::NullableOffsetMarker;
use write_fonts::tables::avar::{Avar, AxisValueMap, SegmentMaps};
use write_fonts::tables::variations::ivs_builder::VariationStoreBuilder;
use write_fonts::tables::variations::{DeltaSetIndexMap, RegionAxisCoordinates, VariationRegion};
use write_fonts::types::F2Dot14;

use crate::limits::MAX_AXES;
use crate::sfnt::{SfntFont, Tag, tag_str};
use crate::{Result, TablesError};

pub use designspace::avar_options_from_designspace;

/// Most points in one axis map.
pub const MAX_MAP_POINTS: usize = 1_024;
/// Most cross-axis mappings in one `avar` build.
pub const MAX_CROSS_AXIS_MAPPINGS: usize = 1_024;
/// Most locations one call maps.
pub const MAX_LOCATIONS: usize = 100_000;
/// Largest designspace document read, in bytes.
pub const MAX_DESIGNSPACE: usize = 64 << 20;

const AVAR: Tag = *b"avar";
const FVAR: Tag = *b"fvar";
/// F2Dot14's one.
const ONE: f64 = 16384.0;
/// Encoded as `outer << 16 | inner`: no variation.
const NO_VARIATION_INDEX: u32 = 0xFFFF_FFFF;

/// The coordinate space of a location.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Space {
    /// User coordinates (`fvar` values).
    #[default]
    User,
    /// Normalised coordinates before `avar`.
    Normalized,
    /// Normalised coordinates after `avar`.
    Mapped,
}

/// One location in all three spaces, by axis tag.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct MappedLocation {
    pub user: Vec<(Tag, f64)>,
    pub normalized: Vec<(Tag, f64)>,
    pub mapped: Vec<(Tag, f64)>,
}

/// One axis of the `fvar` table.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FvarAxis {
    pub tag: Tag,
    pub minimum: f64,
    pub default: f64,
    pub maximum: f64,
}

/// The axes of the font's `fvar` table, in table order.
pub fn fvar_axes(font: &SfntFont) -> Result<Vec<FvarAxis>> {
    let data = font.get(FVAR).ok_or_else(|| TablesError::MissingTable("fvar".into()))?;
    let fvar = Fvar::read(FontData::new(data)).map_err(|e| TablesError::invalid(format!("the fvar table cannot be read: {e}")))?;
    let records = fvar.axes().map_err(|e| TablesError::invalid(format!("the fvar axes cannot be read: {e}")))?;
    if records.len() > MAX_AXES {
        return Err(TablesError::TooLarge { what: "the fvar axis count", limit: MAX_AXES });
    }
    let mut out = Vec::with_capacity(records.len());
    for r in records {
        let axis = FvarAxis {
            tag: r.axis_tag().to_be_bytes(),
            minimum: r.min_value().to_f64(),
            default: r.default_value().to_f64(),
            maximum: r.max_value().to_f64(),
        };
        if !(axis.minimum <= axis.default && axis.default <= axis.maximum) {
            return Err(TablesError::invalid(format!(
                "the fvar axis `{}` has minimum {}, default {} and maximum {} out of order",
                tag_str(axis.tag),
                axis.minimum,
                axis.default,
                axis.maximum
            )));
        }
        if out.iter().any(|a: &FvarAxis| a.tag == axis.tag) {
            return Err(TablesError::invalid(format!("the fvar table has the axis `{}` twice", tag_str(axis.tag))));
        }
        out.push(axis);
    }
    Ok(out)
}

/// A user value normalised by the axis range, clamped to the range: −1 at the minimum, 0 at the
/// default, 1 at the maximum, linear in between.
pub fn normalize_value(v: f64, axis: &FvarAxis) -> f64 {
    normalize_triple(v, (axis.minimum, axis.default, axis.maximum))
}

/// A normalised value back in user coordinates (fontTools' `_denormalize`, which does not clamp).
pub fn denormalize_value(v: f64, axis: &FvarAxis) -> f64 {
    if v >= 0.0 { axis.default + v * (axis.maximum - axis.default) } else { axis.default + v * (axis.default - axis.minimum) }
}

fn normalize_triple(v: f64, (lower, default, upper): (f64, f64, f64)) -> f64 {
    let v = v.max(lower).min(upper);
    if v == default || lower == upper {
        return 0.0;
    }
    let span = if v < default && lower != default { default - lower } else { upper - default };
    if span == 0.0 { 0.0 } else { (v - default) / span }
}

/// fontTools' `piecewiseLinearMap` over `map`, sorted by its first values with no repeats.
fn piecewise(v: f64, map: &[(f64, f64)]) -> f64 {
    let (Some(&(first, first_to)), Some(&(last, last_to))) = (map.first(), map.last()) else { return v };
    if let Some(&(_, to)) = map.iter().find(|(k, _)| *k == v) {
        return to;
    }
    if v < first {
        return v + first_to - first;
    }
    if v > last {
        return v + last_to - last;
    }
    let (Some(&(a, va)), Some(&(b, vb))) = (map.iter().rev().find(|(k, _)| *k < v), map.iter().find(|(k, _)| *k > v)) else { return v };
    va + (vb - va) * (v - a) / (b - a)
}

/// Sorts `pairs` by their first value; of pairs with the same first value the last one is kept,
/// as a dictionary built from them keeps it.
fn as_map(mut pairs: Vec<(f64, f64)>) -> Vec<(f64, f64)> {
    // A stable sort keeps the order of equal keys, so the last of a run is the last given.
    pairs.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut out: Vec<(f64, f64)> = Vec::with_capacity(pairs.len());
    for (k, v) in pairs {
        match out.last_mut() {
            Some(last) if last.0 == k => last.1 = v,
            _ => out.push((k, v)),
        }
    }
    out
}

/// fontTools' `otRound`.
fn ot_round(v: f64) -> f64 {
    (v + 0.5).floor()
}

/// fontTools' `floatToFixed(v, 14)`, saturated to the F2Dot14 range.
fn f2dot14(v: f64) -> F2Dot14 {
    let bits = ot_round(v * ONE).clamp(f64::from(i16::MIN), f64::from(i16::MAX));
    F2Dot14::from_bits(bits as i16)
}

fn f2(v: F2Dot14) -> f64 {
    f64::from(v.to_bits()) / ONE
}

/// The `avar` table as read for mapping.
struct AvarData<'a> {
    major: u16,
    /// Per `fvar` axis, the segment map as a dictionary keeps it.
    segments: Vec<Vec<(f64, f64)>>,
    table: ReadAvar<'a>,
}

fn read_avar<'a>(font: &'a SfntFont, axes: &[FvarAxis]) -> Result<Option<AvarData<'a>>> {
    let Some(data) = font.get(AVAR) else { return Ok(None) };
    let major = avar_major(data)?;
    let bad = |e: read_fonts::ReadError| TablesError::invalid(format!("the avar table cannot be read: {e}"));
    let table = ReadAvar::read(FontData::new(data)).map_err(bad)?;
    let mut segments = Vec::with_capacity(axes.len());
    for map in table.axis_segment_maps().iter().take(axes.len()) {
        let map = map.map_err(bad)?;
        segments.push(as_map(map.axis_value_maps().iter().map(|m| (f2(m.from_coordinate()), f2(m.to_coordinate()))).collect()));
    }
    Ok(Some(AvarData { major, segments, table }))
}

/// fontTools' `supportScalar` with OpenType's rules, over one region of the store.
fn region_scalar(store: &ItemVariationStore<'_>, region: u16, location: &[f64]) -> Result<f64> {
    let bad = |e: read_fonts::ReadError| TablesError::invalid(format!("the avar variation regions cannot be read: {e}"));
    let list = store.variation_region_list().map_err(bad)?;
    let region = list.variation_regions().get(usize::from(region)).map_err(bad)?;
    let mut scalar = 1.0;
    for (i, axis) in region.region_axes().iter().enumerate() {
        let (lower, peak, upper) = (f2(axis.start_coord()), f2(axis.peak_coord()), f2(axis.end_coord()));
        if peak == 0.0 || lower > peak || peak > upper || (lower < 0.0 && upper > 0.0) {
            continue;
        }
        let v = location.get(i).copied().unwrap_or(0.0);
        if v == peak {
            continue;
        }
        if v <= lower || upper <= v {
            return Ok(0.0);
        }
        scalar *= if v < peak { (v - lower) / (peak - lower) } else { (v - upper) / (peak - upper) };
    }
    Ok(scalar)
}

/// The delta of one store item at `location` (unrounded); an index outside the store is 0.
fn store_delta(store: &ItemVariationStore<'_>, index: DeltaSetIndex, location: &[f64]) -> Result<f64> {
    if usize::from(index.outer) >= usize::from(store.item_variation_data_count()) {
        return Ok(0.0);
    }
    let Some(data) = store.item_variation_data().get(usize::from(index.outer)) else { return Ok(0.0) };
    let data = data.map_err(|e| TablesError::invalid(format!("the avar variation data cannot be read: {e}")))?;
    if index.inner >= data.item_count() {
        return Ok(0.0);
    }
    let mut delta = 0.0;
    for (region, d) in data.region_indexes().iter().zip(data.delta_set(index.inner)) {
        let scalar = region_scalar(store, region.get(), location)?;
        if scalar != 0.0 {
            delta += f64::from(d) * scalar;
        }
    }
    Ok(delta)
}

/// `normalized` (one value per `fvar` axis; `given` tells which axes the caller named) through
/// the `avar` table.
fn renormalize(avar: Option<&AvarData<'_>>, given: &[bool], normalized: &[f64]) -> Result<Vec<f64>> {
    let Some(avar) = avar else { return Ok(normalized.to_vec()) };
    let mapped: Vec<f64> = normalized
        .iter()
        .zip(given)
        .enumerate()
        .map(|(i, (&v, &g))| if g { avar.segments.get(i).map_or(v, |s| piecewise(v, s)) } else { 0.0 })
        .collect();
    if avar.major < 2 {
        return Ok(mapped);
    }
    let bad = |e: read_fonts::ReadError| TablesError::invalid(format!("the avar version 2 data cannot be read: {e}"));
    let store = avar.table.var_store().transpose().map_err(bad)?;
    let index_map = avar.table.axis_index_map().transpose().map_err(bad)?;
    let mut out = Vec::with_capacity(mapped.len());
    for (i, &v) in mapped.iter().enumerate() {
        let mut coord = ot_round(v * ONE);
        if let Some(store) = &store {
            let index = match &index_map {
                Some(map) => map.get(u32::try_from(i).unwrap_or(u32::MAX)).map_err(bad)?,
                None => DeltaSetIndex { outer: 0, inner: u16::try_from(i).unwrap_or(u16::MAX) },
            };
            coord += ot_round(store_delta(store, index, &mapped)?);
        }
        out.push(coord.clamp(-ONE, ONE) / ONE);
    }
    Ok(out)
}

/// The inverse of an `avar` version 1 table for the axes the caller named.
fn unmap(avar: Option<&AvarData<'_>>, given: &[bool], mapped: &[f64]) -> Result<Vec<f64>> {
    let Some(avar) = avar else { return Ok(mapped.to_vec()) };
    if avar.major >= 2 && (avar.table.var_store().is_some() || avar.table.axis_index_map().is_some()) {
        return Err(TablesError::Unsupported("mapping back through avar version 2 cross-axis mappings".into()));
    }
    Ok(mapped
        .iter()
        .zip(given)
        .enumerate()
        .map(|(i, (&v, &g))| {
            if !g {
                return 0.0;
            }
            let Some(segments) = avar.segments.get(i) else { return v };
            // Several inputs with one output: the first of them.
            let mut back: Vec<(f64, f64)> = Vec::with_capacity(segments.len());
            for &(from, to) in segments {
                if !back.iter().any(|(t, _)| *t == to) {
                    back.push((to, from));
                }
            }
            back.sort_by(|a, b| a.0.total_cmp(&b.0));
            piecewise(v, &back)
        })
        .collect())
}

/// Maps each location (given in `from`) into every space. At most
/// [`crate::limits::MAX_AXES`] axes per location; values finite.
///
/// Each result lists every `fvar` axis in table order. Axes a location does not name are at their
/// default; they stay at 0 through `avar` version 1, and version 2 may move them.
pub fn map_coordinates(font: &SfntFont, locations: &[Vec<(Tag, f64)>], from: Space) -> Result<Vec<MappedLocation>> {
    if locations.len() > MAX_LOCATIONS {
        return Err(TablesError::TooLarge { what: "the location count", limit: MAX_LOCATIONS });
    }
    let axes = fvar_axes(font)?;
    let avar = read_avar(font, &axes)?;
    let mut out = Vec::with_capacity(locations.len());
    for location in locations {
        if location.len() > MAX_AXES {
            return Err(TablesError::TooLarge { what: "the axes of one location", limit: MAX_AXES });
        }
        let mut unknown: Vec<String> = location.iter().filter(|(t, _)| !axes.iter().any(|a| a.tag == *t)).map(|(t, _)| tag_str(*t)).collect();
        if !unknown.is_empty() {
            unknown.sort();
            unknown.dedup();
            return Err(TablesError::BadOptions(format!("unknown axis tag(s): {}", unknown.join(", "))));
        }
        let mut given = vec![false; axes.len()];
        let mut values = vec![0.0; axes.len()];
        for (tag, v) in location {
            if !v.is_finite() {
                return Err(TablesError::BadOptions(format!("the value of `{}` is not a finite number", tag_str(*tag))));
            }
            let Some(i) = axes.iter().position(|a| a.tag == *tag) else { continue };
            if given.get(i).copied().unwrap_or(false) {
                return Err(TablesError::BadOptions(format!("the axis `{}` is given twice", tag_str(*tag))));
            }
            if let (Some(g), Some(slot)) = (given.get_mut(i), values.get_mut(i)) {
                *g = true;
                *slot = *v;
            }
        }
        let normalized: Vec<f64> = match from {
            Space::User => axes.iter().zip(&values).zip(&given).map(|((a, &v), &g)| if g { normalize_value(v, a) } else { 0.0 }).collect(),
            Space::Normalized => values.clone(),
            Space::Mapped => unmap(avar.as_ref(), &given, &values)?,
        };
        let mapped = match from {
            Space::Mapped => values.clone(),
            _ => renormalize(avar.as_ref(), &given, &normalized)?,
        };
        let user: Vec<f64> = match from {
            Space::User => axes.iter().zip(&values).zip(&given).map(|((a, &v), &g)| if g { v } else { a.default }).collect(),
            _ => axes.iter().zip(&normalized).map(|(a, &v)| denormalize_value(v, a)).collect(),
        };
        let tagged = |vals: &[f64]| axes.iter().zip(vals).map(|(a, &v)| (a.tag, v)).collect::<Vec<_>>();
        out.push(MappedLocation { user: tagged(&user), normalized: tagged(&normalized), mapped: tagged(&mapped) });
    }
    Ok(out)
}

/// The major version of an `avar` table, checked before the table is read: versions other than
/// 1 and 2 have a layout this reader does not know.
fn avar_major(data: &[u8]) -> Result<u16> {
    let major = data.get(..2).map(|b| u16::from_be_bytes([b.first().copied().unwrap_or(0), b.get(1).copied().unwrap_or(0)]));
    match major {
        None => Err(TablesError::invalid("the avar table is cut short")),
        Some(m @ 1..=2) => Ok(m),
        Some(m) => Err(TablesError::Unsupported(format!("avar version {m}"))),
    }
}

/// The major version of the font's `avar` table; `None` without one.
pub fn avar_version(font: &SfntFont) -> Result<Option<u16>> {
    font.get(AVAR).map(avar_major).transpose()
}

/// The segment maps of the font's `avar` table, normalised, one per `fvar` axis in table order;
/// empty without an `avar` table.
pub fn segment_maps(font: &SfntFont) -> Result<Vec<(Tag, Vec<(f64, f64)>)>> {
    let axes = fvar_axes(font)?;
    let Some(avar) = read_avar(font, &axes)? else { return Ok(Vec::new()) };
    Ok(axes.iter().zip(avar.segments).map(|(a, s)| (a.tag, s)).collect())
}

/// A cross-axis mapping of `avar` version 2: where `input` is reached, the axes in `output` move
/// to the values given there. Both in design coordinates, by axis tag.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CrossAxisMapping {
    pub input: Vec<(Tag, f64)>,
    pub output: Vec<(Tag, f64)>,
}

/// The segment maps of an `avar` table, version 1, and the version 2 extras.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AvarOptions {
    /// Per axis, `(from, to)` pairs: user coordinates to design coordinates.
    pub maps: Vec<(Tag, Vec<(f64, f64)>)>,
    /// Write `avar` version 2 even without cross-axis mappings.
    pub version2: bool,
    /// Cross-axis mappings, written as `avar` version 2.
    pub mappings: Vec<CrossAxisMapping>,
}

/// The options that rebuild the font's own `avar` table: its segment maps as user-space maps.
/// A font without `avar` gives empty options. Version 2 cross-axis data cannot be read back into
/// mappings, so a font that has it is refused.
pub fn avar_options_from_font(font: &SfntFont) -> Result<AvarOptions> {
    let axes = fvar_axes(font)?;
    let Some(avar) = read_avar(font, &axes)? else { return Ok(AvarOptions::default()) };
    if avar.major >= 2 && (avar.table.var_store().is_some() || avar.table.axis_index_map().is_some()) {
        return Err(TablesError::Unsupported("rebuilding avar version 2 cross-axis mappings without the designspace".into()));
    }
    let maps = axes
        .iter()
        .zip(&avar.segments)
        .filter(|(_, s)| !s.is_empty())
        .map(|(a, s)| (a.tag, s.iter().map(|&(k, v)| (denormalize_value(k, a), denormalize_value(v, a))).collect()))
        .collect();
    Ok(AvarOptions { maps, version2: avar.major >= 2, mappings: Vec::new() })
}

/// One axis's map checked as fontTools checks it: exact repeats dropped, an input with two
/// outputs refused.
fn validated_map(tag: Tag, pairs: &[(f64, f64)]) -> Result<Vec<(f64, f64)>> {
    if pairs.len() > MAX_MAP_POINTS {
        return Err(TablesError::TooLarge { what: "the points of one axis map", limit: MAX_MAP_POINTS });
    }
    let mut out: Vec<(f64, f64)> = Vec::with_capacity(pairs.len());
    for &(input, output) in pairs {
        if !(input.is_finite() && output.is_finite()) {
            return Err(TablesError::BadOptions(format!("the map of axis `{}` has a value that is not a finite number", tag_str(tag))));
        }
        match out.iter().find(|(i, _)| *i == input) {
            Some(&(_, previous)) if previous != output => {
                return Err(TablesError::BadOptions(format!(
                    "axis `{}`: the map input {input} has the outputs {previous} and {output}",
                    tag_str(tag)
                )));
            }
            Some(_) => {}
            None => out.push((input, output)),
        }
    }
    Ok(out)
}

/// One axis's segment map from its user-to-design map, and the axis range in design
/// coordinates. `None` for the map when it changes nothing.
fn axis_segments(axis: &FvarAxis, map: &[(f64, f64)]) -> Result<(Option<Vec<(f64, f64)>>, (f64, f64, f64))> {
    let name = tag_str(axis.tag);
    let keys_triple = (axis.minimum, axis.default, axis.maximum);
    if map.is_empty() {
        return Ok((None, keys_triple));
    }
    let forward = as_map(map.to_vec());
    let vals_triple = (piecewise(axis.minimum, &forward), piecewise(axis.default, &forward), piecewise(axis.maximum, &forward));
    let mut items = map.to_vec();
    items.sort_by(|a, b| a.0.total_cmp(&b.0));
    let lowest = items.first().map_or(axis.minimum, |p| p.0);
    let highest = items.last().map_or(axis.maximum, |p| p.0);
    if axis.minimum != lowest {
        return Err(TablesError::BadOptions(format!(
            "axis `{name}`: there must be a mapping for the axis minimum value {} and it must be the lowest input mapping value",
            axis.minimum
        )));
    }
    if axis.maximum != highest {
        return Err(TablesError::BadOptions(format!(
            "axis `{name}`: there must be a mapping for the axis maximum value {} and it must be the highest input mapping value",
            axis.maximum
        )));
    }
    if !items.iter().any(|p| p.0 == axis.default) {
        return Err(TablesError::BadOptions(format!("axis `{name}`: there must be a mapping for the axis default value {}", axis.default)));
    }
    if items.windows(2).any(|w| matches!(w, [a, b] if b.1 < a.1)) {
        return Err(TablesError::BadOptions(format!("axis `{name}`: mapping output values must be in ascending order")));
    }
    let pairs: Vec<(f64, f64)> = items.iter().map(|&(k, v)| (normalize_triple(k, keys_triple), normalize_triple(v, vals_triple))).collect();
    if pairs.iter().all(|(k, v)| k == v) {
        return Ok((None, vals_triple));
    }
    let mut curve = vec![(-1.0, -1.0), (0.0, 0.0), (1.0, 1.0)];
    curve.extend(pairs);
    Ok((Some(as_map(curve)), vals_triple))
}

fn segment_maps_table(pairs: &[(f64, f64)]) -> SegmentMaps {
    SegmentMaps::new(pairs.iter().map(|&(k, v)| AxisValueMap::new(f2dot14(k), f2dot14(v))).collect())
}

/// The variation store and index map of the cross-axis mappings, following fontTools'
/// `_add_avar`: deltas of `output − input` per axis at each normalised input location, the
/// default location added when no mapping starts there.
fn cross_axis_store(
    axes: &[FvarAxis],
    design_triples: &[(f64, f64, f64)],
    mappings: &[CrossAxisMapping],
) -> Result<(write_fonts::tables::variations::ItemVariationStore, DeltaSetIndexMap)> {
    use fontcraft_geom::variation::{VariationError, VariationModel};
    if mappings.len() > MAX_CROSS_AXIS_MAPPINGS {
        return Err(TablesError::TooLarge { what: "the cross-axis mapping count", limit: MAX_CROSS_AXIS_MAPPINGS });
    }
    let normalized = |loc: &[(Tag, f64)]| -> Result<Vec<Option<f64>>> {
        let mut out = vec![None; axes.len()];
        for (tag, v) in loc {
            let i = axes.iter().position(|a| a.tag == *tag).ok_or_else(|| {
                TablesError::BadOptions(format!("a cross-axis mapping names the axis `{}`, which the font does not have", tag_str(*tag)))
            })?;
            if !v.is_finite() {
                return Err(TablesError::BadOptions(format!("a cross-axis mapping gives `{}` a value that is not a finite number", tag_str(*tag))));
            }
            let triple = design_triples.get(i).copied().unwrap_or((0.0, 0.0, 0.0));
            if let Some(slot) = out.get_mut(i) {
                *slot = Some(normalize_triple(*v, triple));
            }
        }
        Ok(out)
    };
    let mut inputs: Vec<Vec<Option<f64>>> = Vec::with_capacity(mappings.len() + 1);
    let mut outputs: Vec<Vec<Option<f64>>> = Vec::with_capacity(mappings.len() + 1);
    for m in mappings {
        inputs.push(normalized(&m.input)?);
        outputs.push(normalized(&m.output)?);
    }
    if !inputs.iter().any(|loc| loc.iter().all(|v| v.is_none_or(|v| v == 0.0))) {
        inputs.insert(0, vec![None; axes.len()]);
        outputs.insert(0, vec![None; axes.len()]);
    }
    let locations: Vec<Vec<f64>> = inputs.iter().map(|loc| loc.iter().map(|v| v.unwrap_or(0.0)).collect()).collect();
    let model_error = |e: VariationError| match e {
        VariationError::NotYetImplemented(_) => TablesError::Unsupported(format!("cross-axis mappings ({e})")),
        other => TablesError::BadOptions(format!("the cross-axis mappings: {other}")),
    };
    let model = VariationModel::new(&locations, false).map_err(model_error)?;
    // One row of master values per mapping, one column per axis, in F2Dot14 units.
    let masters: Vec<Vec<f64>> = inputs
        .iter()
        .zip(&outputs)
        .map(|(vi, vo)| vi.iter().zip(vo).map(|(i, o)| o.map_or(0.0, |o| f64::from(f2dot14(o - i.unwrap_or(0.0)).to_bits()))).collect())
        .collect();
    let deltas = model.deltas(&masters).map_err(model_error)?;
    let regions = model.regions();
    if regions.len() != deltas.len() {
        return Err(TablesError::Unsupported("cross-axis mappings (the variation model gives no regions)".into()));
    }
    let axis_count = u16::try_from(axes.len()).map_err(|_| TablesError::TooLarge { what: "the axis count", limit: MAX_AXES })?;
    let mut builder = VariationStoreBuilder::new(axis_count);
    let mut ids = Vec::with_capacity(axes.len());
    for a in 0..axes.len() {
        let row: Vec<(VariationRegion, i32)> = regions
            .iter()
            .zip(&deltas)
            .skip(1)
            .map(|(region, d)| {
                let coords = region.0.iter().map(|r| RegionAxisCoordinates::new(f2dot14(r.start), f2dot14(r.peak), f2dot14(r.end))).collect();
                let delta = ot_round(d.get(a).copied().unwrap_or(0.0)).clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32;
                (VariationRegion::new(coords), delta)
            })
            .collect();
        ids.push(builder.add_deltas(row));
    }
    let (store, remap) = builder.build();
    let map: DeltaSetIndexMap = ids
        .into_iter()
        .map(|id| remap.get(id).map_or(NO_VARIATION_INDEX, |v| (u32::from(v.delta_set_outer_index) << 16) | u32::from(v.delta_set_inner_index)))
        .collect();
    Ok((store, map))
}

/// The font with a new `avar` table.
///
/// Every `fvar` axis gets a segment map: its own when [`AvarOptions::maps`] has one that changes
/// something, (−1, −1), (0, 0), (1, 1) otherwise. When every map is the identity and there is no
/// cross-axis mapping the font gets no `avar` table, and an old one is removed.
pub fn build_avar(font: &SfntFont, options: &AvarOptions) -> Result<SfntFont> {
    let axes = fvar_axes(font)?;
    for (i, (tag, _)) in options.maps.iter().enumerate() {
        if !axes.iter().any(|a| a.tag == *tag) {
            return Err(TablesError::BadOptions(format!("the font has no axis `{}`", tag_str(*tag))));
        }
        if options.maps.iter().skip(i + 1).any(|(t, _)| t == tag) {
            return Err(TablesError::BadOptions(format!("the axis `{}` has two maps", tag_str(*tag))));
        }
    }
    let mut interesting = false;
    let mut segments = Vec::with_capacity(axes.len());
    let mut design_triples = Vec::with_capacity(axes.len());
    for axis in &axes {
        let map = match options.maps.iter().find(|(t, _)| *t == axis.tag) {
            Some((_, pairs)) => validated_map(axis.tag, pairs)?,
            None => Vec::new(),
        };
        let (curve, triple) = axis_segments(axis, &map)?;
        design_triples.push(triple);
        match curve {
            Some(curve) => {
                interesting = true;
                segments.push(segment_maps_table(&curve));
            }
            None => segments.push(segment_maps_table(&[(-1.0, -1.0), (0.0, 0.0), (1.0, 1.0)])),
        }
    }
    let mut out = font.clone();
    out.remove(AVAR);
    if !interesting && options.mappings.is_empty() {
        return Ok(out);
    }
    let mut avar = Avar::new(segments);
    if !options.mappings.is_empty() {
        let (store, map) = cross_axis_store(&axes, &design_triples, &options.mappings)?;
        avar.axis_index_map = NullableOffsetMarker::new(Some(map));
        avar.var_store = NullableOffsetMarker::new(Some(store));
    }
    let mut bytes = write_fonts::dump_table(&avar).map_err(|e| TablesError::invalid(format!("the avar table cannot be written: {e}")))?;
    if options.version2 && options.mappings.is_empty() {
        // Version 2 without cross-axis data: the version number and two null offsets.
        if let Some(major) = bytes.get_mut(..2) {
            major.copy_from_slice(&2u16.to_be_bytes());
        }
        bytes.extend_from_slice(&[0; 8]);
    }
    out.insert(AVAR, bytes);
    Ok(out)
}
