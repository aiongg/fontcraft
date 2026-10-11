//! Font View: the grid and its cells, List View, the status area, the view commands and the
//! batch targets the selection gives.

use std::sync::Arc;

use egui::{Color32, Event, Key, Modifiers, PointerButton, Pos2, Rect, Shape};
use fontcraft_model::{Layer, glyph_props};
use serde_json::{Value, json};

use crate::shell::font_view::columns::Column;
use crate::shell::font_view::grid::{UnicodeMark, unicode_mark};
use crate::shell::font_view::{InlineEdit, list};
use crate::theme::Tokens;
use crate::{FontcraftApp, tools_glue};

/// A headless window: one egui context kept across frames, so pointer and focus state carry over.
struct Window {
    ctx: egui::Context,
}

impl Window {
    fn new() -> Self {
        Self { ctx: egui::Context::default() }
    }

    /// One frame with these events and modifiers; the shapes it drew, nested lists flattened.
    fn frame_with(&self, app: &mut FontcraftApp, events: Vec<Event>, modifiers: Modifiers) -> Vec<Shape> {
        let mut events = events;
        events.insert(0, Event::ModifiersChanged(modifiers));
        let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1280.0, 800.0))), events, ..Default::default() };
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

    fn frame(&self, app: &mut FontcraftApp) -> Vec<Shape> {
        self.frame_with(app, Vec::new(), Modifiers::NONE)
    }

    fn click(&self, app: &mut FontcraftApp, at: Pos2, mods: Modifiers) {
        self.frame_with(app, vec![Event::PointerMoved(at), button(at, true, mods)], mods);
        self.frame_with(app, vec![button(at, false, mods)], mods);
        self.frame(app);
    }

    fn drag(&self, app: &mut FontcraftApp, from: Pos2, to: Pos2) {
        self.frame_with(app, vec![Event::PointerMoved(from), button(from, true, Modifiers::NONE)], Modifiers::NONE);
        let mid = from + (to - from) / 2.0;
        self.frame_with(app, vec![Event::PointerMoved(mid)], Modifiers::NONE);
        self.frame_with(app, vec![Event::PointerMoved(to)], Modifiers::NONE);
        self.frame_with(app, vec![button(to, false, Modifiers::NONE)], Modifiers::NONE);
        self.frame(app);
    }

    fn type_and_enter(&self, app: &mut FontcraftApp, text: &str) {
        self.frame_with(app, vec![Event::Text(text.into())], Modifiers::NONE);
        let enter = |pressed| Event::Key { key: Key::Enter, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE };
        self.frame_with(app, vec![enter(true), enter(false)], Modifiers::NONE);
        self.frame(app);
    }
}

fn button(pos: Pos2, pressed: bool, modifiers: Modifiers) -> Event {
    Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers }
}

fn state(app: &mut FontcraftApp) -> Value {
    app.run("fontView.getState", &json!({})).unwrap()
}

fn names(v: &Value) -> Vec<String> {
    v.as_array().unwrap().iter().map(|n| n.as_str().unwrap().to_string()).collect()
}

fn sorted(mut v: Vec<String>) -> Vec<String> {
    v.sort();
    v
}

/// Where the last frame laid out the cell or row of `glyph`.
fn cell(app: &FontcraftApp, glyph: &str) -> Rect {
    app.font_view.laid_out.iter().find(|(n, _)| n == glyph).unwrap_or_else(|| panic!("{glyph} was not laid out")).1
}

/// The colours a shape is drawn in.
fn colors(shape: &Shape) -> Vec<Color32> {
    match shape {
        Shape::Rect(r) => vec![r.fill, r.stroke.color],
        Shape::Circle(c) => vec![c.fill, c.stroke.color],
        Shape::Path(p) => match p.stroke.color {
            egui::epaint::ColorMode::Solid(c) => vec![p.fill, c],
            _ => vec![p.fill],
        },
        Shape::LineSegment { stroke, .. } => vec![stroke.color],
        _ => Vec::new(),
    }
}

/// The shapes drawn in `color` that lie inside `rect`.
fn drawn_in(shapes: &[Shape], color: Color32, rect: Rect) -> Vec<Rect> {
    shapes.iter().filter(|s| colors(s).contains(&color)).map(Shape::visual_bounding_rect).filter(|b| rect.expand(1.0).contains_rect(*b)).collect()
}

