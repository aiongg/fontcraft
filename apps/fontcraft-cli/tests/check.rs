//! `fontcraft-cli check` and its three shortcuts, run as a user runs them: exit statuses
//! (`audit.exitStatus`), the reference tools' option names, and the ignore list.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::PathBuf;
use std::process::Command;

use fontcraft_engine::Session;
use serde_json::{Value, json};

fn cli(args: &[&str]) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_fontcraft-cli")).args(args).output().unwrap();
    (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).into_owned())
}

/// The nodes of an axis-aligned rectangle, for `path.addPath`.
fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Value {
    json!([{"x": x0, "y": y0}, {"x": x1, "y": y0}, {"x": x1, "y": y1}, {"x": x0, "y": y1}])
}

fn scratch(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fontcraft-cli-check-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// A source with overlapping squares in A and a tiny square in B, saved in `dir`, and its
/// static export.
fn fixtures(dir: &std::path::Path) -> (String, String) {
    let mut s = Session::with_document();
    for (glyph, nodes) in [("A", rect(0.0, 0.0, 100.0, 100.0)), ("A", rect(50.0, 50.0, 150.0, 150.0)), ("B", rect(0.0, 0.0, 3.0, 3.0))] {
        s.execute("path.addPath", &json!({"glyph": glyph, "nodes": nodes})).unwrap();
    }
    let source = dir.join("Test.fontcraft");
    s.execute("file.saveAs", &json!({"path": source})).unwrap();
    let ttf = dir.join("Test.ttf");
    s.execute("file.exportStatic", &json!({"destination": ttf, "timestamp": 0})).unwrap();
    (source.to_string_lossy().into_owned(), ttf.to_string_lossy().into_owned())
}

#[test]
fn check_exit_statuses_follow_the_fail_on_severity() {
    let dir = scratch("exit");
    let (source, ttf) = fixtures(&dir);
    let overlap = ["--check", "check.outlines.overlap"];

    // The overlap is a warning: status 0 at the default --fail-on fail, 1 at warn.
    let (code, stdout, stderr) = cli(&[&["check", source.as_str()][..], &overlap].concat());
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.lines().any(|l| l.starts_with("WARN check.outlines.overlap [f0] A")), "{stdout}");
    assert!(stdout.trim_end().ends_with("errors"), "a summary line ends the text: {stdout}");
    let (code, _, _) = cli(&[&["check", source.as_str(), "--fail-on", "warn"][..], &overlap].concat());
    assert_eq!(code, 1);
    let (code, stdout, _) = cli(&[&["check", source.as_str(), "--fail-on", "warn", "--min-severity", "fail", "--json"][..], &overlap].concat());
    assert_eq!(code, 1, "hidden results still count toward the status");
    let doc: Value = serde_json::from_str(&stdout).unwrap();
    assert!(doc["results"].as_array().unwrap().is_empty());
    let (code, stdout, _) = cli(&[&["check", source.as_str(), "--fail-on", "warn", "-q"][..], &overlap].concat());
    assert_eq!((code, stdout.as_str()), (1, ""), "--quiet prints nothing");

    // A compiled font and a family of two.
    let (code, stdout, stderr) = cli(&["check", ttf.as_str(), "--json", "--profile", "opentype"]);
    assert!(code == 0 || code == 1, "{stderr}");
    let doc: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(doc["target"]["kind"], "binary");
    let (_, stdout, _) = cli(&["check", ttf.as_str(), ttf.as_str(), "--json", "--group", "family"]);
    let doc: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(doc["target"]["kind"], "family");

    // Usage errors are 2, unreadable inputs 3.
    assert_eq!(cli(&["check", source.as_str(), "--profile", "nope"]).0, 2);
    assert_eq!(cli(&["check", source.as_str(), "--fail-on", "loud"]).0, 2);
    assert_eq!(cli(&["check", dir.join("missing.ttf").to_str().unwrap()]).0, 3);
    let junk = dir.join("junk.ttf");
    std::fs::write(&junk, b"not a font at all").unwrap();
    assert_eq!(cli(&["check", junk.to_str().unwrap()]).0, 3);

    // Ignored results never count.
    let mut s = Session::new();
    s.execute("file.open", &json!({"path": source})).unwrap();
    s.execute("audit.ignoreIssue", &json!({"check": "check.outlines.overlap", "glyph": "A"})).unwrap();
    s.execute("file.save", &json!({})).unwrap();
    let (code, stdout, _) = cli(&[&["check", source.as_str(), "--fail-on", "warn"][..], &overlap].concat());
    assert_eq!(code, 0, "{stdout}");
    assert!(stdout.contains("1 ignored"), "{stdout}");
    let (_, stdout, _) = cli(&[&["check", source.as_str(), "--include-ignored"][..], &overlap].concat());
    assert!(stdout.contains("(ignored)"), "{stdout}");

    let (code, stdout, _) = cli(&["check", "--list", "--group", "outlines"]);
    assert_eq!(code, 0);
    assert!(stdout.lines().all(|l| l.starts_with("check.outlines.")));
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn the_shortcuts_take_the_reference_option_names() {
    let dir = scratch("shortcuts");
    let (source, ttf) = fixtures(&dir);

    // check outlines: --no-overlap-checks, --min-area, -g, --glyph-list, -e with -o.
    let (code, stdout, stderr) = cli(&["check", "outlines", source.as_str(), "--json", "--fail-on", "info"]);
    assert_eq!(code, 1, "{stderr}");
    let doc: Value = serde_json::from_str(&stdout).unwrap();
    let checks: Vec<&str> = doc["results"].as_array().unwrap().iter().filter_map(|r| r["check"].as_str()).collect();
    assert!(checks.contains(&"check.outlines.overlap") && checks.contains(&"check.outlines.tinyContour"), "{checks:?}");
    let (_, stdout, _) = cli(&["check", "outlines", source.as_str(), "--json", "--no-overlap-checks", "--min-area", "4", "-g", "B"]);
    let doc: Value = serde_json::from_str(&stdout).unwrap();
    assert!(doc["results"].as_array().unwrap().iter().all(|r| r["check"] != "check.outlines.overlap"));
    assert_eq!(doc["config"]["check.outlines.tinyContour"]["minArea"], 4.0);
    let (_, stdout, _) = cli(&["check", "outlines", source.as_str(), "--json", "--glyph-list", "A", "--no-basic-checks"]);
    let doc: Value = serde_json::from_str(&stdout).unwrap();
    assert!(doc["results"].as_array().unwrap().iter().all(|r| r["location"]["glyph"] == "A"));
    let fixed = dir.join("Fixed.fontcraft");
    let (code, _, stderr) = cli(&["check", "outlines", source.as_str(), "-e", "-g", "A", "-o", fixed.to_str().unwrap(), "-q"]);
    assert!(code == 0 || code == 1, "{stderr}");
    let (code, stdout, _) = cli(&["check", "outlines", fixed.to_str().unwrap(), "-g", "A", "--fail-on", "info"]);
    assert_eq!(code, 0, "the fixed copy has no overlap left: {stdout}");

    // check family: --single, --family, --no-hints, -d.
    let (code, stdout, stderr) = cli(&["check", "family", ttf.as_str(), ttf.as_str(), "--family", "3", "--single", "5", "--no-hints", "--json"]);
    assert!(code == 0 || code == 1, "{stderr}");
    let doc: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(doc["target"]["fonts"].as_array().unwrap().len(), 2);
    assert_eq!(cli(&["check", "family", "-d", dir.to_str().unwrap(), "--single", "99"]).0, 2);

    // check interpolation: --tolerance as a percentage, --kinkiness, --show-all, --name.
    let (code, stdout, stderr) = cli(&["check", "interpolation", source.as_str(), "--tolerance", "90", "--kinkiness", "0.4", "--show-all", "--json"]);
    assert_eq!(code, 0, "{stderr}");
    let doc: Value = serde_json::from_str(&stdout).unwrap();
    assert!(doc["glyphsWithoutProblems"].as_array().is_some());
    assert_eq!(cli(&["check", "interpolation", source.as_str(), ttf.as_str()]).0, 2);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// A repeated --check runs every check it names, and `check family -l FILE` writes what it would
/// print to FILE instead of standard output.
#[test]
fn repeated_checks_add_up_and_family_logs_to_a_file() {
    let dir = scratch("repeat-log");
    let (source, ttf) = fixtures(&dir);
    let (_, stdout, stderr) = cli(&[
        "check",
        source.as_str(),
        "--json",
        "--profile",
        "strict",
        "--check",
        "check.outlines.overlap",
        "--check",
        "check.outlines.tinyContour",
    ]);
    let doc: Value = serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("{e}: {stderr}"));
    let ran: Vec<&str> = doc["checks"].as_array().unwrap().iter().filter_map(|c| c["id"].as_str()).collect();
    assert!(ran.contains(&"check.outlines.overlap") && ran.contains(&"check.outlines.tinyContour"), "{ran:?}");

    let log = dir.join("family.log");
    let (code, stdout, stderr) = cli(&["check", "family", ttf.as_str(), "--family", "3", "--single", "5", "-l", log.to_str().unwrap()]);
    assert!(code == 0 || code == 1, "{stderr}");
    assert!(stdout.is_empty(), "-l keeps standard output empty: {stdout}");
    let text = std::fs::read_to_string(&log).unwrap();
    assert!(text.trim_end().ends_with("errors"), "the log ends with the summary line: {text}");
    std::fs::remove_dir_all(&dir).unwrap();
}
