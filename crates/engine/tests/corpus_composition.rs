//! Measures (M6 plan § 5, composition and default anchors against the corpus): for every corpus
//! `.glyphs` and `.glyphspackage` glyph whose name the glyph database decomposes and that the
//! source holds as a composite (its origin master layer has components and no paths):
//! - Composition: `components_for`'s names and order against the stored components';
//! - Default anchors: `default_anchor_names` against the anchor names the layer stores, and
//!   against the names it has after propagation (a composite usually stores none).
//!
//! Differences are classified coarsely (names missing in ours, extra in ours, both, order only)
//! and counted; the human classification (the designer's choice, our table, our error) is done
//! when reading the output. The most frequent patterns are listed with an example each.
//! The built-in glyph database is used, without the sources' custom glyph data files.
//!
//! `cargo test -p fontcraft-engine --test corpus_composition -- --ignored --nocapture`.
//! `FONTCRAFT_CORPUS` names the corpus folder (default: the repository's `corpus/`);
//! `COMPOSITION_LIST=<n>` lists more than 30 patterns and differing glyphs.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use fontcraft_components::ComponentsError;
use fontcraft_components::compose::components_for;
use fontcraft_components::default_anchors::default_anchor_names;
use fontcraft_components::propagate::effective_anchors_with;
use fontcraft_model::Shape;
use fontcraft_model::glyphdata::GlyphData;

fn corpus() -> PathBuf {
    std::env::var_os("FONTCRAFT_CORPUS").map_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus"), PathBuf::from)
}

fn sources(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.file_name().and_then(|n| n.to_str()) == Some(".git") {
            continue;
        }
        let ext = p.extension().and_then(|x| x.to_str()).unwrap_or("");
        if ext == "glyphs" && p.is_file() || ext == "glyphspackage" && p.is_dir() {
            out.push(p);
        } else if p.is_dir() {
            sources(&p, out);
        }
    }
}

/// The coarse class of an ordered comparison, `None` when equal.
fn classify_ordered(ours: &[String], stored: &[String]) -> Option<&'static str> {
    if ours == stored {
        return None;
    }
    let missing = stored.iter().any(|n| !ours.contains(n));
    let extra = ours.iter().any(|n| !stored.contains(n));
    Some(match (missing, extra) {
        (true, true) => "missing and extra in ours",
        (true, false) => "missing in ours",
        (false, true) => "extra in ours",
        // Same names: another order, or a name repeated another number of times.
        (false, false) => {
            let (mut a, mut b) = (ours.to_vec(), stored.to_vec());
            a.sort();
            b.sort();
            if a == b { "order" } else { "repeats" }
        }
    })
}

/// The coarse class of a comparison of name sets, `None` when equal.
fn classify_set(ours: &BTreeSet<String>, stored: &BTreeSet<String>) -> Option<&'static str> {
    let missing = stored.difference(ours).next().is_some();
    let extra = ours.difference(stored).next().is_some();
    match (missing, extra) {
        (true, true) => Some("missing and extra in ours"),
        (true, false) => Some("missing in ours"),
        (false, true) => Some("extra in ours"),
        (false, false) => None,
    }
}

/// A difference's pattern for grouping: the names only one side has, glyph-independent.
fn pattern(ours: &[String], stored: &[String], class: &str) -> String {
    let only = |a: &[String], b: &[String]| -> Vec<String> {
        let set: BTreeSet<&String> = a.iter().filter(|n| !b.contains(n)).collect();
        set.into_iter().cloned().collect()
    };
    if class == "order" || class == "repeats" {
        return class.to_string();
    }
    format!("{class}: ours +[{}] stored +[{}]", only(ours, stored).join(" "), only(stored, ours).join(" "))
}

/// Counts per class, and per pattern with a count and the first example.
#[derive(Default)]
struct Tally {
    compared: usize,
    by_class: BTreeMap<&'static str, usize>,
    by_pattern: BTreeMap<String, (usize, String)>,
}

impl Tally {
    fn add(&mut self, class: Option<&'static str>, pattern: impl FnOnce() -> String, example: impl FnOnce() -> String) {
        self.compared += 1;
        let Some(class) = class else { return };
        *self.by_class.entry(class).or_default() += 1;
        let e = self.by_pattern.entry(pattern()).or_insert_with(|| (0, example()));
        e.0 += 1;
    }

    fn print(&self, title: &str, list: usize) {
        let differ: usize = self.by_class.values().sum();
        eprintln!("== {title}: {} glyphs compared, {} equal, {differ} differ", self.compared, self.compared - differ);
        for (class, count) in &self.by_class {
            eprintln!("{title}: class {class}: {count}");
        }
        let mut patterns: Vec<_> = self.by_pattern.iter().collect();
        patterns.sort_by(|a, b| b.1.0.cmp(&a.1.0).then(a.0.cmp(b.0)));
        for (p, (count, example)) in patterns.iter().take(list) {
            eprintln!("{title}: {count} x {p}  (e.g. {example})");
        }
        if patterns.len() > list {
            eprintln!("{title}: {} more patterns", patterns.len() - list);
        }
    }
}