/// The texts drawn inside `rect`.
fn texts_in(shapes: &[Shape], rect: Rect) -> Vec<String> {
    shapes
        .iter()
        .filter_map(|s| match s {
            Shape::Text(t) if rect.expand(1.0).contains_rect(s.visual_bounding_rect()) => Some(t.galley.text().to_string()),
            _ => None,
        })
        .collect()
}

fn tint(t: &Tokens, index: usize) -> Color32 {
    let c = t.color_labels[index];
    Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), t.color_label_alpha)
}

#[test]
fn view_commands_change_the_state_get_state_reports() {
    let mut app = FontcraftApp::default();
    let s = state(&mut app);
    assert_eq!((s["mode"].as_str(), s["cellSize"].as_f64(), s["showing"].as_bool()), (Some("grid"), Some(64.0), Some(true)));
    assert_eq!(s["counts"], json!({"selected": 0, "visible": 54, "total": 54}));
    assert_eq!(names(&s["visible"]).first().map(String::as_str), Some(".notdef"), "glyph order");

    app.run("fontView.setViewMode", &json!({"mode": "list"})).unwrap();
    assert_eq!(state(&mut app)["mode"], "list");
    assert!(app.run("fontView.setViewMode", &json!({"mode": "table"})).unwrap_err().contains("grid"));
    assert_eq!(state(&mut app)["mode"], "list", "a failed command changes nothing");

    assert_eq!(app.run("fontView.setCellSize", &json!({"size": 1e9})).unwrap(), json!({"size": 512.0}));
    assert_eq!(app.run("fontView.setCellSize", &json!({"size": 2})).unwrap(), json!({"size": 16.0}));
    assert!(app.run("fontView.setCellSize", &json!({"size": "big"})).is_err());
    assert_eq!(state(&mut app)["cellSize"], 16.0);

    // Sorting: a second request on the same column reverses it; ID ascending is glyph order.
    assert_eq!(app.run("fontView.sortByColumn", &json!({"column": "unicode"})).unwrap(), json!({"column": "unicode", "ascending": true}));
    assert_eq!(app.run("fontView.sortByColumn", &json!({"column": "unicode"})).unwrap()["ascending"], false);
    let s = state(&mut app);
    assert_eq!(s["sort"], json!({"column": "unicode", "ascending": false}));
    let visible = names(&s["visible"]);
    assert_eq!(visible[..3], ["z".to_string(), "y".into(), "x".into()]);
    assert_eq!(visible.last().map(String::as_str), Some(".notdef"), "glyphs without a value go last");
    assert_eq!(app.run("fontView.sortByColumn", &json!({"column": "id"})).unwrap()["ascending"], true);
    let s = state(&mut app);
    assert_eq!((s["sort"].clone(), names(&s["visible"]).first().cloned()), (Value::Null, Some(".notdef".to_string())));
    assert!(app.run("fontView.sortByColumn", &json!({"column": "colour"})).unwrap_err().contains("no column"));

    // Columns: reorder, then show two.
    let r = app.run("fontView.setColumns", &json!({"order": ["note", "name"]})).unwrap();
    assert_eq!(names(&r["columns"])[..3], ["note".to_string(), "name".into(), "id".into()]);
    let r = app.run("fontView.setColumns", &json!({"visible": ["unicode", "name"]})).unwrap();
    assert_eq!(names(&r["columns"]), ["name", "unicode"], "shown in the stored order");
    assert_eq!(names(&state(&mut app)["hidden"]).len(), Column::ALL.len() - 2);
    assert!(app.run("fontView.setColumns", &json!({"visible": []})).is_err());
    assert!(app.run("fontView.setColumns", &json!({"order": ["nope"]})).is_err());

    // Selection modes.
    app.run("fontView.setSelection", &json!({"glyphs": ["A", "B"]})).unwrap();
    app.run("fontView.setSelection", &json!({"glyphs": ["C", "A"], "mode": "add"})).unwrap();
    assert_eq!(names(&state(&mut app)["selected"]), ["A", "B", "C"]);
    app.run("fontView.setSelection", &json!({"glyphs": ["B"], "mode": "remove"})).unwrap();
    let s = state(&mut app);
    assert_eq!((names(&s["selected"]), s["counts"]["selected"].as_u64()), (vec!["A".to_string(), "C".into()], Some(2)));
    assert!(app.run("fontView.setSelection", &json!({"glyphs": ["A"], "mode": "toggle"})).is_err());
    assert!(app.run("fontView.setSelection", &json!({"glyphs": ["nope"]})).unwrap_err().contains("nope"));
}

