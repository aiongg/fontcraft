//! The interpolation queries (`interpolation.*`) and the axis mapping commands
//! (`axisMappings.*`) on a two-master font built in code.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use fontcraft_engine::{EngineError, Session};
use fontcraft_model::{Axis, AxisRule, Font, Glyph, GlyphTable, Instance, Layer, Master, Path, Shape};
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

/// The command's refusal message; panics when it succeeds.
fn refused(s: &mut Session, id: &str, p: Value) -> String {
    match s.execute(id, &p) {
        Err(EngineError::BadParams { message, .. }) => message,
        other => panic!("{id} {p}: expected a refusal, got {other:?}"),
    }
}

fn rect(x0: f64, x1: f64) -> Shape {
    Shape::Path(Path::rect(x0, 0.0, x1, 700.0))
}

fn layer(master: &str, id: &str, width: f64, shapes: Vec<Shape>) -> Layer {
    let mut l = Layer::new(master, width);
    l.id = id.into();
    l.shapes = shapes;
    l
}

fn glyph(name: &str, layers: Vec<Layer>) -> Glyph {
    let mut g = Glyph::new(name);
    g.layers = layers;
    g
}

/// Thin (wght 100) and Black (900). Instances: Regular (400), ExtraBlack (1000, not exported, so
/// it leaves the axis map alone) and the variable font setting VF. Glyphs:
/// - `n`: a rectangle 100 wide in Thin, 300 in Black; advance 500 and 700.
/// - `x`: one rectangle in Thin, two in Black (incompatible).
/// - `i`: compatible masters and an intermediate layer at 500 with two rectangles.
/// - `a`: like `n`, with alternate layers from 600 up, 100 and 280 wide.
fn session() -> Session {
    let mut f = Font::new("Interp Test");
    f.axes = Arc::new(vec![Axis { name: "Weight".into(), tag: "wght".into(), ..Axis::default() }]);
    let template = f.masters[0].clone();
    f.masters = Arc::new(vec![
        Master { id: "thin".into(), name: "Thin".into(), location: vec![100.0], ..template.clone() },
        Master { id: "black".into(), name: "Black".into(), location: vec![900.0], ..template },
    ]);
    f.instances = Arc::new(vec![
        Instance { name: "Regular".into(), location: vec![400.0], ..Instance::default() },
        Instance { name: "ExtraBlack".into(), location: vec![1000.0], exports: false, ..Instance::default() },
        Instance { name: "VF".into(), kind: Some("variable".into()), ..Instance::default() },
    ]);
    let mut intermediate = layer("thin", "i500", 350.0, vec![rect(0.0, 120.0), rect(200.0, 220.0)]);
    intermediate.info.attrs.coordinates = vec![Some(500.0)];
    let alternate = |master: &str, id: &str, width: f64, w: f64| {
        let mut l = layer(master, id, width, vec![rect(0.0, w)]);
        l.info.attrs.axis_rules = vec![AxisRule { min: Some(600.0), max: None }];
        l
    };
    let glyphs = vec![
        glyph("n", vec![layer("thin", "thin", 500.0, vec![rect(0.0, 100.0)]), layer("black", "black", 700.0, vec![rect(0.0, 300.0)])]),
        glyph(
            "x",
            vec![layer("thin", "thin", 500.0, vec![rect(0.0, 100.0)]), layer("black", "black", 700.0, vec![rect(0.0, 300.0), rect(400.0, 450.0)])],
        ),
        glyph("i", vec![layer("thin", "thin", 300.0, vec![rect(0.0, 50.0)]), layer("black", "black", 400.0, vec![rect(0.0, 150.0)]), intermediate]),
        glyph(
            "a",
            vec![
                layer("thin", "thin", 500.0, vec![rect(0.0, 100.0)]),
                layer("black", "black", 700.0, vec![rect(0.0, 300.0)]),
                alternate("thin", "a.alt.thin", 500.0, 100.0),
                alternate("black", "a.alt.black", 700.0, 280.0),
            ],
        ),
    ];
    f.glyphs = GlyphTable::from_glyphs(glyphs.into_iter().map(Arc::new).collect());
    let mut s = Session::new();
    s.add_document(f, None).unwrap();
    s
}

/// The right edge of the first shape of an interpolated layer.
fn first_shape_right(out: &Value) -> f64 {
    out["layer"]["shapes"][0]["path"]["nodes"].as_array().unwrap().iter().map(|n| n["x"].as_f64().unwrap()).fold(f64::MIN, f64::max)
}

fn points(out: &Value) -> Vec<(f64, f64)> {
    out["points"].as_array().unwrap().iter().map(|p| (p["external"].as_f64().unwrap(), p["internal"].as_f64().unwrap())).collect()
}

fn convert(s: &mut Session, value: f64, from: &str, setting: Option<&str>) -> f64 {
    let mut p = json!({"axis": "wght", "value": value, "from": from});
    if let Some(name) = setting {
        p["variableFontSetting"] = json!(name);
    }
    run(s, "axisMappings.convertCoordinate", p)["result"].as_f64().unwrap()
}

