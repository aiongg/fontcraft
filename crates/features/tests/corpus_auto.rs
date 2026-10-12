//! Measure (M8 plan § 5, automatic code against the reference's stored output): for every corpus
//! Glyphs source with automatic entries, Update runs on the source's exporting glyphs and each
//! stored automatic prefix, class and feature is compared with the code Update gives it, after
//! normalising white space.
//!
//! Outcomes per entry: equal; different (with the statements only ours has and only the stored
//! code has); removed (no generator writes the entry for these glyphs); no generator (a name
//! Update does not write). Entries Update would add that the source lacks are listed too.
//!
//! `cargo test -p fontcraft-features --test corpus_auto -- --ignored --nocapture` for the whole
//! corpus; `FONTCRAFT_CORPUS` names the corpus folder (default: the repository's `corpus/`),
//! `AUTO_LIST=<n>` shows more than 60 differing entries, `AUTO_MATCH=<text>` keeps the sources
//! whose path contains the text. The test without `--ignored` checks the entries listed in
//! [`EQUAL`].
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use fontcraft_features::auto::{AutoContext, update};
use fontcraft_features::source_map::EntryKind;
use fontcraft_model::glyphdata::GlyphData;
use fontcraft_model::{Font, Snippet};

fn corpus() -> PathBuf {
    std::env::var_os("FONTCRAFT_CORPUS").map_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus"), PathBuf::from)
}

/// Every `.glyphs` file and `.glyphspackage` folder under `dir`, sorted.
fn sources(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else { continue };
        for e in entries.flatten() {
            let p = e.path();
            let ext = p.extension().and_then(|x| x.to_str()).unwrap_or("");
            if p.is_dir() {
                if ext == "glyphspackage" {
                    out.push(p);
                } else if !p.is_symlink() {
                    stack.push(p);
                }
            } else if ext == "glyphs" {
                out.push(p);
            }
        }
    }
    out.sort();
    out
}

fn read(path: &Path) -> Option<Font> {
    if path.is_dir() {
        let mut files = fontcraft_io_glyphs::Files::new();
        let mut stack = vec![path.to_path_buf()];
        while let Some(d) = stack.pop() {
            for e in std::fs::read_dir(&d).ok()?.flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else {
                    let key = p.strip_prefix(path).ok()?.to_string_lossy().replace('\\', "/");
                    files.insert(key, std::fs::read(&p).ok()?);
                }
            }
        }
        fontcraft_io_glyphs::read_package(&files).ok()
    } else {
        fontcraft_io_glyphs::read(&std::fs::read_to_string(path).ok()?).ok()
    }
}

fn kind_name(kind: EntryKind) -> &'static str {
    match kind {
        EntryKind::Prefix => "prefix",
        EntryKind::Class => "class",
        EntryKind::Feature => "feature",
    }
}