#[test]
fn select_all_and_invert_work_on_the_visible_glyphs() {
    let mut app = FontcraftApp::default();
    // The sidebar's Uppercase entry, as the sidebar will set it.
    app.state.font_view.query = Some(json!({"entries": ["categories/Letter/Uppercase"]}));
    let s = state(&mut app);
    assert_eq!(s["counts"], json!({"selected": 0, "visible": 26, "total": 54}));
    assert_eq!(s["sidebarEntries"], json!(["categories/Letter/Uppercase"]));
    assert_eq!(s["sections"], json!([]), "no section headers in a filtered view");
    let r = app.run("edit.selectAll", &json!({})).unwrap();
    assert_eq!(r["glyphs"].as_array().unwrap().len(), 26);
    assert!(names(&r["glyphs"]).iter().all(|n| n.len() == 1 && n.chars().all(|c| c.is_ascii_uppercase())));
    app.run("fontView.setSelection", &json!({"glyphs": ["A", "B", "a"]})).unwrap();
    let r = app.run("edit.invertSelection", &json!({})).unwrap();
    let inverted = names(&r["glyphs"]);
    assert_eq!((inverted.len(), inverted.first().map(String::as_str)), (24, Some("C")), "among the visible glyphs only");
    app.run("edit.deselectAll", &json!({})).unwrap();
    assert_eq!(state(&mut app)["counts"]["selected"], 0);
    // A query the engine refuses lists every glyph and says why.
    app.state.font_view.query = Some(json!({"entries": ["categories/Nonsense"]}));
    let s = state(&mut app);
    assert_eq!(s["counts"]["visible"], 54);
    assert!(s["error"].is_string(), "{s}");
}

#[test]
fn cells_show_badges_colour_labels_and_unicode_markers() {
    let mut app = FontcraftApp::default();
    for name in ["A", "B", "C", "D", "E", "F"] {
        app.run("path.addRectangle", &json!({"glyph": name, "x": 100, "y": 0, "width": 300, "height": 600})).unwrap();
    }
    let window = Window::new();
    let t = app.tokens.clone();
    let before = window.frame(&mut app);
    let everywhere = Rect::EVERYTHING;
    for color in [t.badge_note, t.badge_info, t.badge_non_export, tint(&t, 0), tint(&t, 7)] {
        assert!(drawn_in(&before, color, everywhere).is_empty(), "{color:?} before any glyph has the property");
    }

    app.run("glyph.setColorLabel", &json!({"glyphs": ["A"], "color": 0})).unwrap();
    app.run("glyph.setLayerColorLabel", &json!({"glyphs": ["B"], "color": 7})).unwrap();
    app.run("glyph.setColorLabel", &json!({"glyphs": ["C"], "color": 3})).unwrap();
    app.run("glyph.setLayerColorLabel", &json!({"glyphs": ["C"], "color": 4})).unwrap();
    app.run("annotation.add", &json!({"glyph": "D", "type": "plus", "position": [0, 0]})).unwrap();
    app.run("glyph.setCategory", &json!({"glyphs": ["E"], "category": "Symbol"})).unwrap();
    app.run("glyph.setExport", &json!({"glyphs": ["F"], "export": false})).unwrap();
    let shapes = window.frame(&mut app);
    let (a, b, c, d, e, f) = (cell(&app, "A"), cell(&app, "B"), cell(&app, "C"), cell(&app, "D"), cell(&app, "E"), cell(&app, "F"));

    // Glyph colour only: the whole cell.
    assert_eq!(drawn_in(&shapes, tint(&t, 0), a), vec![a]);
    // Layer colour only: a five-sided shape over the cell, cut at the top left.
    let layer_only: Vec<&Shape> = shapes.iter().filter(|s| colors(s).contains(&tint(&t, 7))).collect();
    assert!(matches!(layer_only.as_slice(), [Shape::Path(p)] if p.points.len() == 5 && b.contains(p.points[0]) && p.points[0].x > b.min.x));
    // Both: the glyph colour fills the cell, the layer colour its right half.
    assert_eq!(drawn_in(&shapes, tint(&t, 3), c), vec![c]);
    let half = drawn_in(&shapes, tint(&t, 4), c);
    assert!(matches!(half.as_slice(), [h] if (h.min.x - c.center().x).abs() < 0.5 && h.max.x == c.max.x), "{half:?}");

    // Badges: annotation top left, custom info bottom left, not exported top centre.
    let note = drawn_in(&shapes, t.badge_note, d);
    assert!(matches!(note.as_slice(), [r] if r.center().x < d.center().x && r.center().y < d.center().y), "{note:?}");
    let info = drawn_in(&shapes, t.badge_info, e);
    assert!(matches!(info.as_slice(), [r] if r.center().x < e.center().x && r.center().y > e.min.y + e.width() / 2.0), "{info:?}");
    let ring = drawn_in(&shapes, t.badge_non_export, f);
    assert!(!ring.is_empty() && ring.iter().all(|r| (r.center().x - f.center().x).abs() < 1.0 && r.center().y < f.center().y), "{ring:?}");
    for (color, only) in [(t.badge_note, d), (t.badge_info, e), (t.badge_non_export, f)] {
        assert_eq!(drawn_in(&shapes, color, everywhere).len(), drawn_in(&shapes, color, only).len(), "{color:?} only in its glyph's cell");
    }

    // Names and the Unicode marker; the code point with Display Unicode Value on.
    assert!(texts_in(&shapes, a).contains(&"A".to_string()));
    assert_eq!(drawn_in(&shapes, t.unicode_marker, a).len(), 1, "an encoded glyph has the marker");
    assert!(drawn_in(&shapes, t.unicode_marker, cell(&app, ".notdef")).is_empty(), "an unencoded one has none");
    app.run("settings.set", &json!({"id": "appearance.displayUnicodeValue", "value": true})).unwrap();
    let shapes = window.frame(&mut app);
    assert!(texts_in(&shapes, cell(&app, "A")).contains(&"0041".to_string()));
    // CJK glyphs show their character when the interface font has it.
    let mut han = fontcraft_model::Glyph::new("uni4E00");
    han.unicodes = vec![0x4E00];
    assert_eq!(unicode_mark(&han, false, |_| true), UnicodeMark::Char('一'));
    assert_eq!(unicode_mark(&han, false, |_| false), UnicodeMark::Marker);
    assert_eq!(unicode_mark(&han, true, |_| true), UnicodeMark::Value("4E00".into()));

    // Small cells show the outline only.
    app.run("fontView.setCellSize", &json!({"size": 40})).unwrap();
    let shapes = window.frame(&mut app);
    assert!(texts_in(&shapes, cell(&app, "A")).is_empty(), "no name under a small cell");
    assert_eq!(drawn_in(&shapes, tint(&t, 0), cell(&app, "A")).len(), 1, "colours still show");
}

