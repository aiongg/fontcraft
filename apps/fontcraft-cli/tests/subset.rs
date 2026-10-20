//! `fontcraft-cli subset`, run as a user runs it: the options that print instead of subsetting
//! (`--opt?`, `--show-defaults`, `--xml`) and `--pretty-svg`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::PathBuf;
use std::process::Command;

use fontcraft_engine::Session;
use serde_json::{Value, json};

fn cli(args: &[&str]) -> (i32, String, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_fontcraft-cli")).args(args).output().unwrap();
    (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stdout).into_owned(), String::from_utf8_lossy(&out.stderr).into_owned())
}

/// A folder with `Font.ttf` (`.notdef`, `space`, …) whose `SVG ` table has one document for
/// glyph 1.
fn font() -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("fontcraft-cli-subset-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let mut s = Session::with_document();
    let plain = dir.join("Plain.ttf");
    s.execute("file.exportStatic", &json!({ "destination": plain, "timestamp": 0 })).unwrap();
    let doc = br#"<svg xmlns="http://www.w3.org/2000/svg"><g id="glyph1"><path d="M0 0"/></g></svg>"#;
    let mut table = Vec::new();
    table.extend_from_slice(&0u16.to_be_bytes());
    table.extend_from_slice(&10u32.to_be_bytes());
    table.extend_from_slice(&0u32.to_be_bytes());
    for v in [1u16, 1, 1] {
        table.extend_from_slice(&v.to_be_bytes());
    }
    table.extend_from_slice(&14u32.to_be_bytes());
    table.extend_from_slice(&(doc.len() as u32).to_be_bytes());
    table.extend_from_slice(doc);
    let svg = dir.join("svg.bin");
    std::fs::write(&svg, table).unwrap();
    let ttf = dir.join("Font.ttf");
    s.execute("tables.add", &json!({ "font": plain, "tables": [{ "tag": "SVG ", "file": svg }], "output": ttf })).unwrap();
    (dir, ttf)
}

#[test]
fn option_queries_print_the_value_at_their_place_on_the_line() {
    let (code, stdout, stderr) =
        cli(&["subset", "--name-IDs?", "--name-IDs+=7", "--name-IDs?", "--name-languages?", "--flavor?", "--no-notdef-glyph?"]);
    assert_eq!(code, 0, "queries without a font print and stop: {stderr}");
    assert_eq!(
        stdout,
        "Current setting for 'name-IDs' is: [0, 1, 2, 3, 4, 5, 6]\n\
         Current setting for 'name-IDs' is: [0, 1, 2, 3, 4, 5, 6, 7]\n\
         Current setting for 'name-languages' is: [1033]\n\
         Current setting for 'flavor' is: None\n\
         Current setting for 'notdef-glyph' is: True\n"
    );
    let (code, stdout, _) = cli(&["subset", "--drop-tables-=DSIG", "--drop-tables?", "--canonical-order?", "--font-number?"]);
    assert_eq!(code, 0);
    assert_eq!(
        stdout,
        "Current setting for 'drop-tables' is: ['JSTF', 'EBDT', 'EBLC', 'EBSC', 'PCLT', 'LTSH', 'Feat', 'Glat', 'Gloc', 'Silf', 'Sill']\n\
         Current setting for 'canonical-order' is: None\n\
         Current setting for 'font-number' is: -1\n"
    );
    let (code, _, stderr) = cli(&["subset", "--bogus?"]);
    assert_eq!(code, 2, "an unknown option asked about is a usage error: {stderr}");
}

#[test]
fn show_defaults_prints_every_option_and_subsets_nothing() {
    let (code, stdout, stderr) = cli(&["subset", "--show-defaults"]);
    assert_eq!(code, 0, "{stderr}");
    let lines: Vec<&str> = stdout.lines().collect();
    assert!(lines.contains(&"Current setting for 'hinting-tables' is: ['cvt', 'cvar', 'fpgm', 'prep', 'hdmx', 'VDMX']"), "{stdout}");
    assert!(lines.contains(&"Current setting for 'ignore-missing-unicodes' is: True"));
    assert!(lines.contains(&"Current setting for 'pretty-svg' is: False"));
    assert!(!stdout.contains("'show-defaults'"));

    let (code, stdout, _) = cli(&["subset", "missing.ttf", "A", "--no-hinting", "--show-defaults", "--json"]);
    assert_eq!(code, 0, "the font is not read");
    let v: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!((v["hinting"].clone(), v["flavor"].clone()), (json!(false), Value::Null), "the rest of the line counts");
    assert_eq!(v["name-languages"], json!(["0x409"]));
}

#[test]
fn xml_prints_the_subset_as_ttx_and_pretty_svg_indents_documents() {
    let (dir, ttf) = font();
    let ttf_s = ttf.to_str().unwrap();
    let out = dir.join("Out.ttf");
    let (code, stdout, stderr) = cli(&["subset", ttf_s, "--unicodes=20", "--pretty-svg", "--xml", "-o", out.to_str().unwrap()]);
    assert_eq!(code, 0, "{stderr}");
    assert!(stdout.starts_with("<?xml"), "{stdout}");
    assert!(stdout.contains("<ttFont") && stdout.contains("<GlyphOrder>"), "{stdout}");
    assert!(!stdout.contains(" glyphs kept"), "TTX replaces the summary line");
    let expected = b"<svg xmlns=\"http://www.w3.org/2000/svg\">\n  <g id=\"glyph1\">\n    <path d=\"M0 0\"/>\n  </g>\n</svg>\n";
    let bytes = std::fs::read(&out).unwrap();
    assert!(bytes.windows(expected.len()).any(|w| w == expected), "the subset file holds the indented document");

    let (code, _, _) = cli(&["subset", ttf_s, "--unicodes=20", "--xml", "--json"]);
    assert_eq!(code, 2, "--xml and --json together are a usage error");
    let _ = std::fs::remove_dir_all(&dir);
}
