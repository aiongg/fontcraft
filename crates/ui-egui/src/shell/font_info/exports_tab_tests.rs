//! The Exports tab: its controls give the font their commands give, selection with Cmd and
//! Shift, drag to reorder, grey icons, the preview's glyphs, and Cancel in the parameter picker.

use std::sync::Arc;

use egui::{Event, Key, Modifiers, PointerButton, Pos2};
use fontcraft_engine::fontinfo::Scope;
use fontcraft_model::{Axis, Font, Glyph, GlyphTable, Instance, Layer, Master, Path, Shape};
use serde_json::{Value, json};

use super::*;
use crate::FontcraftApp;
use crate::dialogs::custom_parameter_picker;
use crate::shell::font_info::Picker;
use crate::shell::font_info::rows::{Target, plus_id};

fn input(events: Vec<Event>) -> egui::RawInput {
    egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1280.0, 800.0))), events, ..Default::default() }
}

/// The whole app, one egui context kept across frames.
struct Window {
    ctx: egui::Context,
}

impl Window {
    fn new() -> Self {
        Self { ctx: egui::Context::default() }
    }

    fn frame(&self, app: &mut FontcraftApp, events: Vec<Event>) {
        let mut output = self.ctx.run_ui(input(events), |ui| app.ui(ui));
        output.textures_delta.clear();
    }

    fn at(&self, id: egui::Id) -> Pos2 {
        self.ctx.read_response(id).unwrap_or_else(|| panic!("{id:?} is drawn")).rect.center()
    }

    fn click(&self, app: &mut FontcraftApp, id: egui::Id, modifiers: Modifiers) {
        // A modal whose content changed size is centred again over the next two frames.
        self.frame(app, Vec::new());
        self.frame(app, Vec::new());
        let at = self.at(id);
        self.frame(app, vec![Event::ModifiersChanged(modifiers), Event::PointerMoved(at)]);
        self.frame(app, vec![Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers }]);
        self.frame(app, vec![Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers }]);
        self.frame(app, vec![Event::ModifiersChanged(Modifiers::NONE)]);
    }

    /// Opens the menu of `menu` and clicks its entry `item`.
    fn pick(&self, app: &mut FontcraftApp, menu: egui::Id, item: egui::Id) {
        self.click(app, menu, Modifiers::NONE);
        self.frame(app, Vec::new());
        self.click(app, item, Modifiers::NONE);
    }
}

/// A font with the given export entries, and its Font Info window on the Exports tab.
fn exports_app(names: &[&str]) -> FontcraftApp {
    let mut app = FontcraftApp::default();
    for name in names {
        app.run("font.instance.add", &json!({"styleName": name})).unwrap();
    }
    app.run("font.info", &json!({"tab": "exports"})).unwrap();
    app
}

fn instances(app: &FontcraftApp) -> String {
    format!("{:?}", app.session.font().unwrap().instances)
}

fn open(app: &mut FontcraftApp) -> Window {
    let w = Window::new();
    w.frame(app, Vec::new());
    w.frame(app, Vec::new());
    w
}

#[test]
fn exports_tab_plus_menu_adds_what_font_instance_add_adds_and_minus_removes_the_selection() {
    let mut app = exports_app(&["Light"]);
    let w = open(&mut app);
    w.pick(&mut app, plus_menu_id(), plus_item_id("font.instance.add"));
    let mut direct = exports_app(&["Light"]);
    direct.run("font.instance.add", &json!({})).unwrap();
    assert_eq!(instances(&app), instances(&direct), "Add Instance gives the font the command gives");
    assert_eq!(app.state.panels.font_info.selected_instances, [1], "the new entry is selected");
    // Minus removes the selected entries only.
    w.click(&mut app, entry_id(0), Modifiers::NONE);
    w.click(&mut app, minus_id(), Modifiers::NONE);
    direct.run("font.instance.remove", &json!({"instances": [0]})).unwrap();
    assert_eq!(instances(&app), instances(&direct));
    assert_eq!(app.session.font().unwrap().instances.len(), 1);
    // Each is one undo step.
    app.run("edit.undo", &json!({})).unwrap();
    assert_eq!(app.session.font().unwrap().instances.len(), 2);
}

