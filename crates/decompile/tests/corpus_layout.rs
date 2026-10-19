//! The reconstruction measure of `docs/plans/M8.md` § 5 (`open.reconstructsFeatures`): every
//! compiled font with layout tables is opened, its document compiled again, and the `GSUB`,
//! `GPOS` and `GDEF` of the two fonts compared, glyphs by glyph id.
//!
//! - **identical**: the same lookups at the same indices and the same feature records.
//! - **equivalent**: the same lookups and feature records once lookups are compared by their
//!   rules (and the rules of the lookups they call) instead of their indices.
//! - **different**: anything else; the first few differences are printed.
//!
//! The `kern` feature is compared by the value every pair it covers gets (pair lookups of every
//! `kern` record, applied as a shaper applies them), since its lookups are generated again from
//! the document's kerning; the rest of `GPOS` is compared without it.
//!
//! It needs the corpus (`cargo xtask corpus --fetch`), or a folder of fonts in
//! `FONTCRAFT_LAYOUT_DIR`, and is left out of the normal test run:
//! `cargo test -p fontcraft-decompile --release --test corpus_layout -- --ignored --nocapture`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::collections::hash_map::DefaultHasher;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use fontcraft_compile::{Options, compile_ttf};
use fontcraft_decompile::decompile;
use fontcraft_decompile::fea::{LayoutSummary, TableSummary, layout_summary};
use skrifa::raw::tables::gpos::{PairPos, PositionSubtables};
use skrifa::raw::types::{GlyphId16, Tag};
use skrifa::raw::{FontRef, TableProvider};

/// Most covered kerning pairs compared per font.
const MAX_PAIRS: usize = 2_000_000;

