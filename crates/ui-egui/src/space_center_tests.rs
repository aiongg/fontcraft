//! The Space Center: each token of its text, matrix entry with an expression (and that it gives
//! the font its command gives), the arrow and drag steps, the cut-off band and preferred suffix
//! in a headless frame, refusals that change nothing, and a 200-glyph text within a frame's
//! budget. Then the sample strings, the waterfall, the display options (direction, centring,
//! upside down, beam, stroke, overlays, guides, zones, line space, tracking) and their refusals.

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
    refused(&mut app, "window.spaceCenter", json!({"lineMode": "zigzag"}));
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

fn settings_samples(app: &mut FontcraftApp) {
    use crate::dialogs::sample_strings::Group;
    use fontcraft_engine::settings::Reserved;
    let groups = vec![
        Group { name: "Spacing".into(), strings: vec!["nn/Placeholder no".into()] },
        Group { name: "Lines".into(), strings: vec!["/a.sc /H\\nnn".into(), "/Placeholder.alt n".into()] },
    ];
    app.session.settings_mut().set_reserved(Reserved::SampleStrings, &groups).unwrap();
}

#[test]
fn space_center_sample_strings_menu_puts_a_stored_string_into_the_text() {
    let mut app = sc_app();
    settings_samples(&mut app);
    app.state.selected_glyphs = vec!["a".into()];
    // The placeholder is the glyph being edited; other names stay as they are.
    let r = exec(&mut app, "window.spaceCenter", json!({"sample": 0}));
    assert_eq!(r["text"], "nn/? no");
    assert_eq!(r["glyphs"], json!(["n", "n", "a", "n", "o"]));
    // Strings are counted over all groups in order; `\n` is a new line.
    let r = exec(&mut app, "window.spaceCenter", json!({"sample": 1}));
    assert_eq!(r["glyphs"], json!(["a.sc", "H", "n", "n"]));
    let w = Window::new();
    assert_eq!(w.drawn(&mut app).lines, 2);
    assert_eq!(sc::sample_text("/Placeholder.alt n/Placeholder"), "/Placeholder.alt n/?", "only the whole name is the placeholder");

    let before = app.state.panels.space_center.clone();
    refused(&mut app, "window.spaceCenter", json!({"sample": 3}));
    refused(&mut app, "window.spaceCenter", json!({"sample": 0, "text": "n"}));
    refused(&mut app, "window.spaceCenter", json!({"sample": -1}));
    assert_eq!(app.state.panels.space_center, before);
}

#[test]
fn space_center_waterfall_draws_the_line_at_each_size() {
    let mut app = sc_app();
    let w = Window::new();
    let r = exec(&mut app, "window.spaceCenter", json!({"text": "na\\no", "lineMode": "waterfall", "size": 24}));
    assert_eq!(r["lineMode"], "waterfall");
    assert_eq!(r["count"], 3);
    let d = w.drawn(&mut app);
    let sizes = sc::waterfall_sizes(24.0);
    assert_eq!(sizes, [9.0, 10.0, 12.0, 14.0, 18.0, 24.0]);
    assert_eq!(d.lines, 1, "a waterfall is one line");
    assert_eq!(d.rows.len(), sizes.len());
    let upm = f64::from(app.session.font().unwrap().units_per_em);
    for ((scale, _), size) in d.rows.iter().zip(&sizes) {
        assert!((f64::from(*scale) - size / upm).abs() < 1e-6, "{scale} for {size} pt");
    }
    assert!(d.rows.windows(2).all(|p| p[1].1 > p[0].1), "each size below the one before: {:?}", d.rows);
    assert_eq!(names(&d), ["n", "a", "o"].repeat(sizes.len()));
    assert_eq!(d.columns, ["n", "a", "o"], "the matrix keeps one column per glyph");
    assert_eq!(sc::waterfall_sizes(9.0), [9.0]);
}

/// The x of each glyph drawn, in text order.
fn xs(d: &Drawn) -> Vec<f32> {
    d.glyphs.iter().map(|(_, p)| p.x).collect()
}

