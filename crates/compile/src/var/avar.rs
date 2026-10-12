//! The `avar` table (M9.6).
//!
//! Rows: the `avar` table row, `compile.avarMaps`.
//!
//! Contract: one segment map per axis from [`VarAxis::avar`](super::VarAxis::avar), an identity
//! map for axes without one; no table when every axis is identity, as fontc writes it.
//!
//! The segment maps follow fontc (`fontbe` `avar.rs`, Apache-2.0): each point of an axis's
//! external-to-internal map becomes `(normalized external, normalized internal)`; the points −1
//! and 1 are added when the map does not reach them; an axis whose points all map to themselves
//! writes the required (−1, −1), (0, 0), (1, 1) map. Identity is decided on the written F2Dot14
//! values.

use fontcraft_interp::designspace::AxisInfo;
use fontcraft_model::AxisRange;
use write_fonts::tables::avar::{Avar, AxisValueMap, SegmentMaps};
use write_fonts::types::F2Dot14;

use super::VarAxes;
use crate::CompileError;

/// The most points of one segment map (the map's own limit, plus the three required points).
pub const MAX_SEGMENT_POINTS: usize = fontcraft_interp::error::MAX_MAPPING_POINTS + 3;

fn avar_error(message: String) -> CompileError {
    CompileError::Table { table: "avar", message }
}

/// fontTools' `normalizeValue`: `v` clamped to the range, then −1 at the minimum, 0 at the
/// default and 1 at the maximum, linear in between.
fn normalize(v: f64, range: &AxisRange) -> f64 {
    let v = v.clamp(range.minimum.min(range.maximum), range.maximum.max(range.minimum));
    let span = if v < range.default { range.default - range.minimum } else { range.maximum - range.default };
    if span == 0.0 || !span.is_finite() { 0.0 } else { ((v - range.default) / span).clamp(-1.0, 1.0) }
}

fn f2dot14(v: f64) -> F2Dot14 {
    F2Dot14::from_f32(v as f32)
}

/// The segment map of `axis`: `(normalized external, normalized internal)` pairs, sorted, with
/// (−1, −1) and (1, 1) added where the map stops short of them; empty when the map is the identity
/// at F2Dot14 precision.
pub fn segment_map(axis: &AxisInfo) -> Vec<(f64, f64)> {
    let ext = &axis.external;
    let int = &axis.internal;
    let (lo, hi) = (ext.minimum, ext.maximum);
    let mut users: Vec<f64> = vec![lo, ext.default, hi];
    users.extend(axis.map.pairs.iter().map(|(u, _)| *u).filter(|u| u.is_finite() && *u > lo && *u < hi).take(MAX_SEGMENT_POINTS));
    users.retain(|u| u.is_finite());
    users.sort_by(f64::total_cmp);
    let mut map: Vec<(f64, f64)> = Vec::with_capacity(users.len() + 2);
    for u in users {
        let from = normalize(u, ext);
        let to = normalize(axis.map.to_design(u), int);
        // Points that normalize to the same value (an end equal to the default) keep the first.
        if map.last().is_some_and(|(f, _)| f2dot14(*f) == f2dot14(from)) {
            continue;
        }
        map.push((from, to));
    }
    if map.first().is_none_or(|(f, _)| *f > -1.0) {
        map.insert(0, (-1.0, -1.0));
    }
    if map.last().is_none_or(|(f, _)| *f < 1.0) {
        map.push((1.0, 1.0));
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
    use fontcraft_model::axes::Mapping;
    use write_fonts::dump_table;
    use write_fonts::types::Tag;

    use super::super::VarAxis;
    use super::*;

    fn hex(bytes: &[u8]) -> String {
        bytes.iter().map(|b| format!("{b:02x}")).collect()
    }

    fn info(pairs: &[(f64, f64)], user: (f64, f64, f64), design: (f64, f64, f64)) -> AxisInfo {
        AxisInfo {
            tag: "wght".into(),
            name: "Weight".into(),
            hidden: false,
            internal: AxisRange { minimum: design.0, default: design.1, maximum: design.2 },
            external: AxisRange { minimum: user.0, default: user.1, maximum: user.2 },
            map: Mapping { pairs: pairs.to_vec(), default: user.1 },
            font_axis: 0,
        }
    }

    fn axes(avar: Vec<(f64, f64)>) -> VarAxes {
        VarAxes { axes: vec![VarAxis { tag: Tag::new(b"wght"), name: "Weight".into(), min: 300.0, default: 400.0, max: 700.0, hidden: false, avar }] }
    }

    // Expected bytes: fontc 1.0.0 on its test source `WghtVar_Avar.glyphs` (weight class 500 at
    // design 600; user and design range 300, 400, 700).
    #[test]
    fn avar_matches_the_oracle() {
        let map = segment_map(&info(&[(300.0, 300.0), (400.0, 400.0), (500.0, 600.0), (700.0, 700.0)], (300.0, 400.0, 700.0), (300.0, 400.0, 700.0)));
        assert_eq!(map.len(), 4);
        let avar = build_avar(&axes(map)).unwrap().unwrap();
        assert_eq!(hex(&dump_table(&avar).unwrap()), "00010000000000010004c000c0000000000015552aab40004000");
    }

    #[test]
    fn identity_maps_write_no_avar() {
        let map = segment_map(&info(&[(400.0, 400.0), (700.0, 700.0)], (400.0, 400.0, 700.0), (400.0, 400.0, 700.0)));
        assert!(map.is_empty());
        assert_eq!(build_avar(&axes(map)).unwrap(), None);
        // A map shifted by a constant normalizes to the identity too.
        let map = segment_map(&info(&[(200.0, 20.0), (700.0, 220.0)], (200.0, 200.0, 700.0), (20.0, 20.0, 220.0)));
        assert!(map.is_empty());
    }

    #[test]
    fn one_sided_axis_gets_the_missing_end() {
        // Default at the minimum: the map starts at 0 and fontc adds (-1, -1).
        let map = segment_map(&info(&[(400.0, 400.0), (500.0, 600.0), (700.0, 700.0)], (400.0, 400.0, 700.0), (400.0, 400.0, 700.0)));
        assert_eq!(map.first(), Some(&(-1.0, -1.0)));
        assert_eq!(map.get(1), Some(&(0.0, 0.0)));
        assert_eq!(map.last(), Some(&(1.0, 1.0)));
        assert!(build_avar(&axes(map)).unwrap().is_some());
    }

    #[test]
    fn avar_refuses_maps_that_cannot_be_written() {
        assert!(build_avar(&axes(vec![(-1.0, -1.0), (0.5, 0.2), (0.0, 0.0), (1.0, 1.0)])).is_err());
        assert!(build_avar(&axes(vec![(-1.0, -1.0), (0.0, 0.0), (0.5, f64::NAN), (1.0, 1.0)])).is_err());
        assert!(build_avar(&axes(vec![(-1.0, -1.0), (0.5, 0.7), (1.0, 1.0)])).is_err());
    }
}
