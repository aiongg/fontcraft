//! Opt-in tests that run the reference compilers: `cargo test -p fontcraft-oracle --features
//! oracle`. Each test skips, with a message, when its tool is not installed.
#![cfg(feature = "oracle")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::{Path, PathBuf};

use fontcraft_compile::{Options, compile_ttf};
use fontcraft_engine::io::{SaveFormat, open, save};
use fontcraft_model::{Font, Path as Contour, Shape};
use fontcraft_oracle::{AllowList, Flavour, Hint, Oracle, Ours, OursError, Outcome, Settings, Status, check, compare};

fn sample() -> Font {
    let mut f = Font::new("Oracle Fixture");
    f.glyph_mut("A").unwrap().layers[0].shapes.push(Shape::Path(Contour::rect(60.0, 0.0, 560.0, 700.0)));
    f.glyph_mut("H").unwrap().layers[0].shapes.push(Shape::Path(Contour::rect(80.0, 0.0, 520.0, 700.0)));
    f
}

fn temp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fontcraft-oracle-it-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn tool(oracle: Oracle) -> Option<PathBuf> {
    let found = fontcraft_oracle::tools::locate(oracle, None);
    if found.is_none() {
        eprintln!("skipped: {} is not installed ({})", oracle.name(), oracle.install_hint());
    }
    found
}

fn settings(oracle: Oracle, flavour: Flavour, tool: PathBuf) -> Settings {
    Settings {
        oracle,
        flavour,
        tool,
        timestamp: fontcraft_oracle::DEFAULT_TIMESTAMP,
        timeout: fontcraft_oracle::DEFAULT_TIMEOUT,
        keep: None,
        shown: 20,
        allow: AllowList::builtin().unwrap(),
    }
}

/// FontCraft's side: the saved source read back and compiled, as the CLI does it.
fn ours(path: &Path, flavour: Flavour) -> Result<Ours, OursError> {
    let (font, warnings) = open(path, None).map_err(|e| OursError::Failed(e.to_string()))?;
    let hint = Hint { family: font.family_name.clone(), style: font.default_master().unwrap().name.clone() };
    if flavour == Flavour::Cff {
        return Err(OursError::Unsupported { message: "not yet".into(), hint });
    }
    let compiled = compile_ttf(&font, &Options { timestamp: fontcraft_oracle::DEFAULT_TIMESTAMP, ..Default::default() })
        .map_err(|e| OursError::Failed(e.to_string()))?;
    Ok(Ours { bytes: compiled.bytes, warnings, hint })
}

/// What holds today and must keep holding: both builds succeed, the glyph sets agree, and the
/// code points and advances match. Other tables are M2.2's to bring to parity.
fn assert_comparable(oracle: Oracle, source: &Path, label: &str) {
    let Some(tool) = tool(oracle) else { return };
    let report = check(source, label, &settings(oracle, Flavour::TrueType, tool), || ours(source, Flavour::TrueType));
    eprintln!("{}", fontcraft_oracle::render(&report));
    assert_eq!(report.outcome, Outcome::Compared, "{:?}", report.messages);
    assert!(report.source_reading.is_empty(), "{:?}", report.source_reading);
    let status = |tag: &str| report.tables.iter().find(|t| t.tag == tag).map(|t| t.status);
    assert!(matches!(status("cmap"), Some(Status::Identical | Status::Equivalent)));
    let hmtx = report.tables.iter().find(|t| t.tag == "hmtx").unwrap();
    assert!(hmtx.differences.iter().all(|d| !d.key.ends_with("advance")), "{hmtx:?}");
    for tag in ["head", "hhea", "maxp", "OS/2", "name", "post", "glyf"] {
        assert!(status(tag).is_some(), "no {tag}");
    }
}

#[test]
fn fontc_builds_a_glyphs_source_we_can_compare() {
    let dir = temp("fontc-glyphs");
    let path = dir.join("Fixture.glyphs");
    save(&sample(), &path, SaveFormat::Glyphs).unwrap();
    assert_comparable(Oracle::Fontc, &path, "Fixture.glyphs");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fontc_builds_a_ufo_we_can_compare() {
    let dir = temp("fontc-ufo");
    let path = dir.join("Fixture.ufo");
    save(&sample(), &path, SaveFormat::Ufo).unwrap();
    assert_comparable(Oracle::Fontc, &path, "Fixture.ufo");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fontmake_builds_a_glyphs_source_we_can_compare() {
    let dir = temp("fontmake-glyphs");
    let path = dir.join("Fixture.glyphs");
    save(&sample(), &path, SaveFormat::Glyphs).unwrap();
    assert_comparable(Oracle::Fontmake, &path, "Fixture.glyphs");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn fontmake_builds_cff_that_reads_back() {
    let Some(tool) = tool(Oracle::Fontmake) else { return };
    let dir = temp("fontmake-cff");
    let path = dir.join("Fixture.glyphs");
    save(&sample(), &path, SaveFormat::Glyphs).unwrap();
    let mut s = settings(Oracle::Fontmake, Flavour::Cff, tool);
    s.keep = Some(dir.join("out"));
    let report = check(&path, "Fixture.glyphs", &s, || ours(&path, Flavour::Cff));
    assert_eq!(report.outcome, Outcome::NotCompared, "{:?}", report.messages);
    let built = report.reference.expect("the reference was built");
    let bytes = std::fs::read(&built.path).unwrap();
    // The CFF outlines decode: the font compared with itself is identical, outlines included.
    let c = compare::compare(&bytes, &bytes).unwrap();
    let cff = c.tables.iter().find(|t| t.tag == "CFF ").expect("a CFF table");
    assert_eq!(cff.status, Status::Identical);
    let _ = std::fs::remove_dir_all(&dir);
}