fn normalise(code: &str) -> String {
    code.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The items two codes are compared by: a class's glyph names, else the statements.
fn items(kind: EntryKind, code: &str) -> Vec<String> {
    if kind == EntryKind::Class {
        return code.split_whitespace().map(str::to_string).collect();
    }
    code.lines().map(|l| l.split('#').next().unwrap_or("")).flat_map(|l| l.split([';', '{', '}'])).map(normalise).filter(|s| !s.is_empty()).collect()
}

#[derive(Debug, PartialEq)]
enum Outcome {
    Equal,
    /// Only ours, only stored.
    Different(Vec<String>, Vec<String>),
    Removed,
    NoGenerator,
}

/// One stored automatic entry and what Update gives it.
struct Compared {
    kind: EntryKind,
    name: String,
    outcome: Outcome,
}

/// Update on `font`'s exporting glyphs, compared with its stored automatic entries; the second
/// list holds the entries Update adds.
fn compare(font: &Font, data: &GlyphData) -> (Vec<Compared>, Vec<(EntryKind, String)>) {
    let cx = AutoContext::from_font(font, data, None).unwrap();
    let report = update(&font.features, &cx).unwrap();
    let known: Vec<&str> = fontcraft_features::auto::generators().iter().flat_map(|g| g.names().iter().copied()).collect();
    let lists = |f: &fontcraft_model::Features| -> Vec<(EntryKind, Vec<Snippet>)> {
        vec![(EntryKind::Prefix, f.prefixes.clone()), (EntryKind::Class, f.classes.clone()), (EntryKind::Feature, f.features.clone())]
    };
    let after = lists(&report.features);
    let mut out = Vec::new();
    for (kind, stored) in lists(&font.features) {
        let ours = &after.iter().find(|(k, _)| *k == kind).unwrap().1;
        for s in stored.iter().filter(|s| s.automatic) {
            let outcome = if !known.contains(&s.name.as_str()) {
                Outcome::NoGenerator
            } else {
                match ours.iter().find(|o| o.name == s.name) {
                    None => Outcome::Removed,
                    Some(o) if normalise(&o.code) == normalise(&s.code) => Outcome::Equal,
                    Some(o) => {
                        let (a, b) = (items(kind, &o.code), items(kind, &s.code));
                        let only_ours = a.iter().filter(|x| !b.contains(x)).cloned().collect();
                        let only_stored = b.iter().filter(|x| !a.contains(x)).cloned().collect();
                        Outcome::Different(only_ours, only_stored)
                    }
                }
            };
            out.push(Compared { kind, name: s.name.clone(), outcome });
        }
    }
    (out, report.added)
}

/// Stored automatic entries Update reproduces exactly, by source and entry name.
const EQUAL: &[(&str, &[&str])] = &[
    ("markow/sources/Markow.glyphs", &["Languagesystems", "ccmp", "locl", "pnum", "tnum", "case", "ss01", "ss02"]),
    ("oswald/sources/Oswald.glyphs", &["Languagesystems", "Uppercase", "aalt", "sups", "frac", "ordn", "case", "dlig", "liga"]),
    ("oswald/sources/Italic_WIP/Oswald-Italic.glyphs", &["Languagesystems", "aalt", "sups", "frac", "ordn", "liga"]),
    ("glyphspack/tests/fixtures/Andika Mini 4.glyphs", &["Languagesystems", "Uppercase"]),
];

#[test]
fn update_reproduces_the_stored_automatic_code_of_known_sources() {
    let root = corpus();
    let data = GlyphData::new().unwrap();
    for (path, names) in EQUAL {
        let Some(font) = read(&root.join(path)) else {
            eprintln!("skipped: {path} is not in the corpus (cargo xtask corpus --fetch)");
            continue;
        };
        let (compared, _) = compare(&font, &data);
        for name in *names {
            let c = compared.iter().find(|c| c.name == *name).unwrap_or_else(|| panic!("{path}: no stored automatic `{name}`"));
            assert_eq!(c.outcome, Outcome::Equal, "{path}: {name}");
        }
    }
}

#[test]
#[ignore = "reads every corpus Glyphs source: the M8 automatic-code measure"]
fn automatic_code_against_the_stored_output() {
    let root = corpus();
    if !root.is_dir() {
        eprintln!("skipped: no corpus at {} (cargo xtask corpus --fetch)", root.display());
        return;
    }
    let filter = std::env::var("AUTO_MATCH").ok();
    let list: usize = std::env::var("AUTO_LIST").ok().and_then(|s| s.parse().ok()).unwrap_or(60);
    let data = GlyphData::new().unwrap();
    // name → [equal, different, removed, no generator]
    let mut counts: BTreeMap<String, [usize; 4]> = BTreeMap::new();
    let mut shown = 0;
    let (mut read_count, mut unreadable, mut with_auto) = (0, 0, 0);
    let mut added_total: BTreeMap<String, usize> = BTreeMap::new();
    for path in sources(&root) {
        let rel = path.strip_prefix(&root).unwrap_or(&path).display().to_string();
        if filter.as_deref().is_some_and(|f| !rel.contains(f)) {
            continue;
        }
        let Some(font) = read(&path) else {
            unreadable += 1;
            continue;
        };
        read_count += 1;
        let f = &font.features;
        if !f.prefixes.iter().chain(&f.classes).chain(&f.features).any(|s| s.automatic) {
            continue;
        }
        with_auto += 1;
        let (compared, added) = compare(&font, &data);
        let equal: Vec<&str> = compared.iter().filter(|c| c.outcome == Outcome::Equal).map(|c| c.name.as_str()).collect();
        println!("EQUAL {rel}: {}", equal.join(" "));
        for c in &compared {
            let key = format!("{} {}", kind_name(c.kind), c.name);
            let slot = counts.entry(key).or_default();
            match &c.outcome {
                Outcome::Equal => slot[0] += 1,
                Outcome::Different(ours, stored) => {
                    slot[1] += 1;
                    if shown < list {
                        shown += 1;
                        println!("DIFF {rel}: {} {} ({} only ours, {} only stored)", kind_name(c.kind), c.name, ours.len(), stored.len());
                        for x in ours.iter().take(4) {
                            println!("    + {x}");
                        }
                        for x in stored.iter().take(4) {
                            println!("    - {x}");
                        }
                    }
                }
                Outcome::Removed => {
                    slot[2] += 1;
                    println!("REMOVED {rel}: {} {}", kind_name(c.kind), c.name);
                }
                Outcome::NoGenerator => slot[3] += 1,
            }
        }
        if !added.is_empty() {
            let names: Vec<String> = added.iter().map(|(k, n)| format!("{} {n}", kind_name(*k))).collect();
            println!("ADDED {rel}: {}", names.join(", "));
            for n in names {
                *added_total.entry(n).or_default() += 1;
            }
        }
    }
    println!("\n{read_count} sources read, {unreadable} unreadable, {with_auto} with automatic entries");
    println!("{:<28} {:>5} {:>9} {:>7} {:>12}", "entry", "equal", "different", "removed", "no generator");
    let mut total = [0usize; 4];
    for (name, c) in &counts {
        println!("{name:<28} {:>5} {:>9} {:>7} {:>12}", c[0], c[1], c[2], c[3]);
        for (t, v) in total.iter_mut().zip(c) {
            *t += v;
        }
    }
    println!("{:<28} {:>5} {:>9} {:>7} {:>12}", "total", total[0], total[1], total[2], total[3]);
    println!("added by Update: {added_total:?}");
}
