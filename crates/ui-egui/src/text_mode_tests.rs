//! Text mode on the canvas: kerned gaps, the selection, the cursor and the metrics labels in
//! each direction and kerning mode; the pointer placing the cursor and selecting; the bottom
//! bar's direction and kerning mode buttons.

use egui::{Event, Modifiers, PointerButton, Pos2, Shape};
use fontcraft_geom::Point;
use serde_json::{Value, json};

use crate::canvas::{text, view};
use crate::panels::widgets;
use crate::{FontcraftApp, control};

/// A headless window: one egui context kept across frames.
struct Window {
    ctx: egui::Context,
}

impl Window {
    fn new() -> Self {
        Self { ctx: egui::Context::default() }
    }

    /// One frame; the shapes it drew, nested lists flattened.
    fn frame(&self, app: &mut FontcraftApp, events: Vec<Event>) -> Vec<Shape> {
        let input =
            egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1280.0, 800.0))), events, ..Default::default() };
        let mut output = self.ctx.run_ui(input, |ui| app.ui(ui));
        output.textures_delta.clear();
        let mut out: Vec<Shape> = output.shapes.into_iter().map(|c| c.shape).collect();
        let mut flat = Vec::new();
        while let Some(shape) = out.pop() {
            match shape {
                Shape::Vec(list) => out.extend(list),
                other => flat.push(other),
            }
        }
        flat
    }

    fn settle(&self, app: &mut FontcraftApp) -> Vec<Shape> {
        self.frame(app, Vec::new());
        self.frame(app, Vec::new())
    }

    fn press(&self, app: &mut FontcraftApp, at: Pos2, pressed: bool) {
        self.frame(app, vec![Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE }]);
    }

    /// Clicks the middle of the control `id` as it was drawn in the last frame.
    fn click_control(&self, app: &mut FontcraftApp, id: egui::Id) {
        let at = self.ctx.read_response(id).unwrap_or_else(|| panic!("{id:?} was not drawn")).rect.center();
        self.frame(app, vec![Event::PointerMoved(at)]);
        self.press(app, at, true);
        self.press(app, at, false);
    }
}

/// A (outline 50 to 550), V (100 to 500) and H (no outline), each 600 wide, with A V kerned −100
/// and V A +40 in every direction; a tab in text mode holding "AVA", the cursor at 1, at 0.3
/// points per unit.
fn text_app() -> FontcraftApp {
    let mut app = FontcraftApp::default();
    app.run("path.addRectangle", &json!({"glyph": "A", "x": 50, "y": 0, "width": 500, "height": 700})).unwrap();
    app.run("path.addRectangle", &json!({"glyph": "V", "x": 100, "y": 0, "width": 400, "height": 700})).unwrap();
    for direction in ["ltr", "rtl", "vertical"] {
        app.run("kerning.setPair", &json!({"left": "A", "right": "V", "value": -100, "direction": direction})).unwrap();
        app.run("kerning.setPair", &json!({"left": "V", "right": "A", "value": 40, "direction": direction})).unwrap();
    }
    app.run("view.newTab", &json!({"glyphs": ["A"]})).unwrap();
    app.run("editView.toggleTextMode", &json!({"mode": "text"})).unwrap();
    app.run("editView.setText", &json!({"text": "AVA"})).unwrap();
    app.run("editView.setCursor", &json!({"index": 1})).unwrap();
    zoom(&mut app, 300, [870, 300]);
    app
}

/// `view.setZoom` with the canvas centre on the text point `center` (the command takes it in the
/// edited glyph's units).
fn zoom(app: &mut FontcraftApp, size: i32, center: [i32; 2]) {
    let [x, y] = view::edited_origin(&app.session, &app.layouts, app.state.tab().unwrap());
    app.run("view.setZoom", &json!({"size": size, "center": [f64::from(center[0]) - x, f64::from(center[1]) - y]})).unwrap();
}

fn get(app: &mut FontcraftApp) -> Value {
    app.run("editView.getText", &json!({})).unwrap()
}

fn pointer(app: &mut FontcraftApp, events: Value) -> Value {
    let r = control::handle(app, &json!({"id": 1, "method": "ui.pointer", "params": {"events": events}}));
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(r["result"]["errors"], json!([]), "{r}");
    r["result"].clone()
}

/// The text point (x, y) on the screen, as the last frame drew the canvas.
fn screen(app: &FontcraftApp, x: f64, y: f64) -> Pos2 {
    let v = text::text_view(&app.state.tab().unwrap().view, crate::tools_glue::canvas_center(app));
    let p = v.to_screen(Point::new(x, y));
    Pos2::new(p.x as f32, p.y as f32)
}

