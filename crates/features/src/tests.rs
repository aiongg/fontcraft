#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::Arc;

use fontcraft_model::Font;
use fontcraft_spacing::kerning::{Direction, GroupSide, KernIndex};
use read_fonts::tables::gpos::{PairPos, PositionSubtables};
use read_fonts::tables::layout::LookupFlag;
use read_fonts::{FontRef, TableProvider};
use write_fonts::FontBuilder;
use write_fonts::types::Tag;

use crate::evaluate::PairEvaluator;
use crate::{KernGlyph, KernRequest, KernTags, MarkSpacing, kern_feature};

const LATN: Tag = Tag::new(b"latn");
const CYRL: Tag = Tag::new(b"cyrl");
const HEBR: Tag = Tag::new(b"hebr");
const DFLT: Tag = Tag::new(b"DFLT");

/// The new-font glyphs (`.notdef`, `space`, A–Z, a–z) and `glyphs` (name, code point) after them.
fn font(glyphs: &[(&str, Option<u32>)]) -> Font {
    let mut f = Font::new("Kern");
    for (name, cp) in glyphs {
        if f.glyph(name).is_none() {
            f.add_glyph(name, cp.iter().copied().collect(), 500.0).unwrap();
        }
    }
    f
}

fn group(f: &mut Font, glyph: &str, side: GroupSide, name: &str) {
    let mut g = f.glyph_mut(glyph).unwrap();
    match side {
        GroupSide::Left => g.kern_groups.left = Some(name.to_string()),
        GroupSide::Right => g.kern_groups.right = Some(name.to_string()),
        _ => panic!("a horizontal side"),
    }
}

fn set(f: &mut Font, direction: Direction, first: &str, second: &str, value: f64) {
    direction.table_mut(Arc::make_mut(&mut f.kerning)).set("m01", first, second, value);
}

/// The compiled kerning: the font file holding only the `GPOS`, the glyph names by id, and the
/// writer's warnings. `marks` names the glyphs of the mark class.
fn build(f: &Font, marks: &[(&str, MarkSpacing)]) -> (Vec<u8>, Vec<String>, Vec<String>) {
    let index = KernIndex::new(f);
    let names: Vec<String> = f.glyphs.iter().map(|g| g.name.clone()).collect();
    let unicodes: Vec<Vec<u32>> = f.glyphs.iter().map(|g| g.unicodes.clone()).collect();
    let glyphs: Vec<KernGlyph> = names
        .iter()
        .zip(&unicodes)
        .map(|(n, u)| KernGlyph { name: n, codepoints: u, mark: marks.iter().find(|(m, _)| m == n).map(|(_, s)| *s) })
        .collect();
    let out = kern_feature(&KernRequest { font: f, index: &index, master: "m01", glyphs: &glyphs, tags: KernTags::ALL }).unwrap();
    let gpos = out.gpos.expect("a GPOS");
    let bytes = FontBuilder::new().add_table(&gpos).unwrap().build();
    (bytes, names, out.warnings)
}

fn gid(names: &[String], name: &str) -> u16 {
    names.iter().position(|n| n == name).unwrap() as u16
}

/// What `KernIndex` says applies, rounded half up.
fn expected(f: &Font, direction: Direction, first: &str, second: &str) -> i32 {
    let index = KernIndex::new(f);
    index.lookup(direction, "m01", first, second).unwrap().map_or(0, |a| (a.value + 0.5).floor() as i32)
}

/// Every ordered pair of `glyphs` has the value `KernIndex` gives it in `script`.
fn assert_all_pairs(f: &Font, bytes: &[u8], names: &[String], script: Tag, direction: Direction, glyphs: &[&str]) {
    let eval = PairEvaluator::new(bytes).unwrap();
    for a in glyphs {
        for b in glyphs {
            let got = eval.value(script, gid(names, a), gid(names, b));
            assert_eq!(got, expected(f, direction, a, b), "{a} {b} in {script}");
        }
    }
}

