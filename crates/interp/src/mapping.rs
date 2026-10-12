//! Axis mappings: editing the points of one axis's external-to-internal map, and converting a
//! value between the two spaces (M9.2; the commands are M9.17's).
//!
//! Rows: `axisMappings.identityByDefault`, `axisMappings.controlPoints`,
//! `axisMappings.perAxisOnly`, `axisMappings.fontAndExportLevel`,
//! `axisMappings.externalIncludesAxisLocation`, `axisMappings.addPoint`, `axisMappings.setPoint`,
//! `axisMappings.nudgePoint`, `axisMappings.removePoint`, `axisMappings.convertCoordinate`.
//!
//! Contract:
//! - A mapping is a list of `(external, internal)` points sorted by external value, at most
//!   [`MAX_MAPPING_POINTS`]; both coordinates finite, within [`crate::MAX_COORDINATE`] and
//!   strictly increasing. An edit that would break that is refused and changes nothing.
//! - An edit returns the new value of the font's Axis Mappings parameter (every axis, the edited
//!   one changed), ready for [`crate::params::set_axis_mappings`]; the command writes it in one
//!   undo step. Removing the last point of an axis removes the axis's entry (identity).
//! - A mapping read at export that breaks the rules is ignored with a warning (identity).

use fontcraft_model::Font;

use crate::designspace::{Designspace, Space};
use crate::{InterpError, MAX_COORDINATE, MAX_MAPPING_POINTS};

/// One edit of an axis mapping. Point indices count in external order.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MappingEdit {
    /// Adds a point.
    Add { external: f64, internal: f64 },
    /// Moves the point at `index` to new coordinates.
    Set { index: usize, external: f64, internal: f64 },
    /// Moves the point at `index` by the given amounts (the arrow keys of the mapping editor).
    Nudge { index: usize, external: f64, internal: f64 },
    /// Removes the point at `index`.
    Remove { index: usize },
}

/// The points of axis `tag`'s mapping after `edit`, checked; refuses an unknown axis, a bad
/// index, a point that is not a usable number, points out of order and more than
/// [`MAX_MAPPING_POINTS`] points.
///
/// An axis without an entry in the Axis Mappings parameter starts from the map the font implies
/// (Axis Location, else the instances, else the masters), so the first edit keeps the rest of
/// the line the user sees.
pub fn edited_points(font: &Font, tag: &str, edit: &MappingEdit) -> Result<Vec<(f64, f64)>, InterpError> {
    let axis = font.axes.iter().position(|a| a.tag == tag).ok_or_else(|| InterpError::UnknownAxis(tag.to_string()))?;
    let mut points = match crate::params::axis_mappings(font)?.into_iter().find(|(t, _)| t == tag) {
        Some((_, points)) => points,
        None => fontcraft_model::axes::mapping(font, axis).into_pairs(),
    };
    let no_point = |index: usize| InterpError::BadMapping { axis: tag.to_string(), reason: format!("there is no point number {}", index + 1) };
    let given = |v: f64, which: &str| {
        if v.is_finite() && v.abs() <= MAX_COORDINATE { Ok(v) } else { Err(InterpError::BadNumber { what: format!("the {which} value {v}") }) }
    };
    match *edit {
        MappingEdit::Add { external, internal } => {
            if points.len() >= MAX_MAPPING_POINTS {
                return Err(InterpError::LimitExceeded { what: "points in an axis mapping", limit: MAX_MAPPING_POINTS });
            }
            let point = (given(external, "external")?, given(internal, "internal")?);
            let at = points.iter().position(|p| p.0 > point.0).unwrap_or(points.len());
            points.insert(at, point);
        }
        MappingEdit::Set { index, external, internal } => {
            let point = (given(external, "external")?, given(internal, "internal")?);
            *points.get_mut(index).ok_or_else(|| no_point(index))? = point;
        }
        MappingEdit::Nudge { index, external, internal } => {
            let (dx, dy) = (given(external, "external")?, given(internal, "internal")?);
            let p = points.get_mut(index).ok_or_else(|| no_point(index))?;
            *p = (p.0 + dx, p.1 + dy);
        }
        MappingEdit::Remove { index } => {
            if index >= points.len() {
                return Err(no_point(index));
            }
            points.remove(index);
        }
    }
    check_points(tag, &points)?;
    Ok(points)
}

/// Checks a mapping's points: finite, within range, strictly increasing external values with
/// internal values strictly increasing or strictly decreasing, at most [`MAX_MAPPING_POINTS`]. `axis` names the axis in the refusal.
pub fn check_points(axis: &str, points: &[(f64, f64)]) -> Result<(), InterpError> {
    if points.len() > MAX_MAPPING_POINTS {
        return Err(InterpError::LimitExceeded { what: "points in an axis mapping", limit: MAX_MAPPING_POINTS });
    }
    for (n, (external, internal)) in points.iter().enumerate() {
        for v in [external, internal] {
            if !(v.is_finite() && v.abs() <= MAX_COORDINATE) {
                return Err(InterpError::BadNumber { what: format!("point {} of the mapping of axis `{axis}`", n + 1) });
            }
        }
    }
    // External values rise; internal values may rise or fall (an axis whose internal coordinate
    // shrinks as the user value grows), but in one direction throughout.
    let rises = points.windows(2).all(|w| matches!(w, [a, b] if b.0 > a.0 && b.1 > a.1));
    let falls = points.windows(2).all(|w| matches!(w, [a, b] if b.0 > a.0 && b.1 < a.1));
    if !(rises || falls) {
        return Err(InterpError::BadMapping {
            axis: axis.to_string(),
            reason: "each point must lie to the right of the one before it, and the internal values must all rise or all fall".into(),
        });
    }
    Ok(())
}