struct Drawn {
    negative: usize,
    positive: usize,
    selected: usize,
    cursors: Vec<[Pos2; 2]>,
    labels: Vec<String>,
}

/// What two settled frames draw.
fn drawn(app: &mut FontcraftApp, w: &Window) -> Drawn {
    let shapes = w.settle(app);
    let t = &app.tokens;
    let rects = |fill: egui::Color32| shapes.iter().filter(|s| matches!(s, Shape::Rect(r) if r.fill == fill)).count();
    let cursors = shapes
        .iter()
        .filter_map(|s| match s {
            Shape::LineSegment { points, stroke } if stroke.color == t.text_cursor => Some(*points),
            _ => None,
        })
        .collect();
    let mut labels: Vec<String> = shapes
        .iter()
        .filter_map(|s| match s {
            Shape::Text(text) if text.fallback_color == t.sidebearing_label => Some(text.galley.text().to_string()),
            _ => None,
        })
        .collect();
    labels.sort();
    Drawn { negative: rects(t.kerning_negative), positive: rects(t.kerning_positive), selected: rects(t.text_selection), cursors, labels }
}

#[test]
fn text_mode_draws_kerned_gaps_cursor_and_labels_in_every_direction_and_mode() {
    let mut app = text_app();
    let w = Window::new();
    for direction in ["ltr", "rtl", "ttb"] {
        app.run("editView.setWritingDirection", &json!({"direction": direction})).unwrap();
        let (size, center) = match direction {
            "ltr" => (300, [870, 300]),
            "rtl" => (300, [-870, 300]),
            _ => (200, [0, -1470]),
        };
        zoom(&mut app, size, center);
        for mode in ["none", "kerning", "locked"] {
            app.run("kerning.setMode", &json!({"mode": mode})).unwrap();
            let d = drawn(&mut app, &w);
            let gaps = usize::from(mode != "none");
            assert_eq!((d.negative, d.positive), (gaps, gaps), "{direction} {mode}: one gap of each sign while kerning shows");
            assert_eq!(d.selected, 0, "{direction} {mode}");
            assert_eq!(d.cursors.len(), 1, "{direction} {mode}: one cursor");
            assert_eq!(d.labels.len(), 9, "{direction} {mode}: two sidebearings and a width under each glyph: {:?}", d.labels);
            let layout = view::text_layout(&app.session, &app.layouts, app.state.tab().unwrap()).unwrap();
            let (x, y) = layout.caret(1);
            let [a, b] = d.cursors[0];
            let at = screen(&app, x, y);
            if direction == "ttb" {
                assert!((a.y - at.y).abs() < 0.5 && (b.y - at.y).abs() < 0.5 && a.x != b.x, "{direction} {mode}: the cursor runs across the column");
            } else {
                assert!((a.x - at.x).abs() < 0.5 && (b.x - at.x).abs() < 0.5 && a.y != b.y, "{direction} {mode}: the cursor stands at the caret");
            }
            if direction == "ltr" {
                let expected = if mode == "none" { 600.0 } else { 500.0 };
                assert_eq!(x, expected, "{mode}: the cursor follows the kerned V");
            }
        }
    }
    // Horizontal labels: the sidebearings of A and V and the three widths.
    app.run("editView.setWritingDirection", &json!({"direction": "ltr"})).unwrap();
    zoom(&mut app, 300, [870, 300]);
    let d = drawn(&mut app, &w);
    assert_eq!(d.labels, ["100", "100", "50", "50", "50", "50", "600", "600", "600"]);
    // Show Metrics off: no labels. Edit mode: no text overlays at all.
    app.run("view.showMetrics", &json!({})).unwrap();
    assert!(drawn(&mut app, &w).labels.is_empty());
    app.run("editView.toggleTextMode", &json!({"mode": "edit"})).unwrap();
    let d = drawn(&mut app, &w);
    assert_eq!((d.negative, d.positive, d.cursors.len()), (0, 0, 0));
}

#[test]
fn the_selection_is_drawn_instead_of_the_cursor() {
    let mut app = text_app();
    let w = Window::new();
    app.run("editView.setCursor", &json!({"index": 0, "selectionLength": 2})).unwrap();
    let d = drawn(&mut app, &w);
    assert_eq!((d.selected, d.cursors.len()), (2, 0), "a box behind each selected glyph, no cursor");
}

