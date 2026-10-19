//! The Space Center: each token of its text, matrix entry with an expression (and that it gives
//! the font its command gives), the arrow and drag steps, the cut-off band and preferred suffix
//! in a headless frame, refusals that change nothing, and a 200-glyph text within a frame's
//! budget.

use std::sync::Arc;

use egui::{Key, Modifiers, Pos2};
use fontcraft_model::Layer;
use fontcraft_textlayout::Charmap;
use serde_json::{Value, json};

use crate::dialogs::space_center::Drawn;
use crate::shell::space_center::{self as sc, Column, Context, CutOff, Drag, Row, Token};
use crate::{FontcraftApp, control};

/// A headless window: one egui context kept across frames.
struct Window {
    ctx: egui::Context,
}

impl Window {
    fn new() -> Self {
        let ctx = egui::Context::default();
        ctx.global_style_mut(|s| s.animation_time = 0.0);
        Self { ctx }
    }

    fn frame(&self, app: &mut FontcraftApp) {
        let input = egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1280.0, 800.0))), ..Default::default() };
        let mut output = self.ctx.run_ui(input, |ui| app.ui(ui));
        output.textures_delta.clear();
    }

    /// Two frames (a new window sizes itself in the first); what the window drew.
    fn drawn(&self, app: &mut FontcraftApp) -> Drawn {
        self.frame(app);
        self.frame(app);
        app.state.panels.space_center.cache.drawn.clone()
    }
}

fn exec(app: &mut FontcraftApp, command: &str, params: Value) -> Value {
    let r = control::handle(app, &json!({"id": 1, "method": "engine.execute", "params": {"command": command, "params": params}}));
    assert_eq!(r["ok"], true, "{command} {r}");
    r["result"].clone()
}

fn refused(app: &mut FontcraftApp, command: &str, params: Value) -> String {
    let r = control::handle(app, &json!({"id": 1, "method": "engine.execute", "params": {"command": command, "params": params}}));
    assert_eq!(r["ok"], false, "{command} {r}");
    r.to_string()
}

/// n, o, e, a, a.sc and H as rectangles with their code points; o and e in the kerning group
/// `round`; a second master "Bold" where every glyph is 300 units wider.
fn sc_app() -> FontcraftApp {
    let mut app = FontcraftApp::default();
    for (name, code, x, width) in
        [("n", "006E", 50, 400), ("o", "006F", 40, 420), ("e", "0065", 45, 410), ("a", "0061", 60, 380), ("H", "0048", 80, 500)]
    {
        app.run("path.addRectangle", &json!({"glyph": name, "x": x, "y": 0, "width": width, "height": 500})).unwrap();
        app.run("glyph.setUnicodes", &json!({"glyphs": [name], "unicodes": [code]})).unwrap();
    }
    app.run("glyph.newGlyph", &json!({"name": "a.sc"})).unwrap();
    app.run("path.addRectangle", &json!({"glyph": "a.sc", "x": 30, "y": 0, "width": 300, "height": 400})).unwrap();
    app.session
        .edit("setup", |font| {
            font.glyph_mut("o")?.kern_groups.left = Some("round".into());
            font.glyph_mut("e")?.kern_groups.right = Some("round".into());
            let mut m = font.default_master()?.clone();
            m.id = "m02".into();
            m.name = "Bold".into();
            Arc::make_mut(&mut font.masters).push(m);
            let names: Vec<String> = font.glyphs.iter().map(|g| g.name.clone()).collect();
            for name in names {
                let mut g = font.glyph_mut(&name)?;
                let width = g.layers.first().map_or(500.0, |l| l.width);
                let mut layer = g.layers.first().cloned().unwrap_or_else(|| Layer::new("m02", width));
                layer.id = "m02".into();
                layer.master = "m02".into();
                layer.width = width + 300.0;
                g.layers.push(layer);
            }
            Ok(())
        })
        .unwrap();
    app
}

fn master(app: &FontcraftApp) -> String {
    app.session.font().unwrap().masters[0].id.clone()
}

fn value(app: &FontcraftApp, glyph: &str, row: Row) -> f64 {
    sc::value(app.session.font().unwrap(), glyph, &master(app), row).unwrap()
}

fn names(d: &Drawn) -> Vec<&str> {
    d.glyphs.iter().map(|(n, _)| &**n).collect()
}

fn glyph(name: &str) -> Token {
    Token::Glyph { name: name.into(), layer: None }
}

