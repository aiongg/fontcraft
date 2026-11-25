//! The audit runner (M12.44): `audit.run` on compiled fonts, families and sources compiled in
//! memory, the ignore list, and the three shortcut commands. Fixtures are built in code.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::PathBuf;

use fontcraft_engine::Session;
use fontcraft_testkit::rect;
use serde_json::{Value, json};

fn run(s: &mut Session, id: &str, p: Value) -> Value {
    s.execute(id, &p).unwrap_or_else(|e| panic!("{id} {p}: {e}"))
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fontcraft-audit-runner-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A document with one glyph exported to `dir/name`.
fn exported(dir: &std::path::Path, name: &str) -> (Session, PathBuf) {
    let mut s = Session::with_document();
    run(&mut s, "path.addPath", json!({"glyph": "A", "nodes": rect(60.0, 0.0, 560.0, 700.0)}));
    let ttf = dir.join(name);
    run(&mut s, "file.exportStatic", json!({"destination": ttf, "timestamp": 0}));
    (s, ttf)
}

fn results(r: &Value) -> &Vec<Value> {
    r["results"].as_array().unwrap()
}

fn check<'a>(r: &'a Value, id: &str) -> Option<&'a Value> {
    r["checks"].as_array().unwrap().iter().find(|c| c["id"] == id)
}

