//! The glyph inspector, the Selection Info window, the Font View glyph context menu, the glyph
//! part of the canvas context menu and the Info box colour labels, driven like a user would in
//! headless frames: focus a field, type, press Return; click a check box, a chevron, a swatch;
//! then check the command that ran and the font it gave.

use egui::{Event, Key, Modifiers, PointerButton, Pos2, Shape};
use fontcraft_geom::Point;
use serde_json::{Value, json};

use crate::panels::{selection_info, widgets};
use crate::shell::context_menu::{self, GlyphAction, GlyphMenu, TagMark};
use crate::shell::font_view::columns::Column;
use crate::shell::font_view::inspector::{self, Edit, SALT, Shown};
use crate::{FontcraftApp, tools_glue};

/// A headless window: one egui context kept across frames, so focus, popups and fields carry
/// over.
struct Window {
    ctx: egui::Context,
}

impl Window {
    fn new() -> Self {
        Self { ctx: egui::Context::default() }
    }

    /// One frame with these events; the shapes it drew, nested lists flattened.
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

    /// A few empty frames: panels and popups size themselves, focus requests land.
    fn settle(&self, app: &mut FontcraftApp) {
        for _ in 0..3 {
            self.frame(app, Vec::new());
        }
    }

    fn focus(&self, app: &mut FontcraftApp, id: egui::Id) {
        self.ctx.memory_mut(|m| m.request_focus(id));
        self.frame(app, Vec::new());
        assert!(self.ctx.memory(|m| m.has_focus(id)), "the field takes the focus");
    }

    fn type_text(&self, app: &mut FontcraftApp, text: &str) {
        self.frame(app, vec![Event::Text(text.into())]);
    }

    fn enter(&self, app: &mut FontcraftApp) {
        let key = |pressed| Event::Key { key: Key::Enter, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE };
        self.frame(app, vec![key(true), key(false)]);
        self.frame(app, Vec::new());
    }

    /// Focuses field `id`, types `text` and presses Return.
    fn commit(&self, app: &mut FontcraftApp, id: egui::Id, text: &str) {
        self.focus(app, id);
        self.type_text(app, text);
        self.enter(app);
    }

    fn press(&self, app: &mut FontcraftApp, at: Pos2, button: PointerButton) {
        let event = |pressed| Event::PointerButton { pos: at, button, pressed, modifiers: Modifiers::NONE };
        self.frame(app, vec![Event::PointerMoved(at)]);
        self.frame(app, vec![event(true)]);
        self.frame(app, vec![event(false)]);
    }

    /// Clicks the middle of the control `id` as the last frame drew it.
    fn click(&self, app: &mut FontcraftApp, id: egui::Id) {
        let at = self.ctx.read_response(id).unwrap_or_else(|| panic!("{id:?} was not drawn")).rect.center();
        self.press(app, at, PointerButton::Primary);
    }

    fn right_click(&self, app: &mut FontcraftApp, at: Pos2) {
        self.press(app, at, PointerButton::Secondary);
    }

    fn enabled(&self, id: egui::Id) -> bool {
        self.ctx.read_response(id).unwrap_or_else(|| panic!("{id:?} was not drawn")).enabled()
    }

    fn glyph_menu(&self) -> Option<GlyphMenu> {
        self.ctx.data(|d| d.get_temp::<GlyphMenu>(context_menu::glyph_menu_id()))
    }
}

fn names(list: &[&str]) -> Vec<String> {
    list.iter().map(|n| n.to_string()).collect()
}

/// `glyph.getInfo` of one glyph.
fn info(app: &mut FontcraftApp, glyph: &str) -> Value {
    app.run("glyph.getInfo", &json!({ "glyphs": [glyph] })).unwrap()["glyphs"][0].clone()
}

/// The last command the document recorded.
fn last(app: &FontcraftApp) -> (String, Value) {
    app.session.journal().last().cloned().expect("a command ran")
}

fn field(column: Column) -> egui::Id {
    inspector::field_id(SALT, column)
}

/// Where the last frame laid out the cell of `glyph`.
fn cell(app: &FontcraftApp, glyph: &str) -> Pos2 {
    app.font_view.laid_out.iter().find(|(n, _)| n == glyph).unwrap_or_else(|| panic!("{glyph} was not laid out")).1.center()
}

fn texts(shapes: &[Shape]) -> Vec<String> {
    shapes
        .iter()
        .filter_map(|s| match s {
            Shape::Text(t) => Some(t.galley.text().to_string()),
            _ => None,
        })
        .collect()
}

