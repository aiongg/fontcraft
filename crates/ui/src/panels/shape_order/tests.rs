//! The Shape Order panel: the window command and its projection over a two-master glyph, and a row dragged in the
//! panel posting the layer's new order.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;

use craft::host::Host;
use craft::ui::contract::Tracked;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::testing::Fixture;
use fontcraft_engine::Session;
use serde_json::{Value, json};

use super::{Props, show};
use crate::app::models::shape_order::{ShapeColumn, ShapeEntry, ShapeKind, ShapeOrderModel};
use crate::app::{APP, FontCraftApp, Msg, ViewMsg};
use crate::canvas::tokens::Tokens;
use crate::control;
use crate::shell::dock::PanelId;

/// Masters Light and Bold; glyph `o.test` with two rectangles per master, the second master's drawn in the other
/// order; glyph `x.test` with one rectangle. `o.test` shows in the glyph panels.
fn two_masters() -> (FontCraftApp, [String; 2]) {
    let mut s = Session::with_document();
    let mut run = |id: &str, params: Value| {
        s.execute(id, &params).unwrap_or_else(|e| panic!("{id}: {e}"));
    };
    run("font.axes.add", json!({"name": "Weight", "tag": "wght"}));
    run("font.master.add", json!({}));
    run("glyph.newGlyph", json!({"name": "o.test"}));
    run("glyph.newGlyph", json!({"name": "x.test"}));
    let ids: Vec<String> = s.font().unwrap().masters.iter().map(|m| m.id.clone()).collect();
    let [light, bold] = [ids[0].clone(), ids[1].clone()];
    let rect = |x: i32, width: i32| json!({"x": x, "y": 0, "width": width, "height": 500});
    for (layer, rects) in [(&light, [rect(50, 100), rect(300, 60)]), (&bold, [rect(300, 90), rect(50, 200)])] {
        for r in rects {
            let mut params = r;
            params["glyph"] = json!("o.test");
            params["layer"] = json!(layer);
            s.execute("path.addRectangle", &params).unwrap();
        }
    }
    s.execute("path.addRectangle", &json!({"glyph": "x.test", "x": 0, "y": 0, "width": 10, "height": 10})).unwrap();
    let mut app = FontCraftApp::with_edit_tab(s, Arc::new(Host::builder(APP).build()));
    app.sent.push(Msg::View(ViewMsg::SelectGlyph("o.test".into())));
    app.drain();
    (app, [light, bold])
}

fn shell(app: &mut FontCraftApp, params: Value) -> Result<Value, String> {
    let result = app.run_now(Msg::Shell(CommandRef::with_params("window.shapeOrder", params)), false);
    app.project();
    result
}

fn inspected(app: &mut FontCraftApp) -> Value {
    control::dispatch(app, "ui.inspect", &json!({})).unwrap()["shapeOrder"].clone()
}

/// The left edges of the shapes of `layer` of `o.test`, in stacking order.
fn lefts(app: &FontCraftApp, layer: &str) -> Vec<f64> {
    let font = app.session.font().unwrap();
    let glyph = font.glyph("o.test").unwrap();
    let layer = glyph.layers.iter().find(|l| l.id == layer).unwrap();
    layer
        .shapes
        .iter()
        .map(|s| match s {
            fontcraft_engine::model::Shape::Path(p) => p.nodes.iter().map(|n| n.x).fold(f64::INFINITY, f64::min),
            _ => f64::NAN,
        })
        .collect()
}

