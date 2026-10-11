//! The mark attachment measure of M6 (`docs/plans/M6.md` § 5): for every corpus source without
//! hand-written mark feature code that both reference compilers build, every mark-to-base,
//! mark-to-ligature, mark-to-mark and cursive attachment in FontCraft's `GPOS` equals fontc's
//! (built with features) and fontmake's, and so do the `GDEF` glyph classes and ligature carets.
//!
//! Attachments are resolved to facts that do not depend on how the lookups are split:
//! - mark-to-base and mark-to-mark: (mark, base, dx, dy), the offset that puts the mark's anchor
//!   on the base's;
//! - mark-to-ligature: (mark, ligature, component, dx, dy);
//! - cursive: each glyph's entry and exit anchor;
//! - `GDEF`: each glyph's class, and each ligature's caret list.
//!
//! Every type 3 to 6 lookup is read, extension lookups included. A pair covered by several
//! lookups keeps every value it gets, so a duplicate with another value shows as a difference.
//! Glyphs are matched by their smallest code point when encoded, by `post` name otherwise.
//! A multi-master `.glyphs` source is compiled at its origin master, the default of fontc's
//! variable font, as in the kern measure.
//!
//! Opt-in and slow: `cargo test -p fontcraft-oracle --features oracle --test marks -- --ignored
//! --nocapture`. `FONTCRAFT_CORPUS` names the corpus folder (default: the repository's
//! `corpus/`); `MARKS_SHOWN=<n>` prints more than 20 differences per source and reference.
//! Skips, with a message, when a tool is not installed.
#![cfg(feature = "oracle")]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use fontcraft_compile::{Options, compile};
use fontcraft_engine::io::open;
use fontcraft_model::Font;
use fontcraft_oracle::tools::{Request, build_with_features, locate};
use fontcraft_oracle::{Flavour, Hint, Oracle};
use skrifa::raw::tables::gdef::CaretValue;
use skrifa::raw::tables::gpos::{AnchorTable, PositionSubtables};
use skrifa::raw::types::GlyphId16;
use skrifa::raw::{ReadError, TableProvider};
use skrifa::{FontRef, MetadataProvider};

/// Most facts read from one font; a font past it is reported, not compared.
const MAX_FACTS: usize = 5_000_000;
/// The generated features whose hand-written code keeps a source out of the measure.
const MARK_TAGS: [&str; 5] = ["mark", "mkmk", "abvm", "blwm", "curs"];

/// Differences with a known cause: the source's file name, the reference, the kind (see
/// [`Kind::name`]), how many facts of that kind differ, and the cause. They are reported and do
/// not fail the measure; another count does.
const KNOWN: &[(&str, &str, &str, usize, &str)] = &[];

/// Sources of the measure outside the oracle corpus, when fetched.
const EXTRA: &[&str] = &["inter/src/Inter-Roman.glyphspackage"];

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum Kind {
    MarkToBase,
    MarkToLigature,
    MarkToMark,
    CursiveEntry,
    CursiveExit,
    GlyphClass,
    Caret,
}

impl Kind {
    const ALL: [Kind; 7] =
        [Kind::MarkToBase, Kind::MarkToLigature, Kind::MarkToMark, Kind::CursiveEntry, Kind::CursiveExit, Kind::GlyphClass, Kind::Caret];

    fn name(self) -> &'static str {
        match self {
            Kind::MarkToBase => "mark-to-base",
            Kind::MarkToLigature => "mark-to-ligature",
            Kind::MarkToMark => "mark-to-mark",
            Kind::CursiveEntry => "cursive entry",
            Kind::CursiveExit => "cursive exit",
            Kind::GlyphClass => "glyph class",
            Kind::Caret => "ligature carets",
        }
    }
}

/// (kind, first glyph, second glyph or empty, ligature component).
type Key = (Kind, String, String, u16);
/// Every value each key gets: `[dx, dy]`, `[x, y]`, `[class]` or the caret positions.
type Facts = BTreeMap<Key, BTreeSet<Vec<i32>>>;

