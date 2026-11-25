//! Select Component Instance (M6.24): `component.setInstanceLocation`.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use fontcraft_engine::Session;
use fontcraft_geom::Shape as _;
use fontcraft_model::{Axis, Component, Font, Layer, Path, Shape};
use fontcraft_testkit::snapshot;
use serde_json::{Value, json};

const ID: &str = "component.setInstanceLocation";

fn run_err(s: &mut Session, p: Value) -> String {
    let before = snapshot(s);
    let e = s.execute(ID, &p).err().unwrap_or_else(|| panic!("{p} succeeded")).to_string();
    assert_eq!(snapshot(s), before, "{p} failed but changed the session");
    e
}

/// Two masters at weight 100 (Light) and 200 (Bold); `bar` is 100 wide in Light, 300 in Bold and
/// 40 in a backup layer; `host` holds a `bar` on both.
fn session() -> Session {
    let mut font = Font::new("Instances");
    std::sync::Arc::make_mut(&mut font.axes).push(Axis { name: "Weight".into(), tag: "wght".into(), ..Default::default() });
    let masters = std::sync::Arc::make_mut(&mut font.masters);
    let mut bold = masters[0].clone();
    (masters[0].name, masters[0].location) = ("Light".into(), vec![100.0]);
    (bold.id, bold.name, bold.location) = ("m02".into(), "Bold".into(), vec![200.0]);
    masters.push(bold);
    let bar = |w: f64| Shape::Path(Path::rect(0.0, 0.0, w, 50.0));
    let layers = |font: &mut Font, name: &str, shapes: [Shape; 2]| {
        font.add_glyph(name, Vec::new(), 600.0).unwrap();
        let mut g = font.glyph_mut(name).unwrap();
        for (l, shape) in g.layers.iter_mut().zip(shapes) {
            l.shapes = vec![shape];
        }
    };
    layers(&mut font, "bar", [bar(100.0), bar(300.0)]);
    layers(&mut font, "host", [0, 1].map(|_| Shape::Component(Component::new("bar", 0.0, 0.0))));
    {
        let mut g = font.glyph_mut("bar").unwrap();
        let mut l = Layer::new("m01", 600.0);
        l.id = "backup".into();
        l.info.name = Some("old".into());
        l.shapes = vec![bar(40.0)];
        g.layers.push(l);
    }
    let mut s = Session::new();
    s.add_document(font, None).unwrap();
    s
}

fn width(s: &Session) -> f64 {
    s.font().unwrap().outline("host", "m01").unwrap().bounding_box().width()
}

fn run(s: &mut Session, p: Value) -> (Value, f64) {
    let r = s.execute(ID, &p).unwrap_or_else(|e| panic!("{p}: {e}"));
    let w = width(s);
    (r, w)
}

/// A pin reads another master or layer by name or id, a location interpolates and wins over the
/// pin, each stays when the other is set, and omitting both resets in one undo step.
#[test]
fn a_component_is_pinned_located_and_reset() {
    let mut s = session();
    assert_eq!(width(&s), 100.0);
    let (r, w) = run(&mut s, json!({"glyph": "host", "layerName": "Bold"}));
    assert_eq!((r["pinned"].as_str(), w), (Some("m02"), 300.0));
    let (r, w) = run(&mut s, json!({"glyph": "host", "component": 0, "location": {"wght": 150}}));
    assert_eq!((r["pinned"].as_str(), &r["location"], w), (Some("m02"), &json!({"wght": 150.0}), 200.0));
    let (r, w) = run(&mut s, json!({"glyph": "host", "layerName": "old"}));
    assert_eq!((r["pinned"].as_str(), w), (Some("backup"), 200.0));
    // Export writes the interpolated outline; the stored component stays.
    let (drawn, report) = fontcraft_model::pinned::draw_pinned_components(&s.font().unwrap().clone());
    assert_eq!((report.drawn, report.kept.len()), (2, 0), "{:?}", report.kept);
    assert_eq!(drawn.outline("host", "m01").unwrap().bounding_box().width(), 200.0);
    let (_, w) = run(&mut s, json!({"glyph": "host", "location": {}}));
    assert_eq!(w, 40.0);
    run(&mut s, json!({"glyph": "host", "location": {"wght": 250}}));
    let (r, w) = run(&mut s, json!({"glyph": "host"}));
    assert_eq!((&r["pinned"], &r["location"], w), (&Value::Null, &Value::Null, 100.0));
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(width(&s), 400.0, "a location beyond the masters extrapolates");
}

/// Locations that are not numbers or name no axis, masters the base glyph cannot be interpolated
/// between, unknown layers and shapes that are not components are refused.
#[test]
fn bad_instances_are_refused() {
    let mut s = session();
    for (p, why) in [
        (json!({"glyph": "host", "location": {"wght": 1e300}}), "within"),
        (json!({"glyph": "host", "location": {"wght": "bold"}}), "number"),
        (json!({"glyph": "host", "location": {"opsz": 10}}), "no axis"),
        (json!({"glyph": "host", "location": 12}), "location"),
        (json!({"glyph": "host", "layerName": "Black"}), "no master or layer"),
        (json!({"glyph": "host", "component": 3, "layerName": "Bold"}), "not a component"),
        (json!({"glyph": "bar", "layerName": "Bold"}), "no components"),
    ] {
        let e = run_err(&mut s, p.clone());
        assert!(e.contains(why), "{p}: {e}");
    }
    s.edit("open", |f| {
        let mut g = f.glyph_mut("bar")?;
        if let Some(Shape::Path(p)) = g.layers[1].shapes.get_mut(0) {
            p.nodes.pop();
        }
        Ok(())
    })
    .unwrap();
    let e = run_err(&mut s, json!({"glyph": "host", "location": {"wght": 150}}));
    assert!(e.contains("not compatible"), "{e}");
}