#[test]
fn exports_tab_list_selects_with_cmd_and_shift_and_class_menus_set_every_selected_entry() {
    let mut app = exports_app(&["Light", "Regular", "Bold"]);
    let w = open(&mut app);
    w.click(&mut app, entry_id(0), Modifiers::NONE);
    w.click(&mut app, entry_id(2), Modifiers::SHIFT);
    assert_eq!(app.state.panels.font_info.selected_instances, [0, 1, 2], "Shift-click selects the range");
    w.click(&mut app, entry_id(1), Modifiers::NONE);
    assert_eq!(app.state.panels.font_info.selected_instances, [1], "a plain click selects one entry");
    w.click(&mut app, entry_id(0), Modifiers::NONE);
    w.click(&mut app, entry_id(2), Modifiers::COMMAND);
    assert_eq!(app.state.panels.font_info.selected_instances, [0, 2], "Cmd-click adds an entry");
    w.pick(&mut app, class_menu_id("weightClass"), class_item_id("weightClass", 700));
    w.pick(&mut app, class_menu_id("widthClass"), class_item_id("widthClass", 3));
    let mut direct = exports_app(&["Light", "Regular", "Bold"]);
    direct.run("font.instance.setWeightClass", &json!({"instances": [0, 2], "weightClass": 700})).unwrap();
    direct.run("font.instance.setWidthClass", &json!({"instances": [0, 2], "widthClass": 3})).unwrap();
    assert_eq!(instances(&app), instances(&direct), "the menus give the font the commands give");
    w.click(&mut app, entry_id(2), Modifiers::COMMAND);
    assert_eq!(app.state.panels.font_info.selected_instances, [0], "Cmd-click on a selected entry drops it");
}

#[test]
fn exports_tab_active_style_linking_and_style_name_act_through_their_commands() {
    let mut app = exports_app(&["Regular", "Italic"]);
    let w = open(&mut app);
    w.click(&mut app, entry_id(1), Modifiers::NONE);
    w.click(&mut app, field_id("active"), Modifiers::NONE);
    w.click(&mut app, field_id("isItalic"), Modifiers::NONE);
    // The base style name and the style name commit on Return.
    for (key, text) in [("linkStyle", "Regular"), ("styleName", "Oblique")] {
        w.click(&mut app, field_id(key), Modifiers::NONE);
        w.frame(&mut app, vec![Event::Text(text.into())]);
        w.frame(&mut app, vec![Event::Key { key: Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }]);
        w.frame(&mut app, Vec::new());
    }
    let mut direct = exports_app(&["Regular", "Italic"]);
    direct.run("font.instance.setActive", &json!({"instances": [1], "active": false})).unwrap();
    direct.run("font.instance.setStyleLinking", &json!({"instances": [1], "isItalic": true})).unwrap();
    direct.run("font.instance.setStyleLinking", &json!({"instances": [1], "linkStyle": "Regular"})).unwrap();
    direct.run("font.instance.set", &json!({"instances": [1], "styleName": "Oblique"})).unwrap();
    assert_eq!(instances(&app), instances(&direct), "the fields give the font the commands give");
    let font = app.session.font().unwrap();
    let inactive = font.instances.get(1).unwrap();
    assert_eq!(icon_color(&app.tokens, inactive), app.tokens.text_dim, "an inactive entry has a grey icon");
    assert_eq!(icon_color(&app.tokens, font.instances.first().unwrap()), app.tokens.accent);
}

#[test]
fn exports_tab_drag_moves_an_entry_through_reorder() {
    let mut app = exports_app(&["Light", "Regular", "Bold"]);
    let w = open(&mut app);
    let (from, over, to) = (w.at(entry_id(0)), w.at(entry_id(1)), w.at(entry_id(2)));
    let button = |pos, pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    w.frame(&mut app, vec![Event::PointerMoved(from)]);
    w.frame(&mut app, vec![button(from, true)]);
    w.frame(&mut app, vec![Event::PointerMoved(over)]);
    w.frame(&mut app, vec![Event::PointerMoved(to)]);
    w.frame(&mut app, vec![button(to, false)]);
    w.frame(&mut app, Vec::new());
    let mut direct = exports_app(&["Light", "Regular", "Bold"]);
    direct.run("font.instance.reorder", &json!({"instance": 0, "toIndex": 2})).unwrap();
    assert_eq!(instances(&app), instances(&direct));
    assert_eq!(app.state.panels.font_info.selected_instances, [2], "the moved entry stays selected");
}

