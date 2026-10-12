//! The `avar` table (M9.6).
//!
//! Rows: the `avar` table row, `compile.avarMaps`.
//!
//! Contract: one segment map per axis from [`VarAxis::avar`](super::VarAxis::avar), an identity
//! map for axes without one; no table when every axis is identity, as fontc writes it.
//!
//! The segment maps come from [`Designspace::avar_map`], which normalizes both sides of each
//! point of an axis's external-to-internal map with the axis's own orientation, so a map whose
//! internal values fall as the external ones rise writes the same table as its rising mirror.
//! As in fontc (`fontbe` `avar.rs`, Apache-2.0), an axis whose points all map to themselves at
//! F2Dot14 precision writes the required (−1, −1), (0, 0), (1, 1) map.

use fontcraft_interp::Designspace;
use write_fonts::tables::avar::{Avar, AxisValueMap, SegmentMaps};
use write_fonts::types::F2Dot14;

use super::VarAxes;
use crate::CompileError;

/// The most points of one segment map (the map's own limit, plus the three required points).
pub const MAX_SEGMENT_POINTS: usize = fontcraft_interp::error::MAX_MAPPING_POINTS + 3;

fn avar_error(message: String) -> CompileError {
    CompileError::Table { table: "avar", message }
}

fn f2dot14(v: f64) -> F2Dot14 {
    F2Dot14::from_f32(v as f32)
}

/// The segment map of the axis at `axis` in `designspace`: `(normalized external, normalized
/// internal)` pairs, sorted, with −1, 0 and 1 present; empty when the map is the identity at
/// F2Dot14 precision.
pub fn segment_map(designspace: &Designspace, axis: usize) -> Vec<(f64, f64)> {
    let Some(points) = designspace.avar_map(axis) else { return Vec::new() };
    let mut map: Vec<(f64, f64)> = Vec::with_capacity(points.len());
    for (from, to) in points {
        // Points that round to one F2Dot14 value keep one of them; −1, 0 and 1 win.
        if let Some(last) = map.last_mut()
            && f2dot14(last.0) == f2dot14(from)
        {
            if [-1.0, 0.0, 1.0].contains(&from) {
                *last = (from, to);
            }
            continue;
        }
        map.push((from, to));
    }
    if map.iter().all(|(f, t)| f2dot14(*f) == f2dot14(*t)) {
        return Vec::new();
    }
    map
}

/// The required map of an axis without its own: (−1, −1), (0, 0), (1, 1).
fn identity_map() -> SegmentMaps {
    SegmentMaps::new(vec![
        AxisValueMap::new(F2Dot14::from_f32(-1.0), F2Dot14::from_f32(-1.0)),
        AxisValueMap::new(F2Dot14::from_f32(0.0), F2Dot14::from_f32(0.0)),
        AxisValueMap::new(F2Dot14::from_f32(1.0), F2Dot14::from_f32(1.0)),
    ])
}

