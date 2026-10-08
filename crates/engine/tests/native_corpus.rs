//! `roundtrip.nativeLosslessFromGlyphs`: every `.glyphs` source the reference app saved (outside
//! fontc's hand-made test data and the format document's hand-arranged sample) survives being
//! saved as `.fontcraft` and written back, byte for byte. Skipped when `corpus/` is absent.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.file_name().is_some_and(|n| n == ".git" || n == "fontc" || n == "GlyphsFileFormatv2.glyphs") {
            continue;
        }
        if p.is_file() && p.extension().is_some_and(|x| x == "glyphs") {
            out.push(p);
        } else if p.is_dir() {
            sources(&p, out);
        }
    }
}

#[test]
fn glyphs_sources_survive_the_native_format() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus");
    if !corpus.is_dir() {
        eprintln!("corpus/ absent: skipped (cargo xtask corpus --fetch)");
        return;
    }
    let mut all = Vec::new();
    sources(&corpus, &mut all);
    let (mut checked, mut failures) = (0, Vec::new());
    for path in &all {
        let text = std::fs::read_to_string(path).unwrap();
        if !text.lines().take(3).any(|l| l.starts_with(".appVersion = ")) {
            continue;
        }
        checked += 1;
        let font = fontcraft_io_glyphs::read(&text).unwrap();
        let native = fontcraft_engine::format::save(&font).unwrap();
        let back = fontcraft_engine::format::load(&native).unwrap();
        if fontcraft_io_glyphs::write(&back).unwrap() != text {
            failures.push(path.display().to_string());
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    assert!(checked >= 15, "only {checked} sources found");
}