/// The properties a text field of the inspector sets, with the `glyph.getInfo` key it shows up
/// under and a value the engine takes.
const PROPERTIES: [(Column, &str, &str, &str); 4] = [
    (Column::ProductionName, "glyph.setProductionName", "productionName", "prod"),
    (Column::Script, "glyph.setScript", "script", "cyrillic"),
    (Column::Category, "glyph.setCategory", "category", "Symbol"),
    (Column::SubCategory, "glyph.setCategory", "subCategory", "Lowercase"),
];

#[test]
fn every_inspector_field_edits_one_glyph_through_its_command() {
    let mut app = FontcraftApp::default();
    app.run("fontView.setSelection", &json!({"glyphs": ["A"]})).unwrap();
    app.run("fontView.toggleInspectorDetails", &json!({"expanded": true})).unwrap();
    let w = Window::new();
    w.settle(&mut app);
    let s = inspector::summarize(&app, &names(&["A"]));
    assert_eq!((s.field(Column::Name), s.field(Column::Width), s.export), (Shown::Same("A".into()), Shown::Same("600".into()), Some(true)));
    for column in [Column::Lsb, Column::Rsb, Column::Width, Column::LeftGroup, Column::RightGroup] {
        assert!(!w.enabled(field(column)), "{column:?} is shown read-only until M5");
    }
    assert!(w.enabled(field(Column::Name)), "one glyph: the name can be edited");

    w.commit(&mut app, field(Column::Unicode), "0391");
    assert_eq!(last(&app), ("glyph.setUnicodes".into(), json!({"glyphs": ["A"], "unicodes": ["0391"]})));
    assert_eq!(info(&mut app, "A")["unicodes"], json!(["0391"]));

    for (column, command, key, value) in PROPERTIES {
        w.commit(&mut app, field(column), value);
        let (id, params) = last(&app);
        assert_eq!((id.as_str(), &params["glyphs"], &params[key]), (command, &json!(["A"]), &json!(value)), "{column:?}");
        assert_eq!(info(&mut app, "A")["properties"][key], json!({"value": value, "source": "stored"}), "{column:?}");
    }

    let tags = widgets::id((SALT, "tags"));
    w.focus(&mut app, tags);
    w.type_text(&mut app, "ss01,");
    assert_eq!(last(&app), ("glyph.addTag".into(), json!({"glyphs": ["A"], "tag": "ss01"})), "a comma commits the tag");
    w.type_text(&mut app, "round");
    w.enter(&mut app);
    assert_eq!(last(&app).1["tag"], "round", "Return commits the tag");
    assert_eq!(info(&mut app, "A")["tags"], json!(["round", "ss01"]));
    w.settle(&mut app);
    w.click(&mut app, widgets::id((SALT, "removeTag", "ss01")));
    assert_eq!(last(&app), ("glyph.removeTag".into(), json!({"glyphs": ["A"], "tag": "ss01"})));
    assert_eq!(info(&mut app, "A")["tags"], json!(["round"]));

    w.click(&mut app, widgets::id((SALT, "export")));
    assert_eq!(last(&app), ("glyph.setExport".into(), json!({"glyphs": ["A"], "export": false})));
    assert_eq!(info(&mut app, "A")["export"], false);

    w.commit(&mut app, field(Column::Name), "A.alt");
    assert_eq!(last(&app), ("glyph.rename".into(), json!({"glyph": "A", "newName": "A.alt"})));
    assert_eq!(app.state.selected_glyphs, ["A.alt"], "the renamed glyph stays selected");
    let steps = app.session.journal().len();
    w.settle(&mut app);
    assert_eq!(app.session.journal().len(), steps, "each edit commits once");
}