fn corpus() -> PathBuf {
    std::env::var_os("FONTCRAFT_CORPUS").map_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus"), PathBuf::from)
}

fn file_name(label: &str) -> &str {
    Path::new(label).file_name().and_then(|n| n.to_str()).unwrap_or(label)
}

fn shown() -> usize {
    std::env::var("MARKS_SHOWN").ok().and_then(|s| s.parse().ok()).unwrap_or(20)
}

/// Code with its `#` comments removed.
fn without_comments(code: &str) -> impl Iterator<Item = &str> {
    code.lines().map(|l| l.split('#').next().unwrap_or(""))
}

/// Whether the source's feature code writes one of [`MARK_TAGS`] by hand, or includes a file at
/// the top level that may: either keeps the generated feature of that tag out of FontCraft's
/// compile before M8, while the references compile the code.
fn has_mark_feature_code(font: &Font) -> bool {
    let f = &font.features;
    for s in f.prefixes.iter().chain(&f.classes) {
        if s.disabled {
            continue;
        }
        for line in without_comments(&s.code) {
            let words: Vec<&str> = line.split(|c: char| c.is_whitespace() || c == '{' || c == ';').filter(|w| !w.is_empty()).collect();
            if words.first() == Some(&"include") || line.trim_start().starts_with("include(") {
                return true;
            }
            if !s.automatic && words.windows(2).any(|w| w[0] == "feature" && MARK_TAGS.contains(&w[1])) {
                return true;
            }
        }
    }
    f.features
        .iter()
        .any(|s| !s.disabled && !s.automatic && MARK_TAGS.contains(&s.name.as_str()) && without_comments(&s.code).any(|l| !l.trim().is_empty()))
}

/// Each glyph id's matching key: `U+XXXX` for its smallest code point, else its `post` name, else
/// `gid<n>`.
fn glyph_keys(font: &FontRef) -> Vec<String> {
    let count = font.maxp().map(|m| m.num_glyphs()).unwrap_or(0);
    let mut out: Vec<String> = (0..count).map(|g| format!("gid{g}")).collect();
    if let Ok(post) = font.post() {
        for (g, key) in out.iter_mut().enumerate() {
            if let Some(name) = u16::try_from(g).ok().and_then(|g| post.glyph_name(GlyphId16::new(g))) {
                *key = name.to_string();
            }
        }
    }
    let mut smallest: HashMap<usize, u32> = HashMap::new();
    for (cp, gid) in font.charmap().mappings() {
        let g = gid.to_u32() as usize;
        let e = smallest.entry(g).or_insert(cp);
        *e = (*e).min(cp);
    }
    for (g, cp) in smallest {
        if let Some(key) = out.get_mut(g) {
            *key = format!("U+{cp:04X}");
        }
    }
    out
}

fn xy(anchor: Result<AnchorTable, ReadError>) -> Option<(i32, i32)> {
    anchor.ok().map(|a| (i32::from(a.x_coordinate()), i32::from(a.y_coordinate())))
}

struct Reader<'a> {
    keys: &'a [String],
    facts: Facts,
    full: bool,
}

