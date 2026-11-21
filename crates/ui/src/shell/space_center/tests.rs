//! Behaviour tests of the Space Center's text, commands and projection.

use std::sync::Arc;

use craft::host::Host;
use craft::ui::runtime::commands::CommandRef;
use fontcraft_engine::Session;
use fontcraft_engine::model::Layer;
use fontcraft_textlayout::Charmap;
use serde_json::{Value, json};

use super::text::{self, Context, Row, Token};
use crate::app::{APP, FontCraftApp, Msg};

/// n, o, e, a, H drawn as rectangles and encoded; a.sc; o in the left group `round`, e in the right group `round`;
/// a second master `Bold` whose layers are 300 units wider.
fn session() -> Session {
    let mut s = Session::with_document();
    for (name, code, x, width) in
        [("n", "006E", 50, 400), ("o", "006F", 40, 420), ("e", "0065", 45, 410), ("a", "0061", 60, 380), ("H", "0048", 80, 500)]
    {
        s.execute("path.addRectangle", &json!({"glyph": name, "x": x, "y": 0, "width": width, "height": 500})).unwrap();
        s.execute("glyph.setUnicodes", &json!({"glyphs": [name], "unicodes": [code]})).unwrap();
    }
    s.execute("glyph.newGlyph", &json!({"name": "a.sc"})).unwrap();
    s.execute("path.addRectangle", &json!({"glyph": "a.sc", "x": 30, "y": 0, "width": 300, "height": 400})).unwrap();
    s.edit("setup", |font| {
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
    s
}

fn app() -> FontCraftApp {
    FontCraftApp::with_edit_tab(session(), Arc::new(Host::builder(APP).build()))
}

fn shell(app: &mut FontCraftApp, id: &str, params: Value) -> Result<Value, String> {
    app.run_now(Msg::Shell(CommandRef::with_params(id, params)), false)
}

fn ok(app: &mut FontCraftApp, id: &str, params: Value) {
    shell(app, id, params).unwrap_or_else(|e| panic!("{id}: {e}"));
}

fn master(app: &FontCraftApp) -> String {
    app.session.font().unwrap().masters[0].id.clone()
}

fn value(app: &FontCraftApp, glyph: &str, row: Row) -> f64 {
    text::value(app.session.font().unwrap(), glyph, &master(app), row).unwrap()
}

fn glyph(name: &str) -> Token {
    Token::Glyph { name: name.into(), layer: None }
}

/// Characters, `/name`, `/?`, `/!`, `\n`, `/name@layer`, `/:group`, a lone slash and the glyph limit.
#[test]
fn the_text_reads_every_token() {
    let s = session();
    let font = s.font().unwrap();
    let map = Charmap::new(font);
    let selection = vec!["H".to_string(), "e".to_string()];
    let cx = Context { current: Some("o"), selection: &selection };
    let mut problems = Vec::new();
    let read = text::tokens(font, &map, "n/a.sc /?/!\\no/a@Bold \u{E000}\nn", &cx, 1_000, &mut problems);
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

    let mut members = text::tokens(font, &map, "/:round", &cx, 1_000, &mut problems);
    members.sort_by_key(|t| format!("{t:?}"));
    assert_eq!(members, [glyph("e"), glyph("o")]);
    assert!(text::tokens(font, &map, "/:nothing", &cx, 1_000, &mut problems).is_empty());
    assert!(problems.iter().any(|p| p.contains("nothing")), "{problems:?}");

    let none = Context::default();
    assert!(text::tokens(font, &map, "/?", &none, 1_000, &mut problems).is_empty());
    let read = text::tokens(font, &map, "n/ n", &none, 1_000, &mut problems);
    assert_eq!(read.len(), 4, "{read:?}");

    let mut problems = Vec::new();
    assert_eq!(text::tokens(font, &map, "nnnnn", &none, 3, &mut problems).len(), 3);
    assert!(problems.iter().any(|p| p.contains("cut")), "{problems:?}");
}

/// Opening the window with a text and control glyphs shows the panel and projects one column per glyph of the text,
/// in the layer named after `@`; the lines and the outlines follow.
#[test]
fn the_window_projects_columns_lines_and_outlines() {
    let mut app = app();
    ok(&mut app, "window.spaceCenter", json!({"text": "n/a@Bold n", "before": "H"}));
    assert!(super::shows(&app));
    let m = master(&app);
    let model = &app.models.space_center;
    let columns: Vec<(&str, &str)> = model.columns.iter().map(|c| (c.glyph.as_str(), c.layer.as_str())).collect();
    assert_eq!(columns, [("n", m.as_str()), ("a", "m02"), ("n", m.as_str())]);
    let a = &model.columns[1];
    assert_eq!(a.width, Some(value(&app, "a", Row::Width) + 300.0), "the column reads the layer drawn");
    let line = &model.lines[0];
    assert_eq!(line.glyphs.len(), 6, "a control glyph before each of the three");
    assert_eq!(line.glyphs.iter().filter_map(|g| g.column).collect::<Vec<_>>(), [0, 1, 2]);
    assert!(line.glyphs.windows(2).all(|w| w[0].offset < w[1].offset), "left to right");
    for name in ["n", "a", "H"] {
        assert!(!model.art[name].outline.elements().is_empty(), "{name} has its outline");
    }
}

/// `select` takes a column; refused parameters change nothing.
#[test]
fn refusals_change_nothing() {
    let mut app = app();
    ok(&mut app, "window.spaceCenter", json!({"text": "noe", "select": 2}));
    assert_eq!(app.view.space_center.selected, Some(2));
    let before = app.view.space_center.clone();
    for (id, params) in [
        ("window.spaceCenter", json!({"text": "nn", "select": 5})),
        ("window.spaceCenter", json!({"size": 2})),
        ("window.spaceCenter", json!({"lineMode": "spiral"})),
        ("view.setSpacingCutOff", json!({"bottom": 400, "top": 100})),
        ("view.setPreferredSuffix", json!({"suffix": "a b"})),
        ("view.setSpaceCenterDisplay", json!({"tracking": 5000})),
        ("view.setSpaceCenterDisplay", json!({"metrics": false, "lineSpace": 9})),
    ] {
        assert!(shell(&mut app, id, params.clone()).is_err(), "{id} {params}");
        assert_eq!(app.view.space_center, before, "{id} {params}");
    }
    ok(&mut app, "window.spaceCenter", json!({"select": null}));
    assert_eq!(app.view.space_center.selected, None);
}

/// A matrix entry evaluates its expression against the font, runs the row's setter on the column's layer, and
/// leaves nothing linked.
#[test]
fn a_matrix_entry_runs_the_setter_with_the_evaluated_number() {
    let mut app = app();
    ok(&mut app, "window.spaceCenter", json!({"text": "ne"}));
    let e_left = value(&app, "e", Row::Left);
    ok(&mut app, "spaceCenter.setValue", json!({"column": 0, "row": "left", "text": "=e+10"}));
    assert_eq!(value(&app, "n", Row::Left), e_left + 10.0);
    assert_eq!(app.models.space_center.columns[0].left, Some(e_left + 10.0), "the matrix shows the new value");
    let m = master(&app);
    app.run_now(Msg::run("spacing.setLeftSidebearing", json!({"glyphs": ["e"], "layer": m, "value": 0})), false).unwrap();
    assert_eq!(value(&app, "n", Row::Left), e_left + 10.0, "nothing stays linked");

    ok(&mut app, "spaceCenter.setValue", json!({"column": 1, "row": "width", "text": "612.5"}));
    assert_eq!(value(&app, "e", Row::Width), 613.0, "halves round up");
    let width = value(&app, "e", Row::Width);
    for typed in ["=zz+1", "n", "-5"] {
        assert!(shell(&mut app, "spaceCenter.setValue", json!({"column": 1, "row": "width", "text": typed})).is_err(), "{typed}");
    }
    assert!(shell(&mut app, "spaceCenter.setValue", json!({"column": 9, "row": "width", "text": "1"})).is_err());
    assert_eq!(value(&app, "e", Row::Width), width);
}

/// The cut-off band defaults to the baseline and the x-height; the preferred suffix shows a.sc for a; the beam
/// measures each glyph's margins; tracking spreads the glyphs; upside down and right to left reach the model.
#[test]
fn the_display_options_reach_the_model() {
    let mut app = app();
    ok(&mut app, "window.spaceCenter", json!({"text": "an"}));
    ok(&mut app, "view.setSpacingCutOff", json!({}));
    let font = app.session.font().unwrap();
    let x_height = text::x_height(font, &master(&app));
    assert_eq!(app.models.space_center.band, Some((0.0, x_height)));
    ok(&mut app, "view.setPreferredSuffix", json!({"suffix": "sc"}));
    assert_eq!(app.models.space_center.columns[0].glyph, "a.sc");
    assert_eq!(app.view.space_center.suffix.as_deref(), Some(".sc"));

    let plain: Vec<f64> = app.models.space_center.lines[0].glyphs.iter().map(|g| g.offset).collect();
    ok(&mut app, "view.setSpaceCenterDisplay", json!({"tracking": 100, "beamY": 250}));
    let model = &app.models.space_center;
    let tracked: Vec<f64> = model.lines[0].glyphs.iter().map(|g| g.offset).collect();
    assert_eq!(tracked[1] - plain[1], 100.0, "{plain:?} {tracked:?}");
    assert_eq!(model.beam_y, Some(250.0), "beamY shows the beam");
    assert_eq!(model.lines[0].glyphs[1].beam, Some((50.0, value(&app, "n", Row::Right))));

    ok(&mut app, "view.setSpaceCenterDisplay", json!({"rightToLeft": true, "upsideDown": true}));
    let model = &app.models.space_center;
    assert!(model.state.display.right_to_left && model.state.display.upside_down);
    let names: Vec<&str> = model.lines[0].glyphs.iter().filter_map(|g| g.glyph.as_deref()).collect();
    assert_eq!(names, ["n", "a.sc"], "right to left, the first glyph is drawn rightmost");
    assert!(model.lines[0].glyphs[0].offset >= 0.0);
}
