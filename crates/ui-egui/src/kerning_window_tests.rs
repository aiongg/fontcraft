//! The Kerning window: the list per master and direction, the search options, sorting, the
//! selection driving Edit View, Up and Down, Show All Glyphs, the entry colours, in-place
//! editing, copy and paste between masters, deletion, and laying out only the rows in view.

use std::sync::Arc;

use egui::{Event, Key, Modifiers, OutputCommand, PointerButton, Pos2, Shape};
use fontcraft_model::Layer;
use serde_json::{Value, json};

use crate::FontcraftApp;
use crate::shell::kerning_window::{Column, Editing, edit_command, header_id, row_id};

/// A headless window: one egui context kept across frames.
struct Window {
    ctx: egui::Context,
}

impl Window {
    fn new() -> Self {
        Self { ctx: egui::Context::default() }
    }

    fn frame(&self, app: &mut FontcraftApp, events: Vec<Event>) -> egui::FullOutput {
        let input =
            egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1280.0, 800.0))), events, ..Default::default() };
        let mut output = self.ctx.run_ui(input, |ui| app.ui(ui));
        output.textures_delta.clear();
        output
    }

    fn shapes(&self, app: &mut FontcraftApp) -> Vec<Shape> {
        let output = self.frame(app, Vec::new());
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

    fn click(&self, app: &mut FontcraftApp, at: Pos2, modifiers: Modifiers) {
        // Backends report held modifiers as their own event; the input state keeps them.
        self.frame(app, vec![Event::ModifiersChanged(modifiers), Event::PointerMoved(at)]);
        self.frame(app, vec![Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers }]);
        self.frame(app, vec![Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers }]);
        self.frame(app, vec![Event::ModifiersChanged(Modifiers::NONE)]);
    }

    fn key(&self, app: &mut FontcraftApp, key: Key, modifiers: Modifiers) {
        self.frame(app, vec![Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }]);
    }
}

/// T and V in the right group T, e and o in the left group o; five left-to-right pairs and one
/// right-to-left pair in the first master.
fn kern_app() -> FontcraftApp {
    let mut app = FontcraftApp::default();
    app.run("kerning.setRightGroup", &json!({"glyphs": ["T", "V"], "group": "T"})).unwrap();
    app.run("kerning.setLeftGroup", &json!({"glyphs": ["e", "o"], "group": "o"})).unwrap();
    for (left, right, value) in [("@T", "@o", -80), ("A", "V", -100), ("V", "A", 40), ("L", "T", -60), ("T", "a", -50)] {
        app.run("kerning.setPair", &json!({"left": left, "right": right, "value": value})).unwrap();
    }
    app.run("kerning.setPair", &json!({"left": "A", "right": "A", "value": 10, "direction": "rtl"})).unwrap();
    app
}

/// A second master `m02` with a layer for every glyph.
fn add_master(app: &mut FontcraftApp) {
    app.session
        .edit("setup", |font| {
            let mut m = font.default_master()?.clone();
            m.id = "m02".into();
            m.name = "Bold".into();
            Arc::make_mut(&mut font.masters).push(m);
            let names: Vec<String> = font.glyphs.iter().map(|g| g.name.clone()).collect();
            for name in names {
                let mut g = font.glyph_mut(&name)?;
                g.layers.push(Layer::new("m02", 600.0));
            }
            Ok(())
        })
        .unwrap();
}

fn pairs(reply: &Value) -> Vec<(String, String)> {
    reply["selected"].as_array().unwrap().iter().map(|p| (p["left"].as_str().unwrap().into(), p["right"].as_str().unwrap().into())).collect()
}

fn rows(app: &FontcraftApp) -> Vec<(String, String, f64)> {
    app.state.panels.kerning_window.listing.rows.iter().map(|r| (r.left.clone(), r.right.clone(), r.value)).collect()
}

fn items(app: &mut FontcraftApp) -> Vec<Value> {
    app.run("editView.getText", &json!({})).unwrap()["items"].as_array().unwrap().clone()
}

fn value(app: &mut FontcraftApp, left: &str, right: &str, master: Option<&str>) -> Value {
    let mut p = json!({"left": left, "right": right});
    if let Some(m) = master {
        p["master"] = json!(m);
    }
    app.run("kerning.getPair", &p).unwrap()["value"].clone()
}

