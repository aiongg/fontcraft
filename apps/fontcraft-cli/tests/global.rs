//! The common options every subcommand takes, `run --script`, `help` and `commands --cli`, run as
//! a user runs them.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::{Path, PathBuf};
use std::process::Command;

use fontcraft_engine::Session;
use serde_json::{Value, json};

fn cli(args: &[&str]) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_fontcraft-cli")).args(args).env_remove("NO_COLOR").output().unwrap();
    (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).into_owned())
}

fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fontcraft-cli-global-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn font(dir: &Path) -> String {
    let ttf = dir.join("Font.ttf");
    Session::with_document().execute("file.exportStatic", &json!({ "destination": ttf, "timestamp": 0 })).unwrap();
    ttf.to_str().unwrap().to_string()
}

#[test]
fn levels_log_file_and_colour_shape_the_messages() {
    let dir = scratch("levels");
    let ttf = font(&dir);
    // Extracting a table the font lacks is a warning.
    let extract = |extra: &[&str]| {
        let mut args = vec!["tables", "extract", ttf.as_str(), "head", "GPOS"];
        args.extend_from_slice(extra);
        cli(&args)
    };

    let (code, _, stderr) = extract(&[]);
    assert_eq!(code, 0, "{stderr}");
    assert!(stderr.lines().any(|l| l.starts_with("fontcraft-cli: warning: ") && l.contains("GPOS")), "the line names its level: {stderr}");
    assert!(!stderr.contains('\x1b'), "no colour when stderr is not a terminal: {stderr}");

    let (code, _, stderr) = extract(&["-q"]);
    assert_eq!(code, 0, "{stderr}");
    assert!(!stderr.contains("GPOS"), "-q hides warnings: {stderr}");
    let (_, _, stderr) = cli(&["-q", "tables", "extract", &ttf, "head", "GPOS"]);
    assert!(!stderr.contains("GPOS"), "the common options may come before the subcommand: {stderr}");
    let (_, _, stderr) = extract(&["--log", "error"]);
    assert!(!stderr.contains("GPOS"), "--log error is -q: {stderr}");
    let (_, _, stderr) = extract(&["-q", "-v"]);
    assert!(stderr.contains("GPOS"), "a later -v raises the level again: {stderr}");

    let log = dir.join("run.log");
    let (code, _, stderr) = extract(&["--log-file", log.to_str().unwrap()]);
    assert_eq!(code, 0, "{stderr}");
    let logged = std::fs::read_to_string(&log).unwrap();
    assert_eq!(logged.trim_end(), stderr.trim_end(), "the log file gets the same lines");

    let (_, _, stderr) = extract(&["--color", "always"]);
    assert!(stderr.contains("\x1b[1;33mwarning\x1b[0m"), "{stderr:?}");
    let (_, _, stderr) = extract(&["--color", "never"]);
    assert!(!stderr.contains('\x1b'), "{stderr:?}");
    let (code, _, _) = extract(&["--color", "sometimes"]);
    assert_eq!(code, 2, "an unknown --color value is a usage error");
    let (code, _, _) = extract(&["--jobs", "many"]);
    assert_eq!(code, 2, "--jobs needs a whole number");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn verbose_shows_info_and_debug_and_timing_reports_stages() {
    let dir = scratch("verbose");
    let source = dir.join("Font.fontcraft");
    let source_s = source.to_str().unwrap();
    let (code, _, stderr) = cli(&["run", "--save", source_s]);
    assert_eq!(code, 0, "{stderr}");

    let (_, _, stderr) = cli(&["run", "--in", source_s]);
    assert!(!stderr.contains("info:"), "informational messages are hidden by default: {stderr}");
    let (code, _, stderr) = cli(&["run", "--in", source_s, "-v"]);
    assert_eq!(code, 0, "{stderr}");
    assert!(stderr.lines().any(|l| l.starts_with("fontcraft-cli: info: opening")), "-v shows info: {stderr}");
    assert!(!stderr.contains("debug:"), "-v alone shows no debug output: {stderr}");
    let (_, _, stderr) = cli(&["run", "--cmd", "file.new", "-vv"]);
    assert!(stderr.contains("fontcraft-cli: debug: running file.new"), "-vv shows debug output: {stderr}");

    let (code, _, stderr) = cli(&["run", "--cmd", "file.new", "--timing"]);
    assert_eq!(code, 0, "{stderr}");
    assert!(stderr.contains("fontcraft-cli: timing: file.new: "), "{stderr}");
    assert!(stderr.lines().last().is_some_and(|l| l.starts_with("fontcraft-cli: timing: total: ")), "{stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn version_is_a_common_option() {
    let (code, stdout, _) = cli(&["--version"]);
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), format!("fontcraft-cli {}", env!("CARGO_PKG_VERSION")));
    let (code, stdout, _) = cli(&["convert", "--version"]);
    assert_eq!(code, 0, "every subcommand prints the version");
    assert!(stdout.starts_with("fontcraft-cli "), "{stdout}");
    let (code, stdout, _) = cli(&["-V", "-v"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("\ntarget: "), "with -v the build target follows: {stdout}");
}

#[test]
fn run_scripts_stop_at_a_failing_step_unless_told_to_keep_going() {
    let dir = scratch("script");
    let script = dir.join("steps.txt");
    std::fs::write(&script, "# two steps around a failing one\nfile.new\nno.such.command {}\nfile.new\n").unwrap();
    let script_s = script.to_str().unwrap();

    let (code, stdout, stderr) = cli(&["run", "--script", script_s]);
    assert_eq!(code, 5, "a step failed after one succeeded: {stderr}");
    assert_eq!(stdout.lines().count(), 1, "nothing after the failing step ran: {stdout}");
    assert!(stderr.contains("steps.txt:3"), "the failure names its line: {stderr}");

    let (code, stdout, stderr) = cli(&["run", "--script", script_s, "--keep-going"]);
    assert_eq!(code, 5, "{stderr}");
    assert_eq!(stdout.lines().count(), 2, "--keep-going runs the other steps: {stdout}");

    std::fs::write(&script, "no.such.command\nfile.new\n").unwrap();
    let (code, stdout, _) = cli(&["run", "--script", script_s]);
    assert_eq!(code, 4, "the first step failing is the command's own failure");
    assert!(stdout.is_empty());

    std::fs::write(&script, "file.new {not json\n").unwrap();
    assert_eq!(cli(&["run", "--script", script_s]).0, 2, "a line that cannot be read is a usage error");

    let params = dir.join("params.json");
    std::fs::write(&params, r#"{"name": "A.alt"}"#).unwrap();
    let (code, stdout, stderr) = cli(&["run", "--cmd", "glyph.newGlyph", "--params-file", params.to_str().unwrap()]);
    assert_eq!(code, 0, "{stderr}");
    let line: Value = serde_json::from_str(stdout.lines().next().unwrap()).unwrap();
    assert_eq!(line["command"], "glyph.newGlyph");
    assert_eq!(cli(&["run", "--params-file", params.to_str().unwrap()]).0, 2, "--params-file must follow a --cmd");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn prefixes_name_subcommands_and_the_options_of_every_parser() {
    let (code, stdout, _) = cli(&["transf", "--help"]);
    assert_eq!(code, 0);
    assert!(stdout.starts_with("fontcraft-cli transform: ") && stdout.contains("--exclude-fd"), "{stdout}");
    let (code, _, stderr) = cli(&["co", "x"]);
    assert_eq!(code, 2);
    assert!(stderr.contains("ambiguous") && stderr.contains("convert"), "{stderr}");
    // The subcommands that read their own arguments resolve prefixes too: each message names the
    // option in full.
    for (args, message) in [
        (&["oracle", "--diff", "many"][..], "--differences must be a whole number"),
        (&["mcp", "--trans", "pigeon"], "--transport takes stdio or http"),
        (&["compile", "F.glyphs", "--output-l", "nested"], "--output-layout takes flat or by-format"),
        (&["run", "--cm"], "--cmd needs a command id"),
    ] {
        let (code, _, stderr) = cli(args);
        assert_eq!(code, 2, "{args:?}: {stderr}");
        assert!(stderr.contains(message), "{args:?}: {stderr}");
    }
    let (code, stdout, _) = cli(&["oracle", "--vers"]);
    assert_eq!(code, 0, "a prefix of a common option reaches it past a hand parser");
    assert!(stdout.starts_with("fontcraft-cli "), "{stdout}");
}

#[test]
fn help_and_the_cli_listing_cover_every_subcommand() {
    let (code, stdout, _) = cli(&["help", "--all"]);
    assert_eq!(code, 0);
    for name in ["compile", "subset", "run", "commands", "tables"] {
        assert!(stdout.contains(&format!("fontcraft-cli {name}: ")), "help --all has {name}");
    }
    assert!(stdout.contains("COMMON OPTIONS"));
    let (code, stdout, _) = cli(&["help", "compile"]);
    assert_eq!(code, 0);
    assert!(stdout.starts_with("fontcraft-cli compile: ") && stdout.contains("ENGINE COMMANDS: font.compile"), "{stdout}");
    let (code, stdout, _) = cli(&["help", "tables", "list"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("tables list"), "{stdout}");
    assert_eq!(cli(&["help", "nosuch"]).0, 2);

    let (code, stdout, _) = cli(&["commands", "--cli"]);
    assert_eq!(code, 0);
    assert!(stdout.lines().any(|l| l.starts_with("compile") && l.contains("font.compile")), "{stdout}");
    let (code, stdout, _) = cli(&["commands", "--cli", "--json"]);
    assert_eq!(code, 0);
    let list: Value = serde_json::from_str(&stdout).unwrap();
    assert!(list["subcommands"].as_array().unwrap().iter().any(|s| s["name"] == "run"));
    assert!(list["common_options"].as_array().unwrap().iter().any(|o| o["long"] == "log-file"));
}