#[test]
fn space_center_input_syntax_reads_every_token() {
    let app = sc_app();
    let font = app.session.font().unwrap();
    let map = Charmap::new(font);
    let selection = vec!["H".to_string(), "e".to_string()];
    let cx = Context { current: Some("o"), selection: &selection };
    let mut problems = Vec::new();
    let read = sc::tokens(font, &map, "n/a.sc /?/!\\no/a@Bold \u{E000}\nn", &cx, 1_000, &mut problems);
    assert_eq!(
        read,
        [
            glyph("n"),
            glyph("a.sc"),
            glyph("o"),
            glyph("H"),
            glyph("e"),
            Token::LineBreak,
            glyph("o"),
            Token::Glyph { name: "a".into(), layer: Some("Bold".into()) },
            Token::Missing('\u{E000}'),
            Token::LineBreak,
            glyph("n"),
        ]
    );
    assert!(problems.is_empty(), "{problems:?}");

    // A group gives its members on either side, in glyph order; an unknown group says so.
    let read = sc::tokens(font, &map, "/:round", &cx, 1_000, &mut problems);
    let mut members: Vec<Token> = read.clone();
    members.sort_by_key(|t| format!("{t:?}"));
    assert_eq!(members, [glyph("e"), glyph("o")]);
    assert!(sc::tokens(font, &map, "/:nothing", &cx, 1_000, &mut problems).is_empty());
    assert!(problems.iter().any(|p| p.contains("nothing")), "{problems:?}");

    // `/?` with no glyph being edited gives nothing; a lone slash is the slash character.
    let none = Context::default();
    assert!(sc::tokens(font, &map, "/?", &none, 1_000, &mut problems).is_empty());
    let read = sc::tokens(font, &map, "n/ n", &none, 1_000, &mut problems);
    assert_eq!(read.len(), 4, "{read:?}");
    assert_eq!(read.first(), Some(&glyph("n")));
    assert_eq!(read.last(), Some(&glyph("n")));

    // The limit cuts the text and says so.
    let mut problems = Vec::new();
    assert_eq!(sc::tokens(font, &map, "nnnnn", &none, 3, &mut problems).len(), 3);
    assert!(problems.iter().any(|p| p.contains("cut")), "{problems:?}");
}

#[test]
fn space_center_layer_suffix_and_control_glyphs_resolve() {
    let mut app = sc_app();
    exec(&mut app, "window.spaceCenter", json!({"text": "n/a@Bold n", "before": "H"}));
    let font = app.session.font().unwrap();
    let m = master(&app);
    let (current, selection) = sc::context(&app);
    let cx = Context { current: current.as_deref(), selection: &selection };
    let r = sc::resolve(font, &m, &app.state.panels.space_center, &cx);
    let columns: Vec<(&str, &str)> = r.columns.iter().map(|c| (c.glyph.as_str(), c.layer.as_str())).collect();
    assert_eq!(columns, [("n", m.as_str()), ("a", "m02"), ("n", m.as_str())]);
    assert_eq!(r.items.len(), 6, "a control glyph before each of the three");
    let regular = sc::value(font, "a", &m, Row::Width).unwrap();
    assert_eq!(sc::value(font, "a", "m02", Row::Width), Some(regular + 300.0), "the column reads the layer drawn");
    assert_eq!(sc::layer_named(font, "a", &m, "nothing"), None);
}