fn row_rect(app: &FontcraftApp, left: &str, right: &str) -> egui::Rect {
    let id = row_id(left, right);
    app.state.panels.kerning_window.laid_out.iter().find(|(r, _)| *r == id).map(|(_, rect)| *rect).unwrap_or_else(|| panic!("{id:?} not laid out"))
}

#[test]
fn window_kerning_lists_the_pairs_of_the_current_master_and_direction() {
    let mut app = kern_app();
    let shown = app.run("window.kerning", &json!({})).unwrap();
    assert_eq!((shown["open"].clone(), shown["count"].clone(), shown["direction"].clone()), (json!(true), json!(5), json!("ltr")));
    assert_eq!(rows(&app).first(), Some(&("@T".to_string(), "@o".to_string(), -80.0)), "stored order before a sort");

    // The showing tab's direction picks the list; Font View keeps the last one shown.
    app.run("view.newTab", &json!({"glyphs": ["H"]})).unwrap();
    app.run("editView.setWritingDirection", &json!({"direction": "rtl"})).unwrap();
    let shown = app.run("window.kerning", &json!({})).unwrap();
    assert_eq!((shown["count"].clone(), shown["direction"].clone()), (json!(1), json!("rtl")));
    app.run("view.showTab", &json!({"index": 1})).unwrap();
    assert_eq!(app.run("window.kerning", &json!({})).unwrap()["direction"], "rtl");

    // Another master lists its own pairs: none yet.
    add_master(&mut app);
    app.run("view.showTab", &json!({"index": 2})).unwrap();
    app.run("editView.setWritingDirection", &json!({"direction": "ltr"})).unwrap();
    app.run("view.selectMaster", &json!({"master": "m02"})).unwrap();
    let shown = app.run("window.kerning", &json!({})).unwrap();
    assert_eq!((shown["master"].clone(), shown["count"].clone()), (json!("m02"), json!(0)));

    // A frame draws it; closing hides it.
    let w = Window::new();
    w.frame(&mut app, Vec::new());
    assert!(w.ctx.read_response(header_id(Column::Value)).is_some(), "the window is drawn");
    assert_eq!(app.run("window.kerning", &json!({"open": false})).unwrap()["open"], false);
}

#[test]
fn kerning_filter_pairs_and_kerning_reset_filter_match_the_list_query() {
    let mut app = kern_app();
    app.run("window.kerning", &json!({})).unwrap();
    let engine_count = |app: &mut FontcraftApp, p: Value| app.run("kerning.listPairs", &p).unwrap()["count"].clone();
    for (filter, query) in [
        (json!({"term": "T"}), json!({"term": "T"})),
        (json!({"match": "is"}), json!({"term": "T", "match": "is"})),
        (json!({"kind": "group"}), json!({"term": "T", "match": "is", "kind": "group"})),
        (json!({"kind": null, "side": "right"}), json!({"term": "T", "match": "is", "side": "right"})),
        (json!({"match": "doesNotContain", "side": null}), json!({"term": "T", "match": "doesNotContain"})),
    ] {
        let shown = app.run("kerning.filterPairs", &filter).unwrap();
        assert_eq!(shown["count"], engine_count(&mut app, query.clone()), "{filter} lists what listPairs {query} lists");
    }
    // Only the pairs of the glyphs selected in Font View.
    app.state.selected_glyphs = vec!["L".into()];
    let shown = app.run("kerning.filterPairs", &json!({"term": "", "match": "contains", "selectedGlyphsOnly": true})).unwrap();
    assert_eq!(shown["count"], 1);
    // Reset keeps the term and restores the options.
    app.run("kerning.filterPairs", &json!({"term": "A", "kind": "notGroup"})).unwrap();
    let reset = app.run("kerning.resetFilter", &json!({})).unwrap();
    assert_eq!(reset["search"], json!({"term": "A", "match": "contains", "kind": null, "side": null, "selectedGlyphsOnly": false}));
    assert_eq!(reset["count"], engine_count(&mut app, json!({"term": "A"})));
    assert!(app.run("kerning.filterPairs", &json!({"match": "near"})).is_err());
}