#[test]
fn on_three_glyphs_fields_show_multiple_values_and_edit_all_of_them() {
    let mut app = FontcraftApp::default();
    app.run("glyph.addTag", &json!({"glyphs": ["A"], "tag": "x"})).unwrap();
    app.run("glyph.setExport", &json!({"glyphs": ["B"], "export": false})).unwrap();
    app.run("fontView.setSelection", &json!({"glyphs": ["A", "B", "C"]})).unwrap();
    app.run("fontView.toggleInspectorDetails", &json!({"expanded": true})).unwrap();
    let three = names(&["A", "B", "C"]);
    let w = Window::new();
    w.settle(&mut app);
    let s = inspector::summarize(&app, &three);
    assert_eq!((s.field(Column::Name), s.field(Column::Unicode)), (Shown::Mixed, Shown::Mixed));
    assert_eq!((s.field(Column::Script), s.field(Column::Width)), (Shown::Same("latin".into()), Shown::Same("600".into())));
    assert_eq!((s.export, s.tags.clone()), (None, None), "Exports shows a line, Tags Multiple Values");
    let shapes = w.frame(&mut app, Vec::new());
    assert!(texts(&shapes).iter().any(|t| t == inspector::MULTIPLE), "a mixed field shows Multiple Values");
    assert!(!w.enabled(field(Column::Name)), "several glyphs cannot share one name");
    assert!(inspector::commands(&app, &three, &Edit::Text(Column::Name, "X".into())).is_empty());

    w.commit(&mut app, field(Column::Unicode), "E000");
    assert_eq!(last(&app), ("glyph.setUnicodes".into(), json!({"glyphs": three, "unicodes": ["E000"]})));
    for (column, command, key, value) in PROPERTIES {
        w.commit(&mut app, field(column), value);
        let (id, params) = last(&app);
        assert_eq!((id.as_str(), &params["glyphs"], &params[key]), (command, &json!(three), &json!(value)), "{column:?}");
        for glyph in &three {
            assert_eq!(info(&mut app, glyph)["properties"][key]["value"], json!(value), "{column:?} of {glyph}");
        }
    }

    w.commit(&mut app, widgets::id((SALT, "tags")), "new");
    assert_eq!(last(&app), ("glyph.addTag".into(), json!({"glyphs": three, "tag": "new"})));
    assert_eq!(info(&mut app, "A")["tags"], json!(["new", "x"]), "a tag is added, not a set replaced");
    assert_eq!(info(&mut app, "C")["tags"], json!(["new"]));

    w.click(&mut app, widgets::id((SALT, "export")));
    assert_eq!(last(&app), ("glyph.setExport".into(), json!({"glyphs": three, "export": true})), "mixed: the click exports all");
    assert_eq!(info(&mut app, "B")["export"], true);
}

#[test]
fn the_inspector_and_its_extra_rows_show_and_hide() {
    let mut app = FontcraftApp::default();
    app.run("fontView.setSelection", &json!({"glyphs": ["A"]})).unwrap();
    let w = Window::new();
    w.settle(&mut app);
    assert!(w.ctx.read_response(field(Column::Name)).is_some(), "shown by default");
    assert!(w.ctx.read_response(field(Column::Script)).is_none(), "the extra rows start collapsed");

    w.click(&mut app, widgets::id((SALT, "details")));
    assert!(app.state.panels.inspector_details);
    w.settle(&mut app);
    assert!(w.ctx.read_response(field(Column::Script)).is_some(), "the chevron shows the extra rows");

    assert_eq!(app.run("fontView.toggleInspector", &json!({})).unwrap(), json!({"visible": false}));
    w.settle(&mut app);
    assert!(w.ctx.read_response(field(Column::Name)).is_none(), "hidden: no fields");
    w.click(&mut app, widgets::id(("section", SALT)));
    assert!(!app.state.panels.inspector_hidden, "the title row shows it again");
}

