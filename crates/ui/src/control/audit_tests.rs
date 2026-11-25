//! The audit end to end over the control channel (`docs/plans/M12.md` § 5): a flawed font is audited and exported,
//! its faults are fixed in the source, and it is exported again and compared.

use std::path::Path;

use fontcraft_engine::Session;
use fontcraft_engine::model::plist::{Number, Value as Plist};
use fontcraft_engine::model::{Alignment, Component, Shape};
use serde_json::{Value, json};

use crate::testing::TestApp;

const CHECKS: [&str; 4] = ["check.outlines.openPath", "check.components.duplicate", "check.metrics.markWidths", "check.metrics.winAscentDescent"];

fn run(t: &mut TestApp, command: &str, params: Value) -> Value {
    t.ok("engine.execute", json!({"command": command, "params": params}))
}

fn checks_of(doc: &Value) -> Vec<String> {
    let mut v: Vec<String> = doc["results"].as_array().unwrap().iter().map(|r| r["check"].as_str().unwrap().to_string()).collect();
    v.sort();
    v.dedup();
    v
}

fn commands(list: &Value) -> Vec<&str> {
    list.as_array().unwrap().iter().map(|a| a["command"].as_str().unwrap()).collect()
}

/// A font with an open path (D), the same component twice (B), a combining mark with a width and a Windows ascent
/// below the letters.
fn flawed() -> Session {
    let mut s = Session::with_document();
    let square = |x0: f64, y0: f64, x1: f64, y1: f64| json!([{"x": x0, "y": y0}, {"x": x1, "y": y0}, {"x": x1, "y": y1}, {"x": x0, "y": y1}]);
    for (glyph, nodes, closed) in [("A", square(60.0, -10.0, 560.0, 700.0), true), ("D", square(60.0, 0.0, 500.0, 650.0), false)] {
        s.execute("path.addPath", &json!({"glyph": glyph, "nodes": nodes, "closed": closed})).unwrap();
    }
    s.execute("glyph.newGlyph", &json!({"name": "acutecomb", "unicodes": [0x301], "width": 300})).unwrap();
    // The export zeroes the width of a glyph stored as a non-spacing mark.
    s.execute("glyph.setCategory", &json!({"glyphs": ["acutecomb"], "category": "Mark", "subCategory": "Spacing Combining"})).unwrap();
    s.edit("flaws", |font| {
        let mut twice = Component::new("A", 0.0, 0.0);
        twice.info.alignment = Alignment::Disabled;
        font.glyph_mut("B")?.layers[0].shapes = vec![Shape::Component(twice.clone()), Shape::Component(twice)];
        std::sync::Arc::make_mut(&mut font.masters)[0].params.set("winAscent", Plist::Number(Number::int(100)));
        Ok(())
    })
    .unwrap();
    s
}

#[test]
fn a_flawed_font_is_audited_fixed_and_exported_again() {
    let dir = std::env::temp_dir().join(format!("fontcraft-ui-audit-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let at = |name: &str| dir.join(name).to_str().unwrap().to_string();
    let mut t = TestApp::with(flawed());
    run(&mut t, "file.saveAs", json!({"path": at("original.fontcraft")}));
    run(&mut t, "file.exportStatic", json!({"destination": at("before.ttf"), "timestamp": 0}));

    // The export clips on Windows; the source shows every fault.
    let export = run(&mut t, "audit.run", json!({"target": at("before.ttf"), "profile": "universal", "checks": CHECKS}));
    assert!(checks_of(&export).contains(&"check.metrics.winAscentDescent".to_string()), "{export}");
    let found = run(&mut t, "audit.run", json!({"profile": "strict", "checks": CHECKS}));
    let mut all = CHECKS.map(str::to_string).to_vec();
    all.sort();
    assert_eq!(checks_of(&found), all, "{found}");

    // The safe fixes by result, then the two that change the design.
    let safe: Vec<&Value> =
        found["results"].as_array().unwrap().iter().filter(|r| r["fix"]["safe"] == true && r["fix"]["available"] == true).collect();
    let ids: Vec<&Value> = safe.iter().map(|r| &r["id"]).collect();
    let fixed = run(&mut t, "audit.fix", json!({"results": ids, "runId": found["runId"]}));
    let applied = commands(&fixed["applied"]);
    assert!(applied.contains(&"edit.delete") && applied.contains(&"spacing.setWidth"), "{fixed}");
    assert!(!applied.contains(&"path.closeOpenPaths"), "{fixed}");
    let rest = run(&mut t, "audit.fix", json!({"profile": "strict", "checks": CHECKS, "safeOnly": false}));
    let applied = commands(&rest["applied"]);
    assert!(applied.contains(&"path.closeOpenPaths") && applied.contains(&"font.info.syncVerticalMetrics"), "{rest}");
    let again = run(&mut t, "audit.run", json!({"profile": "strict", "checks": CHECKS}));
    assert_eq!(checks_of(&again), Vec::<String>::new(), "{again}");

    run(&mut t, "file.saveAs", json!({"path": at("fixed.fontcraft")}));
    run(&mut t, "file.exportStatic", json!({"destination": at("after.ttf"), "timestamp": 0}));
    let export = run(&mut t, "audit.run", json!({"target": at("after.ttf"), "profile": "universal", "checks": CHECKS}));
    assert_eq!(checks_of(&export), Vec::<String>::new(), "{export}");
    let kept = run(&mut t, "audit.results", json!({"target": at("after.ttf")}));
    assert_eq!((&kept["runId"], &kept["summary"]["stale"]), (&export["runId"], &json!(0)));

    let os2 = run(&mut t, "tables.show", json!({"font": at("after.ttf"), "tables": ["OS/2"]}));
    let fields = os2["tables"][0]["tree"]["children"].as_array().unwrap().clone();
    let ascent = fields.iter().find(|f| f["name"].as_str().unwrap().replace('_', "").eq_ignore_ascii_case("usWinAscent")).unwrap();
    assert_eq!(ascent["value"].as_str().map(str::to_string).unwrap_or_else(|| ascent["value"].to_string()), "700");

    let diff = run(&mut t, "source.diff", json!({"a": at("original.fontcraft"), "b": at("fixed.fontcraft")}));
    let changed: Vec<&str> = diff["changes"].as_array().unwrap().iter().filter_map(|c| c["glyph"].as_str()).collect();
    for glyph in ["B", "D", "acutecomb"] {
        assert!(changed.contains(&glyph), "{glyph}: {diff}");
    }
    assert!(!changed.contains(&"A"), "{diff}");

    let keep = json!({"unicodes": ["0020-007E"]});
    run(&mut t, "font.subset", json!({"font": at("after.ttf"), "keep": keep, "ignoreMissingUnicodes": true, "output": at("latin.ttf")}));
    run(&mut t, "woff.compress", json!({"font": at("latin.ttf"), "output": at("latin.woff2")}));
    assert!(std::fs::read(Path::new(&at("latin.woff2"))).unwrap().starts_with(b"wOF2"));
    let compared = run(&mut t, "font.diff", json!({"a": at("before.ttf"), "b": at("after.ttf")}));
    assert_eq!(compared["identical"], false);
    assert_eq!(compared["tables"]["OS/2"]["status"], "different", "{}", compared["differing"]);
}
