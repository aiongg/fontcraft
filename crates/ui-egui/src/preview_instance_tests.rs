//! The preview of instances and the master icons in headless frames: the instance pop-up and
//! `preview.setInstance` (the instance's widths in the line, the TrueType Instructor following),
//! Show All Instances with the eye toggles, the dash following the tab's master, the panel's own
//! choice, and the toolbar's master icons running `view.selectMaster`.

use std::sync::Arc;

use egui::{Event, Modifiers, PointerButton, Pos2, Rect};
use fontcraft_model::{Axis, Glyph, GlyphTable, Instance, Layer, Path, Shape};
use serde_json::json;

use crate::FontcraftApp;
use crate::shell::master_switcher;
use crate::shell::preview::{self, Shows, Target};

/// A box `w` wide, advance `w + 100`.
fn layer(master: &str, w: f64) -> Layer {
    let mut l = Layer::new(master, w + 100.0);
    l.shapes = vec![Shape::Path(Path::rect(0.0, 0.0, w, 700.0))];
    l
}

/// Masters Thin (100) and Black (900); instances Light (300), Regular (500) and a variable font
/// setting; `a` is 100 to 300 wide, `b` 200 to 600. A tab holds "ab" with `a` edited and shows
/// the Preview Area.
fn instance_app() -> FontcraftApp {
    let mut app = FontcraftApp::default();
    app.session
        .edit("setup", |font| {
            font.axes = Arc::new(vec![Axis { name: "Weight".into(), tag: "wght".into(), ..Axis::default() }]);
            let mut thin = font.default_master()?.clone();
            thin.id = "thin".into();
            thin.name = "Thin".into();
            thin.location = vec![100.0];
            let mut black = thin.clone();
            black.id = "black".into();
            black.name = "Black".into();
            black.location = vec![900.0];
            font.masters = Arc::new(vec![thin, black]);
            font.instances = Arc::new(vec![
                Instance { name: "Light".into(), location: vec![300.0], ..Instance::default() },
                Instance { name: "Regular".into(), location: vec![500.0], ..Instance::default() },
                Instance { name: "Variable".into(), kind: Some("variable".into()), ..Instance::default() },
            ]);
            let glyph = |name: &str, unicode: u32, thin: f64, black: f64| {
                let mut g = Glyph::new(name);
                g.unicodes = vec![unicode];
                g.layers = vec![layer("thin", thin), layer("black", black)];
                Arc::new(g)
            };
            font.glyphs = GlyphTable::from_glyphs(vec![glyph("a", 0x61, 100.0, 300.0), glyph("b", 0x62, 200.0, 600.0)]);
            Ok(())
        })
        .unwrap();
    app.run("view.newTab", &json!({"glyphs": ["a"]})).unwrap();
    app.run("editView.setText", &json!({"text": "ab"})).unwrap();
    app.run("editView.setCursor", &json!({"index": 0})).unwrap();
    app.run("view.togglePreviewArea", &json!({"on": true})).unwrap();
    app
}

/// A headless window: one egui context kept across frames.
struct Window {
    ctx: egui::Context,
}

impl Window {
    fn new() -> Self {
        Self { ctx: egui::Context::default() }
    }

    fn frame(&self, app: &mut FontcraftApp, events: Vec<Event>) {
        let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1280.0, 800.0))), events, ..Default::default() };
        let mut output = self.ctx.run_ui(input, |ui| app.ui(ui));
        output.textures_delta.clear();
    }

    fn settle(&self, app: &mut FontcraftApp) {
        self.frame(app, Vec::new());
        self.frame(app, Vec::new());
    }

    fn drawn(&self, id: egui::Id) -> bool {
        self.ctx.read_response(id).is_some()
    }

    fn click(&self, app: &mut FontcraftApp, id: egui::Id) {
        let at = self.ctx.read_response(id).unwrap_or_else(|| panic!("{id:?} was not drawn")).rect.center();
        self.frame(app, vec![Event::PointerMoved(at)]);
        for pressed in [true, false] {
            self.frame(app, vec![Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE }]);
        }
        self.settle(app);
    }
}

fn advances(layout: &fontcraft_textlayout::Layout) -> Vec<f64> {
    layout.placed.iter().map(|p| p.advance).collect()
}

