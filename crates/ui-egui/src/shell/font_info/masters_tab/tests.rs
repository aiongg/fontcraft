//! The Masters tab drawn on its own: the list's selection, plus menu, minus and drag, the
//! metric and stem fields on several masters, the icon picker and the metric editor. Each
//! control gives the font the command it stands for gives; Cancel changes nothing.

use egui::{Event, Key, Modifiers, PointerButton, Pos2};
use fontcraft_model::metric;
use serde_json::{Value, json};

use super::*;
use crate::FontcraftApp;
use crate::dialogs::metric_editor::Subject;
use crate::dialogs::{master_icon_picker, metric_editor};
use crate::shell::font_info::Tab;

fn input(events: Vec<Event>) -> egui::RawInput {
    egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1600.0, 1200.0))), events, ..Default::default() }
}

/// The tab in a context of its own; the commands a frame queues run after it, as the window
/// runs them.
struct TabUi {
    ctx: egui::Context,
}

impl TabUi {
    fn new() -> Self {
        Self { ctx: egui::Context::default() }
    }

    fn step(&self, app: &mut FontcraftApp, events: Vec<Event>) {
        let mut pending = Vec::new();
        let mut output = self.ctx.run_ui(input(events), |ui| {
            app.state.panels.font_info.shown_rows.clear();
            show(app, ui, &mut pending);
        });
        output.textures_delta.clear();
        for (id, params) in pending {
            app.run(id, &params).unwrap_or_else(|e| panic!("{id} {params}: {e}"));
        }
    }

    fn center(&self, app: &mut FontcraftApp, id: egui::Id) -> Pos2 {
        self.step(app, Vec::new());
        self.ctx.read_response(id).unwrap_or_else(|| panic!("{id:?} is drawn")).rect.center()
    }

    fn click(&self, app: &mut FontcraftApp, id: egui::Id, modifiers: Modifiers) {
        let at = self.center(app, id);
        self.step(app, vec![Event::ModifiersChanged(modifiers), Event::PointerMoved(at)]);
        self.step(app, vec![Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers }]);
        self.step(app, vec![Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers }]);
        self.step(app, vec![Event::ModifiersChanged(Modifiers::NONE)]);
    }

    /// Clicks a field, types `text` and presses Enter.
    fn type_into(&self, app: &mut FontcraftApp, id: egui::Id, text: &str) {
        self.click(app, id, Modifiers::NONE);
        self.step(app, vec![Event::Text(text.to_string())]);
        let enter = Event::Key { key: Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE };
        self.step(app, vec![enter]);
        self.step(app, Vec::new());
    }
}

/// A font with the masters Regular and Bold, Bold's x-height at 520, on the Masters tab.
fn two_masters() -> FontcraftApp {
    let mut app = FontcraftApp::default();
    app.run("font.master.add", &json!({"name": "Bold"})).unwrap();
    app.run("font.master.setMetric", &json!({"masters": [1], "metric": "x-height", "position": 520})).unwrap();
    app.state.panels.font_info.tab = Tab::Masters;
    app
}

fn run(app: &mut FontcraftApp, id: &str, params: Value) {
    app.run(id, &params).unwrap_or_else(|e| panic!("{id}: {e}"));
}

fn masters(app: &FontcraftApp) -> String {
    format!("{:?}", app.session.font().unwrap().masters)
}

fn definitions(app: &FontcraftApp) -> String {
    let font = app.session.font().unwrap();
    format!("{:?} {:?} {:?}", font.metrics, font.stems, font.masters)
}

fn x_height(app: &FontcraftApp) -> usize {
    app.session.font().unwrap().metrics.iter().position(|m| m.kind == metric::X_HEIGHT).unwrap()
}

