//! Corpus round trips through the model. Skipped when `corpus/` has not been fetched (`cargo
//! xtask corpus --fetch`); `cargo xtask roundtrip --mode model` gives the full report. Guards the
//! part that must stay exact: every single-file `.glyphs` source the reference app saved, outside
//! fontc's hand-made test data and the format document's hand-arranged sample, in every format
//! version.
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
        if !text.lines().take(3).any(|l| l.starts_with(".appVersion = ")) {
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
    assert!(checked >= 15, "only {checked} sources found");
    eprintln!("{checked} reference-written sources round-trip through the model");
}

fn packages(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.file_name().is_some_and(|n| n == ".git" || n == "fontc") {
            continue;
        }
        if p.is_dir() && p.extension().is_some_and(|x| x == "glyphspackage") {
            out.push(p);
        } else if p.is_dir() {
            packages(&p, out);
        }
    }
}

fn files_of(dir: &Path, root: &Path, out: &mut fontcraft_io_glyphs::Files) {
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            files_of(&p, root, out);
        } else {
            out.insert(p.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/"), std::fs::read(&p).unwrap());
        }
    }
}

#[test]
fn reference_written_packages_round_trip_through_the_model() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus");
    if !corpus.is_dir() {
        eprintln!("corpus/ absent: skipped (cargo xtask corpus --fetch)");
        return;
    }
    let mut all = Vec::new();
    packages(&corpus, &mut all);
    let (mut checked, mut failures) = (0, Vec::new());
    for dir in &all {
        let mut files = fontcraft_io_glyphs::Files::new();
        files_of(dir, dir, &mut files);
        let head = files.get("fontinfo.plist").map(|b| String::from_utf8_lossy(b).into_owned()).unwrap_or_default();
        if !head.lines().take(3).any(|l| l.starts_with(".appVersion = ")) {
            continue;
        }
        checked += 1;
        match fontcraft_io_glyphs::read_package(&files).and_then(|f| fontcraft_io_glyphs::write_package(&f)) {
            Ok(out) if out == files => {}
            Ok(out) => {
                let first = files.iter().find(|(k, v)| out.get(*k) != Some(*v)).map_or("an extra file", |(k, _)| k.as_str());
                failures.push(format!("{}: {first} differs", dir.display()));
            }
            Err(e) => failures.push(format!("{}: {e}", dir.display())),
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    assert!(checked >= 10, "only {checked} packages found");
    eprintln!("{checked} reference-written packages round-trip through the model");
}

#[test]
fn a_package_and_a_single_file_of_the_same_font_read_the_same() {
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/glyphspack/tests/fixtures");
    if !fixtures.is_dir() {
        eprintln!("corpus/ absent: skipped (cargo xtask corpus --fetch)");
        return;
    }
    for version in ["3", "4"] {
        let single = fontcraft_io_glyphs::read(&std::fs::read_to_string(fixtures.join(format!("Andika Mini {version}.glyphs"))).unwrap()).unwrap();
        let dir = fixtures.join(format!("Andika Mini {version}.glyphspackage"));
        let mut files = fontcraft_io_glyphs::Files::new();
        files_of(&dir, &dir, &mut files);
        let mut package = fontcraft_io_glyphs::read_package(&files).unwrap();
        assert!(package.files.is_empty(), "version {version}: side files left over: {:?}", package.files.0.keys().collect::<Vec<_>>());
        package.source = single.source.clone();
        // The fixtures' single files carry each glyph's last-change date and their packages do
        // not (they were saved differently); that date is not part of the mapping under test.
        let mut single = single;
        for f in [&mut single, &mut package] {
            for i in 0..f.glyphs.len() {
                if let Some(mut g) = f.glyphs.get_mut_at(i) {
                    g.info.last_change = None;
                    g.info.tidy();
                }
            }
        }
        let differs = first_difference(&package, &single);
        assert!(differs.is_none(), "version {version}: the package reads differently from the single file: {}", differs.unwrap_or_default());
        // And the package read back writes the package's files.
        assert!(fontcraft_io_glyphs::write_package(&package).unwrap() == files, "version {version}: the package does not write back");
    }
}

/// The first part in which two fonts differ, for test messages.
fn first_difference(a: &fontcraft_model::Font, b: &fontcraft_model::Font) -> Option<String> {
    macro_rules! part {
        ($($f:ident),*) => {
            $(if a.$f != b.$f {
                return Some(format!("{}: {:?} vs {:?}", stringify!($f), a.$f, b.$f).chars().take(600).collect());
            })*
        };
    }
    part!(
        family_name,
        units_per_em,
        version_major,
        version_minor,
        axes,
        metrics,
        masters,
        instances,
        kerning,
        features,
        display_strings,
        note,
        extra
    );
    for (x, y) in a.glyphs.iter().zip(b.glyphs.iter()) {
        if x != y {
            return Some(format!("glyph {}: {:?} vs {:?}", x.name, x, y).chars().take(600).collect());
        }
    }
    (a.glyphs.len() != b.glyphs.len()).then(|| "glyph count".to_string())
}