#[test]
fn grid_clicks_drags_and_renames_go_through_commands() {
    let mut app = FontcraftApp::default();
    let window = Window::new();
    window.frame(&mut app);
    let selected = |app: &mut FontcraftApp| sorted(names(&state(app)["selected"]));

    let at = cell(&app, "A").center();
    window.click(&mut app, at, Modifiers::NONE);
    assert_eq!(selected(&mut app), ["A"]);
    let at = cell(&app, "C").center();
    window.click(&mut app, at, Modifiers::COMMAND);
    assert_eq!(selected(&mut app), ["A", "C"]);
    let at = cell(&app, "E").center();
    window.click(&mut app, at, Modifiers::SHIFT);
    assert_eq!(selected(&mut app), ["A", "C", "D", "E"], "Shift adds the range from the last clicked glyph");
    let at = cell(&app, "C").center();
    window.click(&mut app, at, Modifiers::COMMAND);
    assert_eq!(selected(&mut app), ["A", "D", "E"], "Cmd+click removes a selected glyph");

    // Empty space below the last cell clears the selection.
    let last = app.font_view.laid_out.last().unwrap().1;
    window.click(&mut app, last.center() + egui::vec2(0.0, last.height() * 1.5), Modifiers::NONE);
    assert_eq!(selected(&mut app), Vec::<String>::new());

    let (from, to) = (cell(&app, "A").center(), cell(&app, "C").center());
    window.drag(&mut app, from, to);
    assert_eq!(selected(&mut app), ["A", "B", "C"], "a marquee selects the cells it touches");

    // A click on the name of the only selected glyph renames it in place.
    let at = cell(&app, "B").center();
    window.click(&mut app, at, Modifiers::NONE);
    let b = cell(&app, "B");
    window.click(&mut app, egui::pos2(b.center().x, b.max.y - 3.0), Modifiers::NONE);
    assert!(matches!(&app.font_view.edit, Some(InlineEdit { glyph, column: Column::Name, .. }) if glyph == "B"), "{:?}", app.font_view.edit);
    window.type_and_enter(&mut app, "ee");
    assert!(app.session.font().unwrap().glyph("Bee").is_some(), "{}", app.state.status);
    assert!(app.font_view.edit.is_none());
    assert_eq!(app.state.selected_glyphs, ["Bee"], "the renamed glyph stays selected");

    // A double-click opens the glyph (after a pause, so the clicks before do not make it a
    // triple-click).
    for _ in 0..40 {
        window.frame(&mut app);
    }
    let a = cell(&app, "A").center();
    window.frame_with(&mut app, vec![Event::PointerMoved(a), button(a, true, Modifiers::NONE)], Modifiers::NONE);
    for pressed in [false, true, false] {
        window.frame_with(&mut app, vec![button(a, pressed, Modifiers::NONE)], Modifiers::NONE);
    }
    window.frame(&mut app);
    assert_eq!(app.state.editing(), Some("A"));
}

