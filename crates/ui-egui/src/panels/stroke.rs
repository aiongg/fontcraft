//! Palette › Stroke (`palette.stroke`): the stroke attributes of the selected paths, at the foot of
//! the Palette while Show Info is on and a path is selected. Every control runs `path.setStroke`
//! on those paths with only the key it changes.
//!
//! | Control | Key | Shown |
//! |---|---|---|
//! | width; emptied, it removes the stroke | `width` | always |
//! | height; empty means the width | `height` | with a stroke |
//! | position in percent, and the presets −100 (left), 0 (centre), 100 (right) | `position` | with a stroke |
//! | Fill | `fill` | with a stroke on a closed path |
//! | join: miter, bevel, round, round full | `join` | with a stroke |
//! | start and end caps: butt, square, round, round inset, axis-aligned | `capStart`, `capEnd` | with a stroke on an open path |
//!
//! A selected path is one selected whole or with a selected node or handle. With several, a field
//! shows a value only when every path has it. Clicking the axis-aligned cap uses the automatic
//! axis; holding it opens a menu that forces the X or the Y axis (`capAxis`, one per path, so it
//! serves both ends).

use std::collections::BTreeSet;
use std::time::Duration;

use fontcraft_engine::cmd::stroke::{format_version, stroke_style};
use fontcraft_model::{Dimension, Path, Shape};
use serde_json::{Map, Value, json};

use super::{Pending, edit_targets, widgets, with};
use crate::app::FontcraftApp;
use crate::tools_glue;

/// How long the axis-aligned cap button is held before its axis menu opens.
const HOLD_SECONDS: f64 = 0.5;

const JOINS: [(&str, &str, &str); 4] =
    [("miter", "Miter", "Miter join"), ("bevel", "Bevel", "Bevel join"), ("round", "Round", "Round join"), ("roundFull", "Full", "Round full join")];
const CAPS: [(&str, &str, &str); 5] = [
    ("butt", "Butt", "Butt cap"),
    ("square", "Square", "Square cap"),
    ("round", "Round", "Round cap"),
    ("roundInset", "Inset", "Round inset cap"),
    ("axisAligned", "Axis", "Cap aligned to an axis; hold to force the X or the Y axis"),
];
const AXES: [(&str, &str); 3] = [("auto", "Automatic"), ("x", "X axis"), ("y", "Y axis")];

/// The paths of the edited layer the selection touches, in shape order: paths selected whole and
/// paths with a selected node or handle.
pub fn selected_paths(app: &FontcraftApp) -> Vec<usize> {
    let (Ok(font), Some(tab)) = (app.session.font(), app.state.tab()) else { return Vec::new() };
    let Some(scene) = tools_glue::scene(font, tab) else { return Vec::new() };
    let s = &tab.selection;
    let all: BTreeSet<usize> = s.shapes.iter().copied().chain(s.points.iter().map(|&(shape, _)| shape)).collect();
    all.into_iter().filter(|&i| matches!(scene.layer.shapes.get(i), Some(Shape::Path(_)))).collect()
}

/// Whether the section shows: a path is selected.
pub fn shown(app: &FontcraftApp) -> bool {
    !selected_paths(app).is_empty()
}

/// One path's stroke as the controls show it.
#[derive(Clone, PartialEq)]
struct One {
    width: Option<f64>,
    /// As stored: `None` follows the width.
    height: Option<f64>,
    /// Percent, −100 to 100.
    position: f64,
    join: &'static str,
    cap_start: &'static str,
    cap_end: &'static str,
    cap_axis: &'static str,
    fill: bool,
}

fn read_one(path: &Path, version: u32) -> Option<One> {
    let attrs = &path.info.attrs;
    // A size stored as an expression cannot be shown or edited here.
    let style = stroke_style(attrs, version).ok()??;
    let height = match &attrs.stroke_height {
        Some(Dimension::Number(n)) => Some(*n),
        _ => None,
    };
    Some(One {
        width: Some(style.width),
        height,
        position: style.position * 100.0,
        join: style.join.name(),
        cap_start: style.cap_start.name(),
        cap_end: style.cap_end.name(),
        cap_axis: style.cap_axis.name(),
        fill: style.fill,
    })
}

/// What the selected paths have in common.
struct Values {
    /// Strokes of the stroked paths; `None` for a path without one.
    strokes: Vec<Option<One>>,
    any_open: bool,
    any_closed: bool,
}

impl Values {
    /// `get` of every path when all agree and all are stroked.
    fn common<T: PartialEq>(&self, get: impl Fn(&One) -> T) -> Option<T> {
        let mut values = self.strokes.iter().map(|s| s.as_ref().map(&get));
        let first = values.next()??;
        values.all(|v| v.as_ref() == Some(&first)).then_some(first)
    }

    fn stroked(&self) -> bool {
        self.strokes.iter().any(Option::is_some)
    }
}

fn read(app: &FontcraftApp, paths: &[usize]) -> Option<Values> {
    let font = app.session.font().ok()?;
    let tab = app.state.tab()?;
    let scene = tools_glue::scene(font, tab)?;
    let version = format_version(font);
    let mut values = Values { strokes: Vec::with_capacity(paths.len()), any_open: false, any_closed: false };
    for &i in paths {
        let Some(Shape::Path(path)) = scene.layer.shapes.get(i) else { continue };
        values.any_open |= !path.closed;
        values.any_closed |= path.closed;
        values.strokes.push(read_one(path, version));
    }
    Some(values)
}

pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    let paths = selected_paths(app);
    let Some(mut targets) = edit_targets(app, "path.setStroke") else { return };
    let Some(values) = read(app, &paths) else { return };
    targets.insert("shapes".into(), json!(paths));
    let tokens = app.tokens.clone();
    let width = tokens.field_width;
    let set = |pending: &mut Pending, extra: Value| pending.push(("path.setStroke", with(targets.clone(), extra)));
    let stroked = values.stroked();

    ui.horizontal(|ui| {
        ui.label("Width");
        match widgets::number(ui, widgets::id("stroke.width"), values.common(|s| s.width).flatten(), width, true) {
            Some(widgets::NumberEdit::Set(v)) => set(pending, json!({ "width": v })),
            Some(widgets::NumberEdit::Clear) => set(pending, json!({ "width": null })),
            None => {}
        }
        if stroked {
            ui.label("Height");
            match widgets::number(ui, widgets::id("stroke.height"), values.common(|s| s.height).flatten(), width, true) {
                Some(widgets::NumberEdit::Set(v)) => set(pending, json!({ "height": v })),
                Some(widgets::NumberEdit::Clear) => set(pending, json!({ "height": null })),
                None => {}
            }
        }
    });
    if !stroked {
        return;
    }

    let position = values.common(|s| s.position);
    ui.horizontal(|ui| {
        ui.label("Position");
        if let Some(widgets::NumberEdit::Set(v)) = widgets::number(ui, widgets::id("stroke.position"), position, width, false) {
            set(pending, json!({ "position": v.clamp(-100.0, 100.0) }));
        }
        for (value, label, hover) in [(-100.0, "−100", "Left of the path"), (0.0, "0", "Centred on the path"), (100.0, "100", "Right of the path")]
        {
            if widgets::button(ui, widgets::id(("stroke.positionPreset", label)), label, hover, true, position == Some(value)) {
                set(pending, json!({ "position": value }));
            }
        }
    });

    if values.any_closed {
        let mut fill = values.common(|s| s.fill).unwrap_or(false);
        if ui.checkbox(&mut fill, "Fill").changed() {
            set(pending, json!({ "fill": fill }));
        }
    }

    let join = values.common(|s| s.join);
    ui.horizontal_wrapped(|ui| {
        for (name, label, hover) in JOINS {
            if widgets::button(ui, widgets::id(("stroke.join", name)), label, hover, true, join == Some(name)) {
                set(pending, json!({ "join": name }));
            }
        }
    });

    if values.any_open {
        let axis = values.common(|s| s.cap_axis);
        for (key, title, current) in [("capStart", "Start", values.common(|s| s.cap_start)), ("capEnd", "End", values.common(|s| s.cap_end))] {
            ui.label(title);
            ui.horizontal_wrapped(|ui| {
                for (name, label, hover) in CAPS {
                    let id = widgets::id(("stroke.cap", key, name));
                    let clicked = widgets::button(ui, id, label, hover, true, current == Some(name));
                    let picked = if name == "axisAligned" { axis_hold(ui, id, axis) } else { HoldResult::None };
                    let chosen = match picked {
                        HoldResult::Axis(a) => a,
                        HoldResult::None if clicked => "auto",
                        HoldResult::None | HoldResult::Open => continue,
                    };
                    let mut extra = Map::new();
                    extra.insert(key.into(), json!(name));
                    if name == "axisAligned" {
                        extra.insert("capAxis".into(), json!(chosen));
                    }
                    set(pending, Value::Object(extra));
                }
            });
        }
    }
}

/// What the axis-aligned cap button's hold did this frame.
enum HoldResult {
    /// No menu: a click counts as a click.
    None,
    /// The menu is open (or opened during this press): a click is not a click.
    Open,
    /// An axis was chosen in the menu.
    Axis(&'static str),
}

/// Opens the axis menu under button `id` once it has been held for [`HOLD_SECONDS`], and draws it
/// while open. `current` is the axis to show as chosen.
fn axis_hold(ui: &mut egui::Ui, id: egui::Id, current: Option<&str>) -> HoldResult {
    let ctx = ui.ctx().clone();
    let menu = id.with("axisMenu");
    let Some(response) = ctx.read_response(id) else { return HoldResult::None };
    let mut open = ctx.data(|d| d.get_temp::<bool>(menu)).unwrap_or(false);
    if response.is_pointer_button_down_on() && !open {
        let held = ui.input(|i| i.pointer.press_start_time().is_some_and(|t| i.time - t >= HOLD_SECONDS));
        if held {
            open = true;
            ctx.data_mut(|d| d.insert_temp(menu, true));
        } else {
            ctx.request_repaint_after(Duration::from_secs_f64(HOLD_SECONDS));
        }
    }
    if !open {
        return HoldResult::None;
    }
    let mut result = HoldResult::Open;
    let area = egui::Area::new(menu).order(egui::Order::Foreground).fixed_pos(response.rect.left_bottom()).show(&ctx, |ui| {
        egui::Frame::popup(ui.style()).show(ui, |ui| {
            for (axis, label) in AXES {
                if ui.add(egui::Button::selectable(current == Some(axis), label)).clicked() {
                    result = HoldResult::Axis(axis);
                }
            }
        });
    });
    // The release that ends the hold lands on the button: that is not a click elsewhere.
    let dismissed = area.response.clicked_elsewhere() && !response.contains_pointer();
    if matches!(result, HoldResult::Axis(_)) || dismissed {
        ctx.data_mut(|d| d.remove::<bool>(menu));
    }
    result
}