/// `value` on the axis at `axis` (an index into the designspace's axes) converted from the
/// space `from` to the other one.
pub fn convert(designspace: &Designspace, axis: usize, value: f64, from: Space) -> Result<f64, InterpError> {
    let info = designspace.axes().get(axis).ok_or_else(|| InterpError::UnknownAxis(format!("number {}", axis + 1)))?;
    if !(value.is_finite() && value.abs() <= MAX_COORDINATE) {
        return Err(InterpError::BadNumber { what: format!("the value {value}") });
    }
    Ok(match from {
        Space::Internal => info.map.to_user(value),
        Space::External => info.map.to_design(value),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::designspace::tests::{edit_params, font};
    use crate::params::{axis_mappings, set_axis_mappings};

    /// Applies an edit the way the command does: the new points written back to the font.
    fn apply(f: &mut Font, edit: MappingEdit) -> Result<Vec<(f64, f64)>, InterpError> {
        let points = edited_points(f, "wght", &edit)?;
        edit_params(f, |p| set_axis_mappings(p, "wght", &points).unwrap());
        Ok(points)
    }

    #[test]
    fn mapping_edits_keep_the_points_in_order() {
        let mut f = font(&[("Weight", "wght")], &[("Light", &[300.0]), ("Bold", &[700.0])]);
        // Without an Axis Mappings entry the edit starts from the line the font implies.
        assert_eq!(apply(&mut f, MappingEdit::Add { external: 500.0, internal: 550.0 }).unwrap(), [(300.0, 300.0), (500.0, 550.0), (700.0, 700.0)]);
        assert_eq!(
            apply(&mut f, MappingEdit::Nudge { index: 1, external: 0.0, internal: 10.0 }).unwrap(),
            [(300.0, 300.0), (500.0, 560.0), (700.0, 700.0)]
        );
        assert_eq!(
            apply(&mut f, MappingEdit::Set { index: 1, external: 450.0, internal: 400.0 }).unwrap(),
            [(300.0, 300.0), (450.0, 400.0), (700.0, 700.0)]
        );

        // Refusals leave the parameter as it was.
        let before = f.info.params.clone();
        let past = edited_points(&f, "wght", &MappingEdit::Set { index: 1, external: 800.0, internal: 650.0 });
        assert!(matches!(past, Err(InterpError::BadMapping { .. })), "a point moved past its neighbour");
        let flat = edited_points(&f, "wght", &MappingEdit::Add { external: 600.0, internal: 400.0 });
        assert!(matches!(flat, Err(InterpError::BadMapping { .. })), "internal values must rise or fall throughout");
        assert!(matches!(edited_points(&f, "wght", &MappingEdit::Remove { index: 3 }), Err(InterpError::BadMapping { .. })));
        assert!(matches!(edited_points(&f, "wght", &MappingEdit::Add { external: f64::NAN, internal: 1.0 }), Err(InterpError::BadNumber { .. })));
        assert_eq!(edited_points(&f, "wdth", &MappingEdit::Remove { index: 0 }), Err(InterpError::UnknownAxis("wdth".into())));
        assert_eq!(f.info.params, before);

        // Removing every point removes the axis's entry and then the parameter.
        for _ in 0..3 {
            apply(&mut f, MappingEdit::Remove { index: 0 }).unwrap();
        }
        assert_eq!(axis_mappings(&f).unwrap(), []);
        assert!(f.info.params.get(crate::params::AXIS_MAPPINGS).is_none());
    }

    #[test]
    fn a_falling_map_can_be_edited() {
        let mut f = font(&[("Weight", "wght")], &[("Light", &[100.0]), ("Bold", &[50.0])]);
        edit_params(&mut f, |p| set_axis_mappings(p, "wght", &[(300.0, 100.0), (700.0, 50.0)]).unwrap());
        assert_eq!(apply(&mut f, MappingEdit::Add { external: 500.0, internal: 60.0 }).unwrap(), [(300.0, 100.0), (500.0, 60.0), (700.0, 50.0)]);
        let turns = edited_points(&f, "wght", &MappingEdit::Set { index: 1, external: 500.0, internal: 120.0 });
        assert!(matches!(turns, Err(InterpError::BadMapping { .. })), "a map that falls, then rises");
    }

    #[test]
    fn point_checks_cap_the_count_and_refuse_bad_numbers() {
        let many: Vec<(f64, f64)> = (0..=MAX_MAPPING_POINTS).map(|i| f64::from(u32::try_from(i).unwrap())).map(|v| (v, v)).collect();
        assert!(matches!(check_points("wght", &many), Err(InterpError::LimitExceeded { .. })));
        assert!(check_points("wght", many.get(..MAX_MAPPING_POINTS).unwrap()).is_ok());
        assert!(matches!(check_points("wght", &[(0.0, f64::INFINITY)]), Err(InterpError::BadNumber { .. })));
        assert!(matches!(check_points("wght", &[(0.0, 2.0 * MAX_COORDINATE)]), Err(InterpError::BadNumber { .. })));
        assert!(check_points("wght", &[]).is_ok());
    }
}