impl Reader<'_> {
    fn key(&self, g: GlyphId16) -> String {
        self.keys.get(usize::from(g.to_u16())).cloned().unwrap_or_else(|| format!("gid{}", g.to_u16()))
    }

    fn add(&mut self, kind: Kind, a: GlyphId16, b: Option<GlyphId16>, component: u16, value: Vec<i32>) {
        if self.facts.len() >= MAX_FACTS {
            self.full = true;
            return;
        }
        let key = (kind, self.key(a), b.map(|b| self.key(b)).unwrap_or_default(), component);
        self.facts.entry(key).or_default().insert(value);
    }

    fn lookups(&mut self, font: &FontRef) -> Result<(), ReadError> {
        let Ok(gpos) = font.gpos() else { return Ok(()) };
        for lookup in gpos.lookup_list()?.lookups().iter() {
            match lookup?.subtables()? {
                PositionSubtables::MarkToBase(subtables) => {
                    for st in subtables.iter() {
                        let st = st?;
                        let marks = st.mark_array()?;
                        let mark_list: Vec<_> = st.mark_coverage()?.iter().zip(marks.mark_records()).collect();
                        let bases = st.base_array()?;
                        for (base, record) in st.base_coverage()?.iter().zip(bases.base_records().iter()) {
                            let anchors = record?.base_anchors(bases.offset_data());
                            for (mark, m) in &mark_list {
                                let Some(Some((bx, by))) = anchors.get(usize::from(m.mark_class())).map(xy) else { continue };
                                let Some((mx, my)) = xy(m.mark_anchor(marks.offset_data())) else { continue };
                                self.add(Kind::MarkToBase, *mark, Some(base), 0, vec![bx - mx, by - my]);
                            }
                        }
                    }
                }
                PositionSubtables::MarkToLig(subtables) => {
                    for st in subtables.iter() {
                        let st = st?;
                        let marks = st.mark_array()?;
                        let mark_list: Vec<_> = st.mark_coverage()?.iter().zip(marks.mark_records()).collect();
                        let ligatures = st.ligature_array()?;
                        for (lig, attach) in st.ligature_coverage()?.iter().zip(ligatures.ligature_attaches().iter()) {
                            let attach = attach?;
                            for (component, record) in attach.component_records().iter().enumerate() {
                                let anchors = record?.ligature_anchors(attach.offset_data());
                                let component = u16::try_from(component).unwrap_or(u16::MAX);
                                for (mark, m) in &mark_list {
                                    let Some(Some((lx, ly))) = anchors.get(usize::from(m.mark_class())).map(xy) else { continue };
                                    let Some((mx, my)) = xy(m.mark_anchor(marks.offset_data())) else { continue };
                                    self.add(Kind::MarkToLigature, *mark, Some(lig), component, vec![lx - mx, ly - my]);
                                }
                            }
                        }
                    }
                }
                PositionSubtables::MarkToMark(subtables) => {
                    for st in subtables.iter() {
                        let st = st?;
                        let marks = st.mark1_array()?;
                        let mark_list: Vec<_> = st.mark1_coverage()?.iter().zip(marks.mark_records()).collect();
                        let bases = st.mark2_array()?;
                        for (base, record) in st.mark2_coverage()?.iter().zip(bases.mark2_records().iter()) {
                            let anchors = record?.mark2_anchors(bases.offset_data());
                            for (mark, m) in &mark_list {
                                let Some(Some((bx, by))) = anchors.get(usize::from(m.mark_class())).map(xy) else { continue };
                                let Some((mx, my)) = xy(m.mark_anchor(marks.offset_data())) else { continue };
                                self.add(Kind::MarkToMark, *mark, Some(base), 0, vec![bx - mx, by - my]);
                            }
                        }
                    }
                }
                PositionSubtables::Cursive(subtables) => {
                    for st in subtables.iter() {
                        let st = st?;
                        for (glyph, record) in st.coverage()?.iter().zip(st.entry_exit_record()) {
                            if let Some((x, y)) = record.entry_anchor(st.offset_data()).and_then(xy) {
                                self.add(Kind::CursiveEntry, glyph, None, 0, vec![x, y]);
                            }
                            if let Some((x, y)) = record.exit_anchor(st.offset_data()).and_then(xy) {
                                self.add(Kind::CursiveExit, glyph, None, 0, vec![x, y]);
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn gdef(&mut self, font: &FontRef) -> Result<(), ReadError> {
        let Ok(gdef) = font.gdef() else { return Ok(()) };
        if let Some(classes) = gdef.glyph_class_def() {
            for (glyph, class) in classes?.iter() {
                if class != 0 {
                    self.add(Kind::GlyphClass, glyph, None, 0, vec![i32::from(class)]);
                }
            }
        }
        if let Some(carets) = gdef.lig_caret_list() {
            let carets = carets?;
            for (glyph, lig) in carets.coverage()?.iter().zip(carets.lig_glyphs().iter()) {
                let mut values = Vec::new();
                for caret in lig?.caret_values().iter() {
                    values.push(match caret? {
                        CaretValue::Format1(c) => i32::from(c.coordinate()),
                        CaretValue::Format3(c) => i32::from(c.coordinate()),
                        // A contour point carries no position of its own; marked so it never equals one.
                        CaretValue::Format2(c) => -1_000_000 - i32::from(c.caret_value_point_index()),
                    });
                }
                self.add(Kind::Caret, glyph, None, 0, values);
            }
        }
        Ok(())
    }
}

/// The attachment facts of a compiled font, or why they could not be read.
fn facts(bytes: &[u8]) -> Result<Facts, String> {
    let font = FontRef::new(bytes).map_err(|e| e.to_string())?;
    let keys = glyph_keys(&font);
    let mut reader = Reader { keys: &keys, facts: Facts::new(), full: false };
    reader.lookups(&font).map_err(|e| format!("GPOS: {e}"))?;
    reader.gdef(&font).map_err(|e| format!("GDEF: {e}"))?;
    if reader.full {
        return Err(format!("more than {MAX_FACTS} attachment facts"));
    }
    Ok(reader.facts)
}

struct Difference {
    oracle: &'static str,
    key: Key,
    ours: Option<BTreeSet<Vec<i32>>>,
    theirs: Option<BTreeSet<Vec<i32>>>,
}

impl Difference {
    fn describe(&self) -> String {
        let (kind, a, b, component) = &self.key;
        let pair = if b.is_empty() {
            a.clone()
        } else if *kind == Kind::MarkToLigature {
            format!("{a} on {b} component {component}")
        } else {
            format!("{a} on {b}")
        };
        format!("{} {} {pair}: ours {:?}, theirs {:?}", self.oracle, kind.name(), self.ours, self.theirs)
    }
}

fn known(label: &str, d: &Difference) -> Option<&'static str> {
    let name = file_name(label);
    KNOWN.iter().find(|(n, o, k, _, _)| *n == name && *o == d.oracle && *k == d.key.0.name()).map(|(_, _, _, _, cause)| *cause)
}

#[test]
#[ignore = "builds every corpus source with fontc and fontmake: the M6 mark attachment measure"]
fn mark_attachment_equals_both_references() {
    let Some(fontc) = locate(Oracle::Fontc, None) else {
        eprintln!("skipped: fontc is not installed ({})", Oracle::Fontc.install_hint());
        return;
    };
    let Some(fontmake) = locate(Oracle::Fontmake, None) else {
        eprintln!("skipped: fontmake is not installed ({})", Oracle::Fontmake.install_hint());
        return;
    };
    let limit = shown();
    let mut sources = fontcraft_oracle::corpus::discover(&corpus()).unwrap();
    sources.extend(EXTRA.iter().map(|p| corpus().join(p)).filter(|p| p.exists()));
    let scratch = std::env::temp_dir().join(format!("fontcraft-marks-measure-{}", std::process::id()));
    let mut skipped = Vec::new();
    let mut with_code = 0usize;
    let mut compared_names: BTreeSet<String> = BTreeSet::new();
    // label, oracle, facts compared per kind (the union of both sides' keys)
    let mut rows: Vec<(String, &str, BTreeMap<Kind, usize>)> = Vec::new();
    let mut all: Vec<(String, Difference)> = Vec::new();
    for (n, source) in sources.iter().enumerate() {
        let label = source.display().to_string();
        let Ok((font, _)) = open(source, None) else {
            skipped.push(format!("{label}: not read"));
            continue;
        };
        if has_mark_feature_code(&font) {
            eprintln!("not compared, a mark feature in the feature code or an include: {label}");
            with_code += 1;
            continue;
        }
        let glyphs = matches!(source.extension().and_then(|e| e.to_str()), Some("glyphs" | "glyphspackage"));
        let compiled = if glyphs { fontcraft_model::axes::origin(&font) } else { font.default_master().ok() };
        let Some((master, master_name)) = compiled.map(|m| (m.id.clone(), m.name.clone())) else { continue };
        let options = Options { timestamp: fontcraft_oracle::DEFAULT_TIMESTAMP, ..Default::default() };
        let ours = match compile(&font, &fontcraft_model::Target::Master(master.clone()), fontcraft_compile::Flavour::TrueType, &options) {
            Ok(c) => c,
            Err(e) => {
                skipped.push(format!("{label}: FontCraft: {e}"));
                continue;
            }
        };
        let our_facts = match facts(&ours.bytes) {
            Ok(f) => f,
            Err(e) => {
                skipped.push(format!("{label}: FontCraft: {e}"));
                continue;
            }
        };
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
        compared_names.insert(file_name(&label).to_string());
        for (oracle, bytes) in &built {
            let their_facts = match facts(bytes) {
                Ok(f) => f,
                Err(e) => {
                    skipped.push(format!("{label}: {oracle}: {e}"));
                    continue;
                }
            };
            let keys: BTreeSet<&Key> = our_facts.keys().chain(their_facts.keys()).collect();
            let mut counts: BTreeMap<Kind, usize> = BTreeMap::new();
            let mut shown_here = 0usize;
            for key in keys {
                *counts.entry(key.0).or_default() += 1;
                let (o, t) = (our_facts.get(key), their_facts.get(key));
                if o == t {
                    continue;
                }
                let d = Difference { oracle, key: key.clone(), ours: o.cloned(), theirs: t.cloned() };
                if shown_here < limit {
                    eprintln!("{label}: {}", d.describe());
                }
                shown_here += 1;
                all.push((label.clone(), d));
            }
            if !counts.is_empty() {
                let summary: Vec<String> = counts.iter().map(|(k, c)| format!("{} {c}", k.name())).collect();
                eprintln!("compared: {label} {oracle}: {}", summary.join(", "));
            }
            rows.push((label.clone(), oracle, counts));
        }
    }
    let _ = std::fs::remove_dir_all(&scratch);
    for s in &skipped {
        eprintln!("skipped: {s}");
    }
    let mut by_source: BTreeMap<(&str, &str, Kind), usize> = BTreeMap::new();
    for (label, d) in &all {
        *by_source.entry((label.as_str(), d.oracle, d.key.0)).or_default() += 1;
    }
    for ((label, oracle, kind), count) in &by_source {
        eprintln!("differences: {label} {oracle} {}: {count}", kind.name());
    }
    eprintln!("sources with mark feature code or an include (not compared): {with_code}");
    for oracle in ["fontc", "fontmake"] {
        let these: Vec<_> = rows.iter().filter(|r| r.1 == oracle).collect();
        for kind in Kind::ALL {
            let compared: usize = these.iter().map(|r| r.2.get(&kind).copied().unwrap_or(0)).sum();
            let differ = all.iter().filter(|(_, d)| d.oracle == oracle && d.key.0 == kind).count();
            eprintln!("total: {oracle}, {}: {} sources, {compared} facts, {differ} differ", kind.name(), these.len());
        }
    }
    let (expected, unexpected): (Vec<_>, Vec<_>) = all.iter().partition(|(label, d)| known(label, d).is_some());
    for (name, oracle, kind, count, cause) in KNOWN {
        eprintln!("known: {name} {oracle} {kind}, {count} facts: {cause}");
    }
    let changed: Vec<String> = KNOWN
        .iter()
        .filter(|(name, ..)| compared_names.contains(*name))
        .filter_map(|(name, oracle, kind, count, _)| {
            let seen = expected.iter().filter(|(label, d)| file_name(label) == *name && d.oracle == *oracle && d.key.0.name() == *kind).count();
            (seen != *count).then(|| format!("{name} {oracle} {kind}: {seen} known differences, {count} expected"))
        })
        .collect();
    assert!(changed.is_empty(), "the known differences changed: {changed:?}");
    if let Some((label, d)) = unexpected.first() {
        panic!("{} attachment facts differ from the references without a known cause; the first: {label} {}", unexpected.len(), d.describe());
    }
}
