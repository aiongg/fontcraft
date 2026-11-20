//! `fontcraft-cli autohint`, `stems` and `dehint`, run as a user runs them: where the hinted
//! source goes, what a compiled font allows, the reference's hinting options, the report files'
//! layout, and clearing a source's hints.
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

/// The `horizontal` count of `glyph`'s first entry under `hinted` in a JSON result line.
fn horizontal(line: &Value, glyph: &str) -> Option<u64> {
    line["result"]["hinted"].as_array().unwrap().iter().find(|h| h["glyph"] == glyph).and_then(|h| h["horizontal"].as_u64())
}

/// A UFO written as text, as otfautohint reads it: zones at the baseline and at 700, stems of
/// 80, and `l` and `o` each a stem from x 100 to 180, 0 to 700 high.
fn ufo(dir: &Path) -> String {
    let font = dir.join("T.ufo");
    std::fs::create_dir_all(font.join("glyphs")).unwrap();
    let plist = |body: &str| {
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\">\n{body}\n</plist>\n"
        )
    };
    let write = |name: &str, text: String| std::fs::write(font.join(name), text).unwrap();
    write("metainfo.plist", plist("<dict><key>creator</key><string>test</string><key>formatVersion</key><integer>3</integer></dict>"));
    write("layercontents.plist", plist("<array><array><string>public.default</string><string>glyphs</string></array></array>"));
    write(
        "fontinfo.plist",
        plist(
            "<dict><key>familyName</key><string>T</string><key>styleName</key><string>Regular</string>\
             <key>unitsPerEm</key><integer>1000</integer><key>ascender</key><integer>800</integer>\
             <key>descender</key><integer>-200</integer><key>xHeight</key><integer>500</integer>\
             <key>capHeight</key><integer>700</integer>\
             <key>postscriptBlueValues</key><array><integer>-10</integer><integer>0</integer><integer>700</integer><integer>710</integer></array>\
             <key>postscriptStemSnapH</key><array><integer>80</integer></array>\
             <key>postscriptStemSnapV</key><array><integer>80</integer></array></dict>",
        ),
    );
    write("glyphs/contents.plist", plist("<dict><key>l</key><string>l.glif</string><key>o</key><string>o.glif</string></dict>"));
    for name in ["l", "o"] {
        let points: String =
            [(100, 0), (180, 0), (180, 700), (100, 700)].iter().map(|(x, y)| format!("<point x=\"{x}\" y=\"{y}\" type=\"line\"/>")).collect();
        write(
            &format!("glyphs/{name}.glif"),
            format!(
                "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<glyph name=\"{name}\" format=\"2\"><advance width=\"600\"/><outline><contour>{points}</contour></outline></glyph>\n"
            ),
        );
    }
    font.to_string_lossy().into_owned()
}

#[test]
fn autohint_hints_with_the_override_sets_beside_each_input() {
    let dir = scratch("fontinfo");
    let font = ufo(&dir);
    // A set for `o` alone: from then on the glyphs no set takes have no alignment zones, so
    // `l` loses the ghost hints at its bottom and top (otfautohint 5.0.1 on this UFO:
    // hstem 21 -21 and hstem 700 -20 without the file, hstem 0 700 with it; `o` keeps both).
    std::fs::write(
        dir.join("fontinfo"),
        "begin FDDict Tall\n  OrigEmSqUnits 1000\n  FontName Tall\n  BaselineOvershoot -10\n  BaselineYCoord 0\n  CapHeight 700\n  CapOvershoot 10\n  DominantV [80]\n  DominantH [80]\nend FDDict Tall\n\nbegin GlyphSet Tall\n  o\nend GlyphSet Tall\n",
    )
    .unwrap();

    let (code, stdout, stderr) = cli(&["autohint", &font, "--report-only", "--json", "--ignore-fontinfo"]);
    assert_eq!(code, 0, "{stderr}");
    let plain: Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(horizontal(&plain, "l"), Some(2), "{plain}");

    let (code, stdout, stderr) = cli(&["autohint", &font, "--report-only", "--json"]);
    assert_eq!(code, 0, "{stderr}");
    let overridden: Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(horizontal(&overridden, "o"), Some(2), "{overridden}");
    assert_ne!(horizontal(&overridden, "l"), Some(2), "{overridden}");

    // --fontinfo-file names the same sets from anywhere; text that cannot be read changes nothing.
    let moved = dir.join("sets.txt");
    std::fs::rename(dir.join("fontinfo"), &moved).unwrap();
    let (code, stdout, stderr) = cli(&["autohint", &font, "--report-only", "--json", "--fontinfo-file", &moved.to_string_lossy()]);
    assert_eq!(code, 0, "{stderr}");
    let named: Value = serde_json::from_str(stdout.trim()).unwrap();
    assert_eq!(horizontal(&named, "l"), horizontal(&overridden, "l"));
    std::fs::write(&moved, "begin GlyphSet Missing\n  o\nend GlyphSet Missing\n").unwrap();
    let (code, _, _) = cli(&["autohint", &font, "--fontinfo-file", &moved.to_string_lossy()]);
    assert_ne!(code, 0, "a glyph set without its FDDict is refused");
    assert!(!dir.join("T.autohinted.ufo").exists());
}