/// The § 2 test font of the M5 plan, left-to-right part, with a half value.
fn latin() -> Font {
    let mut f = font(&[
        ("A", Some(0x41)),
        ("Aacute", Some(0xC1)),
        ("T", Some(0x54)),
        ("V", Some(0x56)),
        ("W", Some(0x57)),
        ("o", Some(0x6F)),
        ("oacute", Some(0xF3)),
        ("period", Some(0x2E)),
    ]);
    group(&mut f, "A", GroupSide::Right, "A");
    group(&mut f, "Aacute", GroupSide::Right, "A");
    group(&mut f, "V", GroupSide::Left, "V");
    group(&mut f, "W", GroupSide::Left, "V");
    group(&mut f, "o", GroupSide::Left, "o");
    group(&mut f, "oacute", GroupSide::Left, "o");
    set(&mut f, Direction::Ltr, "A", "@MMK_R_V", -30.0);
    set(&mut f, Direction::Ltr, "@MMK_L_A", "V", -40.0);
    set(&mut f, Direction::Ltr, "@MMK_L_A", "@MMK_R_V", -50.0);
    set(&mut f, Direction::Ltr, "T", "@MMK_R_o", -60.0);
    set(&mut f, Direction::Ltr, "T", "oacute", 0.0);
    set(&mut f, Direction::Ltr, "V", "period", -10.5);
    set(&mut f, Direction::Ltr, "W", "period", 10.5);
    f
}

#[test]
fn glyph_class_and_exception_pairs_keep_their_effective_values() {
    let f = latin();
    let (bytes, names, warnings) = build(&f, &[]);
    assert!(warnings.is_empty(), "{warnings:?}");
    let glyphs = ["A", "Aacute", "T", "V", "W", "o", "oacute", "period"];
    for script in [LATN, DFLT] {
        assert_all_pairs(&f, &bytes, &names, script, Direction::Ltr, &glyphs);
    }
    let eval = PairEvaluator::new(&bytes).unwrap();
    let v = |a, b| eval.value(LATN, gid(&names, a), gid(&names, b));
    assert_eq!(v("A", "W"), -30, "glyph–group before group–glyph");
    assert_eq!(v("Aacute", "V"), -40);
    assert_eq!(v("Aacute", "W"), -50);
    assert_eq!(v("T", "oacute"), 0, "the zero exception stays");
    assert_eq!(v("T", "o"), -60);
    assert_eq!((v("V", "period"), v("W", "period")), (-10, 11), "half up");

    // One lookup, glyph pairs before class pairs.
    let font = FontRef::new(&bytes).unwrap();
    let gpos = font.gpos().unwrap();
    let lookups = gpos.lookup_list().unwrap();
    assert_eq!(lookups.lookup_count(), 1);
    let lookup = lookups.lookups().get(0).unwrap();
    assert!(lookup.lookup_flag().contains(LookupFlag::IGNORE_MARKS));
    let Ok(PositionSubtables::Pair(subtables)) = lookup.subtables() else { panic!("a pair lookup") };
    let formats: Vec<u16> = subtables
        .iter()
        .map(|s| match s.unwrap() {
            PairPos::Format1(_) => 1,
            PairPos::Format2(_) => 2,
        })
        .collect();
    assert!(formats.windows(2).all(|w| w[0] <= w[1]) && formats.contains(&1) && formats.contains(&2), "{formats:?}");
    let scripts: Vec<Tag> = gpos.script_list().unwrap().script_records().iter().map(|r| r.script_tag()).collect();
    assert_eq!(scripts, [DFLT, LATN]);
}

