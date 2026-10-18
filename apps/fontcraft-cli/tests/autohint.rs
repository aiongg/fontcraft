//! `fontcraft-cli autohint` and `stems`, run as a user runs them: where the hinted source goes,
//! what a compiled font allows, and the report files' layout.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::{Path, PathBuf};
use std::process::Command;

use fontcraft_engine::Session;
use serde_json::{Value, json};

fn cli(args: &[&str]) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_fontcraft-cli")).args(args).env("SOURCE_DATE_EPOCH", "0").output().unwrap();
    (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).into_owned())
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fontcraft-cli-autohint-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A source whose `l` is a stem 80 units wide from 0 to 700, saved in `dir`, and its static
/// TrueType export.
fn fixtures(dir: &Path) -> (String, String) {
    let mut s = Session::with_document();
    let nodes = json!([{"x": 100, "y": 0}, {"x": 180, "y": 0}, {"x": 180, "y": 700}, {"x": 100, "y": 700}]);
    s.execute("path.addPath", &json!({"glyph": "l", "nodes": nodes})).unwrap();
    let source = dir.join("Src.fontcraft");
    s.execute("file.saveAs", &json!({"path": source})).unwrap();
    let ttf = dir.join("Src.ttf");
    s.execute("file.exportStatic", &json!({"destination": ttf, "timestamp": 0})).unwrap();
    (source.to_string_lossy().into_owned(), ttf.to_string_lossy().into_owned())
}

fn stored_hints(path: &Path) -> usize {
    let mut s = Session::new();
    s.execute("file.open", &json!({"path": path})).unwrap();
    let font = s.font().unwrap();
    font.glyph("l").unwrap().layers.iter().map(|l| l.info.hints.len()).sum()
}

#[test]
fn autohint_writes_a_hinted_copy_and_never_changes_the_input() {
    let dir = scratch("run");
    let (source, ttf) = fixtures(&dir);
    let before = std::fs::read(&source).unwrap();

    let (code, stdout, stderr) = cli(&["autohint", &source, "--json"]);
    assert_eq!(code, 0, "{stderr}");
    let line: Value = serde_json::from_str(stdout.trim()).unwrap();
    let output = dir.join("Src.autohinted.fontcraft");
    assert_eq!(line["output"], json!(output.to_string_lossy()));
    assert_eq!((&line["written"], &line["hinter"]), (&json!(true), &json!("postscript")));
    assert!(line["result"]["hinted"].as_array().unwrap().iter().any(|h| h["glyph"] == "l"), "{line}");
    assert_eq!(std::fs::read(&source).unwrap(), before, "the input is left as it was");
    assert!(stored_hints(&output) > 0, "the hints are stored in the output");

    // A second run would replace the first output: --no-clobber refuses and writes nothing.
    let (code, _, _) = cli(&["autohint", &source, "--no-clobber"]);
    assert_eq!(code, 4);

    // --report-only hints in memory and writes nothing.
    let elsewhere = dir.join("Other.fontcraft");
    let (code, stdout, stderr) = cli(&["autohint", &source, "--report-only", "-o", &elsewhere.to_string_lossy()]);
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.lines().any(|l| l.starts_with("l ")), "{stdout}");
    assert!(!elsewhere.exists());

    // A compiled font can be analysed, not hinted in place of its source.
    let (code, _, stderr) = cli(&["autohint", &ttf]);
    assert_eq!(code, 3, "{stderr}");
    assert!(stderr.contains("--report-only"), "{stderr}");
    let (code, _, stderr) = cli(&["autohint", &ttf, "--report-only"]);
    assert_eq!(code, 0, "{stderr}");

    // Unknown glyphs in the selection are warnings, as in the reference tool.
    let (code, _, stderr) = cli(&["autohint", &source, "--report-only", "--glyphs", "l,nope"]);
    assert_eq!(code, 0, "{stderr}");
    assert!(stderr.contains("nope"), "{stderr}");
}

#[test]
fn stems_writes_report_files_in_the_reference_layout() {
    let dir = scratch("stems");
    let (source, _) = fixtures(&dir);
    let base = dir.join("report").to_string_lossy().into_owned();

    let (code, stdout, stderr) = cli(&["stems", &source, "-o", &base]);
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.contains("report.vstm.txt"), "{stdout}");
    let text = std::fs::read_to_string(format!("{base}.vstm.txt")).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    assert_eq!(lines[0], format!("Vertical Stem List for {base} on Thu Jan  1 00:00:00 1970"));
    assert_eq!(lines[1], "count    width    glyphs");
    assert!(lines.contains(&"    1       80    [l]"), "{text}");

    let (code, _, stderr) = cli(&["stems", &source, "--zones", "-o", &base]);
    assert_eq!(code, 0, "{stderr}");
    let top = std::fs::read_to_string(format!("{base}.top.txt")).unwrap();
    assert!(top.starts_with("Top Zone List for ") && top.contains("  700    [l"), "{top}");

    let (code, stdout, _) = cli(&["stems", &source, "-o", "-"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("Vertical Stem List"), "{stdout}");
    let (code, _, _) = cli(&["stems", &source, "-o", &base, "--no-clobber"]);
    assert_eq!(code, 4, "the report files exist");
}