#[test]
fn interpolation_interpolate_glyph_between_beyond_and_on_alternates() {
    let mut s = session();
    let regular = run(&mut s, "interpolation.interpolateGlyph", json!({"glyph": "n", "location": {"wght": 400}}));
    assert_eq!(regular["layer"]["width"], 575.0);
    assert_eq!(first_shape_right(&regular), 175.0);
    assert_eq!(regular["placement"], "inside");
    assert_eq!(regular["location"], json!({"wght": 400.0}));

    let beyond = run(&mut s, "interpolation.interpolateGlyph", json!({"glyph": "n", "location": {"wght": 1000}}));
    assert_eq!(beyond["layer"]["width"], 725.0, "beyond the masters the layer extrapolates");
    assert_eq!(beyond["placement"], "outside");

    let empty = run(&mut s, "interpolation.interpolateGlyph", json!({"glyph": "n", "location": {}}));
    assert_eq!(empty["layer"]["width"], 500.0, "a location without the axis is at the origin, Thin");

    let below = run(&mut s, "interpolation.interpolateGlyph", json!({"glyph": "a", "location": {"wght": 400}}));
    assert_eq!(first_shape_right(&below), 175.0, "below 600 the master layers apply");
    let above = run(&mut s, "interpolation.interpolateGlyph", json!({"glyph": "a", "location": {"wght": 700}}));
    assert_eq!(first_shape_right(&above), 235.0, "from 600 up the alternate layers apply");

    let report = refused(&mut s, "interpolation.interpolateGlyph", json!({"glyph": "x", "location": {"wght": 400}}));
    assert!(report.contains("not compatible") && report.contains("shapes against"), "{report}");
    assert!(refused(&mut s, "interpolation.interpolateGlyph", json!({"glyph": "n", "location": {"wdth": 100}})).contains("wdth"));
    assert!(refused(&mut s, "interpolation.interpolateGlyph", json!({"glyph": "nope", "location": {}})).contains("nope"));
    assert!(refused(&mut s, "interpolation.interpolateGlyph", json!({"glyph": "n", "location": {"wght": 1e300}})).contains("not a usable number"));
    assert!(refused(&mut s, "interpolation.interpolateGlyph", json!({"glyph": "n", "location": {"wght": 400}, "model": "other"})).contains("model"));
}

#[test]
fn interpolation_describe_designspace_lists_axes_masters_instances_and_special_layers() {
    let mut s = session();
    let d = run(&mut s, "interpolation.describeDesignspace", json!({}));
    assert_eq!(d["setting"], Value::Null);
    assert_eq!(d["axes"][0]["tag"], "wght");
    assert_eq!(d["axes"][0]["internal"], json!({"minimum": 100.0, "default": 100.0, "maximum": 900.0}), "the origin is the first master");
    assert_eq!(d["origin"], json!({"id": "thin", "name": "Thin"}));
    assert_eq!(d["masters"].as_array().unwrap().len(), 2);
    assert_eq!(d["masters"][1]["location"], json!({"wght": 900.0}));
    let instances = d["instances"].as_array().unwrap();
    assert_eq!(instances.len(), 2, "the variable font setting is listed apart");
    assert_eq!(instances[0]["placement"], "inside");
    assert_eq!(instances[1]["name"], "ExtraBlack");
    assert_eq!(instances[1]["placement"], "extrapolated");
    assert_eq!(d["settings"], json!([{"index": 2, "name": "VF"}]));
    assert_eq!(d["intermediateLayers"], json!(["i"]));
    assert_eq!(d["alternateLayers"], json!(["a"]));

    let vf = run(&mut s, "interpolation.describeDesignspace", json!({"variableFontSetting": "VF"}));
    assert_eq!(vf["setting"], json!({"index": 2, "name": "VF"}));
    assert!(refused(&mut s, "interpolation.describeDesignspace", json!({"variableFontSetting": "Regular"})).contains("static instance"));

    run(&mut s, "font.master.setAxisLocation", json!({"masters": ["Black"], "location": {"wght": 100}}));
    let d = run(&mut s, "interpolation.describeDesignspace", json!({}));
    let problems = d["problems"].as_array().unwrap();
    assert!(problems.iter().any(|p| p["code"] == "masters-share-location" && p["masters"] == json!(["thin", "black"])), "{problems:?}");
    assert!(problems.iter().any(|p| p["code"] == "axis-without-range" && p["axis"] == "wght"), "{problems:?}");
}

