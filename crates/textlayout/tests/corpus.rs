//! Display strings of the corpus sources read and write back unchanged, and the layout speed
//! measure of `docs/plans/M5.md` § 5 on Inter Roman:
//! `cargo test --release -p fontcraft-textlayout --test corpus -- --ignored --nocapture`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::{Path, PathBuf};
use std::time::Instant;

use fontcraft_model::Font;
use fontcraft_spacing::kerning::KernIndex;
use fontcraft_textlayout::{MAX_PREVIEW_ITEMS, Options, Text, layout, parse};

fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus")
}

fn read_tree(dir: &Path, root: &Path, out: &mut fontcraft_io_glyphs::Files) {
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            read_tree(&p, root, out);
        } else {
            out.insert(p.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/"), std::fs::read(&p).unwrap());
        }
    }
}

fn read_package(dir: &Path) -> Option<Font> {
    let mut files = fontcraft_io_glyphs::Files::new();
    read_tree(dir, dir, &mut files);
    fontcraft_io_glyphs::read_package(&files).ok()
}

/// Sources that store display strings: `.glyphs` files naming `DisplayStrings` and packages
/// with a `UIState.plist`.
fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        let ext = p.extension().and_then(|e| e.to_str());
        if p.is_dir() {
            if ext == Some("glyphspackage") {
                if p.join("UIState.plist").is_file() {
                    out.push(p);
                }
            } else {
                sources(&p, out);
            }
        } else if ext == Some("glyphs") && std::fs::read_to_string(&p).is_ok_and(|t| t.contains("DisplayStrings")) {
            out.push(p);
        }
    }
}

#[test]
fn corpus_display_strings_round_trip() {
    let root = corpus();
    let mut paths = Vec::new();
    sources(&root, &mut paths);
    paths.sort();
    let (mut strings, mut fonts) = (0, 0);
    for path in &paths {
        let font = if path.is_dir() { read_package(path) } else { fontcraft_io_glyphs::read(&std::fs::read_to_string(path).unwrap()).ok() };
        let Some(font) = font else { continue };
        fonts += usize::from(!font.display_strings.is_empty());
        for s in font.display_strings.iter() {
            let text = Text::from_display_string(&font, s).unwrap_or_else(|e| panic!("{}: {s:?}: {e}", path.display()));
            assert_eq!(&text.to_display_string(&font), s, "{}", path.display());
            strings += 1;
        }
    }
    eprintln!("{strings} display strings of {fonts} sources read and written back unchanged");
    assert!(paths.is_empty() || strings > 0);
}

#[test]
#[ignore = "speed measure"]
fn inter_layout_speed() {
    let dir = corpus().join("inter/src/Inter-Roman.glyphspackage");
    if !dir.is_dir() {
        return;
    }
    let font = read_package(&dir).unwrap();
    let master = font.masters.iter().find(|m| m.name == "Regular").unwrap().id.clone();
    let sample = "The quick brown fox jumps over the lazy dog. AVATAR Tõ Wa “Yo” 1234567890 ";

    let t = Instant::now();
    let index = KernIndex::new(&font);
    let build = t.elapsed();

    let tab: String = sample.chars().cycle().take(fontcraft_textlayout::MAX_ITEMS).collect();
    let tab = Text::from_display_string(&font, &tab).unwrap();
    let options = Options { index: Some(&index), ..Options::default() };
    let mut best = f64::MAX;
    for _ in 0..20 {
        let t = Instant::now();
        let l = layout(&font, &master, &tab, &options).unwrap();
        best = best.min(t.elapsed().as_secs_f64() * 1000.0);
        assert_eq!(l.placed.len(), tab.len());
        assert!(l.placed.iter().any(|p| p.kern != 0.0));
    }

    let long: String = sample.chars().cycle().take(MAX_PREVIEW_ITEMS).collect();
    let t = Instant::now();
    let mut preview = Text::with_limit(MAX_PREVIEW_ITEMS).unwrap();
    preview.insert(0, parse(&font, &long, MAX_PREVIEW_ITEMS).unwrap()).unwrap();
    let parse_ms = t.elapsed().as_secs_f64() * 1000.0;
    let t = Instant::now();
    let l = layout(&font, &master, &preview, &options).unwrap();
    let preview_ms = t.elapsed().as_secs_f64() * 1000.0;
    assert_eq!(l.placed.len(), MAX_PREVIEW_ITEMS);

    eprintln!(
        "kern index {:.1} ms; layout of {} items {best:.2} ms (target 16); {} characters: parse {parse_ms:.0} ms, layout {preview_ms:.0} ms (target 500 together), {} lines",
        build.as_secs_f64() * 1000.0,
        tab.len(),
        MAX_PREVIEW_ITEMS,
        l.lines.len()
    );
    assert!(best < 16.0, "10,000 items took {best:.2} ms");
}
