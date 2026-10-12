//! Kerning recovered from a compiled font's GPOS (`open.kerningMostlyPreserved`): the opened
//! document gives, for every pair the `kern` feature covers, the value a shaper takes from the
//! GPOS. The GPOS side is a pair-by-pair walk of the lookups written here, apart from the bulk
//! reader in `gpos_kern.rs`.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use fontcraft_compile::{Options, compile_ttf};
use fontcraft_decompile::decompile;
use fontcraft_model::Font;
use fontcraft_spacing::kerning::{Direction, KernIndex};
use skrifa::raw::tables::gpos::{PairPos, PositionSubtables};
use skrifa::raw::types::{GlyphId16, Tag};
use skrifa::raw::{FontRef, TableProvider};
use write_fonts::tables::gpos as w;
use write_fonts::tables::layout as wl;
use write_fonts::types::{GlyphId16 as WGid, Tag as WTag};

/// Scripts written right to left (the decompiler's list, kept apart on purpose).
const RTL: [&[u8; 4]; 6] = [b"arab", b"hebr", b"syrc", b"thaa", b"nko ", b"adlm"];

/// The value a shaper gives the pair from these lookups: per lookup the first subtable that
/// applies, lookups added up.
fn gpos_value(font: &FontRef, lookups: &[u16], g1: GlyphId16, g2: GlyphId16) -> i32 {
    let list = font.gpos().unwrap().lookup_list().unwrap();
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
                    let (c1, c2) = (t.class_def1().unwrap().get(g1), t.class_def2().unwrap().get(g2));
                    if let Some([v, _]) = t.value_record_refs(c1, c2) {
                        total += v.x_advance().map_or(0, i32::from);
                    }
                    break;
                }
            }
        }
    }
    total
}

/// The pairs these lookups list: format 1 records, and format 2 coverage against every glyph
/// in a second class other than 0.
fn covered(font: &FontRef, lookups: &[u16], out: &mut HashSet<(u16, u16)>) {
    let list = font.gpos().unwrap().lookup_list().unwrap();
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
        }
    }
}

/// Each script's `kern` lookups (sorted, as shapers apply them) with its direction, once per set.
fn lookup_sets(font: &FontRef) -> Vec<(Direction, Vec<u16>)> {
    let Ok(gpos) = font.gpos() else { return Vec::new() };
    let (Ok(scripts), Ok(features)) = (gpos.script_list(), gpos.feature_list()) else { return Vec::new() };
    let mut sets: Vec<(Direction, Vec<u16>, bool)> = Vec::new();
    for rec in scripts.script_records() {
        let tag = rec.script_tag();
        let direction = if RTL.iter().any(|t| Tag::new(t) == tag) { Direction::Rtl } else { Direction::Ltr };
        let Ok(script) = rec.script(scripts.offset_data()) else { continue };
        let langs = script
            .default_lang_sys()
            .into_iter()
            .flatten()
            .chain(script.lang_sys_records().iter().filter_map(|l| l.lang_sys(script.offset_data()).ok()));
        for lang in langs {
            let mut lookups: Vec<u16> = lang
                .feature_indices()
                .iter()
                .filter_map(|fi| features.feature_records().get(usize::from(fi.get())))
                .filter(|r| r.feature_tag() == Tag::new(b"kern"))
                .filter_map(|r| r.feature(features.offset_data()).ok())
                .flat_map(|f| f.lookup_list_indices().iter().map(|i| i.get()).collect::<Vec<_>>())
                .collect();
            lookups.sort_unstable();
            lookups.dedup();
            if !lookups.is_empty() && !sets.iter().any(|(d, l, _)| *d == direction && *l == lookups) {
                sets.push((direction, lookups, tag == Tag::new(b"DFLT")));
            }
        }
    }
    // The default script counts as left to right unless a right-to-left script uses the same lookups.
    let rtl: Vec<Vec<u16>> = sets.iter().filter(|(d, _, _)| *d == Direction::Rtl).map(|(_, l, _)| l.clone()).collect();
    sets.into_iter().filter(|(_, l, dflt)| !(*dflt && rtl.contains(l))).map(|(d, l, _)| (d, l)).collect()
}

