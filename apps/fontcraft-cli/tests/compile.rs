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
    assert_eq!(cli(&["compile", src, "--format", "otf,otf-cff2", "--output-dir", out]).0, 4);
    assert_eq!(cli(&["compile", src, "--output-layout", "nested", "--output-dir", out]).0, 2);
    std::fs::remove_dir_all(&dir).unwrap();
}

/// `--output-layout by-format` writes into the reference compiler's folders: a source without
/// instances builds its first master into master_ttf and master_otf.
#[test]
fn compile_writes_the_reference_folders_with_output_layout_by_format() {
    let dir = temp("by-format");
    let src = source(&dir);
    let out_dir = dir.join("fonts");
    let (code, _, stderr) = cli(&[
        "compile",
        src.to_str().unwrap(),
        "--format=ttf,otf",
        "--output-layout",
        "by-format",
        "--output-dir",
        out_dir.to_str().unwrap(),
        "--timestamp",
        "0",
    ]);
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(magic(&out_dir.join("master_ttf/Untitled-Regular.ttf")), [0, 1, 0, 0]);
    assert_eq!(magic(&out_dir.join("master_otf/Untitled-Regular.otf")), b"OTTO");
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
    assert!(stderr.contains("cbdt"), "{stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Writes `file` holding a property list with `body`.
fn write_plist(file: &Path, body: &str) {
    let text = format!(
        "<?xml version='1.0' encoding='UTF-8'?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">{body}</plist>\n"
    );
    std::fs::write(file, text).unwrap();
}

#[test]
fn a_project_compile_keeps_composites_as_the_file_places_them() {
    let dir = temp("project-composite");
    let ufo = dir.join("Synth.ufo");
    let glyphs = ufo.join("glyphs");
    std::fs::create_dir_all(&glyphs).unwrap();
    write_plist(
        &ufo.join("metainfo.plist"),
        "<dict><key>creator</key><string>org.example</string><key>formatVersion</key><integer>3</integer></dict>",
    );
    write_plist(
        &ufo.join("fontinfo.plist"),
        "<dict><key>familyName</key><string>Synth</string><key>styleName</key><string>Regular</string><key>postscriptFontName</key><string>Synth-Regular</string><key>unitsPerEm</key><integer>1000</integer></dict>",
    );
    write_plist(&ufo.join("layercontents.plist"), "<array><array><string>public.default</string><string>glyphs</string></array></array>");
    let box_outline = r#"<contour><point x="50" y="0" type="line"/><point x="50" y="700" type="line"/><point x="650" y="700" type="line"/><point x="650" y="0" type="line"/></contour>"#;
    let mark = r#"<contour><point x="-100" y="600" type="line"/><point x="-50" y="800" type="line"/><point x="0" y="800" type="line"/></contour>"#;
    let composite = r#"<component base="O"/><component base="acutecomb" xOffset="400" yOffset="150"/>"#;
    let mut contents = String::new();
    for (name, file, width, outline) in [
        (".notdef", "_notdef", 500, box_outline),
        ("O", "O_", 700, box_outline),
        ("acutecomb", "acutecomb", 0, mark),
        ("Oacute", "O_acute", 720, composite),
    ] {
        contents.push_str(&format!("<key>{name}</key><string>{file}.glif</string>"));
        let glif = format!(
            "<?xml version='1.0' encoding='UTF-8'?>\n<glyph name=\"{name}\" format=\"2\"><advance width=\"{width}\"/><outline>{outline}</outline></glyph>\n"
        );
        std::fs::write(glyphs.join(format!("{file}.glif")), glif).unwrap();
    }
    write_plist(&glyphs.join("contents.plist"), &format!("<dict>{contents}</dict>"));
    let out = dir.join("out.otf");
    let (code, _, stderr) = cli(&["compile", ufo.to_str().unwrap(), "--project-defaults", "-o", out.to_str().unwrap(), "--timestamp", "0"]);
    assert_eq!(code, 0, "{stderr}");
    let (font, _) = fontcraft_engine::io::open(&out, None).unwrap();
    // The reference keeps the advance the file gives, which is not its base's.
    assert_eq!(font.glyph("Oacute").unwrap().layers[0].width, 720.0);

    // A project file names the source and the output; the reference's short options apply.
    let project = dir.join("build");
    std::fs::create_dir_all(&project).unwrap();
    let fpr = project.join("current.fpr");
    std::fs::write(&fpr, "InputFontPath ../Synth.ufo\nOutputFontPath Synth.otf\n").unwrap();
    let (code, _, stderr) =
        cli(&["compile", "--project-file", fpr.to_str().unwrap(), "-osbOn", "8", "-osbOff", "7", "-osbOff", "9", "-lic", "L1", "--timestamp", "0"]);
    assert_eq!(code, 0, "{stderr}");
    let bytes = std::fs::read(project.join("Synth.otf")).unwrap();
    let tables = usize::from(u16::from_be_bytes([bytes[4], bytes[5]]));
    let record = (0..tables).map(|i| &bytes[12 + 16 * i..28 + 16 * i]).find(|r| &r[..4] == b"OS/2").unwrap();
    let at = usize::try_from(u32::from_be_bytes([record[8], record[9], record[10], record[11]])).unwrap();
    // Version 4, Regular and bit 8.
    assert_eq!(&bytes[at..at + 2], &[0, 4]);
    assert_eq!(&bytes[at + 62..at + 64], &[0x01, 0x40]);
    let (code, _, stderr) = cli(&["compile", "--project-file", dir.join("none.fpr").to_str().unwrap()]);
    assert_ne!(code, 0);
    assert!(stderr.contains("none.fpr"), "{stderr}");
    std::fs::remove_dir_all(&dir).unwrap();
}