#[test]
fn space_center_matrix_entry_takes_an_expression_and_equals_its_command() {
    let mut app = sc_app();
    let m = master(&app);
    let e_left = value(&app, "e", Row::Left);
    let font = app.session.font().unwrap();
    assert_eq!(sc::entered_value(font, &m, Row::Left, "=e+10"), Ok(e_left + 10.0));
    assert_eq!(sc::entered_value(font, &m, Row::Left, " 25 "), Ok(25.0));
    assert_eq!(sc::entered_value(font, &m, Row::Width, "612.5"), Ok(613.0), "halves round up");
    assert!(sc::entered_value(font, &m, Row::Left, "=zz+1").unwrap_err().contains("no glyph"));
    assert!(sc::entered_value(font, &m, Row::Left, "=e/0").is_err());
    assert!(sc::entered_value(font, &m, Row::Left, "n").is_err(), "a name needs `=`");
    assert!(sc::entered_value(font, &m, Row::Width, "-5").is_err(), "a width is not negative");

    let column = Column { glyph: "n".into(), layer: m.clone(), item: 0 };
    let (id, params) = sc::matrix_command(font, &m, &column, Row::Left, "=e+10").unwrap();
    assert_eq!(id, "spacing.setLeftSidebearing");
    let mut direct = sc_app();
    app.run(id, &params).unwrap();
    direct.run("spacing.setLeftSidebearing", &json!({"glyphs": ["n"], "layer": m, "value": e_left + 10.0})).unwrap();
    assert_eq!(value(&app, "n", Row::Left), e_left + 10.0);
    for row in Row::ALL {
        assert_eq!(value(&app, "n", row), value(&direct, "n", row), "{row:?}: the matrix gives the font its command gives");
    }
    // Nothing stays linked: changing e afterwards leaves n alone.
    app.run("spacing.setLeftSidebearing", &json!({"glyphs": ["e"], "layer": m, "value": 0})).unwrap();
    assert_eq!(value(&app, "n", Row::Left), e_left + 10.0);
}

#[test]
fn space_center_arrow_steps_are_1_10_and_100() {
    let mut app = sc_app();
    let m = master(&app);
    assert_eq!(sc::step(Modifiers::NONE), 1.0);
    assert_eq!(sc::step(Modifiers::SHIFT), 10.0);
    assert_eq!(sc::step(Modifiers::SHIFT | Modifiers::COMMAND), 100.0);
    for (mods, step) in [(Modifiers::NONE, 1.0), (Modifiers::SHIFT, 10.0), (Modifiers::SHIFT | Modifiers::COMMAND, 100.0)] {
        let (id, amount) = sc::key_command(Key::ArrowRight, mods).unwrap();
        assert_eq!((id, amount), ("spacing.increaseRightSidebearing", step));
        let before = value(&app, "n", Row::Right);
        app.run(id, &json!({"glyphs": ["n"], "layer": m, "amount": amount})).unwrap();
        assert_eq!(value(&app, "n", Row::Right), before + step);
    }
    assert_eq!(sc::key_command(Key::ArrowLeft, Modifiers::NONE), Some(("spacing.increaseRightSidebearing", -1.0)));
    assert_eq!(sc::key_command(Key::ArrowLeft, Modifiers::ALT), Some(("spacing.increaseLeftSidebearing", 1.0)));
    assert_eq!(sc::key_command(Key::ArrowRight, Modifiers::ALT | Modifiers::SHIFT), Some(("spacing.increaseLeftSidebearing", -10.0)));
    assert_eq!(sc::key_command(Key::ArrowRight, Modifiers::CTRL | Modifiers::COMMAND), Some(("spacing.shiftOutlineRight", 1.0)));
    assert_eq!(sc::key_command(Key::ArrowRight, Modifiers::COMMAND), None, "Cmd alone is not a Space Center key");
    assert_eq!(sc::key_command(Key::ArrowUp, Modifiers::NONE), None);

    // Matrix navigation.
    assert_eq!(sc::neighbour(0, Row::Width, Key::Tab, Modifiers::NONE, 3), Some((1, Row::Width)));
    assert_eq!(sc::neighbour(0, Row::Width, Key::Tab, Modifiers::SHIFT, 3), None);
    assert_eq!(sc::neighbour(2, Row::Left, Key::Tab, Modifiers::NONE, 3), None);
    assert_eq!(sc::neighbour(1, Row::Width, Key::ArrowDown, Modifiers::ALT, 3), Some((1, Row::Left)));
    assert_eq!(sc::neighbour(1, Row::Width, Key::ArrowUp, Modifiers::ALT, 3), None);
    assert_eq!(sc::neighbour(1, Row::Right, Key::ArrowLeft, Modifiers::ALT, 3), Some((0, Row::Right)));

    // A drag of the left edge 12 units to the left grows the left margin by 12; with Alt, in
    // every master.
    let left = value(&app, "n", Row::Left);
    let commands = sc::drag_commands(Drag::Left, "n", std::slice::from_ref(&m), -12.4);
    assert_eq!(commands.len(), 1);
    for (id, params) in &commands {
        app.run(id, params).unwrap();
    }
    assert_eq!(value(&app, "n", Row::Left), left + 12.0);
    assert_eq!(sc::drag_commands(Drag::Outline, "n", &[m.clone(), "m02".into()], 5.0).len(), 2);
    assert!(sc::drag_commands(Drag::Right, "n", &[m], 0.3).is_empty(), "less than half a unit changes nothing");
}