/// Every difference between the opened document and the GPOS, as text.
fn differences(bytes: &[u8], limit: usize) -> (usize, Vec<String>) {
    let font = FontRef::new(bytes).unwrap();
    let doc = decompile(bytes, None).unwrap().font;
    let names: Vec<String> = doc.glyphs.iter().map(|g| g.name.clone()).collect();
    let master = doc.default_master().unwrap().id.clone();
    let index = KernIndex::new(&doc);
    let (mut checked, mut out) = (0, Vec::new());
    for (direction, lookups) in lookup_sets(&font) {
        let mut pairs = HashSet::new();
        covered(&font, &lookups, &mut pairs);
        let mut pairs: Vec<_> = pairs.into_iter().filter(|&(a, b)| usize::from(a.max(b)) < names.len()).collect();
        pairs.sort_unstable();
        for (a, b) in pairs {
            checked += 1;
            let expected = gpos_value(&font, &lookups, GlyphId16::new(a), GlyphId16::new(b));
            let (n1, n2) = (&names[usize::from(a)], &names[usize::from(b)]);
            let ours = index.lookup(direction, &master, n1, n2).unwrap().map_or(0.0, |p| p.value);
            if ours != f64::from(expected) && out.len() < limit {
                out.push(format!("{} {n1} {n2}: GPOS {expected}, document {ours} (lookups {lookups:?})", direction.name()));
            }
        }
    }
    (checked, out)
}

fn gid(names: &[String], name: &str) -> WGid {
    WGid::new(names.iter().position(|n| n == name).unwrap() as u16)
}

fn adv(v: i16) -> w::ValueRecord {
    w::ValueRecord::new().with_x_advance(v)
}

/// A format 1 subtable from `(first, [(second, value)])`.
fn glyph_pairs(names: &[String], pairs: &[(&str, &[(&str, i16)])]) -> w::PairPos {
    let mut firsts: Vec<(WGid, Vec<(WGid, i16)>)> = pairs
        .iter()
        .map(|(f, s)| {
            let mut s: Vec<_> = s.iter().map(|(n, v)| (gid(names, n), *v)).collect();
            s.sort();
            (gid(names, f), s)
        })
        .collect();
    firsts.sort();
    let coverage = firsts.iter().map(|(g, _)| *g).collect();
    let sets = firsts
        .into_iter()
        .map(|(_, s)| w::PairSet::new(s.into_iter().map(|(g, v)| w::PairValueRecord::new(g, adv(v), w::ValueRecord::new())).collect()))
        .collect();
    w::PairPos::format_1(coverage, sets)
}

/// A format 2 subtable: the covered first glyphs, the first and second classes (class 0 implied),
/// and one row of values per first class, class 0 first.
fn class_pairs(names: &[String], covered: &[&str], class1: &[&[&str]], class2: &[&[&str]], rows: &[&[i16]]) -> w::PairPos {
    let classes = |cs: &[&[&str]]| -> wl::ClassDef {
        cs.iter().enumerate().flat_map(|(i, members)| members.iter().map(move |n| (gid(names, n), i as u16 + 1))).collect()
    };
    let coverage = covered.iter().map(|n| gid(names, n)).collect();
    let records =
        rows.iter().map(|row| w::Class1Record::new(row.iter().map(|&v| w::Class2Record::new(adv(v), w::ValueRecord::new())).collect())).collect();
    w::PairPos::format_2(coverage, classes(class1), classes(class2), records)
}

fn add_gpos(ttf: &[u8], gpos: &w::Gpos) -> Vec<u8> {
    let base = FontRef::new(ttf).unwrap();
    let mut b = write_fonts::FontBuilder::new();
    b.add_table(gpos).unwrap();
    b.copy_missing_tables(base);
    b.build()
}

fn base_font() -> (Vec<u8>, Vec<String>) {
    let ttf = compile_ttf(&Font::new("Kerned"), &Options::default()).unwrap().bytes;
    let names = decompile(&ttf, None).unwrap().font.glyphs.iter().map(|g| g.name.clone()).collect();
    (ttf, names)
}