#[test]
fn preview_set_instance_lays_the_line_out_at_the_instance_and_moves_the_instructor_along() {
    let mut app = instance_app();
    let w = Window::new();
    let r = app.run("preview.setInstance", &json!({"instance": "Regular"})).unwrap();
    assert_eq!(r, json!({"target": "area", "instance": "Regular", "index": 1}));
    assert_eq!(app.state.tt_instructor.instance.as_deref(), Some("Regular"));
    w.settle(&mut app);
    let (_, layout) = preview::drawn_instance(&w.ctx, Target::Area).expect("the area drew the instance");
    assert_eq!(advances(&layout), [300.0, 500.0]);
    assert!(preview::texture_id(&w.ctx, Target::Area).is_some());

    // By index; then the dash, which the Instructor follows too; Show All leaves it alone.
    app.run("preview.setInstance", &json!({"instance": 0})).unwrap();
    assert_eq!(app.state.panels.preview.area_shows, Shows::Instance("Light".into()));
    app.run("preview.setInstance", &json!({"instance": "master"})).unwrap();
    assert_eq!(app.state.tt_instructor.instance, None);
    app.run("preview.setInstance", &json!({"instance": "all"})).unwrap();
    assert_eq!(app.state.panels.preview.area_shows, Shows::All);
    assert_eq!(app.state.tt_instructor.instance, None);

    for bad in [json!({"instance": "Variable"}), json!({"instance": 7}), json!({"instance": "Nope"}), json!({}), json!({"instance": true})] {
        let before = app.state.panels.preview.clone();
        assert!(app.run("preview.setInstance", &bad).is_err(), "{bad}");
        assert_eq!(app.state.panels.preview, before, "a refused choice changes nothing");
    }

    // The panel keeps a choice of its own.
    app.run("preview.setInstance", &json!({"target": "panel", "instance": "Light"})).unwrap();
    assert_eq!(app.state.panels.preview.panel_shows, Shows::Instance("Light".into()));
    assert_eq!(app.state.panels.preview.area_shows, Shows::All);
}

#[test]
fn show_all_instances_draws_the_edited_glyph_once_per_visible_instance() {
    let mut app = instance_app();
    let w = Window::new();
    assert_eq!(app.run("preview.showAllInstances", &json!({})).unwrap(), json!({"target": "area", "instance": "all"}));
    w.settle(&mut app);
    let line = preview::drawn_all(&w.ctx, Target::Area).expect("the area drew all instances");
    assert_eq!(line.glyph, "a");
    // Light 250 and Regular 300 wide; the variable font setting is not an instance here.
    assert_eq!(line.items.len(), 2);
    assert_eq!(line.length, 550.0);

    app.run("preview.toggleInstanceVisibility", &json!({"instance": "Light"})).unwrap();
    w.settle(&mut app);
    let line = preview::drawn_all(&w.ctx, Target::Area).unwrap();
    assert_eq!((line.items.len(), line.length), (1, 300.0), "the hidden instance is left out");

    app.run("edit.undo", &json!({})).unwrap();
    w.settle(&mut app);
    assert_eq!(preview::drawn_all(&w.ctx, Target::Area).unwrap().items.len(), 2);
}

#[test]
fn the_instance_pop_up_runs_the_commands_and_the_dash_follows_the_master() {
    let mut app = instance_app();
    let w = Window::new();
    w.settle(&mut app);
    let menu = preview::instance_menu(&app, Target::Area);
    assert_eq!(menu.label, preview::DASH);
    assert_eq!(menu.instances, [(0, "Light".to_string(), true), (1, "Regular".to_string(), true)]);

    w.click(&mut app, preview::instance_button_id(Target::Area));
    w.click(&mut app, crate::panels::widgets::id(("preview.instance", "area", 1usize)));
    assert_eq!(app.state.panels.preview.area_shows, Shows::Instance("Regular".into()));
    assert_eq!(preview::instance_menu(&app, Target::Area).label, "Regular");

    // With the dash, the line is the tab's master: Black after its toolbar icon is clicked.
    app.run("preview.setInstance", &json!({"instance": "master"})).unwrap();
    w.click(&mut app, master_switcher::icon_id(1));
    assert_eq!(app.state.tab().unwrap().master.as_deref(), Some("black"));
    let tab = app.state.tab().unwrap().clone();
    let layout = preview::line_layout(&mut app.session, &tab).unwrap();
    assert_eq!(advances(&layout), [400.0, 700.0]);
}

#[test]
fn master_icons_show_for_two_or_more_masters_only() {
    let mut app = instance_app();
    let w = Window::new();
    w.settle(&mut app);
    assert!(w.drawn(master_switcher::icon_id(0)) && w.drawn(master_switcher::icon_id(1)));
    assert!(!w.drawn(master_switcher::icon_id(2)));
    assert_eq!(master_switcher::hover("Thin", 0), "Thin (Cmd+1)");
    assert_eq!(master_switcher::hover("Tenth", 9), "Tenth");

    let mut single = FontcraftApp::default();
    single.run("view.newTab", &json!({"glyphs": ["A"]})).unwrap();
    let w = Window::new();
    w.settle(&mut single);
    assert!(!w.drawn(master_switcher::icon_id(0)), "one master shows no icons");
}
