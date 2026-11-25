//! `fontcraft-cli bench` and `completion`, run as a user runs them.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::PathBuf;
use std::process::Command;

use serde_json::Value;

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fontcraft-cli-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn cli(args: &[&str]) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_fontcraft-cli")).args(args).output().unwrap();
    (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).into_owned())
}

#[test]
fn bench_baseline_regression_sets_exit_status_1() {
    let dir = temp("bench");
    let saved = dir.join("saved.json");
    let saved = saved.to_str().unwrap();
    let (code, stdout, stderr) = cli(&["bench", "--runs", "2", "--warmup", "0", "--save", saved, "--json", "commands", "glyph."]);
    assert_eq!(code, 0, "{stderr}");
    let result: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(result["command"], "fontcraft-cli commands glyph.");
    assert!(result["wall_ms"]["median"].as_f64().unwrap() > 0.0);
    let stored: Value = serde_json::from_str(&std::fs::read_to_string(saved).unwrap()).unwrap();
    let stored_keys = stored.as_object().unwrap();
    assert!(stored_keys.iter().all(|(key, value)| result[key] == *value), "--save stores what --json prints: {stored} {result}");

    // A baseline far faster than any real run: a slowdown, exit status 1.
    let fast = dir.join("fast.json");
    std::fs::write(&fast, r#"{"command": "fontcraft-cli commands glyph.", "wall_ms": {"median": 0.000001}}"#).unwrap();
    let (code, stdout, stderr) = cli(&["bench", "--runs", "1", "--warmup", "0", "--baseline", fast.to_str().unwrap(), "commands", "glyph."]);
    assert_eq!(code, 1, "{stdout}{stderr}");
    assert!(stderr.contains("above the baseline"), "{stderr}");
    assert!(stdout.contains("a slowdown"), "{stdout}");

    // A baseline far slower: no slowdown.
    let slow = dir.join("slow.json");
    std::fs::write(&slow, r#"{"command": "fontcraft-cli commands glyph.", "wall_ms": {"median": 1000000000}}"#).unwrap();
    let (code, _, stderr) =
        cli(&["bench", "--runs", "1", "--warmup", "0", "--baseline", slow.to_str().unwrap(), "--threshold", "0", "commands", "glyph."]);
    assert_eq!(code, 0, "{stderr}");

    // An unreadable baseline is an input error before anything runs.
    let (code, _, _) = cli(&["bench", "--baseline", dir.join("missing.json").to_str().unwrap(), "commands"]);
    assert_eq!(code, 3);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn bench_reports_failing_runs_and_missing_reference_tools() {
    // The timed subcommand fails (unreadable input, exit status 3): bench fails with 4.
    let (code, _, stderr) = cli(&["bench", "--runs", "1", "--warmup", "0", "info", "/nonexistent/font.ttf"]);
    assert_eq!(code, 4, "{stderr}");
    assert!(stderr.contains("exited with status 3"), "{stderr}");

    // A reference tool not on PATH is reported in its entry; the run itself succeeds.
    let (code, stdout, stderr) =
        cli(&["bench", "--runs", "1", "--warmup", "0", "--json", "--compare", "fontcraft-no-such-tool --flag", "commands", "glyph."]);
    assert_eq!(code, 0, "{stderr}");
    let result: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(result["compare"][0]["command"], "fontcraft-no-such-tool --flag");
    assert_eq!(result["compare"][0]["error"], "not found on PATH");
}

#[test]
fn completion_prints_a_script_for_each_shell() {
    for (shell, marker) in [
        ("bash", "complete -o default -F"),
        ("zsh", "#compdef fontcraft-cli"),
        ("fish", "complete -c fontcraft-cli"),
        ("powershell", "Register-ArgumentCompleter"),
    ] {
        let (code, stdout, stderr) = cli(&["completion", shell]);
        assert_eq!(code, 0, "{shell}: {stderr}");
        assert!(stdout.contains(marker), "{shell}");
        for sub in ["bench", "completion", "compile", "tables", "subset"] {
            assert!(stdout.contains(sub), "{shell}: {sub}");
        }
    }
    assert_eq!(cli(&["completion", "tcsh"]).0, 2);
}