#[test]
fn list_view_sorts_shows_columns_and_edits_in_place() {
    let mut app = FontcraftApp::default();
    let window = Window::new();
    app.run("fontView.setViewMode", &json!({"mode": "list"})).unwrap();
    app.run("glyph.setNote", &json!({"glyphs": ["b"], "note": "round"})).unwrap();
    app.run("fontView.sortByColumn", &json!({"column": "unicode", "ascending": false})).unwrap();
    let shapes = window.frame(&mut app);
    let rows: Vec<&str> = app.font_view.laid_out.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(rows[..3], ["z", "y", "x"], "rows in the sorted order");
    let header = Rect::from_min_max(Pos2::ZERO, egui::pos2(1280.0, cell(&app, "z").min.y));
    let headers = texts_in(&shapes, header);
    for c in [Column::Id, Column::Char, Column::Name, Column::Unicode, Column::Category] {
        assert!(headers.contains(&c.label().to_string()), "{} in {headers:?}", c.label());
    }
    let b = texts_in(&shapes, cell(&app, "b"));
    for value in ["b", "0062", "Letter", "lower", "round", "latin"] {
        assert!(b.contains(&value.to_string()), "{value} in {b:?}");
    }

    // Clicking an Exports cell flips the flag. The column lies under the Palette at this width.
    app.run("window.palette", &json!({"on": false})).unwrap();
    window.frame(&mut app);
    let columns = list::placed(&app.state.font_view.shown_columns(), &app.tokens);
    let (_, x, w) = *columns.iter().find(|c| c.0 == Column::Export).unwrap();
    let row = cell(&app, "c");
    window.click(&mut app, egui::pos2(row.min.x + x + w / 2.0, row.center().y), Modifiers::NONE);
    assert!(!app.session.font().unwrap().glyph("c").unwrap().export);

    // An inline edit of a Note cell runs glyph.setNote.
    app.font_view.edit = Some(InlineEdit { glyph: "d".into(), column: Column::Note, text: String::new(), fresh: true });
    window.frame(&mut app);
    window.type_and_enter(&mut app, "stem");
    assert_eq!(app.session.font().unwrap().glyph("d").unwrap().info.note, "stem", "{}", app.state.status);

    // Hidden columns are not drawn.
    app.run("fontView.setColumns", &json!({"visible": ["name"]})).unwrap();
    let shapes = window.frame(&mut app);
    let header = Rect::from_min_max(Pos2::ZERO, egui::pos2(1280.0, cell(&app, "z").min.y));
    assert!(!texts_in(&shapes, header).contains(&"Unicode".to_string()));
}

#[test]
fn list_view_char_cells_draw_the_outline_when_the_interface_font_lacks_the_character() {
    let mut app = FontcraftApp::default();
    let window = Window::new();
    for name in ["A", "B"] {
        app.run("path.addRectangle", &json!({"glyph": name, "x": 100, "y": 0, "width": 300, "height": 600})).unwrap();
    }
    // U+1E00 is outside the interface font.
    app.run("glyph.setUnicodes", &json!({"glyphs": ["A"], "unicodes": ["1E00"]})).unwrap();
    app.run("fontView.setViewMode", &json!({"mode": "list"})).unwrap();
    let shapes = window.frame(&mut app);
    let columns = list::placed(&app.state.font_view.shown_columns(), &app.tokens);
    let (_, x, w) = *columns.iter().find(|c| c.0 == Column::Char).unwrap();
    let char_cell = |app: &FontcraftApp, glyph: &str| {
        let row = cell(app, glyph);
        Rect::from_min_size(egui::pos2(row.min.x + x, row.min.y), egui::vec2(w, row.height()))
    };
    let images_in = |rect: Rect| {
        shapes
            .iter()
            .filter(|s| {
                matches!(s, Shape::Mesh(m) if m.texture_id != egui::TextureId::default()) && rect.expand(1.0).contains_rect(s.visual_bounding_rect())
            })
            .count()
    };
    let (a, b) = (char_cell(&app, "A"), char_cell(&app, "B"));
    assert_eq!(images_in(a), 1, "the outline of A stands in for U+1E00");
    assert!(texts_in(&shapes, a).is_empty());
    assert_eq!(texts_in(&shapes, b), ["B"]);
    assert_eq!(images_in(b), 0);
}