/// A font with glyph pairs, class pairs (a class 0 row, a class-0 column, a subtable never
/// reached), an extension lookup added to the first, and right-to-left kerning for Arabic.
fn kerned_font() -> (Vec<u8>, Vec<String>) {
    let (ttf, names) = base_font();
    let n = &names;
    let flag = wl::LookupFlag::empty();
    let plain = w::PositionLookup::Pair(wl::Lookup::new(
        flag,
        vec![
            glyph_pairs(n, &[("T", &[("o", -60), ("e", 0)]), ("A", &[("V", -30)])]),
            // T and Y are class 0 of the first class definition: covered, but in no class.
            class_pairs(n, &["A", "T", "Y"], &[&["A"]], &[&["V", "W"], &["o", "e"]], &[&[0, -20, -50], &[-10, -40, 0]]),
            // Never reached: the class subtable above decides every pair starting with A.
            class_pairs(n, &["A"], &[&["A"]], &[&["o"]], &[&[0, 0], &[0, -99]]),
        ],
    ));
    let extension = w::PositionLookup::Extension(wl::Lookup::new(
        flag,
        vec![w::ExtensionSubtable::Pair(w::ExtensionPosFormat1::new(2, glyph_pairs(n, &[("A", &[("V", 5)]), ("V", &[("A", -35)])])))],
    ));
    let arabic = w::PositionLookup::Pair(wl::Lookup::new(flag, vec![glyph_pairs(n, &[("a", &[("b", -15)])])]));
    let lang = |f: u16| Some(wl::LangSys::new(vec![f]));
    let gpos = w::Gpos::new(
        wl::ScriptList::new(vec![
            wl::ScriptRecord::new(WTag::new(b"DFLT"), wl::Script::new(lang(0), vec![])),
            wl::ScriptRecord::new(WTag::new(b"arab"), wl::Script::new(lang(1), vec![])),
            wl::ScriptRecord::new(WTag::new(b"latn"), wl::Script::new(lang(0), vec![])),
        ]),
        wl::FeatureList::new(vec![
            wl::FeatureRecord::new(WTag::new(b"kern"), wl::Feature::new(None, vec![1, 0])),
            wl::FeatureRecord::new(WTag::new(b"kern"), wl::Feature::new(None, vec![2])),
        ]),
        w::PositionLookupList::new(vec![plain, extension, arabic]),
    );
    (add_gpos(&ttf, &gpos), names)
}

#[test]
fn open_kerning_mostly_preserved() {
    let (bytes, names) = kerned_font();
    let d = decompile(&bytes, None).unwrap();
    let doc = &d.font;
    let index = KernIndex::new(doc);
    let value = |dir, a: &str, b: &str| index.lookup(dir, "m01", a, b).unwrap().map_or(0.0, |p| p.value);
    let ltr = |a, b| value(Direction::Ltr, a, b);
    // Glyph pairs, the class 0 row, a zero exception, class 0 of the second classes, and two
    // lookups added up.
    assert_eq!(ltr("A", "V"), -25.0);
    assert_eq!(ltr("A", "W"), -40.0);
    assert_eq!(ltr("T", "o"), -60.0);
    assert_eq!(ltr("T", "e"), 0.0);
    assert_eq!(ltr("Y", "e"), -50.0);
    assert_eq!(ltr("T", "W"), -20.0);
    assert_eq!(ltr("A", "T"), -10.0);
    assert_eq!(ltr("A", "o"), 0.0);
    assert_eq!(ltr("V", "A"), -35.0);
    assert_eq!(value(Direction::Rtl, "a", "b"), -15.0);
    assert_eq!(ltr("a", "b"), 0.0);
    // Classes come back as groups named after their first glyph.
    let groups = |g: &str| doc.glyph(g).unwrap().kern_groups.clone();
    assert_eq!(groups("A").right.as_deref(), Some("A"));
    assert_eq!((groups("T").right, groups("Y").right), (Some("T".into()), Some("T".into())));
    assert_eq!((groups("V").left, groups("W").left), (Some("V".into()), Some("V".into())));
    assert_eq!((groups("o").left, groups("e").left), (Some("e".into()), Some("e".into())));
    assert_eq!(doc.kerning.ltr.get("m01", "@MMK_L_T", "@MMK_R_e"), Some(-50.0));

    // Every pair of glyphs, against the GPOS read pair by pair.
    let font = FontRef::new(&bytes).unwrap();
    for (dir, lookups) in [(Direction::Ltr, vec![0u16, 1]), (Direction::Rtl, vec![2])] {
        for (a, na) in names.iter().enumerate() {
            for (b, nb) in names.iter().enumerate() {
                let expected = gpos_value(&font, &lookups, GlyphId16::new(a as u16), GlyphId16::new(b as u16));
                assert_eq!(value(dir, na, nb), f64::from(expected), "{} {na} {nb}", dir.name());
            }
        }
    }
    let (checked, diffs) = differences(&bytes, 10);
    assert!(checked > 0 && diffs.is_empty(), "{diffs:?}");
}

