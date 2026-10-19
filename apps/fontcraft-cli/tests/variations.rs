//! `fontcraft-cli variations build-hvar`, `model`, `plot` and `plan-avar`, run as a user runs
//! them.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::{Path, PathBuf};
use std::process::Command;

use fontcraft_engine::Session;
use serde_json::{Value, json};

fn cli(args: &[&str]) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_fontcraft-cli")).args(args).output().unwrap();
    (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).into_owned())
}

/// A source with two masters (wght 400 and 700) whose stem widens from 100 to 400 units.
const TWO_MASTERS: &str = r#"{
.appVersion = "3343";
.formatVersion = 3;
axes = (
{
name = Weight;
tag = wght;
}
);
familyName = "Plan Sans";
fontMaster = (
{
axesValues = (
400
);
id = m01;
name = Regular;
},
{
axesValues = (
700
);
id = m02;
name = Bold;
}
);
glyphs = (
{
glyphname = A;
layers = (
{
layerId = m01;
shapes = (
{
closed = 1;
nodes = (
(100,0,l),
(200,0,l),
(200,700,l),
(100,700,l)
);
}
);
width = 300;
},
{
layerId = m02;
shapes = (
{
closed = 1;
nodes = (
(100,0,l),
(500,0,l),
(500,700,l),
(100,700,l)
);
}
);
width = 600;
}
);
unicode = 65;
}
);
unitsPerEm = 1000;
versionMajor = 1;
versionMinor = 0;
}
"#;

fn variable_font(dir: &Path) -> PathBuf {
    let source = dir.join("PlanSans.glyphs");
    std::fs::write(&source, TWO_MASTERS).unwrap();
    let mut s = Session::new();
    s.execute("file.open", &json!({ "path": source })).unwrap();
    s.execute("file.exportVariable", &json!({ "destination": dir, "timestamp": 0 })).unwrap();
    dir.join("PlanSansVF.ttf")
}

#[test]
fn variations_model_tools_run_from_the_command_line() {
    let dir = std::env::temp_dir().join(format!("fontcraft-cli-variations-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let font = variable_font(&dir);
    let font_s = font.to_str().unwrap();

    let (code, stdout, _) = cli(&["variations", "--help"]);
    assert_eq!(code, 0);
    for word in ["build-hvar", "plan-avar", "--locations", "--no-vvar", "--weights"] {
        assert!(stdout.contains(word), "{word} in the help");
    }

    // build-hvar: the export already has HVAR, so it is rebuilt only with --replace.
    let out = dir.join("Rebuilt.ttf");
    let out_s = out.to_str().unwrap();
    let (code, _, _) = cli(&["variations", "build-hvar", font_s, "-o", out_s]);
    assert_eq!(code, 4, "nothing to build without --replace");
    let (code, stdout, stderr) = cli(&["variations", "build-hvar", font_s, "--replace", "-o", out_s, "--json"]);
    assert_eq!(code, 0, "{stderr}");
    let r: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(r["hvar"], "built");
    assert!(out.exists());
    let (code, _, _) = cli(&["variations", "build-hvar", dir.join("missing.ttf").to_str().unwrap()]);
    assert_eq!(code, 3, "an unreadable input");
    let (code, _, _) = cli(&["variations", "build-hvar"]);
    assert_eq!(code, 2);

    // model and plot from locations.
    let (code, stdout, stderr) = cli(&["variations", "model", "--locations", "0,0", "1,0", "0,1", "--json"]);
    assert_eq!(code, 0, "{stderr}");
    let r: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(r["order"], json!([0, 1, 2]));
    let (code, stdout, _) = cli(&["variations", "model", "--locations", "wght=0", "wght=1", "wght=0.5"]);
    assert_eq!(code, 0);
    assert!(stdout.contains("support {wght=(0.5, 1, 1)}"), "{stdout}");
    let (code, _, _) = cli(&["variations", "model"]);
    assert_eq!(code, 2, "no input");
    let (code, stdout, _) = cli(&["variations", "plot", "--locations", "0", "1", "0.5"]);
    assert_eq!(code, 0);
    assert!(stdout.starts_with("<svg"));
    let svg = dir.join("regions.svg");
    let (code, stdout, _) = cli(&["variations", "plot", font_s, "-o", svg.to_str().unwrap()]);
    assert_eq!(code, 0);
    assert_eq!(stdout.trim(), svg.to_str().unwrap());
    assert!(std::fs::read_to_string(&svg).unwrap().contains("class=\"panel\""));

    // plan-avar prints the designspace fragment and writes the font.
    let planned = dir.join("Planned.ttf");
    let (code, stdout, stderr) = cli(&["variations", "plan-avar", font_s, "--weights", "500,600", "--samples", "4", "-o", planned.to_str().unwrap()]);
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.starts_with("    <axis tag=\"wght\" name=\"Weight\" minimum=\"400\" default=\"400\" maximum=\"700\">"), "{stdout}");
    assert!(stdout.contains("<map input=\"500\""));
    assert!(planned.exists());
    let (code, _, _) = cli(&["variations", "plan-avar", font_s, "--pins", "wght=450", "-o", planned.to_str().unwrap()]);
    assert_eq!(code, 2, "a pin needs FROM:TO");
    let _ = std::fs::remove_dir_all(&dir);
}