#[test]
fn a_compiled_font_by_path_runs_binary_and_outline_checks() {
    let dir = scratch("binary");
    let (mut s, ttf) = exported(&dir, "Font.ttf");
    let r = run(&mut s, "audit.run", json!({"target": ttf, "includePassed": true}));
    assert_eq!(r["target"]["kind"], "binary");
    assert_eq!(r["target"]["fonts"].as_array().unwrap().len(), 1);
    assert_eq!(r["target"]["fonts"][0]["path"], json!(ttf.to_string_lossy()));
    assert!(r.get("document").is_none(), "a file target is not the open document");
    assert!(results(&r).iter().all(|x| x["location"]["font"] == "f0"));
    assert!(results(&r).iter().any(|x| x["location"]["table"] == "name"), "binary checks ran: {}", r["summary"]);
    // The outline checks read the decoded file; checks that need source data are skipped.
    assert_eq!(check(&r, "check.outlines.overlap").unwrap()["status"], "pass");
    let strict = run(&mut s, "audit.run", json!({"target": ttf, "profile": "strict", "checks": ["check.hinting.stemsUndefined"]}));
    assert_eq!(check(&strict, "check.hinting.stemsUndefined").unwrap()["reason"], "the check needs data only a source keeps");
    // Family checks need two fonts.
    let fam = run(&mut s, "audit.run", json!({"target": ttf, "scope": ["family"]}));
    let skipped = fam["checks"].as_array().unwrap();
    assert!(!skipped.is_empty() && skipped.iter().all(|c| c["status"] == "skipped"));
    assert!(s.execute("audit.run", &json!({"target": dir.join("missing.ttf")})).is_err());
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn path_direction_follows_the_truetype_convention_on_a_compiled_glyf_font() {
    let dir = scratch("direction");
    let (mut s, ttf) = exported(&dir, "Font.ttf");
    let id = "check.outlines.pathDirection";
    let source = run(&mut s, "audit.run", json!({"checks": [id], "includePassed": true}));
    assert_eq!(check(&source, id).unwrap()["status"], "pass", "the source winds its outer contour counter-clockwise");
    // The compiler reverses the contour for glyf, where outer contours run clockwise.
    let compiled = run(&mut s, "audit.run", json!({"target": ttf, "checks": [id], "includePassed": true}));
    assert_eq!(check(&compiled, id).unwrap()["status"], "pass", "{}", compiled["results"]);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn a_folder_of_fonts_is_one_family() {
    let dir = scratch("family");
    let (mut s, _) = exported(&dir, "One.ttf");
    run(&mut s, "file.exportStatic", json!({"destination": dir.join("Two.ttf"), "timestamp": 0}));
    std::fs::write(dir.join("notes.txt"), "not a font").unwrap();
    let r = run(&mut s, "audit.run", json!({"target": dir, "scope": ["family"]}));
    assert_eq!(r["target"]["kind"], "family");
    assert_eq!(r["target"]["fonts"].as_array().unwrap().len(), 2, "the text file is passed over");
    // Two fonts with the same names: the family checks see both.
    let both = results(&r).iter().find(|x| x["location"]["fonts"].as_array().is_some_and(|f| f.len() == 2));
    assert!(both.is_some(), "a finding names both fonts: {}", serde_json::to_string(&r["results"]).unwrap());
    let listed = run(&mut s, "audit.run", json!({"target": [dir.join("One.ttf"), dir.join("Two.ttf")], "scope": ["family"]}));
    assert_eq!(listed["summary"], r["summary"]);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn binary_findings_on_a_source_name_the_source_field() {
    let mut s = Session::with_document();
    run(&mut s, "path.addPath", json!({"glyph": "A", "nodes": rect(60.0, 0.0, 560.0, 700.0)}));
    let r = run(&mut s, "audit.run", json!({"scope": ["binary"]}));
    assert_eq!(r["target"]["kind"], "source");
    let fonts = r["target"]["fonts"].as_array().unwrap();
    assert_eq!((fonts[0]["format"].as_str(), fonts[1]["format"].as_str()), (Some("source"), Some("ttf")), "the instance compiled in memory");
    let licence = results(&r).iter().find(|x| x["location"]["nameId"] == 14).expect("the empty licence URL is reported");
    assert_eq!(licence["location"]["sourceField"], "font.licenseURL");
    assert_eq!(licence["location"]["instance"], fonts[1]["instance"]);
    assert_eq!(licence["location"]["font"], "f1");
}

#[test]
fn project_values_reach_the_compiled_font_checks() {
    let mut s = Session::with_document();
    run(&mut s, "path.addPath", json!({"glyph": "A", "nodes": rect(60.0, 0.0, 560.0, 700.0)}));
    let select = json!({"profile": "strict", "checks": ["check.tables.fsType"], "scope": ["binary"]});
    let before = run(&mut s, "audit.run", select.clone());
    assert_eq!(check(&before, "check.tables.fsType").unwrap()["status"], "skipped", "no fsType value yet");
    run(&mut s, "audit.config.set", json!({"values": {"fsType": 12}}));
    let after = run(&mut s, "audit.run", select);
    let found: Vec<&Value> = results(&after).iter().filter(|x| x["check"] == "check.tables.fsType").collect();
    assert_eq!(found.len(), 1, "{after}");
    assert_eq!(found[0]["code"], "unexpected-value");
}

#[test]
fn ignored_results_are_counted_apart_and_hidden() {
    let mut s = Session::with_document();
    run(&mut s, "path.addPath", json!({"glyph": "A", "nodes": rect(0.0, 0.0, 100.0, 100.0)}));
    run(&mut s, "path.addPath", json!({"glyph": "A", "nodes": rect(50.0, 50.0, 150.0, 150.0)}));
    let only = json!({"checks": ["check.outlines.overlap"]});
    let before = run(&mut s, "audit.run", only.clone());
    assert_eq!(before["summary"]["warn"].as_u64().unwrap() + before["summary"]["fail"].as_u64().unwrap(), 1);
    run(&mut s, "audit.ignoreIssue", json!({"result": before["results"][0]["id"], "reason": "intended"}));
    let r = run(&mut s, "audit.run", only.clone());
    assert_eq!(r["summary"]["ignored"], 1);
    assert_eq!((r["summary"]["warn"].as_u64(), r["summary"]["fail"].as_u64()), (Some(0), Some(0)));
    assert!(results(&r).is_empty());
    let mut all = only.clone();
    all["includeIgnored"] = json!(true);
    let r = run(&mut s, "audit.run", all);
    assert_eq!(results(&r)[0]["ignored"], true);
    let kept = run(&mut s, "audit.results", json!({}));
    assert!(results(&kept).is_empty());
    assert_eq!(kept["summary"]["ignored"], 1);
    // Removing the entry brings the result back without a new run.
    run(&mut s, "audit.unignoreIssue", json!({"check": "check.outlines.overlap", "glyph": "A"}));
    let kept = run(&mut s, "audit.results", json!({}));
    assert_eq!((results(&kept).len(), &kept["summary"]["ignored"]), (1, &json!(0)));
}

#[test]
fn check_outlines_takes_the_reference_options() {
    let mut s = Session::with_document();
    run(&mut s, "path.addPath", json!({"glyph": "A", "nodes": rect(0.0, 0.0, 100.0, 100.0)}));
    run(&mut s, "path.addPath", json!({"glyph": "A", "nodes": rect(50.0, 50.0, 150.0, 150.0)}));
    run(&mut s, "path.addPath", json!({"glyph": "B", "nodes": rect(0.0, 0.0, 3.0, 3.0)}));
    let r = run(&mut s, "audit.checkOutlines", json!({"includePassed": true}));
    let ids: Vec<&str> = r["checks"].as_array().unwrap().iter().filter_map(|c| c["id"].as_str()).collect();
    for id in ["check.outlines.overlap", "check.outlines.tinyContour"] {
        assert!(results(&r).iter().any(|x| x["check"] == id), "{id} in {ids:?}");
    }
    assert_eq!(r["config"]["check.outlines.tinyContour"]["minArea"], 25.0);
    let r = run(&mut s, "audit.checkOutlines", json!({"overlapChecks": false, "minArea": 4, "tolerance": 1, "glyphs": ["B"]}));
    assert!(results(&r).iter().all(|x| x["check"] != "check.outlines.overlap" && x["location"]["glyph"] == "B"));
    assert_eq!(r["config"]["check.outlines.flatCurve"]["tolerance"], 1.0);
    let r = run(&mut s, "audit.checkOutlines", json!({"basicChecks": false, "glyphs": "A", "fix": true, "keepOriginal": "none"}));
    assert!(r["fix"]["applied"].as_array().is_some_and(|a| !a.is_empty()), "{}", r["fix"]);
    let after = run(&mut s, "audit.checkOutlines", json!({"glyphs": ["A"]}));
    assert!(results(&after).iter().all(|x| x["check"] != "check.outlines.overlap"));
    for bad in [json!({"minArea": -1}), json!({"keepOriginal": "keep"}), json!({"fix": true, "target": "/no/such.ufo"}), json!({"decimal": 3})] {
        assert!(s.execute("audit.checkOutlines", &bad).is_err(), "{bad}");
    }
}

/// As the reference checker: at tolerance 0 only exactly collinear lines, and every one of them.
#[test]
fn check_outlines_lists_every_exactly_collinear_line() {
    let mut s = Session::with_document();
    let point = |x: f64, y: f64| json!({"x": x, "y": y});
    let nearly = [(0.0, 0.0), (200.0, 1.0), (400.0, 0.0), (400.0, 300.0), (0.0, 300.0)].map(|(x, y)| point(x, y));
    run(&mut s, "path.addPath", json!({"glyph": "A", "nodes": nearly}));
    let mut comb: Vec<Value> = (0..=110).map(|i| point(f64::from(i) * 10.0, 0.0)).collect();
    comb.extend([point(1100.0, 300.0), point(0.0, 300.0)]);
    run(&mut s, "path.addPath", json!({"glyph": "B", "nodes": comb}));
    let r = run(&mut s, "audit.checkOutlines", json!({"overlapChecks": false}));
    let of = |glyph: &str| results(&r).iter().filter(|x| x["check"] == "check.outlines.collinearLines" && x["location"]["glyph"] == glyph).count();
    assert_eq!((of("A"), of("B")), (0, 109));
}

/// After a fix, each changed layer is rounded half to even (three decimals with `decimal`) and
/// its contours go back in the reference's order: AFDKO 5.0.1's checkoutlinesufo -e on the same
/// outline gives the low square, the top square, then the union of the two rectangles.
#[test]
fn check_outlines_fix_rounds_and_restores_the_contour_order() {
    for decimal in [false, true] {
        let mut s = Session::with_document();
        // path.addPath rounds to the grid; the fix must see the fractional coordinates.
        s.edit("grid", |font| {
            std::sync::Arc::make_mut(&mut font.info).settings.grid_length = Some(0.0);
            Ok(())
        })
        .unwrap();
        for nodes in
            [rect(400.2, 600.0, 500.4, 700.6), rect(100.3, 100.0, 300.5, 300.25), rect(200.7, 200.0, 350.5, 400.5), rect(10.0, 10.0, 60.0, 60.5)]
        {
            run(&mut s, "path.addPath", json!({"glyph": "A", "nodes": nodes}));
        }
        let r = run(&mut s, "audit.checkOutlines", json!({"glyphs": ["A"], "fix": true, "keepOriginal": "none", "decimal": decimal}));
        assert_eq!(r["cleanup"][0]["glyph"], "A", "{}", r["cleanup"]);
        let font = s.font().unwrap();
        let master = font.default_master().unwrap().id.clone();
        let layer = font.glyph("A").unwrap().layer(&master).unwrap();
        let paths: Vec<Vec<(f64, f64)>> = layer
            .shapes
            .iter()
            .filter_map(|x| match x {
                fontcraft_model::Shape::Path(p) => Some(p.nodes.iter().map(|n| (n.x, n.y)).collect()),
                _ => None,
            })
            .collect();
        let (low, top) = if decimal { ((60.0, 60.5), (500.4, 700.6)) } else { ((60.0, 60.0), (500.0, 701.0)) };
        assert_eq!(paths.len(), 3, "{paths:?}");
        assert!(paths[0].contains(&low) && paths[0].len() == 4, "decimal {decimal}: {paths:?}");
        assert!(paths[1].contains(&top) && paths[1].len() == 4, "decimal {decimal}: {paths:?}");
        assert_eq!(paths[2].len(), 8, "{paths:?}");
        let fractional = paths.iter().flatten().any(|(x, y)| x.fract() != 0.0 || y.fract() != 0.0);
        assert_eq!(fractional, decimal, "{paths:?}");
        assert_eq!(run(&mut s, "edit.undo", json!({}))["undone"], "Check Outlines");
        let back = s.font().unwrap().glyph("A").unwrap().layer(&master).unwrap().shapes.len();
        assert_eq!(back, 4, "the fix and the clean-up are one undo step");
    }
}

#[test]
fn check_interpolation_keeps_the_problem_order() {
    let mut s = Session::with_document();
    s.edit("setup", |font| {
        let mut m = font.default_master()?.clone();
        m.id = "m02".into();
        m.name = "Bold".into();
        std::sync::Arc::make_mut(&mut font.masters).push(m);
        for i in 0..font.glyphs.len() {
            if let Some(mut g) = font.glyphs.get_mut_at(i) {
                g.layers.push(fontcraft_model::Layer::new("m02", 600.0));
            }
        }
        font.glyph_mut("A")?.layers[0].shapes.push(fontcraft_model::Shape::Path(fontcraft_model::Path::rect(0.0, 0.0, 10.0, 10.0)));
        Ok(())
    })
    .unwrap();
    let r = run(&mut s, "audit.checkInterpolation", json!({"tolerance": 90, "kinkiness": 0.4, "showAll": true}));
    assert_eq!(r["config"]["check.variations.contourOrder"]["tolerance"], 0.9, "90 is read as a percentage");
    assert_eq!(r["config"]["check.variations.kink"]["kinkiness"], 0.4);
    let found: Vec<&str> = results(&r).iter().filter_map(|x| x["check"].as_str()).collect();
    assert!(found.contains(&"check.variations.pathCount"), "{found:?}");
    assert!(r["glyphsWithoutProblems"].as_array().unwrap().iter().all(|g| g != "A"));
    let order =
        ["check.variations.glyphMissingInMaster", "check.variations.openPathInMaster", "check.variations.pathCount", "check.variations.nodeCount"];
    let ranks: Vec<usize> = found.iter().map(|c| order.iter().position(|o| o == c).unwrap_or(order.len())).collect();
    assert!(ranks.windows(2).all(|w| w[0] <= w[1]), "{found:?}");
    for bad in [json!({"tolerance": 5}), json!({"kinkiness": 0}), json!({"target": ["a.ttf", "b.ttf"]})] {
        assert!(s.execute("audit.checkInterpolation", &bad).is_err(), "{bad}");
    }
}

#[test]
fn compare_family_maps_the_reference_test_numbers() {
    let dir = scratch("compare");
    let (mut s, one) = exported(&dir, "One.ttf");
    let two = dir.join("Two.ttf");
    run(&mut s, "file.exportStatic", json!({"destination": two, "timestamp": 0}));
    let r = run(&mut s, "audit.compareFamily", json!({"fonts": [one, two], "familyTests": [3], "singleTests": [5], "includePassed": true}));
    let ids: Vec<&str> = r["checks"].as_array().unwrap().iter().filter_map(|c| c["id"].as_str()).collect();
    assert!(ids.iter().all(|id| ["check.family.consistentFamilyName", "check.names.postScriptNameConsistency"].contains(id)), "{ids:?}");
    let all = run(&mut s, "audit.compareFamily", json!({"directory": dir, "noHints": true}));
    assert_eq!(all["target"]["fonts"].as_array().unwrap().len(), 2);
    assert!(all["checks"].as_array().unwrap().iter().all(|c| c["id"] != "check.hinting.missingStemHints"));
    for bad in [json!({}), json!({"directory": dir, "singleTests": [35]}), json!({"directory": dir, "singleTests": [29], "familyTests": []})] {
        assert!(s.execute("audit.compareFamily", &bad).is_err(), "{bad}");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn audit_folder_and_glyph_limits_refuse_the_run() {
    let dir = scratch("limits");
    let (mut s, ttf) = exported(&dir, "Font.ttf");
    // A face whose maxp claims 65,535 glyphs, listed nine times: past the run's 524,288
    // decoded glyphs before any of them is decoded.
    let mut font = fontcraft_tables::read(&std::fs::read(&ttf).unwrap(), 0).unwrap();
    let mut maxp = font.get(*b"maxp").unwrap().to_vec();
    maxp[4..6].copy_from_slice(&u16::MAX.to_be_bytes());
    font.insert(*b"maxp", maxp);
    let big = dir.join("Big.ttf");
    std::fs::write(&big, fontcraft_tables::write(&font, &fontcraft_tables::WriteOptions::default()).unwrap()).unwrap();
    let e = s.execute("audit.run", &json!({"target": vec![&big; 9]})).unwrap_err().to_string();
    assert!(e.contains("glyphs; audit fewer fonts"), "{e}");

    // A folder with more than 4,096 entries is refused before any file is read.
    let many = dir.join("many");
    std::fs::create_dir_all(&many).unwrap();
    for i in 0..4097 {
        std::fs::write(many.join(format!("{i}.txt")), "").unwrap();
    }
    let e = s.execute("audit.run", &json!({"target": many})).unwrap_err().to_string();
    assert!(e.contains("more than 4096 entries"), "{e}");
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn check_outlines_all_examines_every_glyph_again() {
    let mut s = Session::with_document();
    run(&mut s, "path.addPath", json!({"glyph": "A", "nodes": rect(0.0, 0.0, 100.0, 100.0)}));
    run(&mut s, "path.addPath", json!({"glyph": "A", "nodes": rect(50.0, 50.0, 150.0, 150.0)}));
    let select = json!({"glyphs": ["A"]});
    let first = run(&mut s, "audit.checkOutlines", select.clone());
    let at = fontcraft_audit::layers_examined();
    let again = run(&mut s, "audit.checkOutlines", select);
    assert_eq!(fontcraft_audit::layers_examined(), at, "an unchanged glyph keeps its results");
    assert_eq!(results(&again).len(), results(&first).len());
    let at = fontcraft_audit::layers_examined();
    let all = run(&mut s, "audit.checkOutlines", json!({"glyphs": ["A"], "all": true}));
    assert!(fontcraft_audit::layers_examined() > at, "`all` examines the glyph again");
    assert_eq!(results(&all).len(), results(&first).len());
}

#[test]
fn check_outlines_quiet_keeps_the_counts_only() {
    let mut s = Session::with_document();
    run(&mut s, "path.addPath", json!({"glyph": "A", "nodes": rect(0.0, 0.0, 100.0, 100.0)}));
    run(&mut s, "path.addPath", json!({"glyph": "A", "nodes": rect(50.0, 50.0, 150.0, 150.0)}));
    let loud = run(&mut s, "audit.checkOutlines", json!({}));
    let quiet = run(&mut s, "audit.checkOutlines", json!({"quiet": true}));
    assert!(!results(&loud).is_empty());
    assert!(quiet.get("results").is_none(), "{quiet}");
    assert_eq!(quiet["summary"], loud["summary"]);
}

#[test]
fn compare_family_adds_the_reports_asked_for() {
    let dir = scratch("compare-reports");
    let (mut s, one) = exported(&dir, "One.ttf");
    let two = dir.join("Two.ttf");
    run(&mut s, "file.exportStatic", json!({"destination": two, "timestamp": 0}));
    let r = run(&mut s, "audit.compareFamily", json!({"directory": dir, "familyTests": [3], "singleTests": [], "reports": ["menuNames", "panose"]}));
    let names = run(&mut s, "audit.report.menuNames", json!({"directory": dir}));
    assert_eq!(r["reports"]["menuNames"], names, "the same table as the report command");
    assert!(r["reports"]["panose"]["families"].is_array(), "{r}");
    assert!(r["reports"].get("metrics").is_none());
    let plain = run(&mut s, "audit.compareFamily", json!({"fonts": [one, two], "familyTests": [3], "singleTests": []}));
    assert!(plain.get("reports").is_none());
    assert!(s.execute("audit.compareFamily", &json!({"directory": dir, "reports": ["glyphs"]})).is_err());
    std::fs::remove_dir_all(&dir).unwrap();
}