#[test]
fn kerning_sort_pairs_orders_by_a_column_and_a_second_header_click_reverses_it() {
    let mut app = kern_app();
    app.run("window.kerning", &json!({})).unwrap();
    app.run("kerning.sortPairs", &json!({"column": "value"})).unwrap();
    let values: Vec<f64> = rows(&app).iter().map(|r| r.2).collect();
    assert_eq!(values, vec![-100.0, -80.0, -60.0, -50.0, 40.0]);
    app.run("kerning.sortPairs", &json!({"column": "left", "descending": true})).unwrap();
    assert_eq!(rows(&app).first().map(|r| r.0.clone()), Some("V".into()));

    let w = Window::new();
    w.frame(&mut app, Vec::new());
    let at = w.ctx.read_response(header_id(Column::Value)).unwrap().rect.center();
    w.click(&mut app, at, Modifiers::NONE);
    assert_eq!(app.state.panels.kerning_window.sort, Some((Column::Value, false)));
    w.click(&mut app, at, Modifiers::NONE);
    assert_eq!(app.state.panels.kerning_window.sort, Some((Column::Value, true)));
    assert_eq!(rows(&app).first().map(|r| r.2), Some(40.0));
}

#[test]
fn kerning_show_pair_in_edit_view_replaces_the_pair_at_the_current_glyph() {
    let mut app = kern_app();
    // From Font View: a new tab with the pair; groups by their first member in glyph order.
    let shown = app.run("kerning.showPairInEditView", &json!({"left": "@T", "right": "@o"})).unwrap();
    assert_eq!(shown["glyphs"], json!(["T", "e"]));
    assert_eq!((shown["items"].clone(), shown["current"].clone()), (json!(["T", "e"]), json!("e")));
    // In a tab: the current glyph and the one before it are replaced.
    app.run("editView.setText", &json!({"glyphs": ["H", "T", "e", "H"]})).unwrap();
    app.run("editView.setCursor", &json!({"index": 2})).unwrap();
    let shown = app.run("kerning.showPairInEditView", &json!({"left": "A", "right": "V"})).unwrap();
    assert_eq!((shown["items"].clone(), shown["current"].clone()), (json!(["H", "A", "V", "H"]), json!("V")));
    // No glyph before the current one: the pair goes in at the cursor.
    app.run("editView.setText", &json!({"glyphs": ["H"]})).unwrap();
    app.run("editView.setCursor", &json!({"index": 0})).unwrap();
    let shown = app.run("kerning.showPairInEditView", &json!({"left": "L", "right": "T"})).unwrap();
    assert_eq!(shown["items"], json!(["L", "T", "H"]));
    // A group no glyph has, and a missing glyph, are refused and change nothing.
    assert!(app.run("kerning.showPairInEditView", &json!({"left": "@nobody", "right": "A"})).is_err());
    assert!(app.run("kerning.showPairInEditView", &json!({"left": "A", "right": "Zz"})).is_err());
    assert_eq!(items(&mut app), vec![json!("L"), json!("T"), json!("H")]);
}

#[test]
fn clicking_a_row_selects_it_and_shows_the_pair_in_edit_view() {
    let mut app = kern_app();
    app.run("view.newTab", &json!({"glyphs": ["H"]})).unwrap();
    app.run("window.kerning", &json!({})).unwrap();
    let w = Window::new();
    w.frame(&mut app, Vec::new());
    let at = row_rect(&app, "L", "T").center();
    w.click(&mut app, at, Modifiers::NONE);
    assert_eq!(app.state.panels.kerning_window.selected, vec![row_id("L", "T")]);
    assert!(items(&mut app).windows(2).any(|p| p == [json!("L"), json!("T")]), "{:?}", items(&mut app));
    // Shift+click extends the selection; the clicked pair is shown.
    let at = row_rect(&app, "A", "V").center();
    w.click(&mut app, at, Modifiers::SHIFT);
    assert_eq!(app.state.panels.kerning_window.selected.len(), 3);
    assert!(items(&mut app).windows(2).any(|p| p == [json!("A"), json!("V")]));
}