#[test]
fn interpolation_check_compatibility_reports_the_first_mismatch_per_layer_pair() {
    let mut s = session();
    let all = run(&mut s, "interpolation.checkCompatibility", json!({}));
    assert_eq!(all["checked"], 4);
    assert_eq!(all["masters"], json!(["thin", "black"]), "Regular needs both masters");
    assert_eq!(all["incompatible"], json!(["x", "i"]));
    let x = &all["glyphs"][0];
    assert_eq!(x["glyph"], "x");
    assert_eq!(x["ok"], false);
    assert_eq!(x["mismatches"][0]["kind"], "shapeCount");
    let mut counts = [x["mismatches"][0]["a"].as_u64(), x["mismatches"][0]["b"].as_u64()];
    counts.sort();
    assert_eq!(counts, [Some(1), Some(2)]);

    let masters_only = run(&mut s, "interpolation.checkCompatibility", json!({"includeSpecialLayers": false}));
    assert_eq!(masters_only["incompatible"], json!(["x"]), "without the intermediate layer `i` is compatible");

    let thin = run(&mut s, "interpolation.checkCompatibility", json!({"masters": ["Thin"]}));
    assert_eq!(thin["masters"], json!(["thin"]));
    assert_eq!(thin["incompatible"], json!(["i"]), "the intermediate layer still counts against Thin");

    let n = run(&mut s, "interpolation.checkCompatibility", json!({"glyphs": ["n"], "enforceAll": true}));
    assert_eq!((n["checked"].as_u64(), n["incompatible"].clone(), n["glyphs"].clone()), (Some(1), json!([]), json!([])));
    assert!(refused(&mut s, "interpolation.checkCompatibility", json!({"masters": ["Thin"], "enforceAll": true})).contains("not both"));
}

#[test]
fn axis_mappings_add_set_nudge_remove_and_convert() {
    let mut s = session();
    let added = run(&mut s, "axisMappings.addPoint", json!({"axis": "wght", "internal": 500}));
    assert_eq!(points(&added), [(100.0, 100.0), (500.0, 500.0), (900.0, 900.0)], "a point without `external` lies on the current line");
    assert_eq!(added["index"], 1);
    assert_eq!(convert(&mut s, 500.0, "external", None), 500.0, "adding the point changes no location");

    let set = run(&mut s, "axisMappings.setPoint", json!({"axis": "wght", "index": 1, "external": 400}));
    assert_eq!(points(&set), [(100.0, 100.0), (400.0, 500.0), (900.0, 900.0)], "the internal value is kept");
    assert_eq!(convert(&mut s, 400.0, "external", None), 500.0);
    assert_eq!(convert(&mut s, 500.0, "internal", None), 400.0);
    let at = run(&mut s, "interpolation.interpolateGlyph", json!({"glyph": "n", "location": {"wght": 400}, "coordinates": "external"}));
    assert_eq!(at["location"], json!({"wght": 500.0}));
    assert_eq!(at["layer"]["width"], 600.0);

    let nudged = run(&mut s, "axisMappings.nudgePoint", json!({"axis": "wght", "index": 1, "dInternal": 10}));
    assert_eq!(points(&nudged), [(100.0, 100.0), (400.0, 510.0), (900.0, 900.0)]);

    // An edit that breaks the order is refused and changes nothing.
    let before = s.font().unwrap().info.params.clone();
    assert!(refused(&mut s, "axisMappings.setPoint", json!({"axis": "wght", "index": 1, "external": 950})).contains("mapping"));
    assert!(refused(&mut s, "axisMappings.setPoint", json!({"axis": "wght", "index": 1, "internal": 50})).contains("mapping"));
    assert!(refused(&mut s, "axisMappings.nudgePoint", json!({"axis": "wght", "index": 1, "dExternal": -300})).contains("mapping"));
    assert!(refused(&mut s, "axisMappings.removePoint", json!({"axis": "wght", "index": 3})).contains("point"));
    assert!(refused(&mut s, "axisMappings.addPoint", json!({"axis": "wdth", "internal": 3})).contains("wdth"));
    assert_eq!(s.font().unwrap().info.params, before);

    let removed = run(&mut s, "axisMappings.removePoint", json!({"axis": "wght", "index": 1}));
    assert_eq!(points(&removed), [(100.0, 100.0), (900.0, 900.0)]);
    assert_eq!(convert(&mut s, 400.0, "external", None), 400.0);
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(convert(&mut s, 400.0, "external", None), 510.0, "each edit is one undo step");
}

#[test]
fn axis_mappings_of_a_variable_font_setting_leave_the_font_alone() {
    let mut s = session();
    let added = run(&mut s, "axisMappings.addPoint", json!({"axis": "wght", "internal": 300, "external": 200, "variableFontSetting": "VF"}));
    assert_eq!(points(&added), [(100.0, 100.0), (200.0, 300.0), (900.0, 900.0)]);
    run(&mut s, "axisMappings.nudgePoint", json!({"axis": "wght", "index": 1, "dExternal": 50, "variableFontSetting": "VF"}));
    assert!(s.font().unwrap().info.params.get("Axis Mappings").is_none(), "the font's mapping is untouched");
    assert_eq!(convert(&mut s, 250.0, "external", Some("VF")), 300.0);
    assert_eq!(convert(&mut s, 250.0, "external", None), 250.0);
    assert!(
        refused(&mut s, "axisMappings.removePoint", json!({"axis": "wght", "index": 0, "variableFontSetting": "Regular"}))
            .contains("static instance")
    );
}
