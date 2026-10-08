//! Every GLIF file in the corpus through the model's layer and back, each with its own layout
//! style. Skipped when `corpus/` has not been fetched (`cargo xtask corpus --fetch`). Guards the
//! files fontTools wrote: those must come back byte for byte.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use fontcraft_io_ufo::glif;
use fontcraft_model::{Family, Layer, Shape};

/// Start tags the reader kept because the writer would have written them differently.
fn kept_tags(layer: &Layer) -> usize {
    let tags = |x: &fontcraft_model::Extra| x.of(Family::Ufo).filter(|e| e.key.as_str().starts_with('<')).count();
    let mut n = tags(&layer.extra) + layer.anchors.iter().map(|a| tags(&a.extra)).sum::<usize>();
    for s in &layer.shapes {
        match s {
            Shape::Path(p) => n += tags(&p.extra) + p.nodes.iter().map(|n| tags(&n.info.extra)).sum::<usize>(),
            Shape::Component(c) => n += tags(&c.extra),
            Shape::Image(_) | Shape::Raw(_) => {}
        }
    }
    n
}

fn glifs(dir: &Path, ufolib: bool, out: &mut Vec<(PathBuf, bool)>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if name == ".git" {
            continue;
        }
        if p.is_dir() {
            let by_ufolib = ufolib
                || name.ends_with(".ufo")
                    && std::fs::read_to_string(p.join("metainfo.plist")).is_ok_and(|m| m.contains("<string>com.github.fonttools.ufoLib</string>"));
            glifs(&p, by_ufolib, out);
        } else if name.ends_with(".glif") {
            out.push((p, ufolib));
        }
    }
}

#[test]
fn glif_files_round_trip_through_the_model() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus");
    if !corpus.is_dir() {
        eprintln!("corpus/ absent: skipped (cargo xtask corpus --fetch)");
        return;
    }
    let mut files = Vec::new();
    glifs(&corpus, false, &mut files);
    files.sort();
    let (mut same, mut refused, mut different) = (0, Vec::new(), Vec::new());
    let mut ufolib_hints = 0;
    for (f, ufolib) in &files {
        let Ok(text) = std::fs::read_to_string(f) else { continue };
        let read = glif::read(&text, "test", "m", "m");
        if let Ok(g) = &read
            && *ufolib
        {
            let n = kept_tags(&g.layer);
            if n > 0 && std::env::var_os("FONTCRAFT_ALL").is_some() {
                eprintln!("  {n} kept tags in {}", f.display());
            }
            ufolib_hints += n;
        }
        match read {
            Ok(g) => match glif::write(glif::Head::of(&g), &g.layer, g.format, &g.style) {
                Ok(out) if out == text => same += 1,
                Ok(out) => {
                    let line = text.lines().zip(out.lines()).position(|(a, b)| a != b).unwrap_or(0);
                    let (a, b) = (text.lines().nth(line).unwrap_or(""), out.lines().nth(line).unwrap_or(""));
                    different.push((*ufolib, format!("{}:{}: {a:?} → {b:?}", f.display(), line + 1)));
                }
                Err(e) => refused.push(format!("{}: writing: {e}", f.display())),
            },
            Err(e) => refused.push(format!("{}: {e}", f.display())),
        }
    }
    eprintln!(
        "{} GLIF files: {same} identical, {} different, {} refused; {ufolib_hints} kept start tags in files fontTools wrote",
        files.len(),
        different.len(),
        refused.len()
    );
    for (u, d) in different.iter().take(if std::env::var_os("FONTCRAFT_ALL").is_some() { usize::MAX } else { 40 }) {
        eprintln!("  different{}: {d}", if *u { " (ufoLib)" } else { "" });
    }
    for r in refused.iter().take(if std::env::var_os("FONTCRAFT_ALL").is_some() { usize::MAX } else { 20 }) {
        eprintln!("  refused {r}");
    }
    let ufolib: Vec<_> = different.iter().filter(|(u, _)| *u).collect();
    assert!(ufolib.is_empty(), "{} GLIF files fontTools wrote come back different", ufolib.len());
}
