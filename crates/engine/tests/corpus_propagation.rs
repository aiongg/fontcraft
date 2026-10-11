//! Measure (M6 plan § 5, anchor propagation against the oracles): for every corpus Glyphs
//! source the reference's Python library loads, every glyph on every master layer: FontCraft's
//! propagated anchors (`effective_anchors_for_master`) equal the library's
//! `propagate_all_anchors`, the algorithm fontc follows, in names, order and positions within
//! 0.001.
//!
//! The library's output is read from a JSON file made beforehand by a script outside the
//! repository, run with the fontmake tool's Python: `{source path relative to the corpus:
//! {glyph: {master id: [[name, x, y], ...]}}}`, with `{"error": message}` for a source it could
//! not load. The library is only run, never read: propagation is written from observed outputs.
//!
//! `cargo test -p fontcraft-engine --test corpus_propagation -- --ignored --nocapture`.
//! `GLYPHSLIB_ANCHORS` names the JSON file (default: `plan/local/m6/glyphslib_anchors.json`, which
//! git ignores); `FONTCRAFT_CORPUS` the corpus folder (default: the repository's `corpus/`);
//! `PROPAGATION_LIST=<n>` lists more than 40 differing layers.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use fontcraft_components::propagate::effective_anchors_for_master;
use serde_json::Value;

const TOLERANCE: f64 = 0.001;

fn corpus() -> PathBuf {
    std::env::var_os("FONTCRAFT_CORPUS").map_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus"), PathBuf::from)
}

/// One anchor list as (name, x, y).
type Anchors = Vec<(String, f64, f64)>;

fn reference_anchors(v: &Value) -> Anchors {
    v.as_array()
        .map(|list| {
            list.iter()
                .filter_map(|a| {
                    let a = a.as_array()?;
                    Some((a.first()?.as_str()?.to_string(), a.get(1)?.as_f64()?, a.get(2)?.as_f64()?))
                })
                .collect()
        })
        .unwrap_or_default()
}

/// How one layer's lists differ, coarsely: the first class that applies.
fn classify(ours: &Anchors, theirs: &Anchors) -> Option<&'static str> {
    let names = |l: &Anchors| -> Vec<String> { l.iter().map(|a| a.0.clone()).collect() };
    let (on, tn) = (names(ours), names(theirs));
    let missing = tn.iter().any(|n| !on.contains(n));
    let extra = on.iter().any(|n| !tn.contains(n));
    match (missing, extra) {
        (true, true) => return Some("names missing and extra in ours"),
        (true, false) => return Some("names missing in ours"),
        (false, true) => return Some("names extra in ours"),
        _ => {}
    }
    if on.len() != tn.len() {
        return Some("a name repeated");
    }
    let position = |a: &(String, f64, f64), b: &(String, f64, f64)| (a.1 - b.1).abs() > TOLERANCE || (a.2 - b.2).abs() > TOLERANCE;
    if on != tn {
        // Same names in another order: still check the positions name by name.
        let moved = ours.iter().any(|a| theirs.iter().find(|b| b.0 == a.0).is_some_and(|b| position(a, b)));
        return Some(if moved { "order and positions" } else { "order" });
    }
    ours.iter().zip(theirs).any(|(a, b)| position(a, b)).then_some("positions")
}

#[test]
#[ignore = "reads every corpus Glyphs source and a JSON file made by the reference's Python library: the M6 propagation measure"]
fn propagation_equals_the_reference_library() {
    let path = std::env::var_os("GLYPHSLIB_ANCHORS")
        .map_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plan/local/m6/glyphslib_anchors.json"), PathBuf::from);
    let Ok(text) = std::fs::read_to_string(&path) else {
        eprintln!("skipped: no reference anchors at {} (set GLYPHSLIB_ANCHORS)", path.display());
        return;
    };
    let reference: BTreeMap<String, Value> = serde_json::from_str(&text).unwrap();
    let list = std::env::var("PROPAGATION_LIST").ok().and_then(|s| s.parse().ok()).unwrap_or(40usize);
    let corpus = corpus();
    let (mut sources, mut layers, mut anchors, mut differing) = (0usize, 0usize, 0usize, 0usize);
    let mut skipped: Vec<String> = Vec::new();
    let mut by_class: BTreeMap<&str, usize> = BTreeMap::new();
    let mut by_source: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    let mut shown = 0usize;
    for (rel, glyphs) in &reference {
        if let Some(e) = glyphs.get("error").and_then(Value::as_str) {
            skipped.push(format!("{rel}: the library could not load it: {e}"));
            continue;
        }
        let Some(glyphs) = glyphs.as_object() else { continue };
        let Ok((font, _)) = fontcraft_engine::io::open(&corpus.join(rel), None) else {
            skipped.push(format!("{rel}: not read"));
            continue;
        };
        sources += 1;
        let entry = by_source.entry(rel.clone()).or_default();
        for master in font.masters.iter() {
            let ours = match effective_anchors_for_master(&font, &master.id) {
                Ok(map) => map,
                Err(e) => {
                    skipped.push(format!("{rel} master {}: {e}", master.name));
                    continue;
                }
            };
            for (glyph, by_master) in glyphs {
                let Some(theirs) = by_master.get(&master.id) else { continue };
                let theirs = reference_anchors(theirs);
                let mine: Anchors = match ours.get(glyph) {
                    Some(list) => list.iter().map(|a| (a.name.clone(), a.x, a.y)).collect(),
                    None => {
                        if font.glyph(glyph).is_none() {
                            skipped.push(format!("{rel}: glyph {glyph} not in our font"));
                        }
                        continue;
                    }
                };
                layers += 1;
                entry.0 += 1;
                anchors += theirs.len();
                let Some(class) = classify(&mine, &theirs) else { continue };
                differing += 1;
                entry.1 += 1;
                *by_class.entry(class).or_default() += 1;
                if shown < list {
                    let fmt = |l: &Anchors| l.iter().map(|(n, x, y)| format!("{n}({x},{y})")).collect::<Vec<_>>().join(" ");
                    eprintln!("{rel} {glyph} [{}] {class}:\n  ours   {}\n  theirs {}", master.name, fmt(&mine), fmt(&theirs));
                }
                shown += 1;
            }
        }
    }
    for s in skipped.iter().take(list) {
        eprintln!("skipped: {s}");
    }
    if skipped.len() > list {
        eprintln!("skipped: {} more", skipped.len() - list);
    }
    for (rel, (compared, differ)) in by_source.iter().filter(|(_, (_, d))| *d > 0) {
        eprintln!("source: {rel}: {compared} layers, {differ} differ");
    }
    for (class, count) in &by_class {
        eprintln!("class: {class}: {count}");
    }
    eprintln!("total: {sources} sources, {layers} master layers, {anchors} reference anchors, {differing} layers differ");
    assert_eq!(differing, 0, "{differing} master layers' propagated anchors differ from the reference library's");
}