/// The `avar` table of `axes`; `None` when every map is identity.
pub fn build_avar(axes: &VarAxes) -> Result<Option<Avar>, CompileError> {
    if axes.axes.len() > usize::from(u16::MAX) {
        return Err(avar_error(format!("{} axes are more than the table holds", axes.axes.len())));
    }
    let mut maps = Vec::with_capacity(axes.axes.len());
    let mut any = false;
    for axis in &axes.axes {
        if axis.avar.is_empty() {
            maps.push(identity_map());
            continue;
        }
        if axis.avar.len() > MAX_SEGMENT_POINTS {
            return Err(avar_error(format!("axis `{}` has {} mapping points; at most {MAX_SEGMENT_POINTS} are written", axis.tag, axis.avar.len())));
        }
        let mut values = Vec::with_capacity(axis.avar.len());
        let mut previous: Option<(F2Dot14, F2Dot14)> = None;
        for (from, to) in &axis.avar {
            if !(from.is_finite() && to.is_finite() && (-1.0..=1.0).contains(from) && (-1.0..=1.0).contains(to)) {
                return Err(avar_error(format!("axis `{}` maps {from} to {to}; both must lie between -1 and 1", axis.tag)));
            }
            let (f, t) = (f2dot14(*from), f2dot14(*to));
            if previous.is_some_and(|(pf, pt)| f <= pf || t < pt) {
                return Err(avar_error(format!("the mapping of axis `{}` does not increase", axis.tag)));
            }
            previous = Some((f, t));
            values.push(AxisValueMap::new(f, t));
        }
        let required = [(-1.0, -1.0), (0.0, 0.0), (1.0, 1.0)];
        if !required.iter().all(|(f, t)| values.iter().any(|m| m.from_coordinate == f2dot14(*f) && m.to_coordinate == f2dot14(*t))) {
            return Err(avar_error(format!("the mapping of axis `{}` must map -1, 0 and 1 to themselves", axis.tag)));
        }
        any |= values.iter().any(|m| m.from_coordinate != m.to_coordinate);
        maps.push(SegmentMaps::new(values));
    }
    Ok(any.then(|| Avar::new(maps)))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use fontcraft_model::plist::{Dict, Number, Str, Value};
    use fontcraft_model::{Axis, Font, Master};
    use write_fonts::dump_table;
    use write_fonts::types::Tag;

    use super::super::VarAxis;
    use super::*;

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    /// A weight axis with masters at `masters` (internal), the first or the one `origin` names as
    /// the origin, and the Axis Mappings `map` (user to internal).
    fn designspace(masters: &[f64], origin: Option<usize>, map: &[(i64, i64)]) -> Designspace {
        let mut f = Font::new("Avar");
        f.axes = Arc::new(vec![Axis { name: "Weight".into(), tag: "wght".into(), ..Axis::default() }]);
        f.masters = Arc::new(
            masters
                .iter()
                .enumerate()
                .map(|(k, v)| Master { id: format!("m{k}"), name: format!("M{k}"), location: vec![*v], ..Master::default() })
                .collect(),
        );
        let info = Arc::make_mut(&mut f.info);
        if !map.is_empty() {
            let mut inner = Dict::new();
            for (user, design) in map {
                inner.insert(user.to_string(), Value::Number(Number::int(*design)));
            }
            let mut outer = Dict::new();
            outer.insert("wght", Value::Dict(inner));
            info.params.set(fontcraft_model::axes::AXIS_MAPPINGS, Value::Dict(outer));
        }
        if let Some(k) = origin {
            info.params.set(fontcraft_model::axes::ORIGIN, Value::String(Str::new(format!("m{k}"))));
        }
        Designspace::new(&f).unwrap()
    }

    fn axes(avar: Vec<(f64, f64)>) -> VarAxes {
        VarAxes { axes: vec![VarAxis { tag: Tag::new(b"wght"), name: "Weight".into(), min: 300.0, default: 400.0, max: 700.0, hidden: false, avar }] }
    }

    // Expected bytes: fontc 1.0.0 on its test source `WghtVar_Avar.glyphs` (weight class 500 at
    // design 600; user and design range 300, 400, 700).
    #[test]
    fn avar_matches_the_oracle() {
        let ds = designspace(&[300.0, 400.0, 700.0], Some(1), &[(300, 300), (400, 400), (500, 600), (700, 700)]);
        let map = segment_map(&ds, 0);
        assert_eq!(map.len(), 4);
        let avar = build_avar(&axes(map)).unwrap().unwrap();
        assert_eq!(hex(&dump_table(&avar).unwrap()), "00010000000000010004c000c0000000000015552aab40004000");
    }

    #[test]
    fn identity_maps_write_no_avar() {
        let map = segment_map(&designspace(&[400.0, 700.0], None, &[(400, 400), (700, 700)]), 0);
        assert!(map.is_empty());
        assert_eq!(build_avar(&axes(map)).unwrap(), None);
        // A map shifted by a constant normalizes to the identity too.
        assert!(segment_map(&designspace(&[20.0, 220.0], None, &[(200, 20), (700, 220)]), 0).is_empty());
    }

    #[test]
    fn one_sided_axis_gets_the_missing_end() {
        // Default at the minimum: the map starts at 0 and (-1, -1) is added.
        let map = segment_map(&designspace(&[400.0, 700.0], None, &[(400, 400), (500, 600), (700, 700)]), 0);
        assert_eq!(map.first(), Some(&(-1.0, -1.0)));
        assert_eq!(map.get(1), Some(&(0.0, 0.0)));
        assert_eq!(map.last(), Some(&(1.0, 1.0)));
        assert!(build_avar(&axes(map)).unwrap().is_some());
    }

    #[test]
    fn a_falling_map_exports_with_the_axis_orientation() {
        // Internal values fall from 100 to 50 as the user value rises from 300 to 700; user 500
        // sits at 60, four fifths of the way along the internal range.
        let ds = designspace(&[100.0, 50.0], None, &[(300, 100), (500, 60), (700, 50)]);
        let var = VarAxes::from_designspace(&ds).unwrap();
        let axis = var.axes.first().unwrap();
        assert_eq!((axis.min, axis.default, axis.max), (300.0, 300.0, 700.0));
        assert_eq!(axis.avar.len(), 4, "{:?}", axis.avar);
        let (from, to) = axis.avar[2];
        assert!((from - 0.5).abs() < 1e-9 && (to - 0.8).abs() < 1e-9, "{:?}", axis.avar);
        let avar = build_avar(&var).unwrap().unwrap();
        let written = dump_table(&avar).unwrap();
        assert_eq!(hex(&written), "00010000000000010004c000c000000000002000333340004000");
    }

    #[test]
    fn avar_refuses_maps_that_cannot_be_written() {
        assert!(build_avar(&axes(vec![(-1.0, -1.0), (0.5, 0.2), (0.0, 0.0), (1.0, 1.0)])).is_err());
        assert!(build_avar(&axes(vec![(-1.0, -1.0), (0.0, 0.0), (0.5, f64::NAN), (1.0, 1.0)])).is_err());
        assert!(build_avar(&axes(vec![(-1.0, -1.0), (0.5, 0.7), (1.0, 1.0)])).is_err());
    }
}
