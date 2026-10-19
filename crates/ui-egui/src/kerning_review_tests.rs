//! Kerning review (M13.22): pair lists stepped into the tab, the representative glyph, auto-kerning
//! a list for review, the Kerning Pairs Preview (sorting, changing a value by dragging, group
//! combinations) and the kerning groups editor's stacked view.

use egui::{Event, PointerButton, Pos2};
use serde_json::{Value, json};

use crate::FontcraftApp;

/// A headless window: one egui context kept across frames.
struct Window {
    ctx: egui::Context,
}

impl Window {
    fn new() -> Self {
        Self { ctx: egui::Context::default() }
    }

    fn frame(&self, app: &mut FontcraftApp, events: Vec<Event>) {
        let input =
            egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1280.0, 800.0))), events, ..Default::default() };
        let mut output = self.ctx.run_ui(input, |ui| app.ui(ui));
        output.textures_delta.clear();
    }

    fn drag(&self, app: &mut FontcraftApp, from: Pos2, to: Pos2) {
        let button = |pos, pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        self.frame(app, vec![Event::PointerMoved(from), button(from, true)]);
        for k in 1..=4 {
            self.frame(app, vec![Event::PointerMoved(from + (to - from) * (k as f32 / 4.0))]);
        }
        self.frame(app, vec![button(to, false)]);
        self.frame(app, Vec::new());
    }
}

/// T and V in the right group T, e and o in the left group o; @T @o −80 with the exception T o −30,
/// A V −100, L T −60.
fn kern_app() -> FontcraftApp {
    let mut app = FontcraftApp::default();
    app.run("kerning.setRightGroup", &json!({"glyphs": ["T", "V"], "group": "T"})).unwrap();
    app.run("kerning.setLeftGroup", &json!({"glyphs": ["e", "o"], "group": "o"})).unwrap();
    for (left, right, value) in [("@T", "@o", -80), ("A", "V", -100), ("L", "T", -60), ("T", "o", -30)] {
        app.run("kerning.setPair", &json!({"left": left, "right": right, "value": value})).unwrap();
    }
    app
}

fn items(app: &mut FontcraftApp) -> Vec<Value> {
    app.run("editView.getText", &json!({})).unwrap()["items"].as_array().unwrap().clone()
}

fn value(app: &mut FontcraftApp, left: &str, right: &str) -> f64 {
    app.run("kerning.getPair", &json!({"left": left, "right": right})).unwrap()["value"].as_f64().unwrap_or(0.0)
}

fn sides(reply: &Value) -> Vec<(String, String)> {
    reply["rows"].as_array().unwrap().iter().map(|r| (r["left"].as_str().unwrap().into(), r["right"].as_str().unwrap().into())).collect()
}

fn pair(l: &str, r: &str) -> (String, String) {
    (l.into(), r.into())
}

