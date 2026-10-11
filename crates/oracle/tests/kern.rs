//! The kern measure of M5.17 (`docs/plans/M5.md` § 5): for every corpus source without a
//! hand-written `kern` feature that both reference compilers build, the effective value of every
//! pair of glyphs that a stored pair of the compiled master covers (groups expanded, at most 1,000,000 pairs per
//! source) is the same in FontCraft's `GPOS` as in fontc's (built with features) and fontmake's.
//! Pairs are compared in every script either font has records for. The sources are the oracle
//! corpus and Inter's roman `.glyphspackage` when it is fetched. A multi-master `.glyphs` source
//! is compiled at its origin master, the default of fontc's variable font.
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
use fontcraft_features::evaluate::{PairEvaluator, script_tags};
use fontcraft_model::{Font, side_group};
use fontcraft_oracle::tools::{Request, build_with_features, locate};
use fontcraft_oracle::{Flavour, Hint, Oracle};
use fontcraft_spacing::kerning::{Direction, KernIndex, Position, facing};
use skrifa::raw::TableProvider;
use skrifa::raw::types::{GlyphId16, Tag};
use skrifa::{FontRef, MetadataProvider};

const MAX_PAIRS: usize = 1_000_000;
/// Differences printed per source and reference.
const SHOWN: usize = 20;

/// Differences with a known cause, written down in `docs/spec/40-compile.md` § 4a: the source's
/// file name, the reference, the script, and the cause. They are reported and do not fail the
/// measure.
const KNOWN: &[(&str, &str, &str, &str)] = &[
    (
        "KerningRtlOnlyMaster.glyphs",
        "fontc",
        "hebr",
        "fontc builds the variable font of both masters: the second master's right-to-left pairs give its font a hebr script whose lookups hold only them, and a Latin pair read in hebr gets 0; FontCraft builds the first master, which has no right-to-left pairs, so hebr falls back to DFLT",
    ),
    ("Oswald.glyphs", "fontc", "cyrl", OSWALD_LIGATURES),
    ("Oswald.glyphs", "fontmake", "cyrl", OSWALD_LIGATURES),
];

const OSWALD_LIGATURES: &str = "the unencoded ligature f_f gets the Latin script from the liga substitution that makes it, so the references put its pairs in the Latin lookup; FontCraft compiles no feature code before M8, so f_f has no script and its pairs are common, in every script's lookups";

/// Sources whose compiled kerning differs from the stored pairs `KernIndex` reads, with the
/// cause: the lookup measure reports them and does not fail on them.
const LOOKUP_KNOWN: &[(&str, &str)] = &[
    ("FeatureWritersEmpty.glyphs", "the source's feature-writer setting turns the kern writer off, so the compiled fonts have no pair kerning"),
    ("FeaInc-Bold.ufo", "the hand-written kern feature, in an included file, gives the pair another value than the stored pair"),
];

/// Sources of the measure outside the oracle corpus, when fetched.
const EXTRA: &[&str] = &["inter/src/Inter-Roman.glyphspackage"];

fn known(label: &str, d: &Difference) -> Option<&'static str> {
    let name = Path::new(label).file_name()?.to_str()?;
    let script = d.script.to_string();
    KNOWN.iter().find(|(n, o, s, _)| *n == name && *o == d.oracle && *s == script).map(|(_, _, _, cause)| *cause)
}

fn corpus() -> PathBuf {
    std::env::var_os("FONTCRAFT_CORPUS").map_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus"), PathBuf::from)
}

/// The glyph pairs the stored pairs of `master` cover, by source glyph name.
fn covered(font: &Font, index: &KernIndex, master: &str) -> Vec<(String, String)> {
    let pairs: BTreeSet<(String, String)> = covered_by_direction(font, index, master).into_iter().map(|(_, a, b)| (a, b)).collect();
    pairs.into_iter().collect()
}