#[test]
fn space_center_display_lays_out_right_to_left_centred_and_upside_down() {
    let mut app = sc_app();
    let w = Window::new();
    exec(&mut app, "window.spaceCenter", json!({"text": "nao"}));
    let ltr = w.drawn(&mut app);
    let x = xs(&ltr);
    assert!(x[0] < x[1] && x[1] < x[2], "{x:?}");

    let r = exec(&mut app, "view.setSpaceCenterDisplay", json!({"rightToLeft": true}));
    assert_eq!(r["rightToLeft"], true);
    assert_eq!(r["fill"], true, "options left out keep their value");
    let rtl = w.drawn(&mut app);
    let x = xs(&rtl);
    assert!(x[0] > x[1] && x[1] > x[2], "the first glyph is on the right: {x:?}");
    assert!(x[2] > xs(&ltr)[2], "a right-to-left line starts from the right edge");

    // Centred, the right-to-left text is the left-to-right text of its glyphs in reverse.
    exec(&mut app, "view.setSpaceCenterDisplay", json!({"centered": true}));
    let mut centred_rtl = xs(&w.drawn(&mut app));
    exec(&mut app, "view.setSpaceCenterDisplay", json!({"rightToLeft": false}));
    exec(&mut app, "window.spaceCenter", json!({"text": "oan"}));
    let centred_ltr = xs(&w.drawn(&mut app));
    centred_rtl.reverse();
    for (a, b) in centred_rtl.iter().zip(&centred_ltr) {
        assert!((a - b).abs() < 0.5, "{centred_rtl:?} {centred_ltr:?}");
    }
    assert!(centred_ltr[0] > xs(&ltr)[0], "centring moves a short line right");

    // Upside down: the line is turned half a circle, so the first glyph is on the right and the
    // baseline moves.
    exec(&mut app, "view.setSpaceCenterDisplay", json!({"centered": false, "upsideDown": true}));
    exec(&mut app, "window.spaceCenter", json!({"text": "nao"}));
    let turned = w.drawn(&mut app);
    let x = xs(&turned);
    assert!(x[0] > x[1] && x[1] > x[2], "{x:?}");
    assert_ne!(turned.baseline, ltr.baseline);
    assert_eq!(names(&turned), ["n", "a", "o"]);
}

#[test]
fn space_center_display_beam_shows_the_margins_on_it() {
    let mut app = sc_app();
    let w = Window::new();
    exec(&mut app, "window.spaceCenter", json!({"text": "na"}));
    let m = master(&app);
    let font = app.session.font().unwrap();
    let half = sc::x_height(font, &m) / 2.0;
    let r = exec(&mut app, "view.setSpaceCenterDisplay", json!({"beam": true}));
    assert_eq!(r["beamY"], json!(half), "half the x-height until a height is set");

    let r = exec(&mut app, "view.setSpaceCenterDisplay", json!({"beamY": 250}));
    assert_eq!((r["beam"].clone(), r["beamY"].clone()), (json!(true), json!(250.0)));
    assert!(!app.state.panels.edit_aids.beam, "Edit View's beam stays off; only the height is shared");
    let doc = app.session.active().unwrap().id;
    assert_eq!(app.state.panels.edit_aids.beam_heights.get(&doc), Some(&250.0));
    let d = w.drawn(&mut app);
    let base = d.baseline.unwrap();
    assert!((d.beam.unwrap() - (base - 250.0 * d.scale)).abs() < 0.5);
    let labels: Vec<(&str, f64, f64)> = d.beam_labels.iter().map(|(n, l, r)| (&**n, *l, *r)).collect();
    assert_eq!(
        labels,
        [("n", value(&app, "n", Row::Left), value(&app, "n", Row::Right)), ("a", value(&app, "a", Row::Left), value(&app, "a", Row::Right))]
    );

    // Above the outlines the beam meets nothing.
    exec(&mut app, "view.setSpaceCenterDisplay", json!({"beamY": 900}));
    assert!(w.drawn(&mut app).beam_labels.is_empty());
    exec(&mut app, "view.setSpaceCenterDisplay", json!({"beam": false}));
    assert_eq!(w.drawn(&mut app).beam, None);
}