#[test]
fn clicks_place_the_cursor_drags_select_and_a_double_click_edits() {
    let mut app = text_app();
    // Text coordinates: A 0–600, V 500–1100 (kerned −100), A 1140–1740.
    let click = |app: &mut FontcraftApp, x: f64, mods: Value| {
        pointer(app, json!([{"kind": "down", "x": x, "y": 300, "mods": mods}, {"kind": "up", "x": x, "y": 300, "mods": mods}]))
    };
    click(&mut app, 1000.0, json!([]));
    let t = get(&mut app);
    assert_eq!((&t["cursor"], &t["selection"], &t["current"]), (&json!(2), &json!([2, 2]), &json!("A")), "past V's middle: after V");
    click(&mut app, 100.0, json!([]));
    assert_eq!(get(&mut app)["cursor"], 0);
    pointer(
        &mut app,
        json!([{"kind": "down", "x": 100, "y": 300}, {"kind": "drag", "x": 600, "y": 300}, {"kind": "drag", "x": 1000, "y": 300}, {"kind": "up", "x": 1000, "y": 300}]),
    );
    let t = get(&mut app);
    assert_eq!((&t["cursor"], &t["selection"], &t["current"]), (&json!(0), &json!([0, 2]), &json!("A")), "a drag selects");
    click(&mut app, 1700.0, json!(["shift"]));
    assert_eq!(get(&mut app)["selection"], json!([0, 3]), "Shift+click extends the selection");
    // A click past the end of the line puts the cursor at its end.
    click(&mut app, 2000.0, json!([]));
    assert_eq!(get(&mut app)["cursor"], 3);
    // A double-click on V edits it.
    pointer(
        &mut app,
        json!([{"kind": "down", "x": 800, "y": 300}, {"kind": "up", "x": 800, "y": 300}, {"kind": "down", "x": 800, "y": 300}, {"kind": "doubleclick", "x": 800, "y": 300}, {"kind": "up", "x": 800, "y": 300}]),
    );
    let t = get(&mut app);
    assert_eq!((&t["textMode"], &t["cursor"], &t["current"]), (&json!(false), &json!(1), &json!("V")));
    // In edit mode a click goes to the active tool and leaves the cursor alone.
    click(&mut app, 100.0, json!([]));
    assert_eq!(get(&mut app)["cursor"], 1);
}

#[test]
fn a_mouse_click_on_the_canvas_places_the_cursor() {
    let mut app = text_app();
    let w = Window::new();
    w.settle(&mut app);
    let at = screen(&app, 1000.0, 300.0);
    w.frame(&mut app, vec![Event::PointerMoved(at)]);
    w.press(&mut app, at, true);
    w.press(&mut app, at, false);
    assert_eq!(get(&mut app)["cursor"], 2);
}

#[test]
fn bottom_bar_buttons_set_the_direction_and_cycle_the_kerning_mode() {
    let mut app = text_app();
    let w = Window::new();
    w.settle(&mut app);
    for direction in ["rtl", "ttb", "ltr"] {
        w.click_control(&mut app, widgets::id(("bottomBar.direction", direction)));
        assert_eq!(get(&mut app)["direction"], direction);
    }
    for mode in ["locked", "none", "kerning"] {
        w.click_control(&mut app, widgets::id("bottomBar.kerningMode"));
        assert_eq!(get(&mut app)["kerning"], mode);
    }
}

/// § 5 of the M5 plan: an Edit View frame with 200 glyphs of text in kerning mode under 16 ms.
/// Also times 10,000 items (the text layout target's size), all with Show Metrics on. Run with
/// `cargo test --release -p fontcraft-ui-egui text_mode_frame_speed -- --ignored --nocapture`.
#[test]
#[ignore]
fn text_mode_frame_speed() {
    for n in [200usize, 10_000] {
        let mut app = text_app();
        let text: String = "AVAH ".chars().cycle().take(n - 1).collect();
        app.run("editView.setText", &json!({"text": text})).unwrap();
        app.run("editView.setCursor", &json!({"index": 0})).unwrap();
        zoom(&mut app, 60, [6000, -2000]);
        let w = Window::new();
        w.settle(&mut app);
        // A layout and a frame after a text change, then steady frames.
        let started = std::time::Instant::now();
        app.run("editView.insertText", &json!({"chars": "A", "at": 0})).unwrap();
        w.frame(&mut app, Vec::new());
        let after_edit = started.elapsed();
        let runs = 20;
        let started = std::time::Instant::now();
        for i in 0..runs {
            // Move the cursor so the overlays redraw; the layout stays cached.
            app.run("editView.setCursor", &json!({"index": i})).unwrap();
            w.frame(&mut app, Vec::new());
        }
        let steady = started.elapsed() / runs as u32;
        println!("text mode, {n} items: frame after an edit {after_edit:?}, steady frame {steady:?}");
    }
}