/// A source with two masters whose `t` is a cross of two overlapping bars and whose `l` is a
/// stem, saved in `dir`.
fn two_master_source(dir: &Path) -> String {
    use fontcraft_engine::model::plist::openstep::parse_value;
    use fontcraft_engine::model::{self as m, AlignmentZone, NodeKind, Param};
    let mut s = Session::with_document();
    s.edit("setup", |font| {
        let master = &mut std::sync::Arc::make_mut(&mut font.masters)[0];
        master.zones = vec![AlignmentZone { pos: 0.0, size: -10.0 }, AlignmentZone { pos: 700.0, size: 10.0 }];
        for key in ["postscriptStemSnapH", "postscriptStemSnapV"] {
            master.params.0.push(Param::new(key, parse_value("(80)").unwrap()));
        }
        let mut bold = font.default_master()?.clone();
        bold.id = "m02".into();
        bold.name = "Bold".into();
        std::sync::Arc::make_mut(&mut font.masters).push(bold);
        for i in 0..font.glyphs.len() {
            if let Some(mut g) = font.glyphs.get_mut_at(i) {
                g.layers.push(m::Layer::new("m02", 600.0));
            }
        }
        let bar = |x0: f64, y0: f64, x1: f64, y1: f64| {
            let n = |x, y| m::Node::new(x, y, NodeKind::Line);
            m::Shape::Path(m::Path::new(true, vec![n(x0, y0), n(x1, y0), n(x1, y1), n(x0, y1)]))
        };
        for (layer, w) in [("m01", 80.0), ("m02", 120.0)] {
            font.glyph_mut("t")?.layer_mut(layer).unwrap().shapes = vec![bar(0.0, 300.0, 500.0, 300.0 + w), bar(210.0, 0.0, 210.0 + w, 700.0)];
            font.glyph_mut("l")?.layer_mut(layer).unwrap().shapes = vec![bar(100.0, 0.0, 100.0 + w, 700.0)];
        }
        Ok(())
    })
    .unwrap();
    let source = dir.join("Masters.fontcraft");
    s.execute("file.saveAs", &json!({"path": source})).unwrap();
    source.to_string_lossy().into_owned()
}

#[test]
fn autohint_passes_the_overlap_and_master_options_through() {
    let dir = scratch("overlap");
    let source = two_master_source(&dir);
    let masters_of = |line: &Value, glyph: &str| -> usize {
        line["result"]["hinted"].as_array().unwrap().iter().filter(|h| h["glyph"] == glyph && h["vertical"].as_u64() > Some(0)).count()
    };
    for options in [
        &[][..],
        // --allow-changes is refused with --report-only (the reference refuses the pair).
        &["--force-overlap", "--keep-conflicts", "--decimal", "-w"][..],
        &["--overlap-glyphs", "t"][..],
        &["--force-no-overlap"][..],
        &["--loose-overlap-mapping"][..],
    ] {
        let mut args = vec!["autohint", source.as_str(), "--report-only", "--json"];
        args.extend_from_slice(options);
        let (code, stdout, stderr) = cli(&args);
        assert_eq!(code, 0, "{options:?}: {stderr}");
        assert!(!stderr.contains("could not be removed"), "{options:?}: {stderr}");
        let line: Value = serde_json::from_str(stdout.trim()).unwrap();
        assert_eq!(masters_of(&line, "t"), 2, "{options:?}: the cross is hinted in both masters: {line}");
        assert_eq!(masters_of(&line, "l"), 2, "{options:?}: {line}");
    }
    let (code, _, stderr) = cli(&["autohint", &source, "--overlap-glyphs", "t", "--loose-overlap-mapping"]);
    assert_eq!(code, 2, "one option of the overlap group: {stderr}");
}