#[test]
fn ten_thousand_glyphs_lay_out_only_the_rows_in_view() {
    let mut app = FontcraftApp::default();
    app.session
        .edit("many", |font| {
            for i in 0..10_000 {
                font.add_glyph(&format!("g{i:05}"), vec![], 500.0)?;
            }
            Ok(())
        })
        .unwrap();
    let window = Window::new();
    window.frame(&mut app);
    let grid = app.font_view.laid_out.len();
    assert!(grid > 20 && grid < 400, "{grid} cells laid out");
    app.run("fontView.setCellSize", &json!({"size": 16})).unwrap();
    window.frame(&mut app);
    let small = app.font_view.laid_out.len();
    assert!(small > grid && small < 4_000, "{small} small cells laid out");
    app.run("fontView.setViewMode", &json!({"mode": "list"})).unwrap();
    window.frame(&mut app);
    let rows = app.font_view.laid_out.len();
    assert!(rows > 10 && rows < 100, "{rows} rows laid out");
    assert_eq!(state(&mut app)["counts"]["total"], 10_054);
}

#[test]
fn font_view_has_its_own_master_and_batch_commands_take_its_selection() {
    let mut app = FontcraftApp::default();
    app.run("path.addRectangle", &json!({"glyph": "A", "x": 100, "y": 100, "width": 200, "height": 200})).unwrap();
    app.session
        .edit("setup", |font| {
            let mut m = font.default_master()?.clone();
            m.id = "m02".into();
            m.name = "Bold".into();
            Arc::make_mut(&mut font.masters).push(m);
            for name in ["A", "B"] {
                font.glyph_mut(name)?.layers.push(Layer::new("m02", 700.0));
            }
            Ok(())
        })
        .unwrap();
    app.run("path.addRectangle", &json!({"glyph": "B", "layer": "m02", "x": 50, "y": 50, "width": 400, "height": 300})).unwrap();
    let r = app.run("view.selectMaster", &json!({"master": "Bold"})).unwrap();
    assert_eq!((r["master"].as_str(), r["tab"].clone()), (Some("m02"), Value::Null));
    assert_eq!(state(&mut app)["master"], "m02");
    assert_eq!(app.render_master().as_deref(), Some("m02"), "ui.render draws Font View's master");

    app.run("fontView.setSelection", &json!({"glyphs": ["A", "B"]})).unwrap();
    let targets = tools_glue::with_targets(&app, "path.reverseContours", Value::Null);
    assert_eq!(targets, json!({"glyphs": ["A", "B"], "layer": "m02"}));
    let before = app.session.font().unwrap().clone();
    let undo = app.session.doc().unwrap().history.undo.len();
    app.run("path.reverseContours", &targets).unwrap();
    assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 1, "one undo step for the selection");
    let font = app.session.font().unwrap();
    assert_ne!(font.glyph("B").unwrap().layer("m02"), before.glyph("B").unwrap().layer("m02"), "Font View's master changed");
    assert_eq!(font.glyph("A").unwrap().layer("m01"), before.glyph("A").unwrap().layer("m01"), "the other master did not");
    // A command given its own targets keeps them.
    assert_eq!(tools_glue::with_targets(&app, "path.reverseContours", json!({"glyphs": ["C"]})), json!({"glyphs": ["C"]}));

    // A tab opened from Font View starts in its master, then keeps its own.
    let r = app.run("view.editSelectedGlyphs", &json!({"glyphs": ["A"]})).unwrap();
    assert_eq!(r["master"], "m02");
    let r = app.run("view.selectMaster", &json!({"master": 0})).unwrap();
    assert_eq!(r["master"], "m01");
    app.run("fontView.show", &json!({})).unwrap();
    assert_eq!(state(&mut app)["master"], "m02");
}