#[test]
fn the_window_command_shows_each_layers_shapes_and_follows_the_edited_glyph() {
    let (mut app, [light, bold]) = two_masters();
    assert_eq!(inspected(&mut app)["open"], false);

    shell(&mut app, json!({})).unwrap();
    assert_ne!(app.view.workspace.layout().panel_state(&PanelId::ShapeOrder), craft::behavior::dock::PanelState::Hidden);
    let shown = inspected(&mut app);
    assert_eq!(shown["glyph"], "o.test", "{shown}");
    let layers = shown["layers"].as_array().unwrap();
    assert_eq!(layers.iter().map(|l| l["layer"].clone()).collect::<Vec<_>>(), vec![json!(light), json!(bold)], "{shown}");
    assert_eq!(layers[0]["shapes"], json!([{"index": 0, "kind": "path"}, {"index": 1, "kind": "path"}]));

    // The panel follows the glyph being edited until it is opened on a named glyph.
    app.sent.push(Msg::View(ViewMsg::SelectGlyph("x.test".into())));
    app.drain();
    assert_eq!(inspected(&mut app)["glyph"], "x.test");
    shell(&mut app, json!({"glyph": "o.test"})).unwrap();
    app.sent.push(Msg::View(ViewMsg::SelectGlyph("x.test".into())));
    app.drain();
    assert_eq!(inspected(&mut app)["glyph"], "o.test");

    // A refused open changes nothing.
    let err = shell(&mut app, json!({"glyph": "nope"})).unwrap_err();
    assert!(err.contains("nope"), "{err}");
    assert_eq!(inspected(&mut app)["glyph"], "o.test");

    // The order the panel posts is the engine's command; the panel shows the new order.
    let before = lefts(&app, &bold);
    app.sent.push(Msg::run("glyph.setShapeOrder", json!({"glyph": "o.test", "layer": bold, "order": [1, 0]})));
    app.drain();
    let after = lefts(&app, &bold);
    assert_eq!(after, vec![before[1], before[0]]);
    assert_eq!(after, lefts(&app, &light), "the masters now match");

    shell(&mut app, json!({"open": false})).unwrap();
    assert_eq!(inspected(&mut app)["open"], false);
}

fn rectangle_art(x: f32) -> Arc<crate::app::models::font_view::CellArt> {
    let contours = vec![vec![[x, 0.0], [x + 50.0, 0.0], [x + 50.0, 50.0], [x, 50.0]]];
    Arc::new(crate::app::models::font_view::CellArt { width: 0.0, contours, color: None })
}

/// A row dropped above the first row of its list posts `glyph.setShapeOrder` with that layer's new order.
#[test]
fn a_row_dragged_to_the_top_posts_the_layers_new_order() {
    let path = |x: f32| ShapeEntry { kind: ShapeKind::Path { nodes: 4, closed: true }, art: rectangle_art(x) };
    let column = ShapeColumn {
        layer: "m01".into(),
        title: "Regular".into(),
        shapes: vec![path(0.0), path(100.0), ShapeEntry { kind: ShapeKind::Component { glyph: "acute".into() }, art: rectangle_art(200.0) }],
        total: 3,
        bounds: Some([0.0, 0.0, 250.0, 50.0]),
    };
    let model = Tracked::new(ShapeOrderModel { glyph: Some("A".into()), columns: vec![column], more_layers: 0 });
    let tokens = Tokens::default();
    let posted = Rc::new(RefCell::new(Vec::new()));
    let seen = Rc::clone(&posted);
    let mut fixture = Fixture::new(move |ui| {
        let mut sent = Vec::new();
        show(ui, Props { model: &model, tokens: &tokens, sent: &mut sent });
        seen.borrow_mut().extend(sent);
    });
    fixture.settle();
    let first = fixture.by_label("0: Path (4 nodes)").expect("the row of shape 0");
    let last = fixture.by_label("2: Component acute").expect("the row of shape 2");
    let above_first = egui::pos2(first.center().x, first.rect.top() + first.rect.height() / 4.0);
    fixture.drag(&last, above_first - last.center()).settle();
    assert_eq!(
        *posted.borrow(),
        vec![Msg::run("glyph.setShapeOrder", json!({"glyph": "A", "layer": "m01", "order": [2, 0, 1]}))],
        "{:?}",
        posted.borrow()
    );
}