#[test]
fn space_center_display_draws_strokes_overlays_guides_and_zones() {
    let mut app = sc_app();
    app.session
        .edit("guides and zones", |font| {
            let master = Arc::make_mut(&mut font.masters).first_mut().unwrap();
            master.guides.push(fontcraft_model::Guide { y: 300.0, ..Default::default() });
            master.zones.push(fontcraft_model::AlignmentZone { pos: 500.0, size: 12.0 });
            Ok(())
        })
        .unwrap();
    let w = Window::new();
    exec(&mut app, "window.spaceCenter", json!({"text": "nan"}));
    let plain = w.drawn(&mut app);
    assert!(plain.filled);
    assert_eq!((plain.strokes, plain.overlays, plain.guides, plain.zones), (0, 0, 0, 0));
    assert_eq!(plain.ground, Some(app.tokens.canvas));

    let r = exec(
        &mut app,
        "view.setSpaceCenterDisplay",
        json!({"fill": false, "stroke": true, "layers": ["Bold", "Bold"], "guides": true, "zones": true, "inverse": true}),
    );
    assert_eq!(r["layers"], json!(["Bold"]), "a layer named twice is drawn once");
    let d = w.drawn(&mut app);
    assert!(!d.filled);
    assert_eq!(d.strokes, 3, "one contour per glyph");
    assert_eq!(d.overlays, 3, "each glyph's Bold layer over it");
    assert_eq!(d.guides, 3, "the master's guide across each glyph");
    assert!(d.zones >= 1, "the master's zone");
    assert_eq!(d.ground, Some(app.tokens.glyph_fill), "inverse: the glyph colour behind");
}

#[test]
fn space_center_display_line_space_and_tracking_move_the_glyphs() {
    let mut app = sc_app();
    let w = Window::new();
    exec(&mut app, "window.spaceCenter", json!({"text": "nn\\nn"}));
    let d = w.drawn(&mut app);
    let gap = d.rows[1].1 - d.rows[0].1;
    let x = xs(&d);

    exec(&mut app, "view.setSpaceCenterDisplay", json!({"lineSpace": 2, "tracking": 100}));
    let spaced = w.drawn(&mut app);
    assert!(((spaced.rows[1].1 - spaced.rows[0].1) - 2.0 * gap).abs() < 0.5, "{:?} {gap}", spaced.rows);
    let tracked = xs(&spaced);
    assert!(((tracked[1] - tracked[0]) - (x[1] - x[0] + 100.0 * d.scale)).abs() < 0.01, "{tracked:?} {x:?}");
    assert!((tracked[2] - x[2]).abs() < 0.01, "a line's first glyph stays");
}

#[test]
fn space_center_display_refusals_change_nothing() {
    let mut app = sc_app();
    exec(&mut app, "window.spaceCenter", json!({"text": "nan"}));
    let before = app.state.panels.space_center.clone();
    let heights = app.state.panels.edit_aids.beam_heights.clone();
    refused(&mut app, "view.setSpaceCenterDisplay", json!({"lineSpace": 9, "beamY": 100}));
    refused(&mut app, "view.setSpaceCenterDisplay", json!({"tracking": 5000}));
    refused(&mut app, "view.setSpaceCenterDisplay", json!({"layers": [""]}));
    refused(&mut app, "view.setSpaceCenterDisplay", json!({"layers": ["a", "b", "c", "d", "e", "f", "g", "h", "i"]}));
    refused(&mut app, "view.setSpaceCenterDisplay", json!({"beam": "yes"}));
    refused(&mut app, "view.setSpaceCenterDisplay", json!({"beamY": 1e9}));
    assert_eq!(app.state.panels.space_center, before);
    assert_eq!(app.state.panels.edit_aids.beam_heights, heights);
}

#[test]
fn space_center_frame_with_every_display_option_is_within_budget() {
    let mut app = sc_app();
    let w = Window::new();
    let text = "nonae".repeat(40);
    exec(&mut app, "window.spaceCenter", json!({"text": text}));
    exec(
        &mut app,
        "view.setSpaceCenterDisplay",
        json!({"rightToLeft": true, "centered": true, "beam": true, "beamY": 250, "stroke": true, "layers": ["Bold"], "guides": true, "zones": true, "tracking": 10}),
    );
    exec(&mut app, "view.setSpacingCutOff", json!({"on": true}));
    assert_eq!(w.drawn(&mut app).columns.len(), 200);
    let mut best = std::time::Duration::MAX;
    for k in 0..3 {
        let shifted: String = text.chars().cycle().skip(k + 1).take(200).collect();
        exec(&mut app, "window.spaceCenter", json!({"text": shifted}));
        let start = std::time::Instant::now();
        w.frame(&mut app);
        best = best.min(start.elapsed());
    }
    println!("a Space Center frame with 200 glyphs and every display option: {best:?}");
    if !cfg!(debug_assertions) {
        assert!(best.as_millis() < 16, "a 200-glyph frame took {best:?}");
    }
}
