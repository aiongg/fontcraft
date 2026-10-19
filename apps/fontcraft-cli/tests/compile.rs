//! `fontcraft-cli compile` and `convert` to compiled fonts, run as a user runs them.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::process::Command;

use fontcraft_engine::Session;
use serde_json::{Value, json};

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fontcraft-cli-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A saved source with an outline in `A`.
fn source(dir: &Path) -> PathBuf {
    let mut s = Session::with_document();
    let nodes = json!([{"x": 60, "y": 0}, {"x": 560, "y": 0}, {"x": 560, "y": 700}, {"x": 60, "y": 700}]);
    s.execute("path.addPath", &json!({"glyph": "A", "nodes": nodes})).unwrap();
    let path = dir.join("Source.fontcraft");
    s.execute("file.saveAs", &json!({"path": path})).unwrap();
    path
}

fn cli(args: &[&str]) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_fontcraft-cli")).args(args).output().unwrap();
    (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).into_owned())
}

fn magic(path: &Path) -> Vec<u8> {
    std::fs::read(path).unwrap().into_iter().take(4).collect()
}

#[test]
fn compile_builds_each_format_and_container_and_plans_without_writing() {
    let dir = temp("compile");
    let src = source(&dir);
    let src = src.to_str().unwrap();
    let out_dir = dir.join("fonts");
    let out = out_dir.to_str().unwrap();

    let plan_dir = dir.join("planned");
    let (code, stdout, stderr) = cli(&["compile", src, "--format", "ttf,otf", "--output-dir", plan_dir.to_str().unwrap(), "--plan"]);
    assert_eq!(code, 0, "{stderr}");
    let plan: Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(plan["result"]["outputs"].as_array().unwrap().len(), 2);
    assert!(!plan_dir.exists(), "--plan writes nothing");

    let (code, stdout, stderr) = cli(&[
        "compile",
        src,
        "--format",
        "ttf",
        "--format",
        "otf",
        "--container",
        "plain,woff",
        "--output-dir",
        out,
        "--timestamp",
        "0",
        "--keep-overlaps",
    ]);
    assert_eq!(code, 0, "{stderr}");
    let result: Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(result["source"], src);
    assert_eq!(result["result"]["fonts"].as_array().unwrap().len(), 2);
    assert_eq!(magic(&out_dir.join("ttf/Untitled-Regular.ttf")), [0, 1, 0, 0]);
    assert_eq!(magic(&out_dir.join("otf/Untitled-Regular.otf")), b"OTTO");
    assert_eq!(magic(&out_dir.join("ttf/Untitled-Regular.woff")), b"wOFF");
    assert_eq!(magic(&out_dir.join("otf/Untitled-Regular.woff")), b"wOFF");

    // -o names the one file; convert keeps writing a TrueType font and writes CFF to .otf.
    let one = dir.join("one.otf");
    let (code, _, stderr) = cli(&["compile", src, "--format", "otf", "-o", one.to_str().unwrap()]);
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(magic(&one), b"OTTO");
    let ttf = dir.join("converted.ttf");
    let (code, _, stderr) = cli(&["convert", src, ttf.to_str().unwrap()]);
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(magic(&ttf), [0, 1, 0, 0]);
    let otf = dir.join("converted.otf");
    let (code, _, stderr) = cli(&["convert", src, otf.to_str().unwrap()]);
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(magic(&otf), b"OTTO");

    // Usage errors exit with 2, a refused build with 4.
    assert_eq!(cli(&["compile"]).0, 2);
    assert_eq!(cli(&["compile", src, src, "-o", one.to_str().unwrap()]).0, 2);
    assert_eq!(cli(&["compile", src, "--format", "variable", "--output-dir", out]).0, 4);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn compile_merges_overlaps_unless_told_to_keep_them() {
    let dir = temp("overlaps");
    let mut s = Session::with_document();
    for (x, y) in [(0, 0), (200, 200)] {
        let nodes = json!([{"x": x, "y": y}, {"x": x + 400, "y": y}, {"x": x + 400, "y": y + 400}, {"x": x, "y": y + 400}]);
        s.execute("path.addPath", &json!({"glyph": "A", "nodes": nodes})).unwrap();
    }
    let src = dir.join("Overlaps.fontcraft");
    s.execute("file.saveAs", &json!({"path": src})).unwrap();
    let src = src.to_str().unwrap();
    let paths_in_a = |file: &Path| {
        let (font, _) = fontcraft_engine::io::open(file, None).unwrap();
        let glyph = font.glyph("A").unwrap();
        glyph.layers[0].shapes.len()
    };
    for (name, extra) in [("merged.ttf", None), ("kept.ttf", Some("--keep-overlaps"))] {
        let file = dir.join(name);
        let mut args = vec!["compile", src, "-o", file.to_str().unwrap()];
        args.extend(extra);
        let (code, _, stderr) = cli(&args);
        assert_eq!(code, 0, "{stderr}");
        assert_eq!(paths_in_a(&file), if extra.is_some() { 2 } else { 1 }, "{name}");
    }
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn compile_cbdt_adds_the_bitmap_tables_of_icolor_layers() {
    let dir = temp("cbdt");
    let mut s = Session::with_document();
    s.edit("icolor layer", |font| {
        let mut l = fontcraft_engine::model::Layer::new("m01", 600.0);
        l.id = "x1".into();
        font.glyph_mut("A")?.layers.push(l);
        Ok(())
    })
    .unwrap();
    let layer = "x1";
    s.execute("layer.convertToIColor", &json!({"glyph": "A", "layer": layer, "size": 64})).unwrap();
    s.execute("layer.setIColorImage", &json!({"glyph": "A", "layer": layer, "path": "A 64.png"})).unwrap();
    let src = dir.join("Color.fontcraft");
    s.execute("file.saveAs", &json!({"path": src})).unwrap();
    let src = src.to_str().unwrap();

    let tables = |extra: &[&str]| -> Vec<String> {
        let mut args = vec!["compile", src, "--plan"];
        args.extend_from_slice(extra);
        let (code, stdout, stderr) = cli(&args);
        assert_eq!(code, 0, "{stderr}");
        let plan: Value = serde_json::from_str(stdout.trim()).unwrap();
        plan["result"]["outputs"][0]["tables"].as_array().unwrap().iter().map(|t| t.as_str().unwrap().to_string()).collect()
    };
    assert!(!tables(&[]).iter().any(|t| t == "CBDT"));
    let with = tables(&["--cbdt"]);
    assert!(with.iter().any(|t| t == "CBDT") && with.iter().any(|t| t == "CBLC"), "{with:?}");

    let (code, _, stderr) = cli(&["compile", src, "--variable", "--cbdt"]);
    assert_ne!(code, 0, "variable fonts have no CBDT option");
    assert!(stderr.contains("--variable takes only"), "{stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}