#[test]
fn damaged_kerning_is_skipped_not_a_crash() {
    let (good, _) = kerned_font();
    let font = FontRef::new(&good).unwrap();
    let record = font.table_directory.table_records().iter().find(|r| r.tag() == Tag::new(b"GPOS")).unwrap();
    let (start, len) = (record.offset() as usize, record.length() as usize);
    for i in start..start + len {
        for delta in [1u8, 0x80] {
            let mut bad = good.clone();
            bad[i] = bad[i].wrapping_add(delta);
            let _ = decompile(&bad, None);
        }
    }
}

/// A class table of 65,535 × 65,535 class pairs (with empty value records, so the table itself
/// is small) is skipped with a warning.
#[test]
fn a_huge_class_table_is_skipped_with_a_warning() {
    let (ttf, _) = base_font();
    let words: [u16; 49] = [
        1, 0, 10, 30, 44, // header: version, script list, feature list, lookup list
        1, 0x4446, 0x4C54, 8, 4, 0, 0, 0xFFFF, 1, 0, // DFLT with one feature
        1, 0x6B65, 0x726E, 8, 0, 1, 0, // kern: lookup 0
        1, 4, 2, 0, 1, 8, // one pair lookup, one subtable
        2, 16, 0, 0, 22, 32, 0xFFFF, 0xFFFF, // format 2, 65,535 classes each way
        1, 1, 2, // coverage
        2, 1, 0, 10, 1, // first classes
        2, 1, 0, 10, 1, // second classes
    ];
    let gpos: Vec<u8> = words.iter().flat_map(|w| w.to_be_bytes()).collect();
    let base = FontRef::new(&ttf).unwrap();
    let bytes = write_fonts::FontBuilder::new().add_raw(WTag::new(b"GPOS"), gpos).copy_missing_tables(base).build();
    let d = decompile(&bytes, None).unwrap();
    assert!(d.font.kerning.is_empty());
    assert!(d.warnings.iter().any(|w| w.contains("class pairs were skipped")), "{:?}", d.warnings);
}

/// A script list whose 6,000 records all lead to one Script, whose 6,000 language systems all
/// lead to one empty LangSys: 36 million language systems from a small table. Walking them
/// costs work, so the walk stops with a warning.
#[test]
fn shared_script_and_language_records_spend_the_budget() {
    let (ttf, _) = base_font();
    const N: u16 = 6000;
    // Header, then an empty feature list and lookup list, then the script list.
    let mut words: Vec<u16> = vec![1, 0, 14, 10, 12, 0, 0];
    words.push(N);
    for _ in 0..N {
        words.extend([0x6C61, 0x746E, 2 + 6 * N]);
    }
    words.extend([0, N]);
    for _ in 0..N {
        words.extend([0x5452, 0x4B20, 4 + 6 * N]);
    }
    words.extend([0, 0xFFFF, 0]);
    let gpos: Vec<u8> = words.iter().flat_map(|w| w.to_be_bytes()).collect();
    let base = FontRef::new(&ttf).unwrap();
    let bytes = write_fonts::FontBuilder::new().add_raw(WTag::new(b"GPOS"), gpos).copy_missing_tables(base).build();
    let d = decompile(&bytes, None).unwrap();
    assert!(d.font.kerning.is_empty());
    assert!(d.warnings.iter().any(|w| w.contains("script and feature lists are too large")), "{:?}", d.warnings);
}

