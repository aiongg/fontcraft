//! Snapping suggestions while dragging (View › Suggest, `docs/spec/03b` › Snapping): what the tools' suggestions read
//! from the document, and the parallel outlines Suggest Distance offers near the pointer while a drag runs.
//!
//! | Item | Drawn as | Layer |
//! |---|---|---|
//! | the outlines parallel to the segments that stay put, within [`OUTLINE_REACH`] of the pointer | lines, `snap_line` | `suggestOutlines` |
//!
//! The snapping itself is `fontcraft_tools::suggestions`; [`suggest`] fills the tools' scene with the switches, the
//! declared stems of the edited glyph's master that apply to it and the master's `suggestDistance`. The line a snap
//! follows is one of the tool's overlays. [`outlines`] runs in `overlays::refresh`, so painting reads them alone.

use egui::{Pos2, Shape, Stroke};
use fontcraft_engine::Session;
use fontcraft_engine::model::glyphdata::GlyphData;
use fontcraft_engine::model::plist::Value;
use fontcraft_engine::model::{Font, Glyph, Master, StemDef};
use fontcraft_geom::Point;
use fontcraft_tools::Moving;
use fontcraft_tools::suggestions::{DeclaredStem, Suggest, Switches, distance_outlines};

use crate::app::{FontCraftApp, ViewState};
use crate::canvas::layers::PaintCx;
use crate::canvas::tools::{self, tokens::ToolTokens};

/// The master parameter that holds Suggest Distance's distances.
pub const DISTANCE_PARAMETER: &str = "suggestDistance";
/// How far from the pointer, in screen points, Suggest Distance draws its outlines.
pub const OUTLINE_REACH: f64 = 120.0;
/// Most parallel outlines drawn at once; a glyph with more segments near the pointer shows the first ones.
const MAX_OUTLINES: usize = 512;

/// What the suggestions that are `on` read for the edited glyph: the declared stems of the active layer's master that
/// apply to it, and the master's `suggestDistance`.
pub fn suggest(session: &Session, font: &Font, view: &ViewState, on: Switches) -> Suggest {
    let off = || Suggest { on, ..Suggest::default() };
    if !(on.stems || on.distance) {
        return off();
    }
    let Some(master) = tools::active_master(font, view) else { return off() };
    let glyph = tools::edited_glyph(font, view).and_then(|name| font.glyph(name)).map(|g| &**g);
    let stems = if on.stems { stems(session, font, glyph, master) } else { Vec::new() };
    let distance = if on.distance { master.params.get(DISTANCE_PARAMETER).and_then(distance) } else { None };
    Suggest::new(on, stems, distance)
}

fn stems(session: &Session, font: &Font, glyph: Option<&Glyph>, master: &Master) -> Vec<DeclaredStem> {
    let filtered = font.stems.iter().any(|d| d.filter.as_deref().is_some_and(|f| !f.trim().is_empty()));
    let data = if filtered { session.glyph_data().ok() } else { None };
    font.stems
        .iter()
        .zip(master.stem_values.iter())
        .filter(|(def, _)| applies(def, glyph, font, data.as_deref(), &master.id))
        .map(|(def, &width)| DeclaredStem { width, horizontal: def.horizontal })
        .collect()
}

/// Whether a stem is suggested in `glyph`: always without a filter, and also when its filter does not read as a glyph
/// predicate.
fn applies(def: &StemDef, glyph: Option<&Glyph>, font: &Font, data: Option<&GlyphData>, master: &str) -> bool {
    let Some(text) = def.filter.as_deref().map(str::trim).filter(|t| !t.is_empty()) else { return true };
    let (Some(glyph), Some(data)) = (glyph, data) else { return true };
    let Ok(scope) = fontcraft_glyphquery::parse_predicate(text) else { return true };
    let Ok(env) = fontcraft_glyphquery::Env::new(font, data, Some(master)) else { return true };
    fontcraft_glyphquery::guide_in_scope(Some(&scope), glyph, &env)
}

/// `suggestDistance` as `(x, y)`: a dictionary with `x` and `y` (a missing one is 0), an array of two numbers, or one
/// number for both.
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