#[test]
fn kerning_select_next_pair_and_kerning_select_previous_pair_step_the_list_and_edit_view() {
    let mut app = kern_app();
    app.run("view.newTab", &json!({"glyphs": ["H"]})).unwrap();
    app.run("window.kerning", &json!({})).unwrap();
    let next = app.run("kerning.selectNextPair", &json!({})).unwrap();
    assert_eq!(pairs(&next), vec![("@T".into(), "@o".into())]);
    assert_eq!(next["shown"]["glyphs"], json!(["T", "e"]));
    let next = app.run("kerning.selectNextPair", &json!({})).unwrap();
    assert_eq!(pairs(&next), vec![("A".into(), "V".into())]);
    let text = items(&mut app);
    assert!(text.windows(2).any(|p| p == [json!("A"), json!("V")]) && !text.contains(&json!("e")), "the pair replaced the last one: {text:?}");
    app.run("kerning.selectPreviousPair", &json!({})).unwrap();
    let first = app.run("kerning.selectPreviousPair", &json!({})).unwrap();
    assert_eq!(pairs(&first), vec![("@T".into(), "@o".into())], "at the first row it stays");

    // Down with the list focused (a row was clicked) steps too.
    let w = Window::new();
    w.frame(&mut app, Vec::new());
    let at = row_rect(&app, "V", "A").center();
    w.click(&mut app, at, Modifiers::NONE);
    w.frame(&mut app, Vec::new());
    w.key(&mut app, Key::ArrowDown, Modifiers::NONE);
    w.frame(&mut app, Vec::new());
    assert_eq!(app.state.panels.kerning_window.current, Some(row_id("L", "T")));
    assert!(items(&mut app).windows(2).any(|p| p == [json!("L"), json!("T")]));
    w.key(&mut app, Key::ArrowUp, Modifiers::NONE);
    assert_eq!(app.state.panels.kerning_window.current, Some(row_id("V", "A")));
}

#[test]
fn kerning_show_all_glyphs_expands_every_group_side() {
    let mut app = kern_app();
    app.run("view.newTab", &json!({"glyphs": ["H"]})).unwrap();
    app.run("window.kerning", &json!({"select": [{"left": "@T", "right": "@o"}]})).unwrap();
    let shown = app.run("kerning.showAllGlyphs", &json!({})).unwrap();
    assert_eq!(shown["pairs"], 4);
    let expected = json!(["T", "e", "space", "T", "o", {"lineBreak": true}, "V", "e", "space", "V", "o", "H"]);
    assert_eq!(shown["items"], expected);
    assert_eq!(shown["current"], "e");
    let explicit = app.run("kerning.showAllGlyphs", &json!({"left": "A", "right": "@o"})).unwrap();
    assert_eq!(explicit["pairs"], 2);
    assert!(app.run("kerning.showAllGlyphs", &json!({"left": "A"})).is_err());
}

#[test]
fn entries_are_coloured_by_kind_groups_with_an_at_sign() {
    let mut app = kern_app();
    app.run("window.kerning", &json!({})).unwrap();
    let w = Window::new();
    w.frame(&mut app, Vec::new());
    let shapes = w.shapes(&mut app);
    let colour = |text: &str, row: egui::Rect| {
        shapes.iter().find_map(|s| match s {
            Shape::Text(t) if t.galley.job.text == text && row.expand(1.0).contains(t.pos) => t.galley.job.sections.first().map(|s| s.format.color),
            _ => None,
        })
    };
    let (groups, lt) = (row_rect(&app, "@T", "@o"), row_rect(&app, "L", "T"));
    assert_eq!(colour("@T", groups), Some(app.tokens.kerning_group_text));
    assert_eq!(colour("@o", groups), Some(app.tokens.kerning_group_text));
    assert_eq!(colour("L", lt), Some(app.tokens.kerning_glyph_text));
    assert_eq!(colour("-60", lt), Some(app.tokens.text));
}