#[test]
fn kerning_load_pair_list_and_step_pair_list_walk_the_rows_into_the_tab() {
    let mut app = kern_app();
    app.run("view.newTab", &json!({"glyphs": ["H", "H", "H"]})).unwrap();
    app.run("editView.setCursor", &json!({"index": 1})).unwrap();
    let loaded = app.run("kerning.loadPairList", &json!({"text": "# work\nA V\n@T @o\nno\\nHO\n", "list": "pairs"})).unwrap();
    assert_eq!((loaded["count"].clone(), loaded["source"].clone(), loaded["current"].clone()), (json!(3), json!("text"), Value::Null));
    assert_eq!(loaded["rows"][2], json!({"label": "no \\n HO", "kind": "phrase"}));

    // A pair takes the place of the pair at the current glyph; the H after it stays.
    let shown = app.run("kerning.stepPairList", &json!({"list": "pairs"})).unwrap();
    assert_eq!((shown["row"].clone(), shown["kind"].clone()), (json!(0), json!("pair")));
    assert_eq!(items(&mut app), vec![json!("A"), json!("V"), json!("H")]);
    // A group side shows its first member in glyph order.
    app.run("kerning.stepPairList", &json!({"direction": "next", "list": "pairs"})).unwrap();
    assert_eq!(items(&mut app), vec![json!("T"), json!("e"), json!("H")]);
    // A phrase replaces the text, one line at a time, before the row changes.
    let shown = app.run("kerning.stepPairList", &json!({"list": "pairs"})).unwrap();
    assert_eq!((shown["row"].clone(), shown["line"].clone(), shown["lines"].clone()), (json!(2), json!(0), json!(2)));
    assert_eq!(items(&mut app), vec![json!("n"), json!("o")]);
    app.run("kerning.stepPairList", &json!({"list": "pairs"})).unwrap();
    assert_eq!(items(&mut app), vec![json!("H"), json!("O")]);
    // At the last line of the last row, next stays; previous goes back a line.
    let shown = app.run("kerning.stepPairList", &json!({"list": "pairs"})).unwrap();
    assert_eq!((shown["row"].clone(), shown["line"].clone()), (json!(2), json!(1)));
    let shown = app.run("kerning.stepPairList", &json!({"direction": "previous", "list": "pairs"})).unwrap();
    assert_eq!((shown["row"].clone(), shown["line"].clone(), shown["selected"].clone()), (json!(2), json!(0), json!([2])));
    // An index jumps to a row; selecting changes nothing in the tab.
    app.run("kerning.stepPairList", &json!({"index": 0, "list": "pairs"})).unwrap();
    let before = items(&mut app);
    let selected = app.run("kerning.stepPairList", &json!({"select": [0, 1], "list": "pairs"})).unwrap();
    assert_eq!((selected["selected"].clone(), selected["current"].clone()), (json!([0, 1]), json!(0)));
    assert_eq!(items(&mut app), before);

    // Refusals change nothing.
    assert!(app.run("kerning.stepPairList", &json!({"index": 9, "list": "pairs"})).is_err());
    assert!(app.run("kerning.loadPairList", &json!({"text": "# nothing\n", "list": "pairs"})).is_err());
    assert!(app.run("kerning.loadPairList", &json!({"text": "A V", "reset": true})).is_err());
    let kept = app.run("kerning.loadPairList", &json!({"list": "pairs"})).unwrap();
    assert_eq!((kept["count"].clone(), kept["source"].clone()), (json!(3), json!("text")));
    // Reset brings back the built-in list.
    let reset = app.run("kerning.loadPairList", &json!({"reset": true, "list": "pairs"})).unwrap();
    assert!(reset["count"].as_u64().unwrap() >= 20 && reset["source"].is_null());
}

#[test]
fn kerning_load_pair_list_reads_a_file() {
    let mut app = kern_app();
    let path = std::env::temp_dir().join(format!("fontcraft-pairs-{}.txt", std::process::id()));
    std::fs::write(&path, "A V\nL T\n").unwrap();
    let loaded = app.run("kerning.loadPairList", &json!({"path": path.to_string_lossy(), "list": "pairs"})).unwrap();
    let _ = std::fs::remove_file(&path);
    assert_eq!((loaded["count"].clone(), loaded["source"].clone()), (json!(2), json!(path.to_string_lossy())));
    assert!(app.run("kerning.loadPairList", &json!({"path": "/no/such/pair/list.txt"})).is_err());
}

#[test]
fn kerning_set_pair_list_mode_follows_text_mode_when_auto() {
    let mut app = kern_app();
    app.run("view.newTab", &json!({"glyphs": ["H"]})).unwrap();
    let auto = app.run("kerning.setPairListMode", &json!({"mode": "auto"})).unwrap();
    let text_mode = app.state.tab().unwrap().text_mode;
    assert_eq!(auto["shown"], json!(if text_mode { "pairs" } else { "phrases" }));
    assert_eq!(app.run("kerning.setPairListMode", &json!({"mode": "phrases"})).unwrap()["shown"], "phrases");
    assert_eq!(app.run("kerning.setPairListMode", &json!({"mode": "pairs"})).unwrap()["shown"], "pairs");
    assert!(app.run("kerning.setPairListMode", &json!({"mode": "words"})).is_err());
}

#[test]
fn kerning_step_group_member_cycles_the_representative_glyph() {
    let mut app = kern_app();
    app.run("kerning.showPairInEditView", &json!({"left": "@T", "right": "@o"})).unwrap();
    assert_eq!(items(&mut app), vec![json!("T"), json!("e")]);
    // The right side steps through the left group o: e, o, then back to e.
    let stepped = app.run("kerning.stepGroupMember", &json!({"side": "right"})).unwrap();
    assert_eq!((stepped["group"].clone(), stepped["glyph"].clone(), stepped["members"].clone()), (json!("o"), json!("o"), json!(2)));
    assert_eq!(items(&mut app), vec![json!("T"), json!("o")]);
    app.run("kerning.stepGroupMember", &json!({"side": "right"})).unwrap();
    assert_eq!(items(&mut app), vec![json!("T"), json!("e")]);
    // The left side steps through the right group T backwards, wrapping round.
    let stepped = app.run("kerning.stepGroupMember", &json!({"side": "left", "direction": "previous"})).unwrap();
    assert_eq!(stepped["pair"], json!(["V", "e"]));
    assert_eq!(items(&mut app), vec![json!("V"), json!("e")]);
    // The pair stays the current one, so the next step works on it.
    assert_eq!(app.run("editView.getText", &json!({})).unwrap()["current"], "e");
    // A glyph without a group on that side is refused and changes nothing.
    app.run("editView.setText", &json!({"glyphs": ["A", "V"]})).unwrap();
    app.run("editView.setCursor", &json!({"index": 1})).unwrap();
    assert!(app.run("kerning.stepGroupMember", &json!({"side": "left"})).is_err());
    assert_eq!(items(&mut app), vec![json!("A"), json!("V")]);
}

