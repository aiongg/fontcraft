//! The Designspace section of the Masters tab: the axes, the masters, virtual masters and instances on them, and what
//! is wrong with the set-up (`fontcraft_interp::problems`).

use craft::ui::contract::PlainData;
use fontcraft_engine::fontinfo::target::instance_name;
use fontcraft_engine::interp::problems::{self, problems};
use fontcraft_engine::interp::{Designspace, Location};
use fontcraft_engine::model::Font;
use fontcraft_engine::model::axes::design_value;

use crate::shell::font_info_masters::MastersTabView;

/// Most axes shown.
pub const MAX_AXES_SHOWN: usize = 16;
/// Most points on the map and rows in the table.
pub const MAX_POINTS: usize = 500;
/// Most problems listed.
pub const MAX_PROBLEMS_SHOWN: usize = 20;

#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct DesignspaceModel {
    pub axes: Vec<MapAxis>,
    /// The axes the map shows across and up; the same axis twice for a font with one. `None` without axes.
    pub shown: Option<(usize, usize)>,
    pub points: Vec<MapPoint>,
    pub problems: Vec<MapProblem>,
    /// How many problems are not listed.
    pub more_problems: usize,
}

#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct MapAxis {
    pub name: String,
    pub tag: String,
    /// The least and greatest internal coordinate of the points.
    pub min: f64,
    pub max: f64,
}

/// What a point of the map is, with its index in the font's masters or instances.
#[derive(PlainData, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PointKind {
    Master(usize),
    Virtual(usize),
    Instance(usize),
}

#[derive(PlainData, Clone, Debug, PartialEq)]
pub struct MapPoint {
    pub kind: PointKind,
    pub name: String,
    /// The internal coordinate on each axis.
    pub location: Vec<f64>,
    /// The external ones; empty when the designspace cannot be built.
    pub external: Vec<f64>,
    /// The master variable fonts start from.
    pub origin: bool,
    pub selected: bool,
    /// Its location has a problem.
    pub problem: bool,
}

#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct MapProblem {
    pub code: String,
    pub message: String,
}

pub fn project(font: &Font, view: &MastersTabView, selected: &[usize], selected_instances: &[usize]) -> DesignspaceModel {
    let count = font.axes.len().min(MAX_AXES_SHOWN);
    if count == 0 {
        return DesignspaceModel::default();
    }
    let found = problems(font);
    // The problems of a place, which the map marks; the others name a master for its part in a set-up.
    let places = [problems::SHARED_LOCATION, problems::INSTANCE_EXTRAPOLATED];
    let placed = || found.iter().filter(|p| places.contains(&p.code));
    let ds = Designspace::new(font).ok();
    let place = |location: &[f64]| -> Vec<f64> { (0..font.axes.len()).map(|i| design_value(location, i)).collect() };
    let external = |location: &[f64]| -> Vec<f64> {
        ds.as_ref().and_then(|d| d.to_external(&Location(location.to_vec())).ok()).map_or_else(Vec::new, |u| u.0.into_iter().take(count).collect())
    };
    let point = |kind: PointKind, name: &str, location: Vec<f64>| MapPoint {
        kind,
        name: name.to_string(),
        external: external(&location),
        location: location.into_iter().take(count).collect(),
        origin: false,
        selected: false,
        problem: false,
    };
    let origin = ds.as_ref().and_then(|d| d.origin()).map(|m| m.index);
    let mut points: Vec<MapPoint> = Vec::new();
    for (i, m) in font.masters.iter().enumerate().take(MAX_POINTS) {
        points.push(MapPoint {
            origin: origin == Some(i),
            selected: selected.contains(&i),
            problem: placed().any(|p| p.masters.contains(&i)),
            ..point(PointKind::Master(i), &m.name, place(&m.location))
        });
    }
    for (i, v) in ds.iter().flat_map(|d| d.virtual_masters()).enumerate() {
        points.push(point(PointKind::Virtual(i), &v.name, v.location.0.clone()));
    }
    for (i, inst) in font.instances.iter().enumerate().filter(|(_, inst)| inst.kind.as_deref() != Some("variable")) {
        points.push(MapPoint {
            selected: selected_instances.contains(&i),
            problem: placed().any(|p| p.instances.contains(&i)),
            ..point(PointKind::Instance(i), &instance_name(inst), place(&inst.location))
        });
        if points.len() >= MAX_POINTS {
            break;
        }
    }
    points.truncate(MAX_POINTS);
    let axes: Vec<MapAxis> = font
        .axes
        .iter()
        .take(count)
        .enumerate()
        .map(|(i, a)| {
            let values = points.iter().filter_map(|p| p.location.get(i).copied()).filter(|v| v.is_finite());
            let (min, max) = values.fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| (lo.min(v), hi.max(v)));
            let (min, max) = if min <= max { (min, max) } else { (0.0, 0.0) };
            MapAxis { name: if a.name.trim().is_empty() { a.tag.clone() } else { a.name.clone() }, tag: a.tag.clone(), min, max }
        })
        .collect();
    let last = count.saturating_sub(1);
    let (across, up) = view.map_axes.unwrap_or((0, 1));
    DesignspaceModel {
        axes,
        shown: Some((across.min(last), up.min(last))),
        points,
        more_problems: found.len().saturating_sub(MAX_PROBLEMS_SHOWN),
        problems: found.into_iter().take(MAX_PROBLEMS_SHOWN).map(|p| MapProblem { code: p.code.to_string(), message: p.message }).collect(),
    }
}