#[test]
fn dehint_clears_the_stored_hints_of_a_source() {
    let dir = scratch("dehint");
    let (source, _) = fixtures(&dir);
    let (code, _, stderr) = cli(&["autohint", &source]);
    assert_eq!(code, 0, "{stderr}");
    let hinted = dir.join("Src.autohinted.fontcraft");
    assert!(stored_hints(&hinted) > 0);
    let before = std::fs::read(&hinted).unwrap();

    let (code, stdout, stderr) = cli(&["dehint", &hinted.to_string_lossy(), "--json"]);
    assert_eq!(code, 0, "{stderr}");
    let line: Value = serde_json::from_str(stdout.trim()).unwrap();
    let output = dir.join("Src.autohinted.dehinted.fontcraft");
    assert_eq!(line["result"]["output"], json!(output.to_string_lossy()));
    assert!(line["result"]["layersCleared"].as_u64() > Some(0), "{line}");
    assert_eq!(stored_hints(&output), 0, "the output has no hints");
    assert_eq!(std::fs::read(&hinted).unwrap(), before, "the input is left as it was");

    // Options for a compiled font's font-wide data are reported, not applied, on a source.
    let (code, _, stderr) = cli(&["dehint", &hinted.to_string_lossy(), "--keep-flex", "-o", &dir.join("Kept.fontcraft").to_string_lossy()]);
    assert_eq!(code, 0, "{stderr}");
    assert!(stderr.contains("compiled fonts"), "{stderr}");
    // A selection that leaves `l` out leaves its hints.
    let (code, _, stderr) =
        cli(&["dehint", &hinted.to_string_lossy(), "--exclude-glyphs", "l", "-o", &dir.join("Partial.fontcraft").to_string_lossy()]);
    assert_eq!(code, 0, "{stderr}");
    assert!(stored_hints(&dir.join("Partial.fontcraft")) > 0);
}

#[test]
fn autohint_hints_the_charstrings_of_a_compiled_cff_font() {
    let dir = scratch("otf");
    let mut s = Session::with_document();
    let nodes = json!([{"x": 100, "y": 0}, {"x": 180, "y": 0}, {"x": 180, "y": 700}, {"x": 100, "y": 700}]);
    s.execute("path.addPath", &json!({"glyph": "l", "nodes": nodes})).unwrap();
    let otf = dir.join("Src.otf");
    s.execute("file.exportStatic", &json!({"destination": otf, "outlines": "cff", "autohint": false, "timestamp": 0})).unwrap();
    let before = std::fs::read(&otf).unwrap();
    let otf = otf.to_string_lossy().into_owned();

    let (code, stdout, stderr) = cli(&["autohint", &otf, "--json"]);
    assert_eq!(code, 0, "{stderr}");
    let line: Value = serde_json::from_str(stdout.trim()).unwrap();
    let output = dir.join("Src.autohinted.otf");
    assert_eq!((&line["output"], &line["written"]), (&json!(output.to_string_lossy()), &json!(true)), "{line}");
    assert!(line["result"]["hinted"].as_array().unwrap().iter().any(|h| h["glyph"] == "l"), "{line}");
    assert_eq!(std::fs::read(&otf).unwrap(), before, "the input is left as it was");
    assert_ne!(std::fs::read(&output).unwrap(), before, "the output has hints");

    // --report-only lists the glyphs it would hint and writes nothing.
    let elsewhere = dir.join("Other.otf");
    let (code, stdout, stderr) = cli(&["autohint", &otf, "--report-only", "-o", &elsewhere.to_string_lossy(), "--glyphs", "l"]);
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.lines().any(|l| l.starts_with("l:")), "{stdout}");
    assert!(!elsewhere.exists());

    // The overlap options hint against the overlap-free outline and write the font.
    let (code, stdout, stderr) = cli(&["autohint", &otf, "--force-overlap", "-o", &elsewhere.to_string_lossy(), "--json"]);
    assert_eq!(code, 0, "{stderr}");
    let line: Value = serde_json::from_str(stdout.trim()).unwrap();
    assert!(line["result"]["hinted"].as_array().unwrap().iter().any(|h| h["glyph"] == "l"), "{line}");
    assert!(elsewhere.exists());
    let listed = dir.join("Listed.otf");
    let (code, _, stderr) = cli(&["autohint", &otf, "--overlap-glyphs", "l", "-o", &listed.to_string_lossy()]);
    assert_eq!(code, 0, "{stderr}");
    assert!(listed.exists());
}