#[test]
fn kerning_auto_kern_pair_list_proposes_for_review_then_applies() {
    let mut app = FontcraftApp::default();
    // n has 60 units each side; i 20: n i holds 80 of white against n n's 120.
    for (glyph, nodes, width) in [
        ("n", json!([{"x": 60, "y": 0}, {"x": 440, "y": 0}, {"x": 440, "y": 500}, {"x": 60, "y": 500}]), 500),
        ("i", json!([{"x": 20, "y": 0}, {"x": 80, "y": 0}, {"x": 80, "y": 500}, {"x": 20, "y": 500}]), 100),
    ] {
        app.run("path.addPath", &json!({"glyph": glyph, "nodes": nodes})).unwrap();
        app.run("spacing.setWidth", &json!({"glyphs": [glyph], "value": width})).unwrap();
    }
    app.run("kerning.loadPairList", &json!({"text": "n i\nnnoon\n", "list": "pairs"})).unwrap();
    let proposal = app.run("kerning.autoKernPairList", &json!({"list": "pairs"})).unwrap();
    assert_eq!(proposal["rows"], json!([0]), "the phrase row is left out");
    assert_eq!(proposal["pairs"][0]["value"].as_f64(), Some(40.0));
    assert_eq!(value(&mut app, "n", "i"), 0.0, "a proposal stores nothing");
    assert!(app.state.panels.kerning_review.auto_kern.is_some());
    app.run("kerning.autoKernPairList", &json!({"apply": true})).unwrap();
    assert_eq!(value(&mut app, "n", "i"), 40.0);
    assert!(app.state.panels.kerning_review.auto_kern.is_none());
    // A proposal made before the font changed is refused.
    app.run("kerning.autoKernPairList", &json!({"list": "pairs"})).unwrap();
    app.run("kerning.setPair", &json!({"left": "n", "right": "n", "value": 5})).unwrap();
    assert!(app.run("kerning.autoKernPairList", &json!({"apply": true})).is_err());
    app.run("kerning.autoKernPairList", &json!({"discard": true})).unwrap();
    assert!(app.state.panels.kerning_review.auto_kern.is_none());
}

#[test]
fn window_kerning_pairs_preview_sorts_by_first_glyph_second_glyph_and_value() {
    let mut app = kern_app();
    let first = app.run("window.kerningPairsPreview", &json!({})).unwrap();
    assert_eq!((first["open"].clone(), first["count"].clone(), first["sort"].clone()), (json!(true), json!(4), json!("first")));
    // Groups sort by the glyph they are drawn by: @T by T, @o by e.
    assert_eq!(sides(&first), vec![pair("A", "V"), pair("L", "T"), pair("@T", "@o"), pair("T", "o")]);
    let second = app.run("window.kerningPairsPreview", &json!({"sort": "second"})).unwrap();
    assert_eq!(sides(&second), vec![pair("L", "T"), pair("A", "V"), pair("@T", "@o"), pair("T", "o")]);
    let by_value = app.run("window.kerningPairsPreview", &json!({"sort": "value"})).unwrap();
    assert_eq!(sides(&by_value), vec![pair("A", "V"), pair("@T", "@o"), pair("L", "T"), pair("T", "o")]);
    // Only the pairs of one glyph: T itself, and the right group T it belongs to.
    let of_t = app.run("window.kerningPairsPreview", &json!({"glyph": "T", "sort": "first"})).unwrap();
    assert_eq!(sides(&of_t), vec![pair("L", "T"), pair("@T", "@o"), pair("T", "o")]);
    // Typing a character scrolls to the first pair whose first glyph it encodes.
    let jumped = app.run("window.kerningPairsPreview", &json!({"glyph": null, "jump": "L"})).unwrap();
    assert_eq!(jumped["scrollTo"], json!(1), "{jumped}");
    // Refusals change nothing.
    assert!(app.run("window.kerningPairsPreview", &json!({"sort": "name"})).is_err());
    assert!(app.run("window.kerningPairsPreview", &json!({"size": 50})).is_err());
    assert_eq!(app.run("window.kerningPairsPreview", &json!({})).unwrap()["sort"], "first");
    let closed = app.run("window.kerningPairsPreview", &json!({"open": false})).unwrap();
    assert_eq!(closed["open"], json!(false));
}