#[test]
fn exports_tab_parameter_picker_cancel_changes_nothing_and_add_adds_what_the_command_adds() {
    let mut app = exports_app(&["Regular"]);
    let w = open(&mut app);
    let search = |app: &mut FontcraftApp| match &mut app.state.panels.font_info.picker {
        Some(Picker::Parameter(p)) => {
            assert_eq!(p.target, Target { scope: Scope::Instance, indices: vec![0] }, "the picker adds to the selected export");
            p.query = PREVIEW_PARAMETER.to_string();
        }
        other => panic!("the parameter picker is open, not {other:?}"),
    };
    let before = instances(&app);
    w.click(&mut app, plus_id(Scope::Instance, "parameters"), Modifiers::NONE);
    search(&mut app);
    w.frame(&mut app, Vec::new());
    w.click(&mut app, custom_parameter_picker::item_id(PREVIEW_PARAMETER), Modifiers::NONE);
    w.click(&mut app, custom_parameter_picker::cancel_id(), Modifiers::NONE);
    assert!(app.state.panels.font_info.picker.is_none(), "Cancel closes the picker");
    assert_eq!(instances(&app), before, "Cancel changes nothing");
    w.click(&mut app, plus_id(Scope::Instance, "parameters"), Modifiers::NONE);
    search(&mut app);
    w.frame(&mut app, Vec::new());
    w.click(&mut app, custom_parameter_picker::item_id(PREVIEW_PARAMETER), Modifiers::NONE);
    w.click(&mut app, custom_parameter_picker::add_id(), Modifiers::NONE);
    let mut direct = exports_app(&["Regular"]);
    direct.run("font.customParameters.add", &json!({"scope": "instance", "targets": [0], "name": PREVIEW_PARAMETER})).unwrap();
    assert_eq!(instances(&app), instances(&direct), "the picker adds what the command adds");
}

#[test]
fn exports_tab_preview_draws_aang126_or_the_instance_preview_glyphs_at_a_masters_location() {
    let mut app = exports_app(&["Regular"]);
    let font = app.session.font().unwrap();
    let names = |found: Result<(String, Vec<String>), &'static str>| found.map(|(_, n)| n);
    // The default text's characters the font has (it has no digits).
    assert_eq!(names(preview_glyphs(font, 0)), Ok(vec!["A".to_string(), "a".into(), "n".into(), "g".into()]));
    app.run("font.customParameters.add", &json!({"scope": "instance", "targets": [0], "name": PREVIEW_PARAMETER, "value": "n A x.missing"})).unwrap();
    let font = app.session.font().unwrap();
    assert_eq!(names(preview_glyphs(font, 0)), Ok(vec!["n".to_string(), "A".into()]), "names the font lacks are skipped");
    // The tab draws the line into a texture.
    let w = open(&mut app);
    let kept: Option<PreviewTexture> = w.ctx.data(|d| d.get_temp(preview_id().with("texture")));
    assert!(kept.and_then(|k| k.texture).is_some(), "the preview is drawn");

    // Two masters on a weight axis: an instance at a master's location draws that master; one
    // between them draws its interpolated view.
    let mut font = Font::new("Two");
    Arc::make_mut(&mut font.axes).push(Axis { name: "Weight".into(), tag: "wght".into(), ..Axis::default() });
    let light = Master { id: "light".into(), name: "Light".into(), location: vec![300.0], ..Master::default() };
    let bold = Master { id: "bold".into(), name: "Bold".into(), location: vec![700.0], ..Master::default() };
    *Arc::make_mut(&mut font.masters) = vec![light, bold];
    let at = |w: f64| Instance { name: "X".into(), location: vec![w], ..Instance::default() };
    *Arc::make_mut(&mut font.instances) = vec![at(700.0), at(500.0)];
    assert_eq!(preview_glyphs(&font, 0).map(|(m, _)| m), Ok("bold".to_string()));
    assert_eq!(preview_glyphs(&font, 1).map(|(m, _)| m), Ok(view_master_id(1)), "an instance between masters draws its view");
}

#[test]
fn exports_tab_preview_draws_an_instance_between_masters_interpolated() {
    let mut app = FontcraftApp::default();
    app.session
        .edit("setup", |font| {
            font.axes = Arc::new(vec![Axis { name: "Weight".into(), tag: "wght".into(), ..Axis::default() }]);
            let mut light = font.default_master()?.clone();
            light.id = "light".into();
            light.name = "Light".into();
            light.location = vec![300.0];
            let mut bold = light.clone();
            bold.id = "bold".into();
            bold.name = "Bold".into();
            bold.location = vec![700.0];
            font.masters = Arc::new(vec![light, bold]);
            font.instances = Arc::new(vec![Instance { name: "Medium".into(), location: vec![500.0], ..Instance::default() }]);
            let layer = |master: &str, w: f64| {
                let mut l = Layer::new(master, w + 100.0);
                l.shapes = vec![Shape::Path(Path::rect(0.0, 0.0, w, 700.0))];
                l
            };
            let mut a = Glyph::new("A");
            a.unicodes = vec![0x41];
            a.layers = vec![layer("light", 100.0), layer("bold", 300.0)];
            font.glyphs = GlyphTable::from_glyphs(vec![Arc::new(a)]);
            Ok(())
        })
        .unwrap();
    app.run("font.info", &json!({"tab": "exports"})).unwrap();
    assert_eq!(preview_glyphs(app.session.font().unwrap(), 0), Ok((view_master_id(0), vec!["A".to_string()])));
    let w = open(&mut app);
    let kept: Option<PreviewTexture> = w.ctx.data(|d| d.get_temp(preview_id().with("texture")));
    assert!(kept.and_then(|k| k.texture).is_some(), "the interpolated line is drawn");
}