#[test]
fn double_clicking_a_value_edits_it_in_place_and_escape_cancels() {
    let mut app = kern_app();
    app.run("window.kerning", &json!({})).unwrap();
    let w = Window::new();
    w.frame(&mut app, Vec::new());
    let rect = row_rect(&app, "A", "V");
    // The Value column starts four column units in.
    let at = rect.left_center() + egui::vec2(app.tokens.list_column_unit * 5.0, 0.0);
    w.click(&mut app, at, Modifiers::NONE);
    w.click(&mut app, at, Modifiers::NONE);
    let editing = app.state.panels.kerning_window.editing.clone().expect("a double-click edits the cell");
    assert_eq!((editing.column, editing.text.as_str()), (Column::Value, "-100"));

    // Return confirms what the field holds.
    app.state.panels.kerning_window.editing = Some(Editing { text: "-120".into(), focused: false, ..editing.clone() });
    w.frame(&mut app, Vec::new());
    w.frame(&mut app, Vec::new());
    w.key(&mut app, Key::Enter, Modifiers::NONE);
    w.frame(&mut app, Vec::new());
    assert_eq!(value(&mut app, "A", "V", None), json!(-120.0));
    assert!(app.state.panels.kerning_window.editing.is_none());

    // Escape leaves the pair as it was.
    app.state.panels.kerning_window.editing = Some(Editing { text: "5".into(), focused: false, ..editing });
    w.frame(&mut app, Vec::new());
    w.frame(&mut app, Vec::new());
    w.key(&mut app, Key::Escape, Modifiers::NONE);
    w.frame(&mut app, Vec::new());
    assert_eq!(value(&mut app, "A", "V", None), json!(-120.0));

    // A group name opens the rename question with the pair, the side and the names.
    let kw = &app.state.panels.kerning_window;
    let rename = Editing { row: row_id("@T", "@o"), column: Column::Left, text: "Tee".into(), focused: true };
    let (id, params) = edit_command(kw, &rename).unwrap().unwrap();
    assert_eq!(id, "dialog.renameKerningGroup");
    assert_eq!(
        params,
        json!({"side": "left", "from": "T", "to": "Tee", "pair": {"left": "@T", "right": "@o"}, "master": kw.listing.master, "direction": "ltr"})
    );
    assert!(edit_command(kw, &Editing { text: "@T".into(), ..rename }).unwrap().is_none(), "the same name changes nothing");
    assert!(edit_command(kw, &Editing { row: row_id("A", "V"), column: Column::Value, text: "lots".into(), focused: true }).is_err());
}

#[test]
fn copy_and_paste_move_selected_pairs_between_masters() {
    let mut app = kern_app();
    add_master(&mut app);
    app.run("view.newTab", &json!({"glyphs": ["H"]})).unwrap();
    app.run("window.kerning", &json!({})).unwrap();
    let w = Window::new();
    w.frame(&mut app, Vec::new());
    // Select two rows and copy them as kerning-text.
    let at = row_rect(&app, "A", "V").center();
    w.click(&mut app, at, Modifiers::NONE);
    let at = row_rect(&app, "V", "A").center();
    w.click(&mut app, at, Modifiers::COMMAND);
    let output = w.frame(&mut app, vec![Event::Copy]);
    let copied: Vec<String> =
        output.platform_output.commands.iter().filter_map(|c| if let OutputCommand::CopyText(t) = c { Some(t.clone()) } else { None }).collect();
    let text = copied.first().cloned().expect("copy puts the pairs on the clipboard");
    assert!(text.contains("A\tV\t-100") && text.contains("V\tA\t40") && !text.contains("L\tT"), "{text}");

    // The other master: the list switches, and paste adds the pairs there.
    app.run("view.selectMaster", &json!({"master": "m02"})).unwrap();
    w.frame(&mut app, Vec::new());
    assert!(app.state.panels.kerning_window.listing.rows.is_empty());
    w.frame(&mut app, vec![Event::Paste(text.clone())]);
    assert_eq!(value(&mut app, "A", "V", Some("m02")), json!(-100.0));
    // The list reads the font at the start of the next frame.
    w.frame(&mut app, Vec::new());
    assert_eq!(app.state.panels.kerning_window.listing.rows.len(), 2);

    // A clash pastes nothing and asks.
    app.run("kerning.setPair", &json!({"left": "A", "right": "V", "value": -10, "master": "m02"})).unwrap();
    w.frame(&mut app, vec![Event::Paste(text)]);
    assert_eq!(value(&mut app, "A", "V", Some("m02")), json!(-10.0));

    // Select All and the minus button delete every listed pair of the master.
    w.key(&mut app, Key::A, Modifiers::COMMAND);
    assert_eq!(app.state.panels.kerning_window.selected.len(), 2);
    let at = app.state.panels.kerning_window.minus_button.unwrap().center();
    w.click(&mut app, at, Modifiers::NONE);
    w.frame(&mut app, Vec::new());
    assert!(app.state.panels.kerning_window.listing.rows.is_empty());
    assert_eq!(value(&mut app, "A", "V", None), json!(-100.0), "the first master keeps its pairs");
}

