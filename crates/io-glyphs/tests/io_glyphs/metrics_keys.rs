//! The metrics-key measure of `docs/plans/M5.md` § 5: every key in the corpus's `.glyphs`
//! sources and packages is parsed, and every key that applies to a master layer is evaluated and
//! compared with the layer's metric. Prints the counts per class and samples of each.
//! `cargo test --release -p fontcraft-io-glyphs --test io_glyphs metrics_keys:: -- --ignored --nocapture`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use fontcraft_model::{Alignment, Font, Shape};
use fontcraft_spacing::{EvalError, Field, Scope, effective_key, evaluate, metric, parse_stored, round_half_up};

fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.file_name().is_some_and(|n| n == ".git") {
            continue;
        }
        if p.extension().is_some_and(|x| x == "glyphs") && p.is_file() || p.extension().is_some_and(|x| x == "glyphspackage") && p.is_dir() {
            out.push(p);
        } else if p.is_dir() {
            sources(&p, out);
        }
    }
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

fn load(path: &Path) -> Result<Font, String> {
    if path.is_dir() {
        let mut files = fontcraft_io_glyphs::Files::new();
        read_tree(path, path, &mut files);
        fontcraft_io_glyphs::read_package(&files).map_err(|e| e.to_string())
    } else {
        let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        fontcraft_io_glyphs::read(&text).map_err(|e| e.to_string())
    }
}

#[derive(Default)]
struct Tally {
    counts: BTreeMap<String, usize>,
    samples: BTreeMap<String, Vec<String>>,
}

impl Tally {
    fn add(&mut self, class: &str, sample: impl FnOnce() -> String) {
        *self.counts.entry(class.into()).or_default() += 1;
        let s = self.samples.entry(class.into()).or_default();
        if s.len() < 12 {
            s.push(sample());
        }
    }
}

#[test]
#[ignore = "corpus measure; prints a report"]
fn every_corpus_metrics_key_parses_and_evaluates() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus");
    let mut all = Vec::new();
    sources(&corpus, &mut all);
    all.sort();
    let mut tally = Tally::default();
    let mut with_keys = 0;
    for path in &all {
        let font = match load(path) {
            Ok(f) => f,
            Err(e) => {
                tally.add("source not read", || format!("{}: {e}", path.display()));
                continue;
            }
        };
        let short = path.strip_prefix(&corpus).unwrap_or(path).display().to_string();
        let exists = |n: &str| font.glyph(n).is_some();
        let before: usize = tally.counts.get("key").copied().unwrap_or(0);
        for g in font.glyphs.as_arcs() {
            let mut stored: Vec<(Scope, Field, &str, String)> = Vec::new();
            for f in Field::ALL {
                if let Some(k) = f.key_in(&g.info.metrics_keys) {
                    stored.push((Scope::Glyph, f, k, String::new()));
                }
            }
            for l in &g.layers {
                for f in Field::ALL {
                    if let Some(k) = f.key_in(&l.info.metrics_keys) {
                        stored.push((Scope::Local, f, k, l.id.clone()));
                    }
                }
            }
            for (scope, field, text, layer) in &stored {
                tally.add("key", String::new);
                match parse_stored(text, *scope, *field, exists) {
                    Ok(_) => tally.add("key parses", String::new),
                    Err(e) => tally.add("key does not parse", || format!("{short}: {} {layer} {field:?} `{text}`: {e}", g.name)),
                }
            }
            for l in g.layers.iter().filter(|l| l.is_master_layer()) {
                for field in Field::ALL {
                    let Some((_, text)) = effective_key(g, l, field) else { continue };
                    let sample = || format!("{short}: {} {} {field:?} `{text}`", g.name, l.id);
                    let value = match evaluate(&font, &g.name, &l.id, field) {
                        Ok(Some(v)) => v,
                        Ok(None) => continue,
                        Err(EvalError::Unlinked(n)) => {
                            tally.add("master layer: unlinked", || format!("{} (no `{n}`)", sample()));
                            continue;
                        }
                        Err(EvalError::Key(_)) => continue,
                        Err(e) => {
                            tally.add(&format!("master layer: error: {}", e.to_string().split(':').next().unwrap_or("")), || {
                                format!("{}: {e}", sample())
                            });
                            continue;
                        }
                    };
                    let actual = match metric(&font, l, field) {
                        Ok(Some(a)) => a,
                        Ok(None) => {
                            tally.add("master layer: own layer empty", sample);
                            continue;
                        }
                        Err(e) => {
                            tally.add("master layer: own outline not measured", || format!("{}: {e}", sample()));
                            continue;
                        }
                    };
                    if round_half_up(actual) == value {
                        tally.add("master layer: in sync", String::new);
                        continue;
                    }
                    let comps = l.shapes.iter().filter(|s| matches!(s, Shape::Component(_))).count();
                    let paths = l.shapes.iter().filter(|s| matches!(s, Shape::Path(_))).count();
                    let aligned = comps > 0
                        && paths == 0
                        && l.shapes.iter().all(|s| !matches!(s, Shape::Component(c) if c.info.alignment == Alignment::Disabled));
                    let class = if aligned { "master layer: differs, glyph aligned automatically (M6)" } else { "master layer: differs" };
                    let source = short.split('/').take(3).collect::<Vec<_>>().join("/");
                    tally.add(&format!("{class} in {source}"), String::new);
                    tally.add(class, || format!("{} gives {value}, layer has {actual}", sample()));
                }
            }
        }
        if tally.counts.get("key").copied().unwrap_or(0) > before {
            with_keys += 1;
            let shared = std::sync::Arc::new(font.clone());
            let t = std::time::Instant::now();
            let found = fontcraft_spacing::scan(&shared);
            let full = t.elapsed();
            let mut cache = fontcraft_spacing::SyncCache::default();
            cache.update(&shared);
            let mut edited = shared.clone();
            if let Some(name) = shared.glyphs.as_arcs().iter().find(|g| g.name == "n").map(|g| g.name.clone()) {
                std::sync::Arc::make_mut(&mut edited).glyph_mut(&name).unwrap().layers[0].width += 1.0;
            }
            let t = std::time::Instant::now();
            cache.update(&edited);
            let one = t.elapsed();
            eprintln!("{short}: {} glyphs, {} findings, scan {full:?}, after an edit to `n` {one:?}", shared.glyphs.len(), found.len());
        }
    }
    eprintln!("{} sources read, {with_keys} with keys", all.len());
    for (class, n) in &tally.counts {
        eprintln!("{n:>7}  {class}");
        for s in tally.samples.get(class).into_iter().flatten().filter(|s| !s.is_empty()) {
            eprintln!("           {s}");
        }
    }
}
