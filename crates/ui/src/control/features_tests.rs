//! Feature code end to end over the control channel (`docs/plans/M8.md` § 5): an agent generates the automatic
//! features, adds a feature with a number token and a predicate, breaks and fixes it, compiles, sees a small cap in an
//! Edit View tab, opens Text Preview, exports, and reads the layout tables back.

use serde_json::{Value, json};

use crate::testing::TestApp;

/// `f` before a tabular figure becomes `f.alt`; the tracking number value moves `a`.
const CALT: &str = "lookup move { pos a <$tracking 0 ${tracking * 2} 0>; } move;\nsub f' [$[name endswith \".tf\"]] by f.alt;";

fn run(t: &mut TestApp, command: &str, params: Value) -> Value {
    t.ok("engine.execute", json!({"command": command, "params": params}))
}

/// The glyphs the tab's layout shows.
fn shown(t: &mut TestApp) -> Vec<String> {
    t.app.drain();
    let layout = t.app.models.text.layout.clone().expect("the text is laid out");
    layout.placed.iter().filter_map(|p| p.glyph.as_deref().map(str::to_string)).collect()
}

fn tags(list: &Value) -> Vec<&str> {
    list.as_array().unwrap().iter().filter_map(|e| e["name"].as_str()).collect()
}

/// `text` with every run of white space as one space.
fn flat(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[test]
fn an_agent_writes_compiles_and_exports_feature_code() {
    let dir = std::env::temp_dir().join(format!("fontcraft-ui-features-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = |name: &str| dir.join(name).to_str().unwrap().to_string();
    let mut t = TestApp::new();

    // The font: letters, figures, small caps, tabular figures, two ligatures, a mark on A.
    let text = "A:Z a:z 0:9 a.sc b.sc c.sc zero.tf one.tf f.alt f_f_i f_i acutecomb";
    run(&mut t, "glyph.addGlyphs", json!({"text": text}));
    for glyph in ["A", "a", "b", "f", "a.sc", "zero.tf", "f.alt", "f_i", "acutecomb"] {
        run(&mut t, "path.addRectangle", json!({"glyph": glyph, "x": 50, "y": 0, "width": 300, "height": 500}));
    }
    run(&mut t, "anchor.add", json!({"glyph": "A", "layer": "m01", "name": "top", "x": 200, "y": 700}));
    run(&mut t, "anchor.add", json!({"glyph": "acutecomb", "layer": "m01", "name": "_top", "x": 150, "y": 500}));
    run(&mut t, "font.numbers.add", json!({"name": "tracking", "values": {"m01": 10}}));

    // Update writes the automatic features.
    run(&mut t, "font.features.update", json!({}));
    let features = run(&mut t, "font.features.list", json!({"kind": "feature"}));
    for tag in ["aalt", "smcp", "c2sc", "tnum", "liga"] {
        assert!(tags(&features["entries"]).contains(&tag), "{tag}: {features}");
    }

    // calt: the token and the predicate evaluate; a wrong glyph name is found and fixed.
    run(&mut t, "font.features.addFeature", json!({"tag": "calt", "code": CALT.replace("f.alt", "F.ALT")}));
    assert_eq!(run(&mut t, "font.features.evaluateToken", json!({"token": "${tracking * 2}"}))["value"], 20);
    let figures = run(&mut t, "font.features.evaluateToken", json!({"token": "$[name endswith \".tf\"]"}));
    assert_eq!(figures["glyphs"], json!(["zero.tf", "one.tf"]));
    assert_eq!(run(&mut t, "font.features.compile", json!({}))["ok"], false);
    let errors = run(&mut t, "font.features.listErrors", json!({"kind": "feature", "name": "calt"}));
    let error = &errors["diagnostics"][0];
    assert_eq!(error["fixes"], json!(["Replace `F.ALT` with `f.alt`"]), "{errors}");
    let fixed = run(&mut t, "font.features.applyFix", json!({"error": error["id"]}));
    assert_eq!(fixed["code"], CALT);
    let compiled = run(&mut t, "font.features.compile", json!({}));
    assert_eq!((&compiled["ok"], &compiled["preview"]), (&json!(true), &json!(true)), "{compiled}");

    // The tab shows the small cap once smcp is on.
    run(&mut t, "editView.setText", json!({"text": "aAb"}));
    assert_eq!(shown(&mut t), ["a", "A", "b"]);
    run(&mut t, "editView.toggleFeature", json!({"tag": "smcp"}));
    assert_eq!(shown(&mut t), ["a.sc", "A", "b.sc"]);
    run(&mut t, "editView.clearFeatures", json!({}));

    run(&mut t, "window.textPreview", json!({}));
    run(&mut t, "textPreview.setText", json!({"text": "fi A\u{301}"}));
    t.frames(2);
    let preview = t.ok("ui.inspect", json!({}))["textPreview"].clone();
    assert_eq!((&preview["open"], &preview["length"]), (&json!(true), &json!(5)), "{preview}");

    // The exported tables hold what the font says.
    run(&mut t, "file.exportStatic", json!({"destination": path("out.ttf"), "timestamp": 0}));
    let listed = run(&mut t, "layout.listFeatures", json!({"font": path("out.ttf")}));
    let has = |table: &str, tag: &str| listed["features"].as_array().unwrap().iter().any(|f| f["table"] == table && f["feature"] == tag);
    for (table, tag) in [("GSUB", "smcp"), ("GSUB", "c2sc"), ("GSUB", "tnum"), ("GSUB", "liga"), ("GSUB", "calt"), ("GPOS", "calt"), ("GPOS", "mark")]
    {
        assert!(has(table, tag), "{table} {tag}: {listed}");
    }
    let code = run(&mut t, "layout.decompileFeatures", json!({"font": path("out.ttf"), "classes": "inline"}));
    assert_eq!(code["unsupported"], json!([]), "{code}");
    let text = flat(code["text"].as_str().unwrap());
    for statement in ["sub f f i by f_f_i;", "sub f i by f_i;", "pos a <10 0 20 0>;", "<anchor 200 700>", "<anchor 150 500>"] {
        assert!(text.contains(statement), "{statement}: {text}");
    }
    for pair in [("a", "a.sc"), ("A", "a.sc"), ("zero", "zero.tf")] {
        let (from, to) = pair;
        assert!(text.contains(&format!("sub {from} by {to};")) || text.contains(&format!("sub [{from} ")), "{pair:?}: {text}");
    }
    let gdef = run(&mut t, "layout.decompileFeatures", json!({"font": path("out.ttf"), "tables": ["GDEF"], "classes": "inline"}));
    let gdef = flat(gdef["text"].as_str().unwrap());
    assert!(gdef.contains("GlyphClassDef") && gdef.contains("acutecomb"), "{gdef}");
    let _ = std::fs::remove_dir_all(&dir);
}