/// The glyph pairs each direction's stored pairs of `master` cover, by source glyph name, with
/// `true` for right-to-left.
fn covered_by_direction(font: &Font, index: &KernIndex, master: &str) -> Vec<(bool, String, String)> {
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
                    out.insert((direction == Direction::Rtl, a.clone(), b));
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
    let mut sources = fontcraft_oracle::corpus::discover(&corpus()).unwrap();
    sources.extend(EXTRA.iter().map(|p| corpus().join(p)).filter(|p| p.exists()));
    let scratch = std::env::temp_dir().join(format!("fontcraft-kern-measure-{}", std::process::id()));
    let (mut compared, mut skipped) = (0usize, Vec::new());
    let mut pairs_compared: HashMap<&str, usize> = HashMap::new();
    let mut with_code = 0usize;
    let mut all: Vec<(String, Difference)> = Vec::new();
    for (n, source) in sources.iter().enumerate() {
        let label = source.display().to_string();
        let Ok((font, _)) = open(source, None) else {
            skipped.push(format!("{label}: not read"));
            continue;
        };
        // fontc builds a `.glyphs` source with several masters as a variable font, whose default
        // is the origin master; fontmake's file of that master is compared too.
        let glyphs = matches!(source.extension().and_then(|e| e.to_str()), Some("glyphs" | "glyphspackage"));
        let compiled = if glyphs { fontcraft_model::axes::origin(&font) } else { font.default_master().ok() };
        let Some((master, master_name)) = compiled.map(|m| (m.id.clone(), m.name.clone())) else { continue };
        let index = KernIndex::new(&font);
        let pairs = covered(&font, &index, &master);
        if pairs.is_empty() {
            continue;
        }
        if fontcraft_compile::has_hand_written_kern(&font) {
            eprintln!("not compared, a hand-written kern feature: {label}");
            with_code += 1;
            continue;
        }
        let options = Options { timestamp: fontcraft_oracle::DEFAULT_TIMESTAMP, ..Default::default() };
        let ours = match compile(&font, &fontcraft_model::Target::Master(master.clone()), fontcraft_compile::Flavour::TrueType, &options) {
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
        let hint = Hint { family: font.family_name.clone(), style: master_name.clone() };
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
        eprintln!("compared: {label} ({} pairs)", pairs.len());
        for (oracle, bytes) in &built {
            let Some(their_ids) = ids_by_name(bytes) else {
                skipped.push(format!("{label}: {oracle}: no glyph names to match glyphs by"));
                continue;
            };
            let their_eval = PairEvaluator::new(bytes).unwrap();
            let their_font = FontRef::new(bytes).unwrap();
            let their_cmap = their_font.charmap();
            let scripts: BTreeSet<Tag> = our_eval.scripts().into_iter().chain(their_eval.scripts()).collect();
            let mut shown = 0usize;
            for (a, b) in &pairs {
                let ours_ab = [a, b].map(|name| ours.glyph_order.iter().position(|g| g == name).and_then(|i| u16::try_from(i).ok()));
                let [Some(oa), Some(ob)] = ours_ab else { continue };
                // An encoded glyph is found by its code point, so that a production name given
                // differently does not pair other glyphs; an unencoded one by its `post` name.
                let theirs_of = |name: &str, ours: u16| -> Option<u16> {
                    match font.glyph(name).and_then(|g| g.unicodes.first().copied()) {
                        Some(cp) => their_cmap.map(cp).and_then(|g| u16::try_from(g.to_u32()).ok()),
                        None => their_ids.get(&our_names[usize::from(ours)]).copied(),
                    }
                };
                let (Some(ta), Some(tb)) = (theirs_of(a, oa), theirs_of(b, ob)) else { continue };
                let (ta, tb) = (&ta, &tb);
                *pairs_compared.entry(oracle).or_default() += 1;
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
    eprintln!("sources with kerning and a hand-written kern feature (not compared): {with_code}");
    eprintln!("sources compared: {compared}; pairs compared: {pairs_compared:?}; differences: {}", all.len());
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
    let (expected, unexpected): (Vec<_>, Vec<_>) = all.iter().partition(|(label, d)| known(label, d).is_some());
    for (label, d) in &expected {
        eprintln!("known: {label} {} {} {} {}: {}", d.oracle, d.script, d.first, d.second, known(label, d).unwrap_or_default());
    }
    if let Some((label, d)) = unexpected.first() {
        panic!(
            "{} pair values differ from the references without a known cause; the first: {label} {} {} {} {}: ours {}, theirs {}",
            unexpected.len(),
            d.oracle,
            d.script,
            d.first,
            d.second,
            d.ours,
            d.theirs
        );
    }
}

/// The lookup half of the § 5 kerning measure: for every corpus source with kerning that both
/// references build, hand-written `kern` feature or not, the value `KernIndex` gives each covered
/// pair of the compiled master (rounded half up) equals the references' effective value. A pair
/// is read in the OpenType scripts of its glyphs' code points, or in `DFLT` when both glyphs are
/// common or unencoded. A source that differs fails the measure unless `LOOKUP_KNOWN` gives the
/// cause.
#[test]
#[ignore = "builds every corpus source with fontc and fontmake: the M5 kerning lookup measure"]
fn index_lookup_equals_both_references() {
    let Some(fontc) = locate(Oracle::Fontc, None) else {
        eprintln!("skipped: fontc is not installed ({})", Oracle::Fontc.install_hint());
        return;
    };
    let Some(fontmake) = locate(Oracle::Fontmake, None) else {
        eprintln!("skipped: fontmake is not installed ({})", Oracle::Fontmake.install_hint());
        return;
    };
    let dflt = Tag::new(b"DFLT");
    let mut sources = fontcraft_oracle::corpus::discover(&corpus()).unwrap();
    sources.extend(EXTRA.iter().map(|p| corpus().join(p)).filter(|p| p.exists()));
    let scratch = std::env::temp_dir().join(format!("fontcraft-kern-lookup-{}", std::process::id()));
    let mut skipped = Vec::new();
    // label, hand-written kern, oracle, pairs compared, differences
    let mut rows: Vec<(String, bool, &str, usize, usize)> = Vec::new();
    for (n, source) in sources.iter().enumerate() {
        let label = source.display().to_string();
        let Ok((font, _)) = open(source, None) else {
            skipped.push(format!("{label}: not read"));
            continue;
        };
        let glyphs = matches!(source.extension().and_then(|e| e.to_str()), Some("glyphs" | "glyphspackage"));
        let compiled = if glyphs { fontcraft_model::axes::origin(&font) } else { font.default_master().ok() };
        let Some((master, master_name)) = compiled.map(|m| (m.id.clone(), m.name.clone())) else { continue };
        let index = KernIndex::new(&font);
        let pairs = covered_by_direction(&font, &index, &master);
        if pairs.is_empty() {
            continue;
        }
        let hand_written = fontcraft_compile::has_hand_written_kern(&font);
        let options = Options { timestamp: fontcraft_oracle::DEFAULT_TIMESTAMP, ..Default::default() };
        // Our build gives the production names that match unencoded glyphs.
        let ours = match compile(&font, &fontcraft_model::Target::Master(master.clone()), fontcraft_compile::Flavour::TrueType, &options) {
            Ok(c) => c,
            Err(e) => {
                skipped.push(format!("{label}: FontCraft: {e}"));
                continue;
            }
        };
        let our_ids = ids_by_name(&ours.bytes).unwrap_or_default();
        let mut our_names = vec![String::new(); our_ids.len()];
        for (name, gid) in &our_ids {
            our_names[usize::from(*gid)] = name.clone();
        }
        let hint = Hint { family: font.family_name.clone(), style: master_name.clone() };
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
            let _ = std::fs::remove_dir_all(&work);
        }
        if built.len() < 2 {
            continue;
        }
        for (oracle, bytes) in &built {
            let Some(their_ids) = ids_by_name(bytes) else {
                skipped.push(format!("{label}: {oracle}: no glyph names to match glyphs by"));
                continue;
            };
            let their_eval = PairEvaluator::new(bytes).unwrap();
            let their_font = FontRef::new(bytes).unwrap();
            let their_cmap = their_font.charmap();
            let (mut compared, mut differ, mut shown) = (0usize, 0usize, 0usize);
            for (rtl, a, b) in &pairs {
                let direction = if *rtl { Direction::Rtl } else { Direction::Ltr };
                let Ok(applied) = index.lookup(direction, &master, a, b) else { continue };
                let value = applied.map_or(0.0, |p| p.value);
                let mine = (value + 0.5).floor() as i32;
                let theirs_of = |name: &str| -> Option<u16> {
                    match font.glyph(name).and_then(|g| g.unicodes.first().copied()) {
                        Some(cp) => their_cmap.map(cp).and_then(|g| u16::try_from(g.to_u32()).ok()),
                        None => {
                            let at = ours.glyph_order.iter().position(|g| g == name)?;
                            their_ids.get(our_names.get(at)?).copied()
                        }
                    }
                };
                let (Some(ta), Some(tb)) = (theirs_of(a), theirs_of(b)) else { continue };
                let mut scripts: BTreeSet<Tag> = BTreeSet::new();
                for name in [a, b] {
                    for cp in font.glyph(name).map(|g| g.unicodes.clone()).unwrap_or_default() {
                        scripts.extend(script_tags(cp));
                    }
                }
                if scripts.is_empty() {
                    scripts.insert(dflt);
                }
                compared += 1;
                let wrong: Vec<(Tag, i32)> = scripts.iter().map(|s| (*s, their_eval.value(*s, ta, tb))).filter(|(_, t)| *t != mine).collect();
                if !wrong.is_empty() {
                    differ += 1;
                    if shown < SHOWN {
                        let step = applied.map_or("none", |p| p.step.name());
                        eprintln!("{label}: {oracle} {} {a} {b}: ours {mine} ({step}), theirs {wrong:?}", if *rtl { "rtl" } else { "ltr" });
                    }
                    shown += 1;
                }
            }
            rows.push((label.clone(), hand_written, oracle, compared, differ));
        }
    }
    let _ = std::fs::remove_dir_all(&scratch);
    for s in &skipped {
        eprintln!("skipped: {s}");
    }
    for (label, hand_written, oracle, compared, differ) in &rows {
        eprintln!("row: {label}\t{}\t{oracle}\t{compared}\t{differ}", if *hand_written { "hand-written kern" } else { "generated" });
    }
    for oracle in ["fontc", "fontmake"] {
        for hand_written in [false, true] {
            let these: Vec<_> = rows.iter().filter(|r| r.2 == oracle && r.1 == hand_written).collect();
            let (c, d) = these.iter().fold((0, 0), |(c, d), r| (c + r.3, d + r.4));
            eprintln!(
                "total: {oracle}, {}: {} sources, {c} pairs, {d} differ",
                if hand_written { "hand-written kern" } else { "generated" },
                these.len()
            );
        }
    }
    let unexplained: Vec<&String> = rows
        .iter()
        .filter(|r| r.4 > 0)
        .map(|r| &r.0)
        .filter(|label| !LOOKUP_KNOWN.iter().any(|(name, _)| Path::new(label).file_name().and_then(|n| n.to_str()) == Some(*name)))
        .collect();
    for (name, cause) in LOOKUP_KNOWN {
        eprintln!("known: {name}: {cause}");
    }
    assert!(unexplained.is_empty(), "lookup values differ from the references without a known cause: {unexplained:?}");
}
