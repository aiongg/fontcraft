//! Snapping suggestions while dragging (View › Suggest): what the tools' suggestions read from the
//! document, and the parallel outlines Suggest Distance offers near the pointer while a drag
//! runs. The snapping is `fontcraft_tools::suggestions`; the line a snap follows is drawn with
//! the tool's other snap overlays.

use egui::{Pos2, Stroke};
use fontcraft_engine::Session;
use fontcraft_geom::Point;
use fontcraft_model::glyphdata::GlyphData;
use fontcraft_model::plist::Value;
use fontcraft_model::{Font, Glyph, Master, StemDef};
use fontcraft_tools::Moving;
use fontcraft_tools::suggestions::{DeclaredStem, Suggest, Switches, distance_outlines};

use super::{Painting, Scene};
use crate::panels::Pending;
use crate::state::Tab;

/// The master parameter that holds Suggest Distance's distances.
pub const DISTANCE_PARAMETER: &str = "suggestDistance";
/// How far from the pointer, in screen points, Suggest Distance draws its outlines.
const OUTLINE_REACH: f64 = 120.0;

/// What the suggestions that are `on` read for the edited glyph of `tab`: the declared stems of
/// its master that apply to it, and the master's `suggestDistance`.
pub fn suggest(session: &Session, font: &Font, tab: &Tab, on: Switches) -> Suggest {
    let off = || Suggest { on, ..Suggest::default() };
    if !(on.stems || on.distance) {
        return off();
    }
    let Some(layer) = super::view::active_layer(font, tab) else { return off() };
    let Some(master) = font.masters.iter().find(|m| m.id == layer.master) else { return off() };
    let stems = if on.stems { stems(session, font, tab, master) } else { Vec::new() };
    let distance = if on.distance { master.params.get(DISTANCE_PARAMETER).and_then(distance) } else { None };
    Suggest::new(on, stems, distance)
}

fn stems(session: &Session, font: &Font, tab: &Tab, master: &Master) -> Vec<DeclaredStem> {
    let glyph = tab.edited_glyph().and_then(|g| font.glyph(g)).map(|g| &**g);
    let filtered = font.stems.iter().any(|d| d.filter.as_deref().is_some_and(|f| !f.trim().is_empty()));
    let data = if filtered { session.glyph_data().ok() } else { None };
    font.stems
        .iter()
        .zip(master.stem_values.iter())
        .filter(|(def, _)| applies(def, glyph, font, data.as_deref(), &master.id))
        .map(|(def, &width)| DeclaredStem { width, horizontal: def.horizontal })
        .collect()
}

/// Whether a stem is suggested in `glyph`: always without a filter, and also when its filter
/// does not read as a glyph predicate.
fn applies(def: &StemDef, glyph: Option<&Glyph>, font: &Font, data: Option<&GlyphData>, master: &str) -> bool {
    let Some(text) = def.filter.as_deref().map(str::trim).filter(|t| !t.is_empty()) else { return true };
    let (Some(glyph), Some(data)) = (glyph, data) else { return true };
    let Ok(scope) = fontcraft_glyphquery::parse_predicate(text) else { return true };
    let Ok(env) = fontcraft_glyphquery::Env::new(font, data, Some(master)) else { return true };
    fontcraft_glyphquery::guide_in_scope(Some(&scope), glyph, &env)
}

/// `suggestDistance` as `(x, y)`: a dictionary with `x` and `y` (a missing one is 0), an array of
/// two numbers, or one number for both.
pub fn distance(value: &Value) -> Option<(f64, f64)> {
    let num = |v: &Value| v.as_num().map(|n| n.as_f64()).or_else(|| v.as_str().and_then(|s| s.trim().parse::<f64>().ok()));
    if let Some(d) = value.as_dict() {
        let (x, y) = (d.get("x").and_then(num), d.get("y").and_then(num));
        return (x.is_some() || y.is_some()).then(|| (x.unwrap_or(0.0), y.unwrap_or(0.0)));
    }
    if let Some(a) = value.as_array() {
        let mut items = a.items.iter();
        return match (items.next().and_then(num), items.next().and_then(num), items.next()) {
            (Some(x), Some(y), None) => Some((x, y)),
            _ => None,
        };
    }
    num(value).map(|v| (v, v))
}

/// Draws Suggest Distance's outlines near the pointer while a drag runs.
pub fn draw(ui: &egui::Ui, c: &Painting, scene: &Scene, pending: &mut Pending) {
    let _ = pending;
    let on = scene.state.panels.edit_aids.suggest;
    if !on.distance || scene.session.interaction().is_none() {
        return;
    }
    let Some(pointer) = ui.ctx().pointer_latest_pos() else { return };
    let Some(mut tools_scene) = crate::tools_glue::scene(scene.font, scene.tab) else { return };
    tools_scene.suggest = suggest(scene.session, scene.font, scene.tab, on);
    let moving = Moving::from_selection(&tools_scene, &scene.tab.selection);
    let near = c.view.to_font(Point::new(f64::from(pointer.x), f64::from(pointer.y)));
    let stroke = Stroke::new(c.tokens.snap_line_width, c.tokens.snap_line);
    for line in distance_outlines(&tools_scene, &moving, near, c.view.units(OUTLINE_REACH)) {
        let points: Vec<Pos2> = line
            .iter()
            .map(|p| {
                let s = c.view.to_screen(*p);
                Pos2::new(s.x as f32, s.y as f32)
            })
            .collect();
        if points.len() >= 2 {
            c.painter.add(egui::Shape::line(points, stroke));
        }
    }
}
