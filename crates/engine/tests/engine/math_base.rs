//! MATH and BASE data: `font.setMathData` stores what the compiler writes to `MATH`, and it
//! survives `.glyphs` and UFO; `font.setBaselines` writes the feature code's `BASE` block and
//! reads it back; refused requests change nothing.
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use fontcraft_engine::Session;
use fontcraft_engine::cmd::math_base::{BaseAxis, BaseExtent, BaseScript, Baselines, baselines, find_base, glyph_math, math_constants};
use fontcraft_engine::model::Font;
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn refused(s: &mut Session, id: &str, p: Value) -> String {
    let before = s.doc().unwrap().font.clone();
    let e = s.execute(id, &p).expect_err(id).to_string();
    assert!(Arc::ptr_eq(&s.doc().unwrap().font, &before), "{id} {p}: a refused command changed the font");
    e
}

/// A new font with an italic letter and an integral with a larger size and three parts.
fn session() -> Session {
    let mut s = Session::new();
    run(&mut s, "file.new", json!({"familyName": "Math Sans"}));
    for (name, h) in
        [("mathf", 700), ("integral", 1000), ("integral.size1", 1600), ("integral.bot", 500), ("integral.ext", 200), ("integral.top", 500)]
    {
        run(&mut s, "glyph.newGlyph", json!({"name": name, "unicodes": [], "width": 500}));
        run(&mut s, "path.addRectangle", json!({"glyph": name, "x": 50, "y": 0, "width": 300, "height": h}));
    }
    s
}

fn math_params() -> Value {
    json!({
        "constants": {"AxisHeight": 250, "ScriptPercentScaleDown": 70, "MinConnectorOverlap": 20},
        "glyphs": {
            "mathf": {"italicCorrection": 40, "topAccent": 260, "kerning": {"topRight": [[20, 500], [10, 100]], "bottomLeft": [[-30, -50]]}},
            "integral": {
                "extendedShape": true,
                "vVariants": ["integral", "integral.size1"],
                "vAssembly": [["integral.bot", 0, 0, 100], ["integral.ext", 1, 100, 100], ["integral.top", 0, 100, 0]],
            },
        },
    })
}

/// Everything the commands read back: the constants and the two glyphs' data.
fn state(font: &Font) -> (Vec<(String, f64)>, Value, Value) {
    let master = font.default_master().unwrap().id.clone();
    let mut constants = math_constants(font, &master);
    constants.sort_by(|a, b| a.0.cmp(&b.0));
    (constants, glyph_math(font, &master, "mathf").unwrap().to_json(), glyph_math(font, &master, "integral").unwrap().to_json())
}