fn names_of(list: &[String]) -> String {
    list.join(" ")
}

#[test]
#[ignore = "reads every corpus Glyphs source: the M6 composition and default-anchor measures"]
fn composition_and_default_anchors_against_the_corpus() {
    let list = std::env::var("COMPOSITION_LIST").ok().and_then(|s| s.parse().ok()).unwrap_or(30usize);
    let data = GlyphData::new().unwrap();
    let corpus = corpus();
    let mut all = Vec::new();
    sources(&corpus, &mut all);
    all.sort();
    let mut composition = Tally::default();
    let mut stored_anchors = Tally::default();
    let mut propagated_anchors = Tally::default();
    let (mut read, mut mixed, mut no_stored_anchors) = (0usize, 0usize, 0usize);
    let mut missing_parts: BTreeMap<String, usize> = BTreeMap::new();
    let mut errors: Vec<String> = Vec::new();
    let mut per_source: Vec<(String, usize, usize)> = Vec::new();
    for p in &all {
        let Ok((font, _)) = fontcraft_engine::io::open(p, None) else {
            errors.push(format!("{}: not read", p.display()));
            continue;
        };
        read += 1;
        let source = p.strip_prefix(&corpus).unwrap_or(p).display().to_string();
        let Some(master) = fontcraft_model::axes::origin(&font).or_else(|| font.masters.first()).map(|m| m.id.clone()) else { continue };
        let before = (composition.compared, composition.by_class.values().sum::<usize>());
        for g in font.glyphs.iter() {
            let Some(layer) = g.layer(&master) else { continue };
            let stored: Vec<String> = layer
                .shapes
                .iter()
                .filter_map(|s| match s {
                    Shape::Component(c) => Some(c.glyph.clone()),
                    _ => None,
                })
                .collect();
            if stored.is_empty() {
                continue;
            }
            let ours = match components_for(&font, &data, &g.name) {
                Ok(Some(parts)) => parts,
                Ok(None) => continue,
                Err(ComponentsError::MissingParts { missing, .. }) => {
                    for m in missing {
                        *missing_parts.entry(m).or_default() += 1;
                    }
                    continue;
                }
                Err(e) => {
                    errors.push(format!("{source} {}: {e}", g.name));
                    continue;
                }
            };
            if layer.shapes.iter().any(|s| matches!(s, Shape::Path(_))) {
                mixed += 1;
                continue;
            }
            let class = classify_ordered(&ours, &stored);
            composition.add(
                class,
                || pattern(&ours, &stored, class.unwrap_or_default()),
                || format!("{source} {}: ours [{}], stored [{}]", g.name, names_of(&ours), names_of(&stored)),
            );

            let defaults: BTreeSet<String> = default_anchor_names(&font, &data, &g.name).into_iter().collect();
            let own: BTreeSet<String> = layer.anchors.iter().map(|a| a.name.clone()).collect();
            let defaults_list: Vec<String> = defaults.iter().cloned().collect();
            if own.is_empty() {
                no_stored_anchors += 1;
            } else {
                let own_list: Vec<String> = own.iter().cloned().collect();
                let class = classify_set(&defaults, &own);
                stored_anchors.add(
                    class,
                    || pattern(&defaults_list, &own_list, class.unwrap_or_default()),
                    || format!("{source} {}: ours [{}], stored [{}]", g.name, names_of(&defaults_list), names_of(&own_list)),
                );
            }
            match effective_anchors_with(&font, Some(&data), &g.name, &layer.id) {
                Ok(effective) => {
                    let names: BTreeSet<String> = effective.into_iter().map(|a| a.name).collect();
                    let names_list: Vec<String> = names.iter().cloned().collect();
                    let class = classify_set(&defaults, &names);
                    propagated_anchors.add(
                        class,
                        || pattern(&defaults_list, &names_list, class.unwrap_or_default()),
                        || format!("{source} {}: ours [{}], propagated [{}]", g.name, names_of(&defaults_list), names_of(&names_list)),
                    );
                }
                Err(e) => errors.push(format!("{source} {}: propagation: {e}", g.name)),
            }
        }
        let after = (composition.compared, composition.by_class.values().sum::<usize>());
        if after.0 > before.0 {
            per_source.push((source, after.0 - before.0, after.1 - before.1));
        }
    }
    for (source, compared, differ) in per_source.iter().filter(|r| r.2 > 0) {
        eprintln!("source: {source}: {compared} composites compared, {differ} differ");
    }
    composition.print("composition", list);
    stored_anchors.print("default anchors against stored", list);
    propagated_anchors.print("default anchors against propagated", list);
    let mut missing: Vec<_> = missing_parts.into_iter().collect();
    missing.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let shown: Vec<String> = missing.iter().take(list).map(|(n, c)| format!("{n} {c}")).collect();
    eprintln!("parts missing from the font (composites not compared): {}", shown.join(", "));
    eprintln!("composites with paths too (not compared): {mixed}; composites storing no anchors: {no_stored_anchors}");
    for e in errors.iter().take(list) {
        eprintln!("error: {e}");
    }
    eprintln!("total: {} sources found, {read} read, {} errors", all.len(), errors.len());
}