#[test]
fn right_to_left_pairs_are_in_logical_order_and_move_the_glyph() {
    let mut f = font(&[("alef", Some(0x05D0)), ("bet", Some(0x05D1)), ("gimel", Some(0x05D2))]);
    group(&mut f, "alef", GroupSide::Left, "alef");
    group(&mut f, "bet", GroupSide::Right, "bet");
    set(&mut f, Direction::Rtl, "@MMK_R_alef", "@MMK_L_bet", -77.0);
    set(&mut f, Direction::Rtl, "gimel", "alef", -12.0);
    let (bytes, names, _) = build(&f, &[]);
    assert_all_pairs(&f, &bytes, &names, HEBR, Direction::Rtl, &["alef", "bet", "gimel"]);
    let eval = PairEvaluator::new(&bytes).unwrap();
    assert_eq!(eval.value(HEBR, gid(&names, "alef"), gid(&names, "bet")), -77);
    assert_eq!(eval.value(HEBR, gid(&names, "bet"), gid(&names, "alef")), 0);
    // Only right-to-left scripts: the default script gets their lookups.
    assert_eq!(eval.value(DFLT, gid(&names, "alef"), gid(&names, "bet")), -77);

    let font = FontRef::new(&bytes).unwrap();
    let lookup = font.gpos().unwrap().lookup_list().unwrap().lookups().get(0).unwrap();
    let Ok(PositionSubtables::Pair(subtables)) = lookup.subtables() else { panic!("a pair lookup") };
    for s in subtables.iter() {
        let format = match s.unwrap() {
            PairPos::Format1(t) => t.value_format1(),
            PairPos::Format2(t) => t.value_format1(),
        };
        assert!(format.contains(read_fonts::tables::gpos::ValueFormat::X_PLACEMENT), "{format:?}");
    }
}

#[test]
fn each_script_gets_its_own_lookup() {
    let mut f = font(&[("A", Some(0x41)), ("V", Some(0x56)), ("A-cy", Some(0x410)), ("U-cy", Some(0x423)), ("period", Some(0x2E))]);
    set(&mut f, Direction::Ltr, "A", "V", -80.0);
    set(&mut f, Direction::Ltr, "A-cy", "U-cy", -70.0);
    set(&mut f, Direction::Ltr, "V", "period", -20.0);
    set(&mut f, Direction::Ltr, "U-cy", "period", -25.0);
    let (bytes, names, _) = build(&f, &[]);
    let eval = PairEvaluator::new(&bytes).unwrap();
    let v = |script, a, b| eval.value(script, gid(&names, a), gid(&names, b));
    assert_eq!(eval.scripts(), [DFLT, CYRL, LATN]);
    assert_eq!((v(LATN, "A", "V"), v(LATN, "V", "period"), v(LATN, "A-cy", "U-cy")), (-80, -20, 0), "Latin");
    assert_eq!((v(CYRL, "A-cy", "U-cy"), v(CYRL, "U-cy", "period"), v(CYRL, "A", "V")), (-70, -25, 0), "Cyrillic");
    assert_eq!((v(DFLT, "A", "V"), v(DFLT, "A-cy", "U-cy")), (-80, -70), "the default script gets the left-to-right lookups");
    let font = FontRef::new(&bytes).unwrap();
    assert_eq!(font.gpos().unwrap().lookup_list().unwrap().lookup_count(), 2);
}

#[test]
fn indic_scripts_get_dist_and_mixed_directions_are_left_out() {
    let mut f = font(&[("ka", Some(0x915)), ("kha", Some(0x916)), ("A", Some(0x41)), ("alef", Some(0x05D0))]);
    set(&mut f, Direction::Ltr, "ka", "kha", -15.0);
    set(&mut f, Direction::Ltr, "A", "alef", -5.0);
    let (bytes, names, warnings) = build(&f, &[]);
    assert!(warnings.iter().any(|w| w.contains("opposite directions")), "{warnings:?}");
    let font = FontRef::new(&bytes).unwrap();
    let gpos = font.gpos().unwrap();
    let features = gpos.feature_list().unwrap();
    let tags: Vec<Tag> = features.feature_records().iter().map(|r| r.feature_tag()).collect();
    assert_eq!(tags, [Tag::new(b"dist")]);
    let scripts: Vec<Tag> = gpos.script_list().unwrap().script_records().iter().map(|r| r.script_tag()).collect();
    assert_eq!(scripts, [Tag::new(b"dev2"), Tag::new(b"deva")]);
    let eval = PairEvaluator::new(&bytes).unwrap();
    assert_eq!(eval.value(Tag::new(b"dev2"), gid(&names, "ka"), gid(&names, "kha")), -15);
}