#[test]
fn math_data_is_set_read_back_and_survives_glyphs_and_ufo() {
    let mut s = session();
    let r = run(&mut s, "font.setMathData", math_params());
    assert_eq!(r["constants"]["AxisHeight"], json!(250.0), "{r}");
    let f = &r["glyphs"]["mathf"];
    assert_eq!((f["italicCorrection"].clone(), f["topAccent"].clone()), (json!(40.0), json!(260.0)), "{r}");
    // Points come back sorted by height.
    assert_eq!(f["kerning"]["topRight"], json!([[10.0, 100.0], [20.0, 500.0]]));
    assert_eq!(f["kerning"]["bottomLeft"], json!([[-30.0, -50.0]]));
    let i = &r["glyphs"]["integral"];
    assert_eq!(i["extendedShape"], json!(true));
    assert_eq!(i["vVariants"], json!(["integral", "integral.size1"]));
    assert_eq!(i["vAssembly"][1], json!(["integral.ext", 1, 100.0, 100.0]));
    assert_eq!(i["hVariants"], json!([]));

    // The anchors the compiler reads: math.ic sits at the advance plus the correction.
    let font = s.font().unwrap();
    let layer = font.glyph("mathf").unwrap().layers[0].clone();
    let ic = layer.anchors.iter().find(|a| a.name == "math.ic").unwrap();
    assert_eq!(ic.x, layer.width + 40.0);
    assert_eq!(layer.anchors.iter().filter(|a| a.name.starts_with("math.tr")).count(), 2);
    assert!(
        fontcraft_compile::plan::plan(font, &font.default_target().unwrap(), fontcraft_compile::Flavour::TrueType, &Default::default())
            .unwrap()
            .tables
            .contains(&"MATH".to_string())
    );

    // A later call changes only what it names; null removes.
    run(
        &mut s,
        "font.setMathData",
        json!({"constants": {"AxisHeight": null}, "glyphs": {"mathf": {"topAccent": null}, "integral": {"extendedShape": false}}}),
    );
    let (constants, f, i) = state(s.font().unwrap());
    assert_eq!(constants, [("MinConnectorOverlap".to_string(), 20.0), ("ScriptPercentScaleDown".to_string(), 70.0)]);
    assert_eq!((f["topAccent"].clone(), f["italicCorrection"].clone()), (Value::Null, json!(40.0)));
    assert_eq!((i["extendedShape"].clone(), i["vVariants"].clone()), (json!(false), json!(["integral", "integral.size1"])));
    run(&mut s, "edit.undo", json!({}));

    let expected = state(s.font().unwrap());
    let dir = std::env::temp_dir().join(format!("fontcraft-math-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for (file, params) in [("Math2.glyphs", json!({"version": 2})), ("Math3.glyphs", json!({"version": 3})), ("Math.ufo", json!({}))] {
        let mut s = session();
        run(&mut s, "font.setMathData", math_params());
        let path = dir.join(file);
        let mut p = params.clone();
        p["path"] = json!(path);
        run(&mut s, "file.saveAs", p);
        let mut back = Session::new();
        run(&mut back, "file.open", json!({"path": path}));
        assert_eq!(state(back.font().unwrap()), expected, "{file}");
        // A UFO keeps the variants in the glyph lib and the extended shapes in the font lib once
        // edited there; the data reads back the same.
        run(&mut back, "font.setMathData", json!({"glyphs": {"integral": {"hVariants": ["integral"], "extendedShape": true}}}));
        let (_, _, i) = state(back.font().unwrap());
        assert_eq!(
            (i["hVariants"].clone(), i["vVariants"].clone(), i["extendedShape"].clone()),
            (json!(["integral"]), expected.2["vVariants"].clone(), json!(true)),
            "{file}"
        );
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn refused_math_data_changes_nothing() {
    let mut s = session();
    assert!(refused(&mut s, "font.setMathData", json!({"constants": {"AxisHight": 250}})).contains("AxisHight"));
    assert!(refused(&mut s, "font.setMathData", json!({"constants": {"MinConnectorOverlap": -5}})).contains("negative"));
    assert!(refused(&mut s, "font.setMathData", json!({"constants": {"AxisHeight": 40000}})).contains("32767"));
    assert!(refused(&mut s, "font.setMathData", json!({"glyphs": {"nosuch": {"italicCorrection": 1}}})).contains("nosuch"));
    assert!(refused(&mut s, "font.setMathData", json!({"glyphs": {"integral": {"vVariants": ["integral.size9"]}}})).contains("integral.size9"));
    assert!(refused(&mut s, "font.setMathData", json!({"glyphs": {"integral": {"vAssembly": [["integral.bot", 0, 0]]}}})).contains("[0]"));
    assert!(refused(&mut s, "font.setMathData", json!({"glyphs": {"integral": {"italic": 1}}})).contains("italic"));
    assert!(refused(&mut s, "font.setMathData", json!({"glyphs": {"integral": {"kerning": {"middle": []}}}})).contains("middle"));
    // The good glyph is not changed when another one in the same call is refused.
    refused(&mut s, "font.setMathData", json!({"glyphs": {"mathf": {"italicCorrection": 5}, "integral": {"topAccent": "x"}}}));
}

const BLOCK: &str = "table BASE {\n  HorizAxis.BaseTagList ideo romn;\n  HorizAxis.BaseScriptList latn romn -120 0;\n  HorizAxis.MinMax latn dflt -300, 900;\n} BASE;\n";

#[test]
fn baselines_are_written_as_feature_code_and_read_back() {
    let mut s = session();
    let horizontal = json!({
        // Given out of order: the table lists tags sorted, and the values follow them.
        "tags": ["romn", "ideo"],
        "scripts": [{"script": "latn", "baseline": "romn", "values": [0, -120]}],
        "extents": [{"script": "latn", "min": -300, "max": 900}],
    });
    let r = run(&mut s, "font.setBaselines", json!({"horizontal": horizontal}));
    assert_eq!(r["entry"], json!("Baselines"));
    assert_eq!(r["horizontal"]["tags"], json!(["ideo", "romn"]));
    let font = s.font().unwrap();
    let entry = font.features.prefixes.iter().find(|e| e.name == "Baselines").unwrap();
    assert_eq!(entry.code, BLOCK);
    let expected = Baselines {
        horizontal: Some(BaseAxis {
            tags: vec!["ideo".into(), "romn".into()],
            scripts: vec![BaseScript { script: "latn".into(), baseline: "romn".into(), values: vec![-120, 0] }],
            extents: vec![BaseExtent { script: "latn".into(), language: "dflt".into(), min: -300, max: 900 }],
        }),
        vertical: None,
    };
    assert_eq!(baselines(font).unwrap(), Some(("Baselines".into(), expected.clone())));

    // The vertical axis is added; the horizontal one is kept.
    run(&mut s, "font.setBaselines", json!({"vertical": {"tags": ["ideo"], "scripts": [{"script": "hani", "baseline": "ideo", "values": [-120]}]}}));
    let (_, both) = baselines(s.font().unwrap()).unwrap().unwrap();
    assert_eq!(both.horizontal, expected.horizontal);
    assert_eq!(both.vertical.as_ref().unwrap().scripts[0].script, "hani");

    // Removing both axes removes the prefix the command made.
    run(&mut s, "font.setBaselines", json!({"horizontal": null, "vertical": null}));
    assert!(s.font().unwrap().features.prefixes.iter().all(|e| e.name != "Baselines"));
    assert_eq!(baselines(s.font().unwrap()).unwrap(), None);
}

#[test]
fn a_base_block_in_an_existing_entry_is_edited_in_place() {
    let mut s = session();
    let code = format!("languagesystem DFLT dflt;\n# baselines\n{BLOCK}@digits = [zero one];\n");
    run(&mut s, "font.features.addPrefix", json!({"name": "Setup", "code": code}));
    let (name, base) = baselines(s.font().unwrap()).unwrap().unwrap();
    assert_eq!(name, "Setup");
    assert_eq!(base.horizontal.as_ref().unwrap().extents.len(), 1);
    run(&mut s, "font.setBaselines", json!({"horizontal": {"tags": ["romn"], "scripts": [{"script": "latn", "baseline": "romn", "values": [0]}]}}));
    let font = s.font().unwrap();
    let entry = font.features.prefixes.iter().find(|e| e.name == "Setup").unwrap();
    assert_eq!(
        entry.code,
        "languagesystem DFLT dflt;\n# baselines\ntable BASE {\n  HorizAxis.BaseTagList romn;\n  HorizAxis.BaseScriptList latn romn 0;\n} BASE;\n@digits = [zero one];\n"
    );
    assert!(font.features.prefixes.iter().all(|e| e.name != "Baselines"));

    // A block with statements the command cannot keep is refused, not rewritten.
    let odd = "table BASE {\n  HorizAxis.BaseTagList romn;\n  HorizAxis.MinMax latn dflt -300, 900, kern -200, 1000;\n} BASE;\n";
    assert!(find_base(odd).unwrap().1.is_err());
    run(&mut s, "font.features.setCode", json!({"kind": "prefix", "name": "Setup", "code": odd}));
    assert!(refused(&mut s, "font.setBaselines", json!({"vertical": null})).contains("MinMax"));
}

#[test]
fn refused_baselines_change_nothing() {
    let mut s = session();
    let axis = |v: Value| json!({"horizontal": v});
    assert!(refused(&mut s, "font.setBaselines", axis(json!({"tags": []}))).contains("tags"));
    assert!(refused(&mut s, "font.setBaselines", axis(json!({"tags": ["romanbase"]}))).contains("romanbase"));
    assert!(
        refused(&mut s, "font.setBaselines", axis(json!({"tags": ["romn"], "scripts": [{"script": "latn", "baseline": "ideo", "values": [0]}]})))
            .contains("ideo")
    );
    assert!(
        refused(
            &mut s,
            "font.setBaselines",
            axis(json!({"tags": ["romn", "ideo"], "scripts": [{"script": "latn", "baseline": "romn", "values": [0]}]}))
        )
        .contains("2 tags")
    );
    assert!(
        refused(&mut s, "font.setBaselines", axis(json!({"tags": ["romn"], "extents": [{"script": "latn", "min": 900, "max": -300}]})))
            .contains("above")
    );
}