#[test]
fn dragging_the_second_glyph_in_the_kerning_pairs_preview_changes_the_value() {
    let mut app = kern_app();
    app.run("window.kerningPairsPreview", &json!({"size": 48})).unwrap();
    let w = Window::new();
    w.frame(&mut app, Vec::new());
    w.frame(&mut app, Vec::new());
    let id = crate::dialogs::kerning_pairs_preview::row_id("A", "V");
    let rect = app.state.panels.kerning_review.preview.laid_out.iter().find(|(r, _)| *r == id).map(|(_, r)| *r).expect("A V is laid out");
    // 48 pixels per em of 1000 units: 24 pixels are 500 units.
    let from = rect.center();
    w.drag(&mut app, from, from + egui::vec2(24.0, 0.0));
    assert_eq!(value(&mut app, "A", "V"), 400.0);
    let reply = app.run("window.kerningPairsPreview", &json!({})).unwrap();
    assert_eq!(reply["changed"], json!([{"left": "A", "right": "V", "value": 400.0, "was": -100.0}]));
    assert_eq!(reply["selected"], json!({"left": "A", "right": "V"}));
}

#[test]
fn kerning_preview_group_combinations_name_the_pair_that_applies() {
    let mut app = kern_app();
    let shown = app.run("window.kerningPairsPreview", &json!({"select": {"left": "@T", "right": "@o"}})).unwrap();
    let combos: Vec<(String, String, Value, Value)> = shown["combinations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| (c["first"].as_str().unwrap().into(), c["second"].as_str().unwrap().into(), c["value"].clone(), c["pair"].clone()))
        .collect();
    let group = json!({"left": "@T", "right": "@o"});
    assert_eq!(
        combos,
        vec![
            ("T".into(), "e".into(), json!(-80.0), group.clone()),
            ("T".into(), "o".into(), json!(-30.0), json!({"left": "T", "right": "o"})),
            ("V".into(), "e".into(), json!(-80.0), group.clone()),
            ("V".into(), "o".into(), json!(-80.0), group),
        ]
    );
    // Clicking a combination selects the pair that applies to it.
    let picked = app.run("window.kerningPairsPreview", &json!({"select": combos[1].3})).unwrap();
    assert_eq!(picked["selected"], json!({"left": "T", "right": "o"}));
    assert!(picked.get("combinations").is_none(), "a glyph pair lists no combinations");
}

#[test]
fn dialog_kerning_groups_editor_stacks_members_on_the_group_side() {
    let mut app = kern_app();
    app.run("spacing.setWidth", &json!({"glyphs": ["V"], "value": 640})).unwrap();
    let right = app.run("dialog.kerningGroupsEditor", &json!({"side": "right", "group": "T", "view": "stacked"})).unwrap();
    assert_eq!((right["members"].clone(), right["view"].clone()), (json!(["T", "V"]), json!("stacked")));
    assert_eq!(right["groups"], json!([{"name": "T", "glyphs": ["T", "V"]}]));
    // A right group lines its members up on their advance widths.
    let t_width = app.session.font().unwrap().glyph("T").unwrap().layers[0].width;
    assert_eq!(right["stacked"], json!({"aligned": "advance", "glyphs": [{"name": "T", "offset": -t_width}, {"name": "V", "offset": -640.0}]}));
    // A left group lines them up on their origins; changing the side drops the current group.
    let left = app.run("dialog.kerningGroupsEditor", &json!({"side": "left"})).unwrap();
    assert_eq!((left["group"].clone(), left["members"].clone()), (Value::Null, json!([])));
    let left = app.run("dialog.kerningGroupsEditor", &json!({"group": "o"})).unwrap();
    assert_eq!(left["stacked"], json!({"aligned": "origin", "glyphs": [{"name": "e", "offset": 0.0}, {"name": "o", "offset": 0.0}]}));
    // A new name starts an empty group; a bad name is refused.
    assert_eq!(app.run("dialog.kerningGroupsEditor", &json!({"group": "round"})).unwrap()["members"], json!([]));
    assert!(app.run("dialog.kerningGroupsEditor", &json!({"group": "two words"})).is_err());
    // The stacked view draws without trouble.
    let w = Window::new();
    w.frame(&mut app, Vec::new());
    assert!(app.run("dialog.kerningGroupsEditor", &json!({"apply": true})).is_err(), "no proposal to apply");
}
