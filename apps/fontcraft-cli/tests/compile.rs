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