#[test]
fn space_center_draws_the_cut_off_band_and_the_preferred_suffix() {
    let mut app = sc_app();
    let w = Window::new();
    let r = exec(&mut app, "window.spaceCenter", json!({"text": "nan"}));
    assert_eq!(r["glyphs"], json!(["n", "a", "n"]));
    let d = w.drawn(&mut app);
    assert_eq!(names(&d), ["n", "a", "n"]);
    assert_eq!(d.columns, ["n", "a", "n"]);
    assert!(d.bands.is_empty(), "the band is off");

    let m = master(&app);
    let (bottom, top) = sc::band(app.session.font().unwrap(), &m, &CutOff::default());
    let r = exec(&mut app, "view.setSpacingCutOff", json!({"on": true}));
    assert_eq!(r, json!({"on": true, "bottom": bottom, "top": top}));
    let d = w.drawn(&mut app);
    assert_eq!(d.bands.len(), 1, "one band per line");
    let base = d.baseline.unwrap();
    let band = d.bands[0];
    assert!((band.max.y - (base - bottom as f32 * d.scale)).abs() < 0.5, "{band:?} {base}");
    assert!((band.min.y - (base - top as f32 * d.scale)).abs() < 0.5, "{band:?} {base}");
    let r = exec(&mut app, "view.setSpacingCutOff", json!({"bottom": 100, "top": 200}));
    assert_eq!(r, json!({"on": true, "bottom": 100.0, "top": 200.0}));

    let r = exec(&mut app, "view.setPreferredSuffix", json!({"suffix": "sc"}));
    assert_eq!(r, json!({"suffix": ".sc", "replaced": 1}));
    let d = w.drawn(&mut app);
    assert_eq!(names(&d), ["n", "a.sc", "n"]);
    assert_eq!(d.columns, ["n", "a.sc", "n"]);
    exec(&mut app, "view.setPreferredSuffix", json!({}));
    assert_eq!(names(&w.drawn(&mut app)), ["n", "a", "n"]);

    let r = exec(&mut app, "window.spaceCenter", json!({"select": 1}));
    assert_eq!(r["selected"], 1);
    assert!(w.drawn(&mut app).selected.is_some());
    exec(&mut app, "window.spaceCenter", json!({"open": false}));
    assert!(!app.state.panels.space_center.open);
}

#[test]
fn space_center_refusals_change_nothing() {
    let mut app = sc_app();
    exec(&mut app, "window.spaceCenter", json!({"text": "nan"}));
    let before = app.state.panels.space_center.clone();
    refused(&mut app, "window.spaceCenter", json!({"text": "nonono", "select": 9}));
    refused(&mut app, "window.spaceCenter", json!({"size": 2}));
    refused(&mut app, "window.spaceCenter", json!({"lineMode": "waterfall"}));
    refused(&mut app, "window.spaceCenter", json!({"text": "n".repeat(sc::MAX_TEXT_CHARS + 1)}));
    refused(&mut app, "view.setSpacingCutOff", json!({"bottom": 500, "top": 100}));
    refused(&mut app, "view.setPreferredSuffix", json!({"suffix": "a b"}));
    assert_eq!(app.state.panels.space_center, before);
    assert_eq!(app.state.panels.space_center.text, "nan");
}

#[test]
fn space_center_frame_with_200_glyphs_is_within_budget() {
    let mut app = sc_app();
    let w = Window::new();
    let text = "nonae".repeat(40);
    let r = exec(&mut app, "window.spaceCenter", json!({"text": text}));
    assert_eq!(r["count"], 200);
    let d = w.drawn(&mut app);
    assert_eq!(d.columns.len(), 200);
    let mut best = std::time::Duration::MAX;
    for k in 0..3 {
        // A changed text each time, so the layout, the matrix and the texture are all redone.
        let shifted: String = text.chars().cycle().skip(k + 1).take(200).collect();
        exec(&mut app, "window.spaceCenter", json!({"text": shifted}));
        let start = std::time::Instant::now();
        w.frame(&mut app);
        best = best.min(start.elapsed());
    }
    println!("a Space Center frame with 200 glyphs: {best:?}");
    if !cfg!(debug_assertions) {
        assert!(best.as_millis() < 16, "a 200-glyph frame took {best:?}");
    }
}
