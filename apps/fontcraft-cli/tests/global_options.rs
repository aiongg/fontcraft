//! The behaviours every subcommand shares (`cli.global.*`): dry runs, no-clobber, standard input
//! and output, options files, traces and parallel builds, run as a user runs them.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::collections::BTreeMap;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use fontcraft_engine::Session;
use serde_json::{Value, json};

struct Ran {
    code: i32,
    stdout: Vec<u8>,
    stderr: String,
}

impl Ran {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.stdout).into_owned()
    }
}

fn cli_with(dir: &Path, args: &[&str], input: &[u8]) -> Ran {
    let mut child = Command::new(env!("CARGO_BIN_EXE_fontcraft-cli"))
        .args(args)
        .current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let _ = child.stdin.take().unwrap().write_all(input);
    let out = child.wait_with_output().unwrap();
    Ran { code: out.status.code().unwrap_or(-1), stdout: out.stdout, stderr: String::from_utf8_lossy(&out.stderr).into_owned() }
}

fn cli(dir: &Path, args: &[&str]) -> Ran {
    cli_with(dir, args, b"")
}

/// A folder with `Font.ttf`, `Source.fontcraft` and `Old.ufo`.
fn fixture(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fontcraft-cli-options-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = Session::with_document();
    s.execute("file.exportStatic", &json!({ "destination": dir.join("Font.ttf"), "timestamp": 0 })).unwrap();
    s.execute("file.saveAs", &json!({ "path": dir.join("Old.ufo") })).unwrap();
    s.execute("file.saveAs", &json!({ "path": dir.join("Source.fontcraft") })).unwrap();
    dir
}

/// Every file under `dir` with its bytes, and every folder.
fn contents(dir: &Path) -> BTreeMap<PathBuf, Option<Vec<u8>>> {
    let mut out = BTreeMap::new();
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            out.extend(contents(&path));
            out.insert(path, None);
        } else {
            out.insert(path.clone(), Some(std::fs::read(&path).unwrap()));
        }
    }
    out
}

/// One line per way files are written: the compile stage, single files, dumps staged together,
/// conversions staged together, sources and UFO folders, in-place edits, extracted tables.
const WRITERS: &[&[&str]] = &[
    &["compile", "Source.fontcraft", "--output-dir", "new/fonts", "--timestamp", "0"],
    &["subset", "Font.ttf", "--glyphs=space", "-o", "Small.ttf"],
    &["dump", "Font.ttf", "-o", "Font.ttx"],
    &["convert", "Font.ttf", "--to", "otf", "-o", "Font.otf"],
    &["convert", "Source.fontcraft", "New.ufo"],
    &["convert", "Source.fontcraft", "Old.ufo"],
    &["convert", "Source.fontcraft", "New.glyphs"],
    &["woff", "compress", "Font.ttf", "-o", "Font.woff2"],
    &["tables", "extract", "Font.ttf", "head", "--output-dir", "tables"],
    &["dehint", "Font.ttf", "--in-place"],
    &["run", "--in", "Source.fontcraft", "--cmd", "glyph.newGlyph", "--params", "{\"name\": \"A.alt\"}", "--save", "--export", "Run.ttf"],
];