#[test]
fn selection_info_edits_case_direction_and_the_sort_name() {
    let mut app = FontcraftApp::default();
    app.run("fontView.setSelection", &json!({"glyphs": ["A", "B", "C"]})).unwrap();
    assert_eq!(app.run("edit.infoForSelection", &json!({})).unwrap(), json!({"open": true}));
    let three = names(&["A", "B", "C"]);
    let id = |c| inspector::field_id(selection_info::SALT, c);
    let w = Window::new();
    w.settle(&mut app);
    assert!(!w.enabled(id(Column::Name)), "several glyphs cannot share one name");

    w.commit(&mut app, id(Column::Case), "minor");
    assert_eq!(last(&app), ("glyph.setCase".into(), json!({"glyphs": three, "case": "minor"})));
    w.commit(&mut app, id(Column::Direction), "RTL");
    assert_eq!(last(&app), ("glyph.setDirection".into(), json!({"glyphs": three, "direction": "RTL"})));
    assert_eq!(info(&mut app, "C")["properties"]["direction"], json!({"value": "RTL", "source": "stored"}));

    assert!(!w.enabled(id(Column::SortName)), "the sort name waits for its check box");
    w.click(&mut app, selection_info::sort_name_box());
    w.settle(&mut app);
    assert!(w.enabled(id(Column::SortName)));
    w.commit(&mut app, id(Column::SortName), "zzz");
    assert_eq!(last(&app), ("glyph.setSortName".into(), json!({"glyphs": three, "sortName": "zzz"})));
    assert_eq!(info(&mut app, "B")["properties"]["sortName"], json!({"value": "zzz", "source": "stored"}));
    w.settle(&mut app);
    w.click(&mut app, selection_info::sort_name_box());
    assert_eq!(last(&app), ("glyph.setSortName".into(), json!({"glyphs": three, "sortName": null})));

    app.run("view.editSelectedGlyphs", &json!({"glyphs": ["D"]})).unwrap();
    assert_eq!(selection_info::targets(&app), ["D"], "in Edit View: the edited glyph");
    w.settle(&mut app);
    assert!(w.enabled(id(Column::Name)));
    w.commit(&mut app, id(Column::Script), "greek");
    assert_eq!(last(&app), ("glyph.setScript".into(), json!({"glyphs": ["D"], "script": "greek"})));

    assert_eq!(app.run("edit.infoForSelection", &json!({"open": false})).unwrap(), json!({"open": false}));
    w.settle(&mut app);
    assert!(w.ctx.read_response(id(Column::Case)).is_none(), "closed");
}

#[test]
fn the_glyph_context_menu_colours_tags_and_copies_names() {
    let mut app = FontcraftApp::default();
    app.run("glyph.addTag", &json!({"glyphs": ["A", "B"], "tag": "round"})).unwrap();
    app.run("glyph.addTag", &json!({"glyphs": ["A"], "tag": "x"})).unwrap();
    app.run("fontView.setSelection", &json!({"glyphs": ["A"]})).unwrap();
    let w = Window::new();
    w.settle(&mut app);

    let at = cell(&app, "C");
    w.right_click(&mut app, at);
    assert_eq!(app.state.selected_glyphs, ["C"], "a glyph that is not selected becomes the selection");
    let menu = w.glyph_menu().expect("the menu is open");
    assert_eq!(menu.glyphs, ["C"]);
    assert_eq!(menu.tags, [("round".to_string(), TagMark::None), ("x".to_string(), TagMark::None)], "every tag of the font");
    w.settle(&mut app);
    w.click(&mut app, widgets::id((context_menu::GLYPH_MENU_SALT, "color", 3usize)));
    assert_eq!(last(&app), ("glyph.setColorLabel".into(), json!({"glyphs": ["C"], "color": 3})));
    assert_eq!(info(&mut app, "C")["color"], 3);
    assert!(w.glyph_menu().is_none(), "picking a swatch closes the menu");

    let three = names(&["A", "B", "C"]);
    app.run("fontView.setSelection", &json!({"glyphs": three})).unwrap();
    w.settle(&mut app);
    let at = cell(&app, "B");
    w.right_click(&mut app, at);
    assert_eq!(app.state.selected_glyphs, three, "on a selected glyph the selection stays");
    let menu = w.glyph_menu().expect("the menu is open");
    assert_eq!(menu.glyphs, three);
    assert_eq!(menu.tags, [("round".to_string(), TagMark::Some), ("x".to_string(), TagMark::Some)], "a dash: only some carry it");

    context_menu::invoke_glyph(&mut app, &menu, &GlyphAction::Tag("round".into())).unwrap();
    assert_eq!(last(&app), ("glyph.addTag".into(), json!({"glyphs": three, "tag": "round"})));
    let menu = context_menu::glyph_menu(&app, three.clone(), [0.0, 0.0]);
    assert_eq!(menu.tags.first(), Some(&("round".to_string(), TagMark::All)), "a checkmark: all carry it");

    context_menu::invoke_glyph(&mut app, &menu, &GlyphAction::CopyNames("slashed")).unwrap();
    assert_eq!(app.clipboard_out.as_deref(), Some("/A/B/C"));
    context_menu::invoke_glyph(&mut app, &menu, &GlyphAction::CopyNames("unicodes")).unwrap();
    assert_eq!(app.clipboard_out.as_deref(), Some("0041\n0042\n0043"));

    let master = app.session.font().unwrap().masters[0].id.clone();
    context_menu::invoke_glyph(&mut app, &menu, &GlyphAction::LayerColor(Some(5))).unwrap();
    assert_eq!(last(&app), ("glyph.setLayerColorLabel".into(), json!({"glyphs": three, "layer": master, "color": 5})), "Alt: the layer colour");
    context_menu::invoke_glyph(&mut app, &menu, &GlyphAction::Color(None)).unwrap();
    assert_eq!(last(&app), ("glyph.setColorLabel".into(), json!({"glyphs": three, "color": null})), "the cross clears the label");
    assert_eq!(info(&mut app, "C")["color"], Value::Null);

    let fresh = FontcraftApp::default();
    assert!(context_menu::glyph_menu(&fresh, names(&["A"]), [0.0, 0.0]).tags.is_empty(), "no tags in the font: no Tags submenu");
}

