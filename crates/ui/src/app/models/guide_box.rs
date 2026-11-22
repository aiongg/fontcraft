//! The guide part of the Info box (M3.41, row `guides.infoBoxFields`): the one selected guide's address and the
//! properties `guide.setProperties` sets, read with the values Guide Properties opens with
//! (`fontcraft_tools::guide_drag::properties`).

use craft::ui::contract::PlainData;
use fontcraft_tools::guide_drag;
use fontcraft_tools::selection::GuideScope;
use serde_json::{Map, Value};

use super::ProjectCx;
use super::palette::Targets;
use crate::canvas::tools::{Tools, scene};

/// The keys of `guide_drag::properties` that are values, not the guide's address.
const VALUE_KEYS: [&str; 8] = ["x", "y", "angle", "angleLocked", "relativeTo", "measurement", "name", "color"];

/// The guide part of the Info box.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct GuideBox {
    /// `{glyph, layer, side?, guide: {scope, index}}`: the address `guide.setProperties` takes.
    pub target: Targets,
    /// A global guide (of the master) rather than a local one (of the layer).
    pub global: bool,
    pub x: f64,
    pub y: f64,
    /// Degrees counter-clockwise from horizontal, from 0 to 360.
    pub angle: f64,
    pub angle_locked: bool,
    /// The sidebearing the guide follows: `left`, `right` or `both`.
    pub relative_to: String,
    pub measurement: bool,
    pub name: String,
    /// The global guide's stored filter, empty for none.
    pub filter: String,
    /// The guide may not change: it is locked, or it is local to a locked glyph.
    pub locked: bool,
}

/// The part for the one guide the canvas selection holds; `None` without exactly one selected guide.
pub fn guide_box(cx: &ProjectCx<'_>, tools: &Tools) -> Option<GuideBox> {
    let mut guides = tools.selection.guides.iter();
    let (&r, None) = (guides.next()?, guides.next()) else { return None };
    let font = cx.session.font().ok()?;
    let scene = scene(font, cx.session.glyph_data().ok().as_deref(), cx.view, tools.focus.as_ref())?;
    let g = guide_drag::guide(&scene, r)?;
    let values = guide_drag::properties(&scene, r, g);
    let mut target: Map<String, Value> = values.clone();
    for key in VALUE_KEYS {
        target.remove(key);
    }
    let number = |key: &str| values.get(key).and_then(Value::as_f64).filter(|v| v.is_finite()).unwrap_or(0.0);
    let flag = |key: &str| values.get(key).and_then(Value::as_bool).unwrap_or(false);
    let text = |key: &str| values.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
    Some(GuideBox {
        target: Targets { json: Value::Object(target).to_string() },
        global: r.scope == GuideScope::Global,
        x: number("x"),
        y: number("y"),
        angle: number("angle"),
        angle_locked: flag("angleLocked"),
        relative_to: text("relativeTo"),
        measurement: flag("measurement"),
        name: text("name"),
        filter: g.filter.clone().unwrap_or_default(),
        locked: !guide_drag::editable(&scene, r, g),
    })
}