#[test]
fn a_dry_run_writes_nothing_and_says_what_it_would_write() {
    let dir = fixture("dry");
    let before = contents(&dir);
    for line in WRITERS {
        let mut args = line.to_vec();
        args.extend(["--dry-run", "--log-file", "run.log", "--trace", "run.json"]);
        let ran = cli(&dir, &args);
        assert_eq!(ran.code, 0, "{line:?}: {}", ran.stderr);
        assert!(ran.stderr.contains("would write"), "{line:?}: {}", ran.stderr);
        assert!(contents(&dir) == before, "{line:?} changed the folder");
    }
    // A script's line keeps its own switch.
    std::fs::write(
        dir.join("s.txt"),
        "fontcraft-cli subset Font.ttf --glyphs=space -o One.ttf --dry-run\nfontcraft-cli subset Font.ttf --glyphs=space -o Two.ttf\n",
    )
    .unwrap();
    let ran = cli(&dir, &["run", "--script", "s.txt"]);
    assert_eq!(ran.code, 0, "{}", ran.stderr);
    assert!(!dir.join("One.ttf").exists() && dir.join("Two.ttf").exists());
    assert_eq!(cli(&dir, &["bench", "--dry-run", "--", "true"]).code, 2, "other programs write as they like");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn no_clobber_refuses_before_anything_is_written() {
    let dir = fixture("clobber");
    for line in WRITERS.iter().filter(|l| !l.contains(&"--in-place") && !l.contains(&"run")) {
        let made = cli(&dir, line);
        assert_eq!(made.code, 0, "{line:?}: {}", made.stderr);
        let before = contents(&dir);
        let mut args = line.to_vec();
        args.push("--no-clobber");
        let ran = cli(&dir, &args);
        assert_ne!(ran.code, 0, "{line:?} replaced a file");
        assert!(contents(&dir) == before, "{line:?} changed the folder");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn standard_input_and_output_carry_fonts() {
    let dir = fixture("streams");
    let font = std::fs::read(dir.join("Font.ttf")).unwrap();
    let ran = cli_with(&dir, &["subset", "-", "--glyphs=space", "-o", "-"], &font);
    assert_eq!(ran.code, 0, "{}", ran.stderr);
    assert_eq!(&ran.stdout[..4], [0, 1, 0, 0], "only the font is on standard output");
    let woff = cli_with(&dir, &["woff", "compress", "-", "-o", "-"], &ran.stdout);
    assert_eq!(woff.code, 0, "{}", woff.stderr);
    assert_eq!(&woff.stdout[..4], b"wOF2");
    let built = cli(&dir, &["compile", "Source.fontcraft", "--format", "otf", "-o", "-"]);
    assert_eq!(built.code, 0, "{}", built.stderr);
    assert_eq!(&built.stdout[..4], b"OTTO");
    assert!(!dir.join("<stdout>").exists() && !dir.join("<stdout>.otf").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn options_files_traces_and_jobs() {
    let dir = fixture("files");
    std::fs::write(dir.join("o.toml"), "format = [\"otf\"]\noutput = \"From.otf\"\ntimestamp = 0\n").unwrap();
    let ran = cli(&dir, &["compile", "Source.fontcraft", "--options", "o.toml", "--save-options", "saved.toml", "--trace", "t.json", "--json"]);
    assert_eq!(ran.code, 0, "{}", ran.stderr);
    let line: Value = serde_json::from_str(ran.text().trim()).unwrap();
    assert_eq!((&line["ok"], line["outputs"].as_array().unwrap().len()), (&json!(true), 1));
    assert!(dir.join("From.otf").is_file());
    let trace: Value = serde_json::from_slice(&std::fs::read(dir.join("t.json")).unwrap()).unwrap();
    assert!(trace["traceEvents"].as_array().unwrap().iter().any(|e| e["name"] == "font.compile"));
    // What was saved gives the same build; the line overrides it.
    let ran = cli(&dir, &["compile", "Source.fontcraft", "--options", "saved.toml", "-o", "Again.otf"]);
    assert_eq!(ran.code, 0, "{}", ran.stderr);
    assert_eq!(std::fs::read(dir.join("Again.otf")).unwrap(), std::fs::read(dir.join("From.otf")).unwrap());
    let ran = cli(&dir, &["subset", "Font.ttf", "--glyphs=space", "-o", "S.ttf", "--layout-features=kern", "--save-options", "s.toml"]);
    assert_eq!(ran.code, 0, "{}", ran.stderr);
    assert!(std::fs::read_to_string(dir.join("s.toml")).unwrap().contains("layout-features"));
    assert_eq!(cli(&dir, &["merge", "Font.ttf", "--layer", "x"]).code, 2);

    std::fs::copy(dir.join("Source.fontcraft"), dir.join("Other.fontcraft")).unwrap();
    let build = |jobs: &str, out: &str| {
        let ran = cli(
            &dir,
            &["compile", "Source.fontcraft", "Other.fontcraft", "missing.fontcraft", "--output-dir", out, "-j", jobs, "--timestamp", "0", "--json"],
        );
        assert_eq!(ran.code, 5, "one input is missing: {}", ran.stderr);
        ran.text().replace(out, "DIR")
    };
    let one = build("1", "one");
    assert_eq!(one, build("8", "two"), "the results do not depend on the threads");
    let last: Value = serde_json::from_str(one.lines().last().unwrap()).unwrap();
    assert_eq!((&last["input"], &last["ok"]), (&json!("missing.fontcraft"), &json!(false)));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_ufo_layer_is_read_in_place_of_the_default_one() {
    let dir = fixture("layer");
    let mut s = Session::with_document();
    let square = json!([{"x": 0, "y": 0}, {"x": 500, "y": 0}, {"x": 500, "y": 500}, {"x": 0, "y": 500}]);
    s.execute("path.addPath", &json!({"glyph": "A", "nodes": square})).unwrap();
    s.edit("a layer", |font| {
        let mut glyph = font.glyph_mut("A")?;
        let mut l = fontcraft_engine::model::Layer::new("m01", 321.0);
        l.id = "x1".into();
        l.info.name = Some("drawn".into());
        glyph.layers.push(l);
        Ok(())
    })
    .unwrap();
    s.execute("file.saveAs", &json!({ "path": dir.join("Layers.ufo") })).unwrap();
    let advance = |layer: &str, out: &str| {
        let ran = cli(&dir, &["compile", "Layers.ufo", "--layer", layer, "-o", out, "--timestamp", "0"]);
        assert_eq!(ran.code, 0, "{}", ran.stderr);
        let (font, _) = fontcraft_engine::io::open(&dir.join(out), None).unwrap();
        font.glyph("A").unwrap().layers[0].width
    };
    assert_eq!(advance("drawn", "drawn.ttf"), 321.0);
    assert_ne!(advance("processed", "processed.ttf"), 321.0, "without a processed layer the default one is read");
    assert_ne!(cli(&dir, &["compile", "Layers.ufo", "--layer", "nosuch", "-o", "x.ttf"]).code, 0);
    let _ = std::fs::remove_dir_all(&dir);
}