/// Glyph A edited at actual size, nothing selected.
fn edit_app() -> FontcraftApp {
    let mut app = FontcraftApp::default();
    app.run("view.editSelectedGlyphs", &json!({"glyphs": ["A"]})).unwrap();
    app.run("view.setZoom", &json!({"size": 1000, "center": [200, 200]})).unwrap();
    app
}

#[test]
fn the_canvas_menu_colours_and_exports_the_edited_glyph() {
    let mut app = edit_app();
    let w = Window::new();
    w.settle(&mut app);
    let p = tools_glue::current_view(&app).unwrap().to_screen(Point::new(200.0, 200.0));
    w.right_click(&mut app, Pos2::new(p.x as f32, p.y as f32));
    let menu = app.tools.context_menu.clone().expect("a context menu is open");
    let export = menu.item("glyph.setExport").expect("Exports is a glyph item");
    assert_eq!((export.enabled, export.checked), (true, Some(true)));
    assert!(menu.item(context_menu::SHOW_IN_FONT_VIEW).is_some());
    w.settle(&mut app);
    w.click(&mut app, widgets::id((context_menu::CANVAS_SALT, "color", 1usize)));
    assert_eq!(last(&app), ("glyph.setColorLabel".into(), json!({"glyphs": ["A"], "color": 1})));
    assert!(app.tools.context_menu.is_none(), "picking a swatch closes the menu");

    context_menu::open(&mut app, Point::new(200.0, 200.0), tools_glue::Space::Font).unwrap();
    context_menu::invoke(&mut app, "glyph.setExport").unwrap();
    assert_eq!(last(&app), ("glyph.setExport".into(), json!({"glyphs": ["A"], "export": false})));

    context_menu::open(&mut app, Point::new(200.0, 200.0), tools_glue::Space::Font).unwrap();
    context_menu::invoke(&mut app, context_menu::SHOW_IN_FONT_VIEW).unwrap();
    assert!(app.state.tab().is_none(), "Font View shows");
    assert_eq!(app.state.selected_glyphs, ["A"]);
}

#[test]
fn the_info_box_shows_the_glyph_and_layer_colours_side_by_side() {
    let mut app = FontcraftApp::default();
    app.run("path.addRectangle", &json!({"glyph": "A", "x": 100, "y": 100, "width": 200, "height": 200})).unwrap();
    let master = app.session.font().unwrap().masters[0].id.clone();
    app.run("glyph.setColorLabel", &json!({"glyphs": ["A"], "color": 3})).unwrap();
    app.run("glyph.setLayerColorLabel", &json!({"glyphs": ["A"], "layer": master, "color": 7})).unwrap();
    app.run("view.editSelectedGlyphs", &json!({"glyphs": ["A"]})).unwrap();
    app.run("selection.set", &json!({"points": [[0, 0], [0, 1], [0, 2], [0, 3]]})).unwrap();
    let w = Window::new();
    // The Info box fades in over several frames; its colours are exact once it has.
    for _ in 0..4 {
        w.settle(&mut app);
    }
    let shapes = w.frame(&mut app, Vec::new());
    let filled = |color| -> Vec<egui::Rect> {
        shapes
            .iter()
            .filter_map(|s| match s {
                Shape::Rect(r) if r.fill == color => Some(r.rect),
                _ => None,
            })
            .collect()
    };
    let (glyph, layer) = (filled(app.tokens.color_labels[3]), filled(app.tokens.color_labels[7]));
    let split = glyph.iter().any(|g| layer.iter().any(|l| g.max.x == l.min.x && g.min.y == l.min.y && g.width() == l.width()));
    assert!(split, "the glyph colour on the left half, the layer colour on the right: {glyph:?} {layer:?}");
}
