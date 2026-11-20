//! `fontcraft-cli proof` with the table dumper's sheets, `--show`, `--hide` and `--no-header`,
//! run as a user runs it.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::process::Command;

use fontcraft_engine::Session;
use serde_json::json;

fn cli(args: &[&str]) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_fontcraft-cli")).args(args).output().unwrap();
    (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).into_owned())
}

#[test]
fn proof_draws_the_dumper_sheets_with_shown_and_hidden_elements() {
    let dir = std::env::temp_dir().join(format!("fontcraft-cli-proof-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = Session::with_document();
    let rect = |x0: f64, x1: f64| json!([{"x": x0, "y": 0.0}, {"x": x1, "y": 0.0}, {"x": x1, "y": 700.0}, {"x": x0, "y": 700.0}]);
    for (glyph, x0, x1) in [("A", 60.0, 540.0), ("B", 70.0, 520.0)] {
        s.execute("path.addPath", &json!({"glyph": glyph, "nodes": rect(x0, x1)})).unwrap();
    }
    let font = dir.join("Spot.ttf");
    s.execute("file.exportStatic", &json!({"destination": font, "timestamp": 0})).unwrap();
    let font_s = font.to_str().unwrap();

    let (code, help, _) = cli(&["proof", "--help"]);
    assert_eq!(code, 0);
    for option in ["--show", "--hide", "--no-header", "--features", "synopsis"] {
        assert!(help.contains(option), "the help names {option}");
    }

    let out = dir.join("synopsis.svg");
    let (code, stdout, stderr) = cli(&[
        "proof",
        font_s,
        "--layout",
        "synopsis",
        "--format",
        "svg",
        "--show",
        "em-box,bbox",
        "--hide",
        "name",
        "--no-header",
        "-o",
        out.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(stdout.trim(), out.to_str().unwrap());
    let page = std::fs::read_to_string(&out).unwrap();
    // The SVG's <title> metadata still names the page; no drawn line does.
    let titled = page.lines().any(|l| l.contains("page 1 of") && !l.starts_with("<title>"));
    assert!(page.contains("data-glyph=\"A\"") && !titled, "the glyphs without a title line");

    let cmap = dir.join("cmap.pdf");
    let (code, _, stderr) = cli(&["proof", font_s, "--layout", "cmap", "-o", cmap.to_str().unwrap()]);
    assert_eq!(code, 0, "{stderr}");
    assert!(std::fs::read(&cmap).unwrap().starts_with(b"%PDF-"));

    let digi = dir.join("digi.svg");
    let (code, _, stderr) = cli(&[
        "proof",
        font_s,
        "--layout",
        "digi",
        "--format",
        "svg",
        "--glyphs",
        "A",
        "--panel",
        "1",
        "--hide",
        "points,handles",
        "-o",
        digi.to_str().unwrap(),
    ]);
    assert_eq!(code, 0, "{stderr}");

    // An unknown element, a feature the font lacks and --sort lookup outside the features layout fail and write nothing.
    let refused = dir.join("refused.svg");
    for bad in [
        vec!["--layout", "synopsis", "--hide", "sparkles"],
        vec!["--layout", "features", "--features", "smcp"],
        vec!["--layout", "palette", "--sort", "lookup"],
        vec!["--layout", "palette", "--show", "em-box"],
    ] {
        let mut args = vec!["proof", font_s, "--format", "svg", "-o", refused.to_str().unwrap()];
        args.extend(bad.iter().copied());
        let (code, _, stderr) = cli(&args);
        assert_ne!(code, 0, "{bad:?} is refused");
        assert!(!stderr.trim().is_empty(), "{bad:?} says why");
    }
    assert!(!refused.exists());
    std::fs::remove_dir_all(&dir).unwrap();
}
