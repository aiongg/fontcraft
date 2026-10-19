//! `fontcraft-cli tables`, run as a user runs it.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::PathBuf;
use std::process::Command;

use fontcraft_engine::Session;
use serde_json::{Value, json};

fn cli(args: &[&str]) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_fontcraft-cli")).args(args).output().unwrap();
    (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).into_owned())
}

fn font() -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("fontcraft-cli-tables-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = Session::with_document();
    let ttf = dir.join("Font.ttf");
    s.execute("file.exportStatic", &json!({ "destination": ttf, "timestamp": 0 })).unwrap();
    (dir, ttf)
}

#[test]
fn tables_actions_print_results_and_exit_statuses() {
    let (dir, ttf) = font();
    let ttf_s = ttf.to_str().unwrap();

    let (code, stdout, stderr) = cli(&["tables", "extract", ttf_s, "head", "GPOS"]);
    assert_eq!(code, 0, "{stderr}");
    assert!(stderr.contains("no 'GPOS' table"), "a missing table is a warning: {stderr}");
    let r: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(r["files"][0]["tag"], "head");
    assert!(dir.join("Font.head").exists());

    let (code, _, stderr) = cli(&["tables", "checksum", ttf_s]);
    assert_eq!(code, 0, "{stderr}");
    let mut bytes = std::fs::read(&ttf).unwrap();
    bytes[16] ^= 0xFF;
    let bad = dir.join("Bad.ttf");
    std::fs::write(&bad, &bytes).unwrap();
    let (code, _, _) = cli(&["tables", "checksum", bad.to_str().unwrap()]);
    assert_eq!(code, 1, "a wrong checksum is exit status 1");
    let (code, _, stderr) = cli(&["tables", "checksum", bad.to_str().unwrap(), "--fix"]);
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(cli(&["tables", "checksum", dir.join("Bad.fixed.ttf").to_str().unwrap()]).0, 0);

    let (code, stdout, stderr) = cli(&["tables", "show", ttf_s, "head", "--depth", "1"]);
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.lines().any(|l| l.trim_start().starts_with("unitsPerEm: 1000")), "{stdout}");

    let (code, _, _) = cli(&["tables", "delete", ttf_s]);
    assert_eq!(code, 2, "a missing TAG is a usage error");
    let (code, _, _) = cli(&["tables", "add", dir.join("missing.ttf").to_str().unwrap(), "zzzz=x"]);
    assert_eq!(code, 3, "an unreadable input is exit status 3");
    let (code, _, _) = cli(&["tables", "import", ttf_s, dir.join("missing.ttx").to_str().unwrap()]);
    assert_eq!(code, 3, "an unreadable dump is exit status 3");
    let _ = std::fs::remove_dir_all(&dir);
}