#[test]
fn fit_curve_in_font_view_uses_each_glyphs_last_node_selection() {
    let mut app = FontcraftApp::default();
    for name in ["O", "Q"] {
        app.run("path.addEllipse", &json!({"glyph": name, "x": 100, "y": 0, "width": 400, "height": 600})).unwrap();
        app.run("view.editSelectedGlyphs", &json!({"glyphs": [name]})).unwrap();
        app.run("edit.selectAll", &json!({})).unwrap();
    }
    app.run("fontView.show", &json!({})).unwrap();
    app.run("fontView.setSelection", &json!({"glyphs": ["O", "Q", "A"]})).unwrap();
    let targets = crate::panels::fit_curve::font_view_targets(&app);
    let glyphs: Vec<&str> = targets.iter().filter_map(|t| t["glyph"].as_str()).collect();
    assert_eq!(glyphs, ["O", "Q"], "A was never edited, so it has no nodes to fit");
    assert!(targets.iter().all(|t| t["points"].as_array().is_some_and(|p| p.len() > 4)));

    let before = app.session.font().unwrap().clone();
    let undo = app.session.doc().unwrap().history.undo.len();
    let steps: Vec<Value> =
        targets.iter().map(|t| json!({"command": "fitCurve.applyStep", "params": crate::panels::with(t.clone(), json!({"step": 8}))})).collect();
    app.run("command.batch", &json!({"label": "Fit Curve", "steps": steps})).unwrap();
    let font = app.session.font().unwrap();
    for name in ["O", "Q"] {
        assert_ne!(font.glyph(name).unwrap().layers, before.glyph(name).unwrap().layers, "{name} changed");
    }
    assert_eq!(app.session.doc().unwrap().history.undo.len(), undo + 1);
}