/// The parallel outlines Suggest Distance offers near the pointer, in font units: none unless the switch is on, a drag
/// runs and the pointer is over the canvas.
pub fn outlines(app: &FontCraftApp) -> Vec<Vec<Point>> {
    let on = app.overlays.switches.suggest;
    if !on.distance || app.session.interaction().is_none() {
        return Vec::new();
    }
    let Some((near, _)) = app.overlays.hover else { return Vec::new() };
    let Ok(font) = app.session.font() else { return Vec::new() };
    let Some(mut scene) = tools::scene(font, &app.view, app.tools.focus.as_ref()) else { return Vec::new() };
    scene.suggest = suggest(&app.session, font, &app.view, on);
    let moving = Moving::from_selection(&scene, &app.tools.selection);
    let scale = tools::apply::camera(app).scale();
    if !(scale.is_finite() && scale > 0.0) {
        return Vec::new();
    }
    let mut lines = distance_outlines(&scene, &moving, near, OUTLINE_REACH / scale);
    lines.truncate(MAX_OUTLINES);
    lines
}

/// Suggest Distance's outlines near the pointer while a drag runs.
pub fn paint(cx: &PaintCx<'_>) {
    cx.painter.extend(outline_shapes(cx));
}

pub fn outline_shapes(cx: &PaintCx<'_>) -> Vec<Shape> {
    let t = ToolTokens::standard();
    let stroke = Stroke::new(t.snap_line_width, t.snap_line);
    let mut out = Vec::new();
    for line in &cx.overlays.data.suggest_outlines {
        let points: Vec<Pos2> = line.iter().map(|p| cx.camera.to_screen(*p)).collect();
        if points.len() >= 2 && points.iter().all(|p| p.x.is_finite() && p.y.is_finite()) {
            out.push(Shape::line(points, stroke));
        }
    }
    out
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]
mod tests {
    use std::sync::Arc;

    use craft::behavior::CanvasView;
    use craft::behavior::kurbo::Point as KPoint;
    use craft::host::Host;
    use craft::ui::runtime::commands::CommandRef;
    use egui::{Pos2, Rect, vec2};
    use fontcraft_engine::Session;
    use fontcraft_engine::model::plist::{Array, Dict, Number, Value as Plist};
    use fontcraft_engine::model::{Font, Shape as ModelShape, StemDef};
    use fontcraft_geom::Point;
    use fontcraft_tools::suggestions::Switches;
    use fontcraft_tools::{Modifiers, PointerEvent, PointerKind};
    use serde_json::json;

    use super::{DISTANCE_PARAMETER, distance, suggest};
    use crate::app::{APP, FontCraftApp, Msg, ViewMsg};
    use crate::canvas::tools::ToolInput;