#[test]
fn class_names_of_older_sources_show_as_their_numbers() {
    assert_eq!(class_value(None, WEIGHT_NAMES, 400.0), Some(400.0), "no class is the default");
    assert_eq!(class_value(Some(&StyleClass::Name("SemiBold".into())), WEIGHT_NAMES, 400.0), Some(600.0));
    assert_eq!(class_value(Some(&StyleClass::Name("Medium (normal)".into())), WIDTH_NAMES, 5.0), Some(5.0));
    assert_eq!(class_value(Some(&StyleClass::Name("Wobbly".into())), WEIGHT_NAMES, 400.0), None);
}

#[test]
fn a_preview_wider_than_the_largest_texture_is_drawn_smaller_to_fit() {
    let app = exports_app(&["Regular"]);
    let font = app.session.font().unwrap();
    let master = font.masters.first().unwrap().id.clone();
    let names = vec!["A".to_string(); 200];
    let ctx = egui::Context::default();
    let texture = render_preview(&ctx, &app.tokens, font, &master, &names, 60.0, 256).expect("the preview is drawn smaller");
    let [width, height] = texture.size();
    assert!(width <= 256 && height <= 256 && width > 64, "{width}×{height}");
    // The context's own largest texture side is 2048, so the full-size line is shorter.
    let short = names.get(..30).unwrap();
    let full = render_preview(&ctx, &app.tokens, font, &master, short, 60.0, 2048).unwrap();
    assert!(full.size()[0] > 256, "a large enough texture keeps the asked size");
}

#[test]
fn exports_tab_coordinates_act_through_set_axis_location_and_warn_of_extrapolation() {
    let setup = || {
        let mut app = exports_app(&["Light", "Bold"]);
        app.run("font.master.add", &json!({"name": "Bold"})).unwrap();
        app.run("font.axes.add", &json!({"name": "Weight"})).unwrap();
        app.run("font.master.setAxisLocation", &json!({"masters": [1], "location": {"wght": 100}})).unwrap();
        app
    };
    let type_into = |w: &Window, app: &mut FontcraftApp, text: &str| {
        w.click(app, axis_field_id(0), Modifiers::NONE);
        w.frame(app, vec![Event::Text(text.into())]);
        w.frame(app, vec![Event::Key { key: Key::Enter, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE }]);
        w.frame(app, Vec::new());
    };
    let mut app = setup();
    let mut direct = setup();
    let w = open(&mut app);
    // Both entries take one typed coordinate; between the masters there is no warning.
    w.click(&mut app, entry_id(0), Modifiers::NONE);
    w.click(&mut app, entry_id(1), Modifiers::COMMAND);
    type_into(&w, &mut app, "50");
    direct.run("font.instance.setAxisLocation", &json!({"instances": [0, 1], "location": {"wght": 50}})).unwrap();
    assert_eq!(instances(&app), instances(&direct));
    assert!(w.ctx.read_response(extrapolation_id(0)).is_none(), "an instance between the masters is interpolated");

    // Beyond the masters: the tab shows the warning the command gives.
    w.click(&mut app, entry_id(1), Modifiers::NONE);
    type_into(&w, &mut app, "150");
    let reply = direct.run("font.instance.setAxisLocation", &json!({"instances": [1], "location": {"wght": 150}})).unwrap();
    assert_eq!(instances(&app), instances(&direct));
    assert!(w.ctx.read_response(extrapolation_id(0)).is_some(), "the extrapolated instance is marked");
    let font = app.session.font().unwrap();
    let warning = font.instances.get(1).and_then(|i| extrapolation_warning(font, i));
    assert_eq!(warning.as_deref(), reply.get("warnings").and_then(|w| w.get(0)).and_then(Value::as_str), "the command's warning");
    w.click(&mut app, entry_id(0), Modifiers::NONE);
    assert!(w.ctx.read_response(extrapolation_id(0)).is_none(), "the entry inside the masters has none");
}
