//! The kern measure of M5.17 (`docs/plans/M5.md` § 5): for every corpus source without feature
//! code that both reference compilers build, the effective value of every pair of glyphs that a
//! stored pair of the default master covers (groups expanded, at most 1,000,000 pairs per
//! source) is the same in FontCraft's `GPOS` as in fontc's (built with features) and fontmake's.
//! Pairs are compared in every script either font has records for.
//!
//! Opt-in and slow: `cargo test -p fontcraft-oracle --features oracle --test kern -- --ignored
//! --nocapture`. `FONTCRAFT_CORPUS` names the corpus folder (default: the repository's
//! `corpus/`). Skips, with a message, when a tool is not installed.
#![cfg(feature = "oracle")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use fontcraft_compile::{Options, compile};
use fontcraft_engine::io::open;
use fontcraft_features::evaluate::PairEvaluator;
use fontcraft_model::{Font, side_group};
use fontcraft_oracle::tools::{Request, build_with_features, locate};
use fontcraft_oracle::{Flavour, Hint, Oracle};
use fontcraft_spacing::kerning::{Direction, KernIndex, Position, facing};
use skrifa::FontRef;
use skrifa::raw::TableProvider;
use skrifa::raw::types::{GlyphId16, Tag};

const MAX_PAIRS: usize = 1_000_000;
/// Differences printed per source and reference.
const SHOWN: usize = 20;

fn corpus() -> PathBuf {
    std::env::var_os("FONTCRAFT_CORPUS").map_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus"), PathBuf::from)
}

fn has_feature_code(font: &Font) -> bool {
    let f = &font.features;
    f.prefixes.iter().chain(&f.classes).chain(&f.features).any(|s| !s.disabled && !s.code.trim().is_empty())
}

/// The glyph pairs the default master's stored pairs cover, by source glyph name.
fn covered(font: &Font, index: &KernIndex, master: &str) -> Vec<(String, String)> {
    let mut out = BTreeSet::new();
    for direction in [Direction::Ltr, Direction::Rtl] {
        let Ok(table) = index.table(direction, master) else { continue };
        let side = |stored: &str, position: Position| -> Vec<String> {
            match side_group(stored) {
                Some(group) => index.members(facing(direction, position), group).map(str::to_string).collect(),
                None => font.glyph(stored).map(|g| vec![g.name.clone()]).unwrap_or_default(),
            }
        };
        for (first, second, _) in table.pairs() {
            for a in side(first, Position::First) {
                for b in side(second, Position::Second) {
                    if out.len() >= MAX_PAIRS {
                        return out.into_iter().collect();
                    }
                    out.insert((a.clone(), b));
                }
            }
        }
    }
    out.into_iter().collect()
}

/// Glyph id by `post` name.
fn ids_by_name(bytes: &[u8]) -> Option<HashMap<String, u16>> {
    let font = FontRef::new(bytes).ok()?;
    let post = font.post().ok()?;
    let count = font.maxp().ok()?.num_glyphs();
    let mut out = HashMap::new();
    for gid in 0..count {
        out.insert(post.glyph_name(GlyphId16::new(gid))?.to_string(), gid);
    }
    Some(out)
}

struct Difference {
    oracle: &'static str,
    script: Tag,
    first: String,
    second: String,
    ours: i32,
    theirs: i32,
}