    /// Glyph A, a 100-unit square at the origin, in a font whose master declares a vertical stem of 80 and a
    /// `suggestDistance` of 40 across and 25 up; shown at one point per unit.
    fn app() -> FontCraftApp {
        let mut font = Font::new("Suggest");
        Arc::make_mut(&mut font.stems).push(StemDef { name: "lc".into(), ..Default::default() });
        let count = font.stems.len();
        let master = Arc::make_mut(&mut font.masters).first_mut().unwrap();
        master.stem_values.resize(count, 0.0);
        *master.stem_values.last_mut().unwrap() = 80.0;
        let mut d = Dict::new();
        d.insert("x", Plist::Number(Number::int(40)));
        d.insert("y", Plist::Number(Number::int(25)));
        master.params.set(DISTANCE_PARAMETER, Plist::Dict(d));
        let mut session = Session::new();
        session.add_document(font, None).unwrap();
        let mut app = FontCraftApp::with_edit_tab(session, Arc::new(Host::builder(APP).build()));
        app.sent.push(Msg::View(ViewMsg::SelectGlyph("A".into())));
        app.drain();
        app.run_now(Msg::run("path.addRectangle", json!({"glyph": "A", "x": 0, "y": 0, "width": 100, "height": 100})), false).unwrap();
        app.view.canvas = CanvasView::new(Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 800.0)), KPoint::new(400.0, 400.0), 1.0);
        app.drain();
        app
    }

    fn switch(app: &mut FontCraftApp, id: &str, on: bool) {
        app.sent.push(Msg::Shell(CommandRef::with_params(id, json!({"on": on}))));
        app.drain();
        assert!(app.status.is_empty(), "{id}: {}", app.status);
    }

    fn pointer(app: &mut FontCraftApp, kind: PointerKind, x: f64, y: f64) {
        app.sent.push(Msg::Tool(ToolInput::Pointer(PointerEvent { kind, pos: Point::new(x, y), mods: Modifiers::NONE })));
        app.drain();
    }

    fn drag(app: &mut FontCraftApp, from: (f64, f64), to: (f64, f64)) {
        pointer(app, PointerKind::Down, from.0, from.1);
        pointer(app, PointerKind::Drag, (from.0 + to.0) / 2.0, (from.1 + to.1) / 2.0);
        pointer(app, PointerKind::Drag, to.0, to.1);
        pointer(app, PointerKind::Up, to.0, to.1);
    }

    /// The x of every node of glyph A's first path, sorted, without repeats.
    fn xs(app: &FontCraftApp) -> Vec<f64> {
        let font = app.session.font().unwrap();
        let layer = font.glyph("A").unwrap().layer(&font.masters[0].id).unwrap();
        let Some(ModelShape::Path(path)) = layer.shapes.first() else { panic!("no path") };
        let mut xs: Vec<f64> = path.nodes.iter().map(|n| n.x).collect();
        xs.sort_by(f64::total_cmp);
        xs.dedup();
        xs
    }

    /// The master's stems and distances reach the tools; with Suggest Stems on a dragged edge snaps to the declared
    /// stem, with it off the edge goes where it is dragged.
    #[test]
    fn suggest_stems_makes_a_dragged_edge_snap_to_a_declared_stem() {
        let mut app = app();
        assert_eq!(app.overlays.switches.suggest, Switches::default(), "every suggestion starts off");
        for id in ["view.suggestStems", "view.suggestDistance"] {
            switch(&mut app, id, true);
        }
        {
            let font = app.session.font().unwrap();
            let s = suggest(&app.session, font, &app.view, app.overlays.switches.suggest);
            assert_eq!(s.stems.len(), 1);
            assert_eq!(s.stems[0].width, 80.0);
            assert_eq!(s.distance, Some((40.0, 25.0)));
        }
        switch(&mut app, "view.suggestDistance", false);
        // The right edge dragged from x 100 to 82 lands at 80: the stem from the left edge.
        drag(&mut app, (100.0, 50.0), (82.0, 50.0));
        assert_eq!(xs(&app), vec![0.0, 80.0]);
        switch(&mut app, "view.suggestStems", false);
        drag(&mut app, (80.0, 50.0), (62.0, 50.0));
        assert_eq!(xs(&app), vec![0.0, 62.0]);
    }

    /// While a drag runs with Suggest Distance on, the outlines parallel to the edges that stay put are drawn near the
    /// pointer; they go when the drag ends, and never show with the switch off.
    #[test]
    fn suggest_distance_draws_parallel_outlines_while_a_drag_runs() {
        let mut app = app();
        pointer(&mut app, PointerKind::Down, 100.0, 50.0);
        pointer(&mut app, PointerKind::Drag, 95.0, 50.0);
        assert!(app.overlays.data.suggest_outlines.is_empty(), "the switch is off");
        pointer(&mut app, PointerKind::Up, 95.0, 50.0);

        switch(&mut app, "view.suggestDistance", true);
        pointer(&mut app, PointerKind::Down, 95.0, 50.0);
        pointer(&mut app, PointerKind::Drag, 90.0, 50.0);
        let lines = app.overlays.data.suggest_outlines.clone();
        assert!(lines.len() >= 2, "both sides of the left edge: {lines:?}");
        // The left edge stays put at x 0: one parallel runs 40 units to its right.
        assert!(lines.iter().any(|l| l.iter().all(|p| (p.x - 40.0).abs() < 1e-6)), "{lines:?}");
        pointer(&mut app, PointerKind::Up, 90.0, 50.0);
        assert!(app.overlays.data.suggest_outlines.is_empty(), "the drag ended");
    }

    #[test]
    fn suggest_distance_reads_a_dictionary_an_array_or_one_number() {
        let n = |v: i64| Plist::Number(Number::int(v));
        let mut d = Dict::new();
        d.insert("x", n(30));
        assert_eq!(distance(&Plist::Dict(d)), Some((30.0, 0.0)));
        assert_eq!(distance(&Plist::Array(Array { items: vec![n(10), n(20)], ..Default::default() })), Some((10.0, 20.0)));
        assert_eq!(distance(&n(12)), Some((12.0, 12.0)));
        assert_eq!(distance(&Plist::Dict(Dict::new())), None);
    }
}