#[test]
fn a_long_pair_list_lays_out_only_the_rows_in_view() {
    let mut app = FontcraftApp::default();
    let letters: Vec<String> = ('A'..='Z').chain('a'..='z').map(String::from).collect();
    let list: Vec<Value> = letters.iter().flat_map(|l| letters.iter().map(move |r| json!({"left": l, "right": r, "value": -1}))).collect();
    let count = list.len();
    app.run("kerning.pastePairs", &json!({"pairs": list})).unwrap();
    app.run("window.kerning", &json!({})).unwrap();
    let w = Window::new();
    w.frame(&mut app, Vec::new());
    w.frame(&mut app, Vec::new());
    let kw = &app.state.panels.kerning_window;
    assert_eq!(kw.listing.rows.len(), count);
    assert!(!kw.laid_out.is_empty() && kw.laid_out.len() <= 24, "{} rows laid out", kw.laid_out.len());
    // Selecting the last row scrolls it into view.
    let last = json!({"left": "z", "right": "z"});
    app.run("window.kerning", &json!({"select": [last]})).unwrap();
    w.frame(&mut app, Vec::new());
    w.frame(&mut app, Vec::new());
    let laid = &app.state.panels.kerning_window.laid_out;
    assert!(laid.iter().any(|(id, _)| *id == row_id("z", "z")) && laid.len() <= 24);
}

/// The § 5 measure on the corpus: Inter's Regular master. Run with
/// `cargo test -p fontcraft-ui-egui kerning_window_on_inter -- --ignored --nocapture`.
#[test]
#[ignore = "needs the fetched corpus; prints timings"]
fn kerning_window_on_inter_scrolls_with_only_visible_rows_laid_out() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../corpus/inter/src/Inter-Roman.glyphspackage");
    let mut app = FontcraftApp::default();
    app.run("file.open", &json!({"path": path})).unwrap();
    let start = std::time::Instant::now();
    let shown = app.run("window.kerning", &json!({})).unwrap();
    let listed = start.elapsed();
    let count = shown["count"].as_u64().unwrap();
    assert!(count > 1000, "Inter's Regular master has {count} pairs");
    let w = Window::new();
    w.frame(&mut app, Vec::new());
    let mut frames = Vec::new();
    for _ in 0..20 {
        app.run("kerning.selectNextPair", &json!({})).unwrap();
        let start = std::time::Instant::now();
        w.frame(&mut app, Vec::new());
        frames.push(start.elapsed());
        assert!(app.state.panels.kerning_window.laid_out.len() <= 24);
    }
    frames.sort();
    println!("{count} pairs; listed in {listed:?}; median frame while stepping {:?}", frames.get(10).unwrap());
}

#[test]
fn edit_menu_copy_and_paste_reach_the_list_that_had_the_keyboard() {
    let mut app = kern_app();
    add_master(&mut app);
    app.run("view.newTab", &json!({"glyphs": ["H"]})).unwrap();
    app.run("window.kerning", &json!({})).unwrap();
    let w = Window::new();
    w.frame(&mut app, Vec::new());
    let at = row_rect(&app, "A", "V").center();
    w.click(&mut app, at, Modifiers::NONE);
    // A press on the menu bar takes the focus; Copy and Paste stay the list's.
    let bar = app.menu_bar_rect.right_center() - egui::vec2(8.0, 0.0);
    w.click(&mut app, bar, Modifiers::NONE);
    let menu = |app: &mut FontcraftApp, command: &str| {
        let r = crate::control::handle(app, &json!({"id": 1, "method": "ui.menu.invoke", "params": {"command": command}}));
        assert_eq!(r["ok"], true, "{r}");
        r["result"].clone()
    };
    assert_eq!(menu(&mut app, "edit.copy")["kerningPairs"], true);
    let text = app.clipboard_out.take().expect("the pairs are on the clipboard");
    assert!(text.contains("A\tV\t-100"), "{text}");
    app.run("view.selectMaster", &json!({"master": "m02"})).unwrap();
    w.frame(&mut app, Vec::new());
    w.frame(&mut app, vec![Event::Paste(text)]);
    assert_eq!(value(&mut app, "A", "V", Some("m02")), json!(-100.0), "the pasted text went to the list");

    // A press on the canvas gives Copy back to the Edit View.
    w.click(&mut app, Pos2::new(640.0, 600.0), Modifiers::NONE);
    let r = crate::control::handle(&mut app, &json!({"id": 1, "method": "ui.menu.invoke", "params": {"command": "edit.copy"}}));
    assert!(r["error"].as_str().is_some_and(|e| e.contains("nothing to copy")), "the empty glyph selection is copied: {r}");
}
