//! `fontcraft-cli outlines` on CFF fonts, run as a user runs it.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::process::Command;

use fontcraft_engine::Session;
use serde_json::{Value, json};

fn cli(args: &[&str]) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_fontcraft-cli")).args(args).output().unwrap();
    (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).into_owned())
}

#[test]
fn outlines_repairs_cff_fonts_one_or_several_at_once() {
    let dir = std::env::temp_dir().join(format!("fontcraft-cli-outlines-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("out")).unwrap();
    let mut s = Session::with_document();
    let rect = |x0: f64, y0: f64, x1: f64, y1: f64| json!([{"x": x0, "y": y0}, {"x": x1, "y": y0}, {"x": x1, "y": y1}, {"x": x0, "y": y1}]);
    for nodes in [rect(0.0, 0.0, 100.0, 100.0), rect(50.0, 50.0, 150.0, 150.0)] {
        s.execute("path.addPath", &json!({"glyph": "A", "nodes": nodes})).unwrap();
    }
    let a = dir.join("A.otf");
    s.execute("file.exportStatic", &json!({"outlines": "cff", "removeOverlap": false, "destination": a, "timestamp": 0})).unwrap();
    let b = dir.join("B.otf");
    std::fs::copy(&a, &b).unwrap();
    let (a_s, b_s, out_s) = (a.to_str().unwrap(), b.to_str().unwrap(), dir.join("out").to_str().unwrap().to_string());

    let merged = dir.join("merged.otf");
    let (code, stdout, stderr) = cli(&["outlines", "remove-overlaps", a_s, "A", "-o", merged.to_str().unwrap()]);
    assert_eq!(code, 0, "{stderr}");
    let r: Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!((r["kind"].as_str(), r["changed"].clone()), (Some("otf"), json!(["A"])));

    // Several inputs run through the worker pool; the results keep the order given.
    let (code, stdout, stderr) = cli(&["outlines", "round", a_s, b_s, "--output-dir", &out_s, "-j", "2"]);
    assert_eq!(code, 0, "{stderr}");
    let inputs: Vec<String> = stdout.lines().map(|l| serde_json::from_str::<Value>(l).unwrap()["input"].as_str().unwrap().to_string()).collect();
    assert_eq!(inputs, vec![a_s.to_string(), b_s.to_string()]);
    assert!(dir.join("out").join("B.otf").exists());

    let missing = dir.join("missing.otf");
    let (code, stdout, stderr) = cli(&["outlines", "direction", a_s, missing.to_str().unwrap(), "--output-dir", &out_s, "-j", "2"]);
    assert_eq!(code, 5, "one input of two failed: {stderr}");
    assert_eq!(stdout.lines().count(), 1);
    assert!(stderr.contains("missing.otf"), "{stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}