#[test]
fn pairs_with_marks_go_to_a_lookup_that_sees_marks() {
    let mut f = font(&[("o", Some(0x6F)), ("acutecomb", Some(0x301)), ("e", Some(0x65))]);
    group(&mut f, "o", GroupSide::Right, "round");
    group(&mut f, "acutecomb", GroupSide::Right, "round");
    group(&mut f, "e", GroupSide::Left, "e");
    set(&mut f, Direction::Ltr, "@MMK_L_round", "@MMK_R_e", -20.0);
    let (bytes, names, _) = build(&f, &[("acutecomb", MarkSpacing::NonSpacing)]);
    let font = FontRef::new(&bytes).unwrap();
    let lookups = font.gpos().unwrap().lookup_list().unwrap();
    let flags: Vec<bool> = (0..usize::from(lookups.lookup_count()))
        .map(|i| lookups.lookups().get(i).unwrap().lookup_flag().contains(LookupFlag::IGNORE_MARKS))
        .collect();
    assert_eq!(flags, [true, false], "the base lookup ignores marks, the mark lookup does not");
    let eval = PairEvaluator::new(&bytes).unwrap();
    assert_eq!(eval.value(LATN, gid(&names, "o"), gid(&names, "e")), -20);
    assert_eq!(eval.value(LATN, gid(&names, "acutecomb"), gid(&names, "e")), -20);
}

#[test]
fn a_font_without_pairs_gets_no_table() {
    let f = font(&[("A", Some(0x41))]);
    let index = KernIndex::new(&f);
    let glyphs: Vec<KernGlyph> = f.glyphs.iter().map(|g| KernGlyph { name: &g.name, codepoints: &[], mark: None }).collect();
    let out = kern_feature(&KernRequest { font: &f, index: &index, master: "m01", glyphs: &glyphs, tags: KernTags::ALL }).unwrap();
    assert!(out.gpos.is_none());
}

#[test]
fn the_feature_writer_setting_decides_which_kern_features_are_written() {
    use crate::kern_tags;
    let tags = |text: Option<&str>| {
        let value = text.map(|t| fontcraft_model::plist::openstep::parse_value(t).unwrap());
        let mut warnings = Vec::new();
        (kern_tags(value.as_ref(), &mut warnings), warnings.len())
    };
    assert_eq!(tags(None), (KernTags::ALL, 0), "without the setting every writer runs");
    assert_eq!(tags(Some("()")), (KernTags::NONE, 0), "an empty list runs none");
    assert_eq!(tags(Some("({class = MarkFeatureWriter;})")), (KernTags::NONE, 0));
    assert_eq!(tags(Some("({class = KernFeatureWriter;})")), (KernTags::ALL, 0));
    let only_dist = "({class = KernFeatureWriter; module = ufo2ft.featureWriters.kernFeatureWriter; options = {features = (dist); mode = append;};})";
    assert_eq!(tags(Some(only_dist)), (KernTags { kern: false, dist: true }, 0));
    assert_eq!(tags(Some("({class = KernFeatureWriter; options = {other = 1;};})")), (KernTags::NONE, 0), "an unknown option drops the entry");
    assert_eq!(tags(Some("({class = KernFeatureWriter; options = {ignoreMarks = 0;};})")), (KernTags::ALL, 1), "written, with a warning");
    assert_eq!(tags(Some("{class = KernFeatureWriter;}")), (KernTags::NONE, 1), "not a list");

    let mut f = font(&[]);
    set(&mut f, Direction::Ltr, "A", "V", -80.0);
    let index = KernIndex::new(&f);
    let glyphs: Vec<KernGlyph> = f.glyphs.iter().map(|g| KernGlyph { name: &g.name, codepoints: &g.unicodes, mark: None }).collect();
    let request = |tags| KernRequest { font: &f, index: &index, master: "m01", glyphs: &glyphs, tags };
    assert!(kern_feature(&request(KernTags::NONE)).unwrap().gpos.is_none());
    assert!(kern_feature(&request(KernTags { kern: false, dist: true })).unwrap().gpos.is_none(), "Latin kerning goes to kern only");
    assert!(kern_feature(&request(KernTags::ALL)).unwrap().gpos.is_some());
}