fn fonts(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let path = e.path();
        let Ok(meta) = std::fs::metadata(&path) else { continue };
        if meta.is_dir() && depth < 32 {
            fonts(&path, depth + 1, out);
        } else if meta.is_file()
            && path.extension().and_then(|x| x.to_str()).is_some_and(|x| matches!(x.to_ascii_lowercase().as_str(), "ttf" | "otf"))
        {
            out.push(path);
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Verdict {
    Identical,
    Equivalent,
    Different,
}

/// Lookup signatures: the rules, with the signatures of the lookups they call.
fn signatures(t: &TableSummary) -> HashMap<u16, u64> {
    let mut out: HashMap<u16, u64> = HashMap::new();
    fn sig(t: &TableSummary, li: u16, out: &mut HashMap<u16, u64>, depth: usize) -> u64 {
        if let Some(s) = out.get(&li) {
            return *s;
        }
        let mut h = DefaultHasher::new();
        if let Some(l) = t.lookups.get(&li) {
            l.kind.hash(&mut h);
            l.lines.hash(&mut h);
            let mut calls: Vec<u64> =
                if depth < 64 { l.calls.iter().filter(|c| **c != li).map(|c| sig(t, *c, out, depth + 1)).collect() } else { Vec::new() };
            calls.sort_unstable();
            calls.hash(&mut h);
        }
        let s = h.finish();
        out.insert(li, s);
        s
    }
    for li in t.lookups.keys() {
        sig(t, *li, &mut out, 0);
    }
    out
}

/// The table as multisets of lookup and feature-record signatures.
fn shape(t: &TableSummary) -> (Vec<u64>, Vec<String>) {
    let sigs = signatures(t);
    let mut lookups: Vec<u64> = sigs.values().copied().collect();
    lookups.sort_unstable();
    let mut features: Vec<String> = t
        .features
        .iter()
        .map(|f| {
            let mut ls: Vec<u64> = f.lookups.iter().map(|l| sigs.get(l).copied().unwrap_or(0)).collect();
            ls.sort_unstable();
            format!("{} {} {} {} {ls:?}", f.tag, f.script, f.language, f.required)
        })
        .collect();
    features.sort();
    (lookups, features)
}

fn compare(tag: &str, a: &LayoutSummary, b: &LayoutSummary, notes: &mut Vec<String>) -> Verdict {
    let find = |s: &LayoutSummary| s.tables.iter().find(|t| t.tag == tag).cloned().unwrap_or_default();
    let (ta, tb) = (find(a), find(b));
    if ta == tb {
        return Verdict::Identical;
    }
    let (sa, sb) = (shape(&ta), shape(&tb));
    if sa == sb {
        return Verdict::Equivalent;
    }
    let only = |x: &[u64], y: &[u64]| x.iter().filter(|s| !y.contains(s)).count();
    notes.push(format!(
        "{tag}: {} lookups, {} after; {} lookups only before, {} only after; features {} before, {} after",
        ta.lookups.len(),
        tb.lookups.len(),
        only(&sa.0, &sb.0),
        only(&sb.0, &sa.0),
        sa.1.len(),
        sb.1.len()
    ));
    // The first lookup that differs, by index, when the counts match.
    if let Some((li, la)) = ta.lookups.iter().find(|(li, l)| tb.lookups.get(li) != Some(l)) {
        let lb = tb.lookups.get(li);
        let first = la.lines.iter().find(|x| !lb.is_some_and(|lb| lb.lines.contains(x)));
        notes.push(format!("{tag} lookup {li}: kind {} → {:?}; first line only before: {first:?}", la.kind, lb.map(|l| l.kind)));
    }
    if let Some(f) = sa.1.iter().find(|f| !sb.1.contains(f)) {
        notes.push(format!("{tag} feature record only before: {f}"));
    }
    Verdict::Different
}

/// The pair lookups of every `kern` record, in lookup-list order.
fn kern_lookups(font: &FontRef) -> Vec<u16> {
    let Ok(gpos) = font.gpos() else { return Vec::new() };
    let Ok(features) = gpos.feature_list() else { return Vec::new() };
    let mut out: Vec<u16> = features
        .feature_records()
        .iter()
        .filter(|r| r.feature_tag() == Tag::new(b"kern"))
        .filter_map(|r| r.feature(features.offset_data()).ok())
        .flat_map(|f| f.lookup_list_indices().iter().map(|i| i.get()).collect::<Vec<_>>())
        .collect();
    out.sort_unstable();
    out.dedup();
    out
}

fn pair_value(font: &FontRef, lookups: &[u16], g1: GlyphId16, g2: GlyphId16) -> i32 {
    let Ok(list) = font.gpos().and_then(|g| g.lookup_list()) else { return 0 };
    let mut total = 0;
    for &li in lookups {
        let Ok(lookup) = list.lookups().get(usize::from(li)) else { continue };
        let Ok(PositionSubtables::Pair(subtables)) = lookup.subtables() else { continue };
        for sub in subtables.iter().flatten() {
            match sub {
                PairPos::Format1(t) => {
                    let Some(i) = t.coverage().ok().and_then(|c| c.get(g1)) else { continue };
                    let Ok(set) = t.pair_sets().get(usize::from(i)) else { continue };
                    if let Some(r) = set.find_pair_value_record(g2) {
                        total += r.value_record1().x_advance().map_or(0, i32::from);
                        break;
                    }
                }
                PairPos::Format2(t) => {
                    if t.coverage().ok().and_then(|c| c.get(g1)).is_none() {
                        continue;
                    }
                    let (Ok(cd1), Ok(cd2)) = (t.class_def1(), t.class_def2()) else { break };
                    if let Some([v, _]) = t.value_record_refs(cd1.get(g1), cd2.get(g2)) {
                        total += v.x_advance().map_or(0, i32::from);
                    }
                    break;
                }
            }
        }
    }
    total
}

fn covered(font: &FontRef, lookups: &[u16], out: &mut HashSet<(u16, u16)>) {
    let Ok(list) = font.gpos().and_then(|g| g.lookup_list()) else { return };
    for &li in lookups {
        let Ok(lookup) = list.lookups().get(usize::from(li)) else { continue };
        let Ok(PositionSubtables::Pair(subtables)) = lookup.subtables() else { continue };
        for sub in subtables.iter().flatten() {
            match sub {
                PairPos::Format1(t) => {
                    let Ok(cov) = t.coverage() else { continue };
                    for g1 in cov.iter() {
                        let Some(Ok(set)) = cov.get(g1).map(|i| t.pair_sets().get(usize::from(i))) else { continue };
                        out.extend(set.pair_value_records().iter().flatten().map(|r| (g1.to_u16(), r.second_glyph().to_u16())));
                    }
                }
                PairPos::Format2(t) => {
                    let (Ok(cov), Ok(cd2)) = (t.coverage(), t.class_def2()) else { continue };
                    let seconds: Vec<u16> = cd2.iter().filter(|(_, c)| *c != 0).map(|(g, _)| g.to_u16()).collect();
                    for g1 in cov.iter() {
                        out.extend(seconds.iter().map(|&g2| (g1.to_u16(), g2)));
                    }
                }
            }
            if out.len() > MAX_PAIRS {
                return;
            }
        }
    }
}

/// Kerning pairs whose value differs between the two fonts, and the pairs compared.
fn kern_differences(a: &[u8], b: &[u8]) -> (usize, usize) {
    let (fa, fb) = (FontRef::new(a).unwrap(), FontRef::new(b).unwrap());
    let (la, lb) = (kern_lookups(&fa), kern_lookups(&fb));
    let mut pairs = HashSet::new();
    covered(&fa, &la, &mut pairs);
    covered(&fb, &lb, &mut pairs);
    let mut wrong = 0;
    for &(g1, g2) in pairs.iter().take(MAX_PAIRS) {
        let (g1, g2) = (GlyphId16::new(g1), GlyphId16::new(g2));
        if pair_value(&fa, &la, g1, g2) != pair_value(&fb, &lb, g1, g2) {
            wrong += 1;
        }
    }
    (wrong, pairs.len().min(MAX_PAIRS))
}

#[test]
#[ignore = "needs the corpus and compiles every compiled font with layout tables"]
fn corpus_fonts_reconstruct_their_layout() {
    let root = std::env::var_os("FONTCRAFT_LAYOUT_DIR").map_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus"), PathBuf::from);
    if !root.exists() {
        eprintln!("skipped: no fonts at {}", root.display());
        return;
    }
    let mut paths = Vec::new();
    fonts(&root, 0, &mut paths);
    paths.sort();
    let mut totals: BTreeMap<(String, Verdict), usize> = BTreeMap::new();
    let (mut checked, mut failed, mut kern_fonts, mut kern_wrong) = (0, 0, 0, 0);
    let mut not_written: BTreeMap<String, usize> = BTreeMap::new();
    for path in &paths {
        let Ok(bytes) = std::fs::read(path) else { continue };
        let Ok(font) = FontRef::new(&bytes) else { continue };
        if !["GSUB", "GPOS", "GDEF"].iter().any(|t| font.table_data(Tag::new_checked(t.as_bytes()).unwrap()).is_some()) {
            continue;
        }
        checked += 1;
        let name = path.strip_prefix(&root).unwrap_or(path).display().to_string();
        let opened = match decompile(&bytes, None) {
            Ok(d) => d,
            Err(e) => {
                failed += 1;
                println!("{name}: does not open: {e}");
                continue;
            }
        };
        for w in &opened.warnings {
            if let Some(rest) = w.strip_prefix("the feature code made from the layout tables leaves out: ") {
                for item in rest.split("; ") {
                    *not_written.entry(item.split(" and ").next().unwrap_or(item).to_string()).or_default() += 1;
                }
            } else if w.contains("feature code") || w.contains("stays feature code") {
                println!("{name}: {w}");
            }
        }
        let again = match compile_ttf(&opened.font, &Options::default()) {
            Ok(c) => c.bytes,
            Err(e) => {
                failed += 1;
                println!("{name}: does not export: {e}");
                continue;
            }
        };
        let count = font.maxp().map(|m| usize::from(m.num_glyphs())).unwrap_or(0);
        let ids: Vec<String> = (0..count).map(|i| format!("g{i}")).collect();
        let summarise = |b: &[u8], exclude: &[&str]| layout_summary(b, &ids, exclude);
        let (Ok(a), Ok(b), Ok(ak), Ok(bk)) =
            (summarise(&bytes, &[]), summarise(&again, &[]), summarise(&bytes, &["kern"]), summarise(&again, &["kern"]))
        else {
            failed += 1;
            println!("{name}: layout cannot be summarised");
            continue;
        };
        let mut notes = Vec::new();
        let gsub = compare("GSUB", &a, &b, &mut notes);
        let gpos = compare("GPOS", &ak, &bk, &mut notes);
        let gdef = if a.gdef == b.gdef { Verdict::Identical } else { Verdict::Different };
        if gdef == Verdict::Different {
            notes.push(format!("GDEF: {} lines before, {} after", a.gdef.len(), b.gdef.len()));
        }
        for (t, v) in [("GSUB", gsub), ("GPOS without kern", gpos), ("GDEF", gdef)] {
            *totals.entry((t.to_string(), v)).or_default() += 1;
        }
        let (wrong, pairs) = kern_differences(&bytes, &again);
        if pairs > 0 {
            kern_fonts += 1;
            if wrong > 0 {
                kern_wrong += 1;
                notes.push(format!("kern: {wrong} of {pairs} pairs differ"));
            }
        }
        if !notes.is_empty() {
            println!("{name}:");
            for n in notes.iter().take(6) {
                println!("    {n}");
            }
        }
    }
    println!("\n{checked} fonts with layout tables; {failed} failed to open, export or summarise");
    for ((t, v), n) in &totals {
        println!("{t}: {v:?} {n}");
    }
    println!("kern: {kern_fonts} fonts with pair kerning, {kern_wrong} with pairs whose value differs");
    println!("not written as feature code (fonts):");
    for (what, n) in &not_written {
        println!("    {n} × {what}");
    }
}
