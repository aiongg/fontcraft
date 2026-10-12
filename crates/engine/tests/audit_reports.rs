//! Audit reports (M12.60): `audit.export` in each format and the four family reports on fonts
//! exported in the test. Fixtures are built in code.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::PathBuf;

use fontcraft_engine::Session;
use fontcraft_testkit::rect;
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fontcraft-audit-reports-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
fn audit_export_writes_the_last_results_in_each_format() {
    let dir = scratch("export");
    let mut s = Session::with_document();
    run(&mut s, "path.addPath", json!({"glyph": "A", "nodes": [{"x": 0, "y": 0}, {"x": 100, "y": 0}, {"x": 100, "y": 100}], "closed": false}));
    let doc = run(&mut s, "audit.run", json!({"checks": ["check.outlines.openPath"]}));
    assert_eq!(doc["results"].as_array().unwrap().len(), 1);

    let txt = dir.join("report.txt");
    let out = run(&mut s, "audit.export", json!({"path": txt}));
    assert_eq!((out["format"].as_str(), out["results"].as_u64()), (Some("text"), Some(1)), "the extension picks the format");
    let text = std::fs::read_to_string(&txt).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines.len(), 2, "{text}");
    assert!(lines[0].starts_with("FAIL check.outlines.openPath ") && lines[0].contains(" A/"), "{text}");
    assert!(lines[1].starts_with("1 failed, 0 warnings, 0 notes;"), "{text}");

    let md = dir.join("report.md");
    run(&mut s, "audit.export", json!({"path": md}));
    let md = std::fs::read_to_string(&md).unwrap();
    assert!(md.starts_with("# Audit report") && md.contains("(check.outlines.openPath)") && md.contains("- **FAIL** A/"), "{md}");

    let page = dir.join("report.html");
    run(&mut s, "audit.export", json!({"path": page}));
    assert!(std::fs::read_to_string(&page).unwrap().starts_with("<!doctype html>"));

    let json_path = dir.join("report.data");
    let out = run(&mut s, "audit.export", json!({"path": json_path}));
    assert_eq!(out["format"], "json", "an unknown extension writes JSON");
    let written: Value = serde_json::from_str(&std::fs::read_to_string(&json_path).unwrap()).unwrap();
    assert_eq!(written["schema"], doc["schema"]);
    assert_eq!(written["results"][0]["check"], "check.outlines.openPath");

    // Refusals write nothing and leave existing files alone.
    let refused = dir.join("refused.txt");
    assert!(s.execute("audit.export", &json!({"path": refused, "format": "pdf"})).is_err());
    assert!(!refused.exists());
    assert!(s.execute("audit.export", &json!({"path": txt, "noClobber": true})).is_err());
    assert_eq!(std::fs::read_to_string(&txt).unwrap(), text);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn audit_export_filters_a_given_result_document_by_severity() {
    let dir = scratch("given");
    let mut s = Session::new();
    let doc = json!({
        "schema": "fontcraft.audit/1",
        "tool": {"name": "fontcraft", "version": "0"},
        "target": {"kind": "binary", "path": "A.otf", "fonts": [{"id": "f0", "name": "A"}]},
        "profile": "universal",
        "summary": {"fail": 1, "warn": 1, "info": 0, "passed": 0, "skipped": 0},
        "checks": [],
        "results": [
            {"id": "r1", "check": "check.metrics.winAscentDescent", "code": "ascent", "severity": "fail", "message": "Too low.", "location": {"font": "f0", "table": "OS/2", "field": "usWinAscent"}},
            {"id": "r2", "check": "check.names.copyright", "code": "missing", "severity": "warn", "message": "No copyright.", "location": {"font": "f0", "table": "name"}},
        ],
    });
    let path = dir.join("fails.txt");
    let out = run(&mut s, "audit.export", json!({"path": path, "results": doc, "minSeverity": "fail"}));
    assert_eq!(out["results"], 1);
    let text = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        text,
        "FAIL check.metrics.winAscentDescent [A] OS/2.usWinAscent: Too low.\n1 failed, 0 warnings, 0 notes; 0 checks passed, 0 skipped\n"
    );
    assert!(s.execute("audit.export", &json!({"path": dir.join("x.txt"), "results": "nope"})).is_err());
    assert!(s.execute("audit.export", &json!({"path": dir.join("x.txt"), "results": doc, "target": "document"})).is_err());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn family_reports_read_compiled_fonts_from_a_folder() {
    let dir = scratch("family");
    let mut s = Session::with_document();
    run(&mut s, "path.addPath", json!({"glyph": "A", "nodes": rect(60.0, 0.0, 560.0, 700.0)}));
    let ttf = dir.join("Font.ttf");
    run(&mut s, "file.exportStatic", json!({"destination": ttf, "timestamp": 0}));
    std::fs::write(dir.join("notes.txt"), "not a font").unwrap();

    let menus = run(&mut s, "audit.report.menuNames", json!({"directory": dir}));
    let families = menus["families"].as_array().unwrap();
    assert_eq!(families.len(), 1);
    assert_eq!(families[0]["family"], "Untitled");
    let row = &families[0]["fonts"][0];
    assert_eq!(row["preferredMenu"], format!("{}/{}", row["typographicFamily"].as_str().unwrap(), row["typographicSubfamily"].as_str().unwrap()));
    assert!(menus.get("skipped").is_none(), "files that are not fonts are passed over");

    let metrics = run(&mut s, "audit.report.metrics", json!({"fonts": [ttf]}));
    let values = metrics["families"][0]["values"].as_array().unwrap();
    let glyphs = values.iter().find(|v| v["field"] == "numGlyphs").unwrap();
    assert_eq!(glyphs["same"], true, "one face agrees with itself");
    assert!(glyphs["value"].as_u64().unwrap() >= 2, "at least .notdef and A");
    assert_eq!(values.iter().find(|v| v["field"] == "fontBBox").unwrap()["fonts"].as_array().unwrap().len(), 1);

    let panose = run(&mut s, "audit.report.panose", json!({"fonts": [ttf]}));
    assert_eq!(panose["families"][0]["fonts"][0]["digits"].as_array().unwrap().len(), 10);

    let features = run(&mut s, "audit.report.featureLanguages", json!({"directory": dir}));
    assert_eq!(features["same"], true);
    assert_eq!(features["groups"][0]["tables"].as_array().unwrap().len(), 2, "GPOS and GSUB");

    for bad in
        [json!({}), json!({"fonts": [dir.join("notes.txt")]}), json!({"fonts": [dir.join("missing.ttf")]}), json!({"directory": dir.join("missing")})]
    {
        assert!(s.execute("audit.report.menuNames", &bad).is_err(), "{bad}");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_collection_with_too_many_fonts_is_refused_before_reading_them() {
    let dir = scratch("collection");
    let mut s = Session::with_document();
    let ttf = dir.join("Font.ttf");
    run(&mut s, "file.exportStatic", json!({"destination": ttf, "timestamp": 0}));
    // A collection header listing 300 fonts that all start at the same table directory.
    let font = std::fs::read(&ttf).unwrap();
    let count: u32 = 300;
    let mut ttc = b"ttcf".to_vec();
    ttc.extend(0x0001_0000u32.to_be_bytes());
    ttc.extend(count.to_be_bytes());
    let start = 12 + 4 * count;
    for _ in 0..count {
        ttc.extend(start.to_be_bytes());
    }
    ttc.extend(font);
    let path = dir.join("Many.ttc");
    std::fs::write(&path, ttc).unwrap();

    let err = s.execute("audit.report.metrics", &json!({"fonts": [path]})).err().unwrap().to_string();
    assert!(err.contains("300 fonts"), "{err}");
    let found = run(&mut s, "audit.report.metrics", json!({"directory": dir}));
    let skipped = found["skipped"].as_array().unwrap();
    assert!(skipped.iter().any(|v| v["reason"].as_str().unwrap().contains("300 fonts")), "{found}");
    std::fs::remove_dir_all(&dir).unwrap();
}

/// Measure (M12 plan § 4, M12.60): the four reports on Oswald's static CFF fonts carry the values
/// the comparison tool (AFDKO 5.0.1 comparefamily `-rn -rm -rp`, and `-rf`) printed for the same
/// folder. Run with `cargo test -p fontcraft-engine --test audit_reports -- --ignored`.
#[test]
#[ignore = "measure"]
fn comparefamily_reports_on_oswald() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/oswald/fonts/otf");
    if !dir.is_dir() {
        eprintln!("skipped: {} is missing (cargo xtask corpus --fetch)", dir.display());
        return;
    }
    let mut s = Session::new();
    let names = ["Oswald-Regular", "Oswald-Bold", "Oswald-ExtraLight", "Oswald-Light", "Oswald-Medium", "Oswald-SemiBold"];

    let menus = run(&mut s, "audit.report.menuNames", json!({"directory": dir}));
    let rows: Vec<[String; 4]> = menus["families"][0]["fonts"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| ["postScriptName", "preferredMenu", "macCompatibleMenu", "windowsCompatibleMenu"].map(|k| r[k].as_str().unwrap().to_string()))
        .collect();
    let expected: Vec<[String; 4]> = [
        ["Oswald-Regular", "Oswald/Regular", "Oswald Regular", "Oswald/Regular"],
        ["Oswald-Bold", "Oswald/Bold", "Oswald Bold", "Oswald/Bold"],
        ["Oswald-ExtraLight", "Oswald/ExtraLight", "Oswald ExtraLight", "Oswald ExtraLight/Regular"],
        ["Oswald-Light", "Oswald/Light", "Oswald Light", "Oswald Light/Regular"],
        ["Oswald-Medium", "Oswald/Medium", "Oswald Medium", "Oswald Medium/Regular"],
        ["Oswald-SemiBold", "Oswald/SemiBold", "Oswald SemiBold", "Oswald SemiBold/Regular"],
    ]
    .iter()
    .map(|r| r.map(str::to_string))
    .collect();
    assert_eq!(rows, expected);

    let metrics = run(&mut s, "audit.report.metrics", json!({"directory": dir}));
    let fam = &metrics["families"][0];
    let field = |name: &str| fam["values"].as_array().unwrap().iter().find(|v| v["field"] == name).unwrap().clone();
    let listed = |f: &Value| f["fonts"].as_array().unwrap().iter().map(|c| c["postScriptName"].as_str().unwrap().to_string()).collect::<Vec<_>>();
    assert_eq!(field("numGlyphs")["value"], 771);
    assert_eq!(field("typoMetrics")["value"], json!([1193, -289, 0, 810, 578]));
    assert_eq!(field("italicAngle")["value"], 0.0);
    assert_eq!(listed(&field("bold")), ["Oswald-Bold", "Oswald-Regular", "Oswald-ExtraLight", "Oswald-Light", "Oswald-Medium", "Oswald-SemiBold"]);
    assert_eq!(field("italic")["label"], "Not Italic");
    assert_eq!(field("fontRevision")["value"], 4.103);
    assert_eq!((field("fsType")["value"].clone(), field("fsType")["label"].clone()), (json!(0), json!("Installable embedding")));
    let bbox: Vec<Value> = field("fontBBox")["fonts"].as_array().unwrap().iter().map(|c| c["value"].clone()).collect();
    assert_eq!(
        bbox,
        [
            json!([-197, -287, 1223, 1297]),
            json!([-223, -357, 1390, 1318]),
            json!([-160, -240, 1017, 1293]),
            json!([-171, -258, 1043, 1295]),
            json!([-210, -323, 1309, 1308]),
            json!([-217, -342, 1354, 1314])
        ]
    );
    let weights = field("usWeightClass");
    assert_eq!(listed(&weights), ["Oswald-Bold", "Oswald-SemiBold", "Oswald-Medium", "Oswald-Regular", "Oswald-Light", "Oswald-ExtraLight"]);
    assert_eq!(weights["fonts"][5]["label"], "Extra-Light (Ultra-light)");
    assert_eq!((field("usWidthClass")["value"].clone(), field("usWidthClass")["label"].clone()), (json!(5), json!("Medium (normal)")));
    assert_eq!(field("underlinePosition")["value"], -100);
    assert_eq!(field("vendorId")["value"], "newt");
    assert_eq!(fam["firstFace"]["postScriptName"], "Oswald-Regular");
    assert_eq!(fam["firstFace"]["copyright"], "Copyright 2016 The Oswald Project Authors (https://github.com/googlefonts/OswaldFont)");
    assert_eq!(fam["firstFace"]["trademark"], "");

    let panose = run(&mut s, "audit.report.panose", json!({"directory": dir}));
    let fam = &panose["families"][0];
    assert_eq!(fam["base"], "Oswald-Regular");
    assert_eq!(fam["fonts"][0]["digits"], json!([0, 0, 0, 0, 0, 0, 0, 0, 0, 0]));
    assert_eq!(fam["fonts"][0]["kind"], "Any");

    let features = run(&mut s, "audit.report.featureLanguages", json!({"directory": dir}));
    assert_eq!(features["same"], true);
    assert_eq!(features["groups"][0]["fonts"], json!(names));
    let tables = features["groups"][0]["tables"].as_array().unwrap();
    let gpos = tables.iter().find(|t| t["table"] == "GPOS").unwrap();
    let gpos_rows: Vec<(&str, bool)> =
        gpos["features"].as_array().unwrap().iter().map(|f| (f["feature"].as_str().unwrap(), f["same"].as_bool().unwrap())).collect();
    assert_eq!(gpos_rows, [("kern", true), ("mark", true), ("mkmk", true)]);
    let gsub = tables.iter().find(|t| t["table"] == "GSUB").unwrap();
    assert_eq!(
        gsub["scripts"],
        json!([{"script": "DFLT", "languages": ["dflt"]}, {"script": "latn", "languages": ["AZE", "CAT", "CRT", "KAZ", "MOL", "ROM", "TAT", "TRK", "dflt"]}])
    );
    let row = |tag: &str| gsub["features"].as_array().unwrap().iter().find(|f| f["feature"] == tag).unwrap().clone();
    assert_eq!(row("ccmp")["sets"], json!([3, 3, 3, 3, 3, 3, 3, 3, 3, 12]));
    assert_eq!(row("locl")["sets"], json!([null, 9, 10, 9, 9, 11, 11, 9, 9, null]));
    assert_eq!(row("liga")["same"], true);
    let set = |id: u64| gsub["lookupSets"].as_array().unwrap().iter().find(|s| s["id"] == id).unwrap()["lookups"].clone();
    assert_eq!((set(3), set(12)), (json!([1, 2, 8, 10, 12]), json!([1, 2, 8, 10, 12, 12])));
}