/// A second glyph whose class is past the class count gets nothing from the class table: a
/// shaper skips the subtable for that pair, so the glyph is not part of class 0.
#[test]
fn a_second_class_past_the_count_is_not_class_0() {
    let (ttf, names) = base_font();
    let g = |n: &str| gid(&names, n).to_u16();
    let (mut v, mut w) = ((g("V"), 1), (g("W"), 5));
    if w.0 < v.0 {
        std::mem::swap(&mut v, &mut w);
    }
    let words: Vec<u16> = vec![
        1,
        0,
        10,
        30,
        44, // header: version, script list, feature list, lookup list
        1,
        0x4446,
        0x4C54,
        8,
        4,
        0,
        0,
        0xFFFF,
        1,
        0, // DFLT with one feature
        1,
        0x6B65,
        0x726E,
        8,
        0,
        1,
        0, // kern: lookup 0
        1,
        4,
        2,
        0,
        1,
        8, // one pair lookup, one subtable
        2,
        20,
        4,
        0,
        26,
        30,
        1,
        2, // format 2, x advances, one first class, two second classes
        (-10i16) as u16,
        (-30i16) as u16, // class 0 against classes 0 and 1
        1,
        1,
        g("A"), // coverage
        2,
        0, // first classes: all class 0
        2,
        2,
        v.0,
        v.0,
        v.1,
        w.0,
        w.0,
        w.1, // second classes: V is 1, W is 5
    ];
    let gpos: Vec<u8> = words.iter().flat_map(|w| w.to_be_bytes()).collect();
    let base = FontRef::new(&ttf).unwrap();
    let bytes = write_fonts::FontBuilder::new().add_raw(WTag::new(b"GPOS"), gpos).copy_missing_tables(base).build();
    let doc = decompile(&bytes, None).unwrap().font;
    let index = KernIndex::new(&doc);
    let value = |a: &str, b: &str| index.lookup(Direction::Ltr, "m01", a, b).unwrap().map_or(0.0, |p| p.value);
    assert_eq!(value("A", "V"), -30.0);
    assert_eq!(value("A", "o"), -10.0);
    assert_eq!(value("A", "W"), 0.0);
}

fn fonts(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            if p.file_name().is_some_and(|n| n != ".git") {
                fonts(&p, out);
            }
        } else if p.extension().and_then(|x| x.to_str()).is_some_and(|x| x.eq_ignore_ascii_case("ttf") || x.eq_ignore_ascii_case("otf")) {
            out.push(p);
        }
    }
}

/// The measure "Kerning from compiled fonts" (`docs/plans/M5.md` § 5): for every font with a
/// `kern` feature under `FONTCRAFT_KERN_FONTS` (a folder searched recursively, such as the
/// `--output-dir` of `fontcraft-cli oracle --corpus --against fontmake`; `corpus/` when unset),
/// the opened document gives every covered pair the GPOS value. Pairs whose second glyph is in
/// class 0 of a class table are not listed here. Prints each difference and fails when there is
/// one. Run with `cargo test --release -p fontcraft-decompile --test decompile gpos_kern:: -- --ignored`.
#[test]
#[ignore = "a measure over many fonts: run it on purpose"]
fn measure_kerning_from_compiled_fonts() {
    let root =
        std::env::var_os("FONTCRAFT_KERN_FONTS").map(PathBuf::from).unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus"));
    let mut all = Vec::new();
    fonts(&root, &mut all);
    all.sort();
    let (mut kerned, mut pairs, mut failing) = (0, 0, Vec::new());
    for path in &all {
        let bytes = std::fs::read(path).unwrap();
        let Ok(font) = FontRef::new(&bytes) else { continue };
        if lookup_sets(&font).is_empty() || decompile(&bytes, None).is_err() {
            continue;
        }
        kerned += 1;
        let (checked, diffs) = differences(&bytes, 20);
        pairs += checked;
        println!("{}: {checked} pairs, {} differences", path.display(), diffs.len());
        for d in &diffs {
            println!("  {d}");
        }
        if !diffs.is_empty() {
            failing.push(path.display().to_string());
        }
    }
    println!("{kerned} kerned fonts, {pairs} covered pairs checked, {} fonts with differences", failing.len());
    assert!(failing.is_empty(), "fonts with differences: {failing:#?}");
}
