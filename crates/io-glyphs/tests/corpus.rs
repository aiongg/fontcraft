//! Corpus round trips through the model. Skipped when `corpus/` has not been fetched (`cargo
//! xtask corpus --fetch`); `cargo xtask roundtrip --mode model` gives the full report. Guards the
//! part that must stay exact: every single-file `.glyphs` source the reference app saved, outside
//! fontc's hand-made test data, in a format version the reader supports.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.file_name().is_some_and(|n| n == ".git" || n == "fontc") {
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
fn reference_written_sources_round_trip_through_the_model() {
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
        let head: Vec<&str> = text.lines().take(3).collect();
        let supported = head.iter().any(|l| *l == ".formatVersion = 3;" || *l == ".formatVersion = 4;");
        if !supported || !head.iter().any(|l| l.starts_with(".appVersion = ")) {
            continue;
        }
        checked += 1;
        match fontcraft_io_glyphs::read(&text).and_then(|f| fontcraft_io_glyphs::write(&f)) {
            Ok(out) if out == text => {}
            Ok(_) => failures.push(format!("{}: different", path.display())),
            Err(e) => failures.push(format!("{}: {e}", path.display())),
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    assert!(checked >= 10, "only {checked} sources found");
    eprintln!("{checked} reference-written sources round-trip through the model");
}