#[test]
#[ignore = "builds every corpus source with fontc and fontmake: the M5.17 kern measure"]
fn generated_kerning_equals_both_references() {
    let Some(fontc) = locate(Oracle::Fontc, None) else {
        eprintln!("skipped: fontc is not installed ({})", Oracle::Fontc.install_hint());
        return;
    };
    let Some(fontmake) = locate(Oracle::Fontmake, None) else {
        eprintln!("skipped: fontmake is not installed ({})", Oracle::Fontmake.install_hint());
        return;
    };
    let sources = fontcraft_oracle::corpus::discover(&corpus()).unwrap();
    let scratch = std::env::temp_dir().join(format!("fontcraft-kern-measure-{}", std::process::id()));
    let (mut compared, mut pairs_compared, mut skipped) = (0usize, 0usize, Vec::new());
    let mut all: Vec<(String, Difference)> = Vec::new();
    for (n, source) in sources.iter().enumerate() {
        let label = source.display().to_string();
        let Ok((font, _)) = open(source, None) else {
            skipped.push(format!("{label}: not read"));
            continue;
        };
        if has_feature_code(&font) {
            continue;
        }
        let Ok(master) = font.default_master().map(|m| m.id.clone()) else { continue };
        let index = KernIndex::new(&font);
        let pairs = covered(&font, &index, &master);
        if pairs.is_empty() {
            continue;
        }
        let options = Options { timestamp: fontcraft_oracle::DEFAULT_TIMESTAMP, ..Default::default() };
        let ours = match compile(&font, &font.default_target().unwrap(), fontcraft_compile::Flavour::TrueType, &options) {
            Ok(c) => c,
            Err(e) => {
                skipped.push(format!("{label}: FontCraft: {e}"));
                continue;
            }
        };
        let our_ids = ids_by_name(&ours.bytes).unwrap_or_default();
        let our_names: Vec<String> = {
            let mut v = vec![String::new(); our_ids.len()];
            for (name, gid) in &our_ids {
                v[usize::from(*gid)] = name.clone();
            }
            v
        };
        let our_eval = PairEvaluator::new(&ours.bytes).unwrap();
        let hint = Hint { family: font.family_name.clone(), style: font.default_master().unwrap().name.clone() };
        let mut built = Vec::new();
        for (oracle, tool) in [(Oracle::Fontc, &fontc), (Oracle::Fontmake, &fontmake)] {
            let work = scratch.join(format!("{n}-{}", oracle.name()));
            let _ = std::fs::remove_dir_all(&work);
            std::fs::create_dir_all(&work).unwrap();
            let req = Request {
                oracle,
                tool,
                source,
                flavour: Flavour::TrueType,
                work_dir: &work,
                timestamp: fontcraft_oracle::DEFAULT_TIMESTAMP,
                timeout: fontcraft_oracle::DEFAULT_TIMEOUT,
                hint: &hint,
            };
            match build_with_features(&req).and_then(|b| std::fs::read(&b.path).map_err(|e| e.to_string())) {
                Ok(bytes) => built.push((oracle.name(), bytes)),
                Err(e) => skipped.push(format!("{label}: {}: {e}", oracle.name())),
            }
        }
        if built.len() < 2 {
            continue;
        }
        compared += 1;
        for (oracle, bytes) in &built {
            let Some(their_ids) = ids_by_name(bytes) else {
                skipped.push(format!("{label}: {oracle}: no glyph names to match glyphs by"));
                continue;
            };
            let their_eval = PairEvaluator::new(bytes).unwrap();
            let scripts: BTreeSet<Tag> = our_eval.scripts().into_iter().chain(their_eval.scripts()).collect();
            let mut shown = 0usize;
            for (a, b) in &pairs {
                let ours_ab = [a, b].map(|name| ours.glyph_order.iter().position(|g| g == name).and_then(|i| u16::try_from(i).ok()));
                let [Some(oa), Some(ob)] = ours_ab else { continue };
                let names = [&our_names[usize::from(oa)], &our_names[usize::from(ob)]];
                let (Some(ta), Some(tb)) = (their_ids.get(names[0]), their_ids.get(names[1])) else { continue };
                pairs_compared += 1;
                for script in &scripts {
                    let (o, t) = (our_eval.value(*script, oa, ob), their_eval.value(*script, *ta, *tb));
                    if o != t {
                        if shown < SHOWN {
                            eprintln!("{label}: {oracle} {script} {a} {b}: ours {o}, theirs {t}");
                        }
                        shown += 1;
                        all.push((label.clone(), Difference { oracle, script: *script, first: a.clone(), second: b.clone(), ours: o, theirs: t }));
                    }
                }
            }
        }
    }
    let _ = std::fs::remove_dir_all(&scratch);
    eprintln!("sources compared: {compared}; pairs compared (per reference): {pairs_compared}; differences: {}", all.len());
    for s in &skipped {
        eprintln!("skipped: {s}");
    }
    let mut by_source: HashMap<(&str, &str), usize> = HashMap::new();
    for (label, d) in &all {
        *by_source.entry((label.as_str(), d.oracle)).or_default() += 1;
    }
    for ((label, oracle), count) in &by_source {
        eprintln!("{label}: {count} differences from {oracle}");
    }
    if let Some((label, d)) = all.first() {
        panic!(
            "{} pair values differ from the references; the first: {label} {} {} {} {}: ours {}, theirs {}",
            all.len(),
            d.oracle,
            d.script,
            d.first,
            d.second,
            d.ours,
            d.theirs
        );
    }
}