#[test]
fn several_selected_masters_show_multiple_values_and_take_one_edit() {
    let mut app = two_masters();
    let t = TabUi::new();
    let xh = x_height(&app);
    t.click(&mut app, master_id(1), Modifiers::NONE);
    assert_eq!(app.state.panels.font_info.selected_masters, [1]);
    t.click(&mut app, master_id(0), Modifiers::COMMAND);
    assert_eq!(app.state.panels.font_info.selected_masters, [0, 1], "Cmd-click adds a master");
    let target = Target { scope: fontcraft_engine::fontinfo::Scope::Master, indices: vec![0, 1] };
    let (position, zone) = metric_values(app.session.font().unwrap(), &target, xh);
    assert_eq!(position, None, "the x-heights differ: Multiple Values");
    assert!(zone.is_some(), "the zones agree");
    // One typed value goes to both masters, as the command does.
    let mut direct = two_masters();
    t.type_into(&mut app, metric_id(xh, "position"), "500");
    run(&mut direct, "font.master.setMetric", json!({"masters": [0, 1], "metric": xh, "position": 500}));
    assert_eq!(masters(&app), masters(&direct));
    // A stem: the plus beside Stems adds one, its width field writes both masters.
    t.click(&mut app, stems_plus_id(), Modifiers::NONE);
    t.type_into(&mut app, stem_id(0, "value"), "80");
    run(&mut direct, "font.stems.add", json!({"orientation": "vertical"}));
    run(&mut direct, "font.master.setStem", json!({"masters": [0, 1], "stem": 0, "value": 80}));
    assert_eq!(definitions(&app), definitions(&direct));
    t.click(&mut app, stem_id(0, "orientation"), Modifiers::NONE);
    run(&mut direct, "font.stems.set", json!({"stem": 0, "orientation": "horizontal"}));
    assert_eq!(definitions(&app), definitions(&direct));
    // Shift-click selects the range from the master clicked last.
    run(&mut app, "font.master.add", json!({"name": "Black"}));
    t.click(&mut app, master_id(2), Modifiers::NONE);
    t.click(&mut app, master_id(0), Modifiers::SHIFT);
    assert_eq!(app.state.panels.font_info.selected_masters, [0, 1, 2]);
}

#[test]
fn the_plus_menu_and_minus_give_the_font_the_master_commands_give() {
    let mut app = two_masters();
    let mut direct = two_masters();
    let t = TabUi::new();
    t.click(&mut app, plus_id(), Modifiers::NONE);
    t.click(&mut app, menu_item_id("add"), Modifiers::NONE);
    run(&mut direct, "font.master.add", json!({}));
    assert_eq!(masters(&app), masters(&direct), "Add Master");
    assert_eq!(app.state.panels.font_info.selected_masters, [2], "the new master is selected");
    t.click(&mut app, master_id(1), Modifiers::NONE);
    t.click(&mut app, plus_id(), Modifiers::NONE);
    t.click(&mut app, menu_item_id("duplicate"), Modifiers::NONE);
    run(&mut direct, "font.master.duplicate", json!({"masters": [1]}));
    assert_eq!(masters(&app), masters(&direct), "Duplicate Selected");
    t.click(&mut app, master_id(1), Modifiers::NONE);
    t.click(&mut app, minus_id(), Modifiers::NONE);
    run(&mut direct, "font.master.remove", json!({"masters": [1]}));
    assert_eq!(masters(&app), masters(&direct), "minus");
    // With every master selected the minus does nothing: a font keeps one master.
    let mut one = FontcraftApp::default();
    one.state.panels.font_info.tab = Tab::Masters;
    t.click(&mut one, minus_id(), Modifiers::NONE);
    assert_eq!(one.session.font().unwrap().masters.len(), 1);
}

#[test]
fn dragging_a_master_onto_another_moves_it_as_reorder_does() {
    let mut app = two_masters();
    let t = TabUi::new();
    let from = t.center(&mut app, master_id(0));
    let to = t.center(&mut app, master_id(1));
    let press = |pressed: bool, pos: Pos2| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    t.step(&mut app, vec![Event::PointerMoved(from)]);
    t.step(&mut app, vec![press(true, from)]);
    t.step(&mut app, vec![Event::PointerMoved(from + egui::vec2(0.0, 10.0))]);
    t.step(&mut app, vec![Event::PointerMoved(to)]);
    t.step(&mut app, vec![Event::PointerMoved(to)]);
    t.step(&mut app, vec![press(false, to)]);
    t.step(&mut app, Vec::new());
    let mut direct = two_masters();
    run(&mut direct, "font.master.reorder", json!({"master": 0, "toIndex": 1}));
    assert_eq!(masters(&app), masters(&direct));
    assert_eq!(app.session.font().unwrap().masters.first().map(|m| m.name.as_str()), Some("Bold"));
}

