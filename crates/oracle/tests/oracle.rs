//! Opt-in tests that run the reference compilers: `cargo test -p fontcraft-oracle --features
//! oracle`. Each test skips, with a message, when its tool is not installed.
#![cfg(feature = "oracle")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::{Path, PathBuf};

use fontcraft_compile::{Options, compile};
use fontcraft_engine::io::{SaveFormat, open, save};
use fontcraft_model::{Font, Path as Contour, Shape};
use fontcraft_oracle::{AllowList, Flavour, Hint, Oracle, Ours, OursError, Outcome, Settings, Status, check};

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
fn ours(path: &Path, oracle: Oracle, flavour: Flavour) -> Result<Ours, OursError> {
    let (font, warnings) = open(path, None).map_err(|e| OursError::Failed(e.to_string()))?;
    let hint = Hint { family: font.family_name.clone(), style: font.default_master().unwrap().name.clone() };
    let outlines = match flavour {
        Flavour::TrueType => fontcraft_compile::Flavour::TrueType,
        Flavour::Cff => fontcraft_compile::Flavour::Cff,
    };
    let options = Options { timestamp: fontcraft_oracle::DEFAULT_TIMESTAMP, remove_overlap: oracle.removes_overlaps(), ..Default::default() };
    let compiled = compile(&font, &font.default_target().unwrap(), outlines, &options).map_err(|e| OursError::Failed(e.to_string()))?;
    Ok(Ours { bytes: compiled.bytes, warnings, hint })
}

/// What holds today and must keep holding: both builds succeed, the glyph sets agree, and the
/// code points and advances match. Other tables are M2.2's to bring to parity.
fn assert_comparable(oracle: Oracle, source: &Path, label: &str) {
    let Some(tool) = tool(oracle) else { return };
    let report = check(source, label, &settings(oracle, Flavour::TrueType, tool), || ours(source, oracle, Flavour::TrueType));
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
fn fontmake_and_fontcraft_build_the_same_cff_outlines() {
    let Some(tool) = tool(Oracle::Fontmake) else { return };
    let dir = temp("fontmake-cff");
    let path = dir.join("Fixture.glyphs");
    save(&sample(), &path, SaveFormat::Glyphs).unwrap();
    let report = check(&path, "Fixture.glyphs", &settings(Oracle::Fontmake, Flavour::Cff, tool), || ours(&path, Oracle::Fontmake, Flavour::Cff));
    eprintln!("{}", fontcraft_oracle::render(&report));
    assert_eq!(report.outcome, Outcome::Compared, "{:?}", report.messages);
    let status = |tag: &str| report.tables.iter().find(|t| t.tag == tag).map(|t| t.status);
    // Decoded outlines within half a unit; the table's bytes are the reference's too.
    assert_eq!(status("CFF "), Some(Status::Identical));
    for tag in ["maxp", "hmtx", "post", "cmap"] {
        assert!(matches!(status(tag), Some(Status::Identical | Status::Equivalent)), "{tag}: {:?}", status(tag));
    }
    assert_eq!(status("glyf"), None, "no TrueType outlines");
    let _ = std::fs::remove_dir_all(&dir);
}