/// The § 5 speed measures on the corpus source with the most glyphs. Run with
/// `cargo test -p fontcraft-ui-egui measure_font_view -- --ignored --nocapture`.
#[test]
#[ignore = "needs the fetched corpus; prints timings"]
fn measure_font_view_on_the_largest_corpus_source() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../corpus/inter/src/Inter-Roman.glyphspackage");
    let mut app = FontcraftApp::default();
    app.run("file.open", &json!({"path": path})).unwrap();
    let font = app.session.font().unwrap().clone();
    let data = app.session.glyph_data().unwrap();
    let keep = fontcraft_model::keeps_alternates(&font);
    let start = std::time::Instant::now();
    let count = font.glyphs.iter().map(|g| glyph_props(g, &data, keep)).filter(|p| p.has_custom_info).count();
    let props = start.elapsed();
    let window = Window::new();
    let start = std::time::Instant::now();
    window.frame(&mut app);
    let first = start.elapsed();
    let mut frames = Vec::new();
    for _ in 0..20 {
        let start = std::time::Instant::now();
        window.frame(&mut app);
        frames.push(start.elapsed());
    }
    frames.sort();
    let first_glyph = font.glyphs.iter().next().unwrap().name.clone();
    app.run("glyph.setNote", &json!({"glyphs": [first_glyph.clone()], "note": "edited"})).unwrap();
    let start = std::time::Instant::now();
    crate::shell::font_view::refresh(&mut app);
    let refresh = start.elapsed();
    app.run("glyph.setNote", &json!({"glyphs": [first_glyph], "note": "edited again"})).unwrap();
    let start = std::time::Instant::now();
    window.frame(&mut app);
    let after_edit = start.elapsed();
    // Frames after one glyph is renamed and after one glyph's colour is set, six of each, and
    // the sidebar's refresh after an edit on its own.
    let timed_frame = |app: &mut FontcraftApp| {
        let start = std::time::Instant::now();
        window.frame(app);
        start.elapsed()
    };
    let (mut renamed, mut coloured, mut sidebar_refresh) = (Vec::new(), Vec::new(), Vec::new());
    for i in 0..6 {
        let (from, to) = if i % 2 == 0 { ("a", "a.renamed") } else { ("a.renamed", "a") };
        app.run("glyph.rename", &json!({"glyph": from, "newName": to})).unwrap();
        renamed.push(timed_frame(&mut app));
        app.run("glyph.setColorLabel", &json!({"glyphs": ["b"], "color": i})).unwrap();
        coloured.push(timed_frame(&mut app));
        app.run("glyph.setColorLabel", &json!({"glyphs": ["c"], "color": i})).unwrap();
        crate::shell::font_view::refresh(&mut app);
        let start = std::time::Instant::now();
        crate::shell::font_view::sidebar::refresh(&mut app);
        sidebar_refresh.push(start.elapsed());
        window.frame(&mut app);
    }
    // Frames after a width edit to `n`, which other glyphs' metrics keys read: the out-of-sync
    // badges are scanned again for the glyphs that read it.
    let n_width = app.run("spacing.getMetrics", &json!({"glyph": "n"})).unwrap()["width"].as_f64().unwrap();
    let mut spaced = Vec::new();
    for i in 0..6 {
        app.run("spacing.setWidth", &json!({"glyphs": ["n"], "value": n_width + f64::from(i % 2) * 10.0})).unwrap();
        spaced.push(timed_frame(&mut app));
    }
    let spread = |mut v: Vec<std::time::Duration>| {
        v.sort();
        format!("median {:?}, max {:?}", v[v.len() / 2], v.last().unwrap())
    };
    app.run("fontView.setViewMode", &json!({"mode": "list"})).unwrap();
    window.frame(&mut app);
    let start = std::time::Instant::now();
    window.frame(&mut app);
    let list = start.elapsed();
    // A three-rule smart filter: lowercase letters whose names have no suffix.
    let rules = json!({"all": [
        {"property": "category", "value": "Letter"},
        {"property": "case", "value": "lower"},
        {"property": "glyphName", "op": "doesNotContain", "value": "."},
    ]});
    let mut queries = Vec::new();
    let mut shown = 0;
    for _ in 0..10 {
        let start = std::time::Instant::now();
        let r = app.run("fontView.queryGlyphs", &json!({"rules": rules})).unwrap();
        queries.push(start.elapsed());
        shown = r["count"].as_u64().unwrap();
    }
    let first_query = queries[0];
    let start = std::time::Instant::now();
    app.run("fontView.queryGlyphs", &json!({"sidebar": true})).unwrap();
    let sidebar = start.elapsed();
    queries.sort();
    println!(
        "{} glyphs ({count} custom): effective properties {props:?}; first frame {first:?}; later frames median {:?}, max {:?}; cache refresh after an edit {refresh:?}, frame after an edit {after_edit:?}; frame after a rename {}; frame after a colour {}; frame after a width edit {}; sidebar refresh after an edit {}; list frame {list:?}; three-rule query ({shown} glyphs) first {first_query:?}, median {:?}, max {:?}; sidebar entries with counts {sidebar:?}",
        font.glyphs.len(),
        frames[frames.len() / 2],
        frames.last().unwrap(),
        spread(renamed),
        spread(coloured),
        spread(spaced),
        spread(sidebar_refresh),
        queries[queries.len() / 2],
        queries.last().unwrap()
    );
}

#[test]
fn glyph_order_section_headers_head_their_glyphs_in_the_grid() {
    use fontcraft_model::GLYPH_ORDER_PARAM;
    use fontcraft_model::plist::{Array, Str, Value as Plist};
    let mut app = FontcraftApp::default();
    app.session
        .edit("order", |f| {
            let items = ["**Capitals**", "A", "B", "**Small**", "a", "b"].iter().map(|n| Plist::String(Str::new(*n))).collect();
            Arc::make_mut(&mut f.info).params.set(GLYPH_ORDER_PARAM, Plist::Array(Array { items, ..Default::default() }));
            Ok(())
        })
        .unwrap();
    let s = state(&mut app);
    assert_eq!(s["sections"], json!([{"at": 0, "title": "Capitals"}, {"at": 2, "title": "Small"}]));
    assert_eq!(names(&s["visible"])[..5], ["A".to_string(), "B".into(), "a".into(), "b".into(), ".notdef".into()]);
    let window = Window::new();
    let shapes = window.frame(&mut app);
    let (a, b, small_a) = (cell(&app, "A"), cell(&app, "B"), cell(&app, "a"));
    assert_eq!(a.min.y, b.min.y);
    assert!(small_a.min.y > b.max.y, "a new section starts a new line");
    let header = Rect::from_min_max(egui::pos2(0.0, b.max.y), egui::pos2(1280.0, small_a.min.y));
    assert!(texts_in(&shapes, header).contains(&"Small".to_string()));
    // Sorted, the view has no sections.
    app.run("fontView.sortByColumn", &json!({"column": "name"})).unwrap();
    assert_eq!(state(&mut app)["sections"], json!([]));
}