#[test]
fn the_icon_picker_sets_what_set_icon_sets_and_cancel_changes_nothing() {
    let mut app = two_masters();
    let mut direct = two_masters();
    let t = TabUi::new();
    t.click(&mut app, icon_id(), Modifiers::NONE);
    let picker = open_icon_picker(&t.ctx).expect("the icon opens the picker");
    assert_eq!(picker.masters, [0]);
    t.click(&mut app, master_icon_picker::icon_id("Bold_Condensed"), Modifiers::NONE);
    assert!(open_icon_picker(&t.ctx).is_none(), "a pick closes the picker");
    run(&mut direct, "font.master.setIcon", json!({"masters": [0], "icon": "Bold_Condensed"}));
    assert_eq!(masters(&app), masters(&direct));
    // Cancel changes nothing.
    t.click(&mut app, icon_id(), Modifiers::NONE);
    assert!(open_icon_picker(&t.ctx).is_some());
    t.click(&mut app, master_icon_picker::cancel_id(), Modifiers::NONE);
    assert!(open_icon_picker(&t.ctx).is_none());
    assert_eq!(masters(&app), masters(&direct), "Cancel changes nothing");
    // A glyph of the font (a new font has `n`) typed into the field.
    t.click(&mut app, icon_id(), Modifiers::NONE);
    t.click(&mut app, master_icon_picker::glyph_id(), Modifiers::NONE);
    t.step(&mut app, vec![Event::Text("n".into())]);
    t.click(&mut app, master_icon_picker::use_glyph_id(), Modifiers::NONE);
    run(&mut direct, "font.master.setIcon", json!({"masters": [0], "glyph": "n"}));
    assert_eq!(masters(&app), masters(&direct), "the glyph field");
}

#[test]
fn the_metric_editor_gives_the_font_the_metric_commands_give_and_cancel_changes_nothing() {
    let mut app = two_masters();
    let mut direct = two_masters();
    let t = TabUi::new();
    let xh = x_height(&app);
    t.click(&mut app, metric_id(xh, "name"), Modifiers::NONE);
    assert_eq!(open_editor(&t.ctx).map(|e| e.subject), Some(Subject::Metric(xh)), "a metric's name opens the editor");
    t.click(&mut app, metric_editor::name_id(), Modifiers::NONE);
    t.step(&mut app, vec![Event::Text("small".into())]);
    t.click(&mut app, metric_editor::ok_id(), Modifiers::NONE);
    assert!(open_editor(&t.ctx).is_none(), "OK closes the editor");
    run(&mut direct, "font.metrics.set", json!({"metric": xh, "name": "small"}));
    assert_eq!(definitions(&app), definitions(&direct));
    // Cancel changes nothing.
    t.click(&mut app, metric_id(0, "name"), Modifiers::NONE);
    t.click(&mut app, metric_editor::name_id(), Modifiers::NONE);
    t.step(&mut app, vec![Event::Text("other".into())]);
    t.click(&mut app, metric_editor::cancel_id(), Modifiers::NONE);
    assert!(open_editor(&t.ctx).is_none());
    assert_eq!(definitions(&app), definitions(&direct), "Cancel changes nothing");
    // The plus beside the heading adds a metric of a category the font lacks.
    t.click(&mut app, metrics_plus_id(), Modifiers::NONE);
    let new = open_editor(&t.ctx).expect("the plus opens the editor");
    assert_eq!(new.subject, Subject::NewMetric);
    t.click(&mut app, metric_editor::ok_id(), Modifiers::NONE);
    run(&mut direct, "font.metrics.add", json!({"category": new.category}));
    assert_eq!(definitions(&app), definitions(&direct));
    // A metric's minus removes it from every master.
    t.click(&mut app, metric_id(0, "minus"), Modifiers::NONE);
    run(&mut direct, "font.metrics.remove", json!({"metric": 0}));
    assert_eq!(definitions(&app), definitions(&direct));
}
