#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::collections::BTreeMap;

use fontcraft_compile::{Flavour as CompileFlavour, Options, compile};
use fontcraft_model::{Component, Font, Path, Shape};
use read_fonts::types::{GlyphId, GlyphId16, Tag as OtTag};
use read_fonts::{FontRef, TableProvider};
use write_fonts::tables::gsub::{Gsub, SingleSubst, SingleSubstFormat2, SubstitutionLookup, SubstitutionLookupList};
use write_fonts::tables::layout::{
    CoverageFormat1, CoverageTable, Feature, FeatureList, FeatureRecord, LangSys, Lookup, LookupFlag, Script, ScriptList, ScriptRecord,
};

use super::*;
use crate::reorder::glyph_names;
use crate::sfnt::{WriteOptions, read, write};

/// A font of `.notdef`, `space` and `glyphs` (name, code point, advance), each a rectangle; `kern` adds kerning between
/// the first two.
fn source(family: &str, glyphs: &[(&str, u32, f64)], kern: bool) -> Font {
    let mut f = Font::new(family);
    f.glyphs.retain(|g| g.name == ".notdef" || g.name == "space");
    for (i, (name, cp, width)) in glyphs.iter().enumerate() {
        if f.glyph(name).is_none() {
            f.add_glyph(name, vec![*cp], *width).unwrap();
        }
        let mut g = f.glyph_mut(name).unwrap();
        g.layers[0].width = *width;
        g.layers[0].shapes.push(Shape::Path(Path::rect(10.0 * i as f64, 0.0, 200.0 + 10.0 * i as f64, 700.0)));
    }
    if kern && glyphs.len() >= 2 {
        std::sync::Arc::make_mut(&mut f.kerning).ltr.set("m01", glyphs[0].0, glyphs[1].0, -50.0);
    }
    f
}

fn built(f: &Font, flavour: CompileFlavour) -> SfntFont {
    let out = compile(f, &f.default_target().unwrap(), flavour, &Options::default()).unwrap();
    read(&out.bytes, 0).unwrap()
}

fn gid(font: &SfntFont, name: &str) -> u16 {
    glyph_names(font).unwrap().iter().position(|n| n == name).unwrap_or_else(|| panic!("no glyph {name}")) as u16
}

fn cmap_of(font: &SfntFont) -> BTreeMap<u32, u32> {
    let bytes = write(font, &WriteOptions::default()).unwrap();
    let f = FontRef::new(&bytes).unwrap();
    let cmap = f.cmap().unwrap();
    let mut out = BTreeMap::new();
    for record in cmap.encoding_records() {
        for (u, g) in record.subtable(cmap.offset_data()).unwrap().iter() {
            out.entry(u).or_insert(g.to_u32());
        }
    }
    out
}

/// Font 2 of the merge tests: its own A (wider), B, and a `latn` GSUB with one alternate
/// feature B → A, so the duplicate A gets a `locl` lookup.
fn second() -> SfntFont {
    let mut font = built(&source("Second Sans", &[("A", 0x41, 700.0), ("B", 0x42, 650.0)], false), CompileFlavour::TrueType);
    let (a, b) = (GlyphId16::new(gid(&font, "A")), GlyphId16::new(gid(&font, "B")));
    let subst = SingleSubst::Format2(SingleSubstFormat2::new(CoverageTable::Format1(CoverageFormat1::new(vec![b])), vec![a]));
    let lookup = SubstitutionLookup::Single(Lookup::new(LookupFlag::empty(), vec![subst]));
    let scripts = ScriptList::new(vec![ScriptRecord::new(OtTag::new(b"latn"), Script::new(Some(LangSys::new(vec![0])), vec![]))]);
    let features = FeatureList::new(vec![FeatureRecord::new(OtTag::new(b"salt"), Feature::new(None, vec![0]))]);
    let gsub = Gsub::new(scripts, features, SubstitutionLookupList::new(vec![lookup]));
    font.insert(*b"GSUB", write_fonts::dump_table(&gsub).unwrap());
    font
}

fn first() -> SfntFont {
    built(&source("First Sans", &[("A", 0x41, 600.0), ("V", 0x56, 650.0)], true), CompileFlavour::TrueType)
}

#[test]
fn font_merge_concatenates_glyphs_unites_cmap_and_adds_locl_for_shared_characters() {
    let (one, two) = (first(), second());
    let names_one = glyph_names(&one).unwrap();
    let names_two = glyph_names(&two).unwrap();
    let result = merge_with_report(&[one.clone(), two.clone()], &MergeOptions::default()).unwrap();
    let merged = &result.font;

    // Every glyph of both fonts, the second font's clashing names suffixed.
    let names = glyph_names(merged).unwrap();
    assert_eq!(names.len(), names_one.len() + names_two.len());
    assert_eq!(&names[..names_one.len()], names_one.as_slice());
    let expected_two: Vec<String> = names_two.iter().map(|n| if names_one.contains(n) { format!("{n}.1") } else { n.clone() }).collect();
    assert_eq!(&names[names_one.len()..], expected_two.as_slice());
    assert!(result.conflicts.iter().any(|c| c.contains("`A` of font 2 is renamed `A.1`")), "{:?}", result.conflicts);

    // The first font keeps U+0041; the second's B is mapped.
    let cmap = cmap_of(merged);
    assert_eq!(cmap[&0x41], u32::from(gid(merged, "A")));
    assert_eq!(cmap[&0x42], u32::from(gid(merged, "B")));
    assert_eq!(cmap[&0x56], u32::from(gid(merged, "V")));

    // Metrics and outlines follow their glyphs.
    let bytes = write(merged, &WriteOptions::default()).unwrap();
    let f = FontRef::new(&bytes).unwrap();
    assert_eq!(f.maxp().unwrap().num_glyphs() as usize, names.len());
    assert_eq!(f.hmtx().unwrap().advance(GlyphId::new(u32::from(gid(merged, "A.1")))), Some(700));
    assert_eq!(f.hmtx().unwrap().advance(GlyphId::new(u32::from(gid(merged, "A")))), Some(600));
    assert_eq!(f.head().unwrap().units_per_em(), 1000);

    // GSUB: the second font's lookup moved to the merged ids, and a locl lookup A → A.1 first
    // in a new locl feature under latn.
    let gsub = f.gsub().unwrap();
    let features = gsub.feature_list().unwrap();
    let tags: Vec<OtTag> = features.feature_records().iter().map(|r| r.feature_tag()).collect();
    assert!(tags.contains(&OtTag::new(b"locl")) && tags.contains(&OtTag::new(b"salt")), "{tags:?}");
    let locl = features.feature_records().iter().find(|r| r.feature_tag() == OtTag::new(b"locl")).unwrap();
    let locl_lookup = locl.feature(features.offset_data()).unwrap().lookup_list_indices()[0].get();
    let lookups = gsub.lookup_list().unwrap();
    let read_fonts::tables::gsub::SubstitutionLookup::Single(l) = lookups.lookups().get(usize::from(locl_lookup)).unwrap() else {
        panic!("locl is a single substitution")
    };
    let sub = l.subtables().get(0).unwrap();
    let pairs: Vec<(u16, u16)> = match sub {
        read_fonts::tables::gsub::SingleSubst::Format1(t) => {
            t.coverage().unwrap().iter().map(|g| (g.to_u16(), (g.to_u16() as i32 + t.delta_glyph_id() as i32) as u16)).collect()
        }
        read_fonts::tables::gsub::SingleSubst::Format2(t) => {
            t.coverage().unwrap().iter().zip(t.substitute_glyph_ids()).map(|(g, s)| (g.to_u16(), s.get().to_u16())).collect()
        }
    };
    // Both fonts map the space too (every new font has one); fontTools adds it even though
    // the two glyphs are identical.
    let mut expected: Vec<(u16, u16)> = ["space", "A"].iter().map(|n| (gid(merged, n), gid(merged, &format!("{n}.1")))).collect();
    expected.sort();
    assert_eq!(pairs, expected);
    let scripts: Vec<OtTag> = gsub.script_list().unwrap().script_records().iter().map(|r| r.script_tag()).collect();
    assert_eq!(scripts, [OtTag::new(b"latn")]);

    // GPOS: the first font's kerning survives, on the first font's glyph ids.
    assert!(f.gpos().is_ok(), "the first font's kerning is kept");
    assert!(merged.get(*b"name").is_some() && merged.get(*b"OS/2").is_some());
    // The input fonts are unchanged by the merge.
    assert_eq!(glyph_names(&one).unwrap(), names_one);
}

#[test]
fn merging_scripts_several_fonts_share_builds_new_features_and_drops_unused_ones() {
    // Both fonts kern under the same scripts, so each shared language system gets one new
    // kern feature with both fonts' lookups.
    let one = first();
    let two = built(&source("Third Sans", &[("C", 0x43, 600.0), ("D", 0x44, 600.0)], true), CompileFlavour::TrueType);
    let lookups_one = FontRef::new(&write(&one, &WriteOptions::default()).unwrap()).unwrap().gpos().unwrap().lookup_list().unwrap().lookup_count();
    let lookups_two = FontRef::new(&write(&two, &WriteOptions::default()).unwrap()).unwrap().gpos().unwrap().lookup_list().unwrap().lookup_count();
    let merged = merge(&[one, two], &MergeOptions::default()).unwrap();
    let bytes = write(&merged, &WriteOptions::default()).unwrap();
    let f = FontRef::new(&bytes).unwrap();
    let gpos = f.gpos().unwrap();
    assert_eq!(gpos.lookup_list().unwrap().lookup_count(), lookups_one + lookups_two);
    let features = gpos.feature_list().unwrap();
    for record in features.feature_records() {
        let feature = record.feature(features.offset_data()).unwrap();
        if record.feature_tag() == OtTag::new(b"kern") {
            assert_eq!(feature.lookup_list_indices().len(), usize::from(lookups_one + lookups_two), "both fonts' kerning");
        }
    }
    // Every feature is used by some language system.
    let list = gpos.script_list().unwrap();
    let mut used = vec![false; features.feature_records().len()];
    for s in list.script_records() {
        let script = s.script(list.offset_data()).unwrap();
        for ls in
            script.default_lang_sys().into_iter().flatten().chain(script.lang_sys_records().iter().map(|r| r.lang_sys(script.offset_data()).unwrap()))
        {
            for i in ls.feature_indices() {
                used[usize::from(i.get())] = true;
            }
        }
    }
    assert!(used.iter().all(|u| *u), "unused features are dropped");
}

#[test]
fn glyph_merge_takes_only_mapped_glyphs_under_their_new_names() {
    let (one, two) = (first(), second());
    let n_one = glyph_names(&one).unwrap().len();
    let options = MergeOptions { glyph_maps: vec![vec![], vec![("B.alt".into(), "B".into())]], ..MergeOptions::default() };
    let result = merge_with_report(&[one.clone(), two], &options).unwrap();
    let merged = &result.font;
    let names = glyph_names(merged).unwrap();
    assert_eq!(names.len(), n_one + 1);
    assert_eq!(names.last().map(String::as_str), Some("B.alt"));
    assert_eq!(result.glyphs.last().unwrap(), &MergedGlyph { name: "B.alt".into(), font: 1, source: "B".into() });
    let cmap = cmap_of(merged);
    assert_eq!(cmap[&0x42], u32::from(gid(merged, "B.alt")));
    assert_eq!(cmap[&0x41], u32::from(gid(merged, "A")), "the second font's A is not taken");
    assert!(merged.get(*b"GSUB").is_none() && merged.get(*b"GPOS").is_none(), "a glyph merge drops layout by default");
    assert_eq!(merged.get(*b"OS/2"), one.get(*b"OS/2"), "font-wide values come from the first font");

    // Asking to merge layout while a font loses glyphs is refused.
    let refused = MergeOptions { layout: Some(LayoutMode::Merge), ..options.clone() };
    let e = merge(&[one.clone(), second()], &refused).unwrap_err();
    assert!(e.to_string().contains("layout"), "{e}");
}

#[test]
fn a_composite_needs_its_components_in_the_map() {
    let mut f = source("Comp Sans", &[("A", 0x41, 600.0), ("acute", 0xB4, 300.0)], false);
    f.add_glyph("Aacute", vec![0xC1], 600.0).unwrap();
    {
        let mut c = f.glyph_mut("Aacute").unwrap();
        c.layers[0].shapes.push(Shape::Component(Component::new("A", 0.0, 0.0)));
        c.layers[0].shapes.push(Shape::Component(Component::new("acute", 200.0, 200.0)));
    }
    let font = built(&f, CompileFlavour::TrueType);
    let only =
        MergeOptions { glyph_maps: vec![vec![(".notdef".into(), ".notdef".into()), ("Aacute".into(), "Aacute".into())]], ..MergeOptions::default() };
    let e = merge(std::slice::from_ref(&font), &only).unwrap_err();
    assert!(e.to_string().contains("component"), "{e}");

    let all = MergeOptions {
        glyph_maps: vec![vec![
            (".notdef".into(), ".notdef".into()),
            ("Aacute".into(), "Aacute".into()),
            ("A".into(), "A".into()),
            ("acute".into(), "acute".into()),
        ]],
        ..MergeOptions::default()
    };
    let merged = merge(std::slice::from_ref(&font), &all).unwrap();
    let bytes = write(&merged, &WriteOptions::default()).unwrap();
    let f = FontRef::new(&bytes).unwrap();
    let glyf = f.glyf().unwrap();
    let loca = f.loca(None).unwrap();
    let Some(read_fonts::tables::glyf::Glyph::Composite(c)) = loca.get_glyf(GlyphId::new(1), &glyf).unwrap() else {
        panic!("Aacute is glyph 1 and composite")
    };
    let parts: Vec<u32> = c.components().map(|c| c.glyph.to_u32()).collect();
    assert_eq!(parts, [2, 3], "components point at the new ids of A and acute");
}

#[test]
fn refusals() {
    let one = first();
    assert!(matches!(merge(std::slice::from_ref(&one), &MergeOptions::default()), Err(TablesError::BadOptions(_))), "one font needs a map");
    let cff = built(&source("Cff Sans", &[("A", 0x41, 600.0)], false), CompileFlavour::Cff);
    let e = merge(&[one.clone(), cff], &MergeOptions::default()).unwrap_err();
    assert!(matches!(e, TablesError::BadOptions(_)) && e.to_string().contains("mix TrueType and CFF"), "{e}");

    let mut other_upem = second();
    let mut head = other_upem.get(*b"head").unwrap().to_vec();
    head[18..20].copy_from_slice(&2048u16.to_be_bytes());
    other_upem.insert(*b"head", head);
    let e = merge(&[one.clone(), other_upem], &MergeOptions::default()).unwrap_err();
    assert!(e.to_string().contains("units per em"), "{e}");

    let missing =
        MergeOptions { glyph_maps: vec![vec![(".notdef".into(), ".notdef".into()), ("X".into(), "nosuch".into())]], ..MergeOptions::default() };
    assert!(merge(std::slice::from_ref(&one), &missing).unwrap_err().to_string().contains("nosuch"));

    let error = MergeOptions { duplicates: Some(Duplicates::Error), ..MergeOptions::default() };
    assert!(merge(&[one.clone(), second()], &error).is_err(), "the two .notdef or A glyphs differ");

    let too_many = vec![one; crate::limits::MAX_MERGE_FONTS + 1];
    assert!(matches!(merge(&too_many, &MergeOptions::default()), Err(TablesError::TooLarge { .. })));
}

#[test]
fn drop_tables_and_unknown_tables() {
    let mut one = first();
    one.insert(*b"zzzz", vec![1, 2, 3, 4]);
    let options = MergeOptions { drop_tables: vec![*b"GPOS"], ..MergeOptions::default() };
    let result = merge_with_report(&[one, second()], &options).unwrap();
    assert!(result.font.get(*b"GPOS").is_none());
    assert!(result.font.get(*b"zzzz").is_none());
    assert!(result.warnings.iter().any(|w| w.contains("'zzzz'")), "{:?}", result.warnings);
}

#[test]
fn glyph_map_files() {
    let map = parse_glyph_map("MergeFonts MyFont-Regular 1\n# comment\nA.alt A\nB\n\n  cid00001   one # trailing\n").unwrap();
    assert_eq!(map.ps_name.as_deref(), Some("MyFont-Regular"));
    assert_eq!(map.language_group, Some(1));
    assert_eq!(map.entries, [("A.alt".into(), "A".into()), ("B".into(), "B".into()), ("cid00001".into(), "one".into())]);
    assert!(parse_glyph_map("a b c\n").is_err());
    assert!(parse_glyph_map("mergefonts X 7\n").is_err());
    assert!(parse_glyph_map(&"x".repeat(2000)).is_err());
}

/// Sets the left side bearing of glyph `gid` in `hmtx`.
fn set_lsb(font: &mut SfntFont, gid: usize, lsb: i16) {
    let long = usize::from(u16::from_be_bytes(font.get(*b"hhea").unwrap()[34..36].try_into().unwrap()));
    let at = if gid < long { 4 * gid + 2 } else { 4 * long + 2 * (gid - long) };
    let mut hmtx = font.get(*b"hmtx").unwrap().to_vec();
    hmtx[at..at + 2].copy_from_slice(&lsb.to_be_bytes());
    font.insert(*b"hmtx", hmtx);
}

#[test]
fn font_merge_recalculates_maxp_and_the_head_flags_from_the_merged_glyphs() {
    let (mut one, mut two) = (first(), second());
    for font in [&mut one, &mut two] {
        let mut maxp = font.get(*b"maxp").unwrap().to_vec();
        for at in [6, 8, 10, 12, 28, 30] {
            maxp[at..at + 2].copy_from_slice(&999u16.to_be_bytes());
        }
        font.insert(*b"maxp", maxp);
    }
    let merged = merge(&[one.clone(), two.clone()], &MergeOptions::default()).unwrap();
    let maxp = merged.get(*b"maxp").unwrap();
    let maxima: Vec<u16> = [6, 8, 10, 12, 28, 30].iter().map(|at| u16::from_be_bytes([maxp[*at], maxp[at + 1]])).collect();
    assert_eq!(maxima, [8, 2, 0, 0, 0, 0], "the hollow .notdef box is the largest glyph; no composites");
    let flags = |f: &SfntFont| u16::from_be_bytes([f.get(*b"head").unwrap()[16], f.get(*b"head").unwrap()[17]]);
    assert_eq!(flags(&merged) & 2, 2, "every left side bearing equals xMin");

    let b = usize::from(gid(&two, "B"));
    set_lsb(&mut two, b, 1);
    let merged = merge(&[one, two], &MergeOptions::default()).unwrap();
    assert_eq!(flags(&merged) & 2, 0, "B's side bearing no longer equals its xMin");
}

#[test]
fn a_name_repeated_inside_one_font_gets_a_dot_suffix_and_keeps_its_stored_name() {
    let mut one = first();
    // Font 1 stores `V` under the name `A` too.
    let stored: Vec<String> = glyph_names(&one).unwrap().into_iter().map(|n| if n == "V" { "A".to_string() } else { n }).collect();
    let post = crate::reorder::names::write_post_v2(one.get(*b"post").unwrap(), &stored).unwrap();
    one.insert(*b"post", post);
    let two = second();
    let names_two = glyph_names(&two).unwrap();
    let result = merge_with_report(&[one, two], &MergeOptions::default()).unwrap();

    let names: Vec<&str> = result.glyphs.iter().map(|g| g.name.as_str()).collect();
    let v = stored.iter().rposition(|n| n == "A").unwrap();
    let mut expected: Vec<String> = stored.iter().enumerate().map(|(i, n)| if i == v { "A.1".to_string() } else { n.clone() }).collect();
    // Font 2 as pyftmerge names it: `A.1` is taken, so its A becomes `A.2`.
    expected.extend(names_two.iter().map(|n| match n.as_str() {
        "A" => "A.2".to_string(),
        n if stored.iter().any(|s| s == n) => format!("{n}.1"),
        n => n.to_string(),
    }));
    assert_eq!(names, expected);
    assert_eq!(result.glyphs[v].source, "A.1", "the name font 1 reads as");

    // `post` stores the glyph under the name font 1 stored, as fontTools' mapping does.
    let n = names.len();
    let stored_back = crate::reorder::names::post_names(result.font.get(*b"post").unwrap(), n).unwrap().unwrap();
    assert_eq!(stored_back[v], "A");
    assert_eq!(stored_back[stored.len()..], expected[stored.len()..]);
}

/// Advance of every glyph as its charstring stores it, read against the table's Private DICT.
fn charstring_advances(table: &[u8], n: usize) -> Vec<f64> {
    let parsed = crate::reorder::cff::parse(table, n).unwrap();
    let (default, nominal) = parsed.widths(0);
    let (default, nominal) = (default.unwrap() as f64, nominal.unwrap() as f64);
    let set = fontcraft_cff::subr::desubroutinize(&fontcraft_cff::subr::charstrings_of_table(table, false).unwrap()).unwrap();
    set.charstrings.iter().map(|cs| cff::width_operand(cs).unwrap().map_or(default, |(v, _)| nominal + v)).collect()
}

fn advances(font: &SfntFont) -> Vec<f64> {
    let bytes = write(font, &WriteOptions::default()).unwrap();
    let f = FontRef::new(&bytes).unwrap();
    let hmtx = f.hmtx().unwrap();
    (0..f.maxp().unwrap().num_glyphs()).map(|g| f64::from(hmtx.advance(GlyphId::new(u32::from(g))).unwrap())).collect()
}

#[test]
fn cff_fonts_merge_their_charstrings_under_the_first_fonts_private_dict() {
    let one = built(&source("Cff One", &[("A", 0x41, 600.0), ("V", 0x56, 650.0)], false), CompileFlavour::Cff);
    let two = built(&source("Cff Two", &[("A", 0x41, 731.0), ("B", 0x42, 655.0), ("C", 0x43, 655.0)], false), CompileFlavour::Cff);
    let result = merge_with_report(&[one.clone(), two.clone()], &MergeOptions::default()).unwrap();
    let font = &result.font;
    assert_eq!(font.flavour, Flavour::Cff);
    assert!(font.get(*b"glyf").is_none() && font.get(*b"loca").is_none());
    let table = font.get(*b"CFF ").unwrap();

    // Names come from the charsets, renamed as pyftmerge renames them.
    let (names_one, names_two) = (glyph_names(&one).unwrap(), glyph_names(&two).unwrap());
    let mut expected = names_one.clone();
    expected.extend(names_two.iter().map(|n| if names_one.contains(n) { format!("{n}.1") } else { n.clone() }));
    assert_eq!(glyph_names(font).unwrap(), expected);

    // Every charstring stores the advance hmtx has, against the first font's widths.
    let n = expected.len();
    let first_widths = crate::reorder::cff::parse(one.get(*b"CFF ").unwrap(), names_one.len()).unwrap().widths(0);
    assert_eq!(crate::reorder::cff::parse(table, n).unwrap().widths(0), first_widths, "the first font's Private DICT is kept");
    assert_eq!(charstring_advances(table, n), advances(font));
    assert_eq!(advances(font)[names_one.len() + gid(&two, "A") as usize], 731.0);

    // Outlines are unchanged, and FontBBox and head follow the merged glyphs.
    let bounds = crate::ttx::read_cff::glyph_bounds(table).unwrap();
    let bounds_two = crate::ttx::read_cff::glyph_bounds(two.get(*b"CFF ").unwrap()).unwrap();
    assert_eq!(bounds[names_one.len()..], bounds_two[..]);
    let union = bounds
        .iter()
        .flatten()
        .fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |a, b| [a[0].min(b[0]), a[1].min(b[1]), a[2].max(b[2]), a[3].max(b[3])]);
    let outwards = [union[0].floor(), union[1].floor(), union[2].ceil(), union[3].ceil()];
    assert_eq!(crate::ttx::read_cff::font_bbox(table).unwrap(), Some(outwards));
    let head = font.get(*b"head").unwrap();
    let head_box: Vec<f64> = (0..4).map(|i| f64::from(i16::from_be_bytes([head[36 + 2 * i], head[37 + 2 * i]]))).collect();
    assert_eq!(head_box, outwards);
    assert_eq!(u16::from_be_bytes([font.get(*b"maxp").unwrap()[4], font.get(*b"maxp").unwrap()[5]]) as usize, n);
}

#[test]
fn cff2_jstf_and_variable_inputs_follow_fonttools() {
    let mut cff2 = first();
    cff2.insert(*b"CFF2", vec![2, 0, 5, 0, 0]);
    let e = merge(&[cff2, second()], &MergeOptions::default()).unwrap_err();
    assert!(e.to_string().contains("CFF2"), "{e}");

    let mut jstf = second();
    jstf.insert(*b"JSTF", vec![0, 1, 0, 0, 0, 0]);
    let e = merge(&[first(), jstf.clone()], &MergeOptions::default()).unwrap_err();
    assert!(matches!(e, TablesError::Unsupported(_)) && e.to_string().contains("'JSTF'"), "{e}");
    let dropped = MergeOptions { drop_tables: vec![*b"JSTF"], ..MergeOptions::default() };
    assert!(merge(&[first(), jstf], &dropped).unwrap().get(*b"JSTF").is_none());

    // A variable font merges as its default outlines, its variation tables left out.
    let mut variable = first();
    variable.insert(*b"fvar", vec![0; 16]);
    variable.insert(*b"gvar", vec![0; 20]);
    let result = merge_with_report(&[variable, second()], &MergeOptions::default()).unwrap();
    assert!(result.font.get(*b"fvar").is_none() && result.font.get(*b"gvar").is_none());
    assert!(result.warnings.iter().any(|w| w.contains("font 1 is variable")), "{:?}", result.warnings);
}

mod math_base {
    use read_fonts::{FontData, FontRead};
    use write_fonts::from_obj::ToOwnedTable;
    use write_fonts::tables::base::{Axis, Base, BaseCoord, BaseCoordFormat1, BaseScript, BaseScriptList, BaseScriptRecord, BaseTagList, BaseValues};
    use write_fonts::tables::math::{
        Math, MathConstants, MathGlyphConstruction, MathGlyphInfo, MathGlyphVariantRecord, MathItalicsCorrectionInfo, MathKern, MathKernInfo,
        MathKernInfoRecord, MathTopAccentAttachment, MathValueRecord, MathVariants,
    };
    use write_fonts::types::{FWord, UfWord};

    use super::*;

    fn cov(font: &SfntFont, names: &[&str]) -> CoverageTable {
        CoverageTable::Format1(CoverageFormat1::new(names.iter().map(|n| GlyphId16::new(gid(font, n))).collect()))
    }

    fn value(v: i16) -> MathValueRecord {
        MathValueRecord::new(FWord::new(v), None)
    }

    fn variants(font: &SfntFont, list: &[(&str, u16)]) -> MathGlyphConstruction {
        MathGlyphConstruction::new(
            None,
            list.iter().map(|(n, a)| MathGlyphVariantRecord::new(GlyphId16::new(gid(font, n)), UfWord::new(*a))).collect(),
        )
    }

    fn names_of(merged: &SfntFont, c: &CoverageTable) -> Vec<String> {
        let names = glyph_names(merged).unwrap();
        super::super::layout::coverage_glyphs(c).iter().map(|g| names[usize::from(g.to_u16())].clone()).collect()
    }

    /// fontTools' test_merge_MATH, on fonts built here.
    #[test]
    fn math_tables_unite_their_glyph_data_and_keep_the_first_constants() {
        let mut one = built(
            &source("Math One", &[("a", 0x61, 500.0), ("parenleft", 0x28, 500.0), ("parenleft.size1", 0xE000, 500.0)], false),
            CompileFlavour::TrueType,
        );
        let mut two =
            built(&source("Math Two", &[("b", 0x62, 500.0), ("integral", 0x222B, 500.0), ("minus", 0x2212, 500.0)], false), CompileFlavour::TrueType);
        let three = built(&source("Math Three", &[("c", 0x63, 500.0)], false), CompileFlavour::TrueType);

        let kern = MathKernInfoRecord::new(Some(MathKern::new(vec![value(100)], vec![value(10), value(20)])), None, None, None);
        let info = MathGlyphInfo::new(
            Some(MathItalicsCorrectionInfo::new(cov(&one, &["a"]), vec![value(10)])),
            None,
            None,
            Some(MathKernInfo::new(cov(&one, &["a"]), vec![kern])),
        );
        let vert = variants(&one, &[("parenleft", 700), ("parenleft.size1", 1000)]);
        let math = Math::new(
            MathConstants { axis_height: value(250), ..MathConstants::default() },
            info,
            MathVariants::new(UfWord::new(20), Some(cov(&one, &["parenleft"])), None, vec![vert], vec![]),
        );
        one.insert(*b"MATH", write_fonts::dump_table(&math).unwrap());

        let info = MathGlyphInfo::new(
            Some(MathItalicsCorrectionInfo::new(cov(&two, &["b"]), vec![value(20)])),
            Some(MathTopAccentAttachment::new(cov(&two, &["b"]), vec![value(250)])),
            Some(cov(&two, &["integral"])),
            None,
        );
        let horiz = variants(&two, &[("minus", 500)]);
        let math = Math::new(
            MathConstants { axis_height: value(300), ..MathConstants::default() },
            info,
            MathVariants::new(UfWord::new(30), None, Some(cov(&two, &["minus"])), vec![], vec![horiz]),
        );
        two.insert(*b"MATH", write_fonts::dump_table(&math).unwrap());

        let merged = merge(&[one, two, three], &MergeOptions::default()).unwrap();
        let table: Math = read_fonts::tables::math::Math::read(FontData::new(merged.get(*b"MATH").unwrap())).unwrap().to_owned_table();
        assert_eq!(table.math_constants.axis_height.value, FWord::new(250));
        let info = &*table.math_glyph_info;
        let italics = info.math_italics_correction_info.as_ref().unwrap();
        assert_eq!(names_of(&merged, &italics.coverage), ["a", "b"]);
        assert_eq!(italics.italics_correction.iter().map(|v| v.value.to_i16()).collect::<Vec<_>>(), [10, 20]);
        let accents = info.math_top_accent_attachment.as_ref().unwrap();
        assert_eq!(names_of(&merged, &accents.top_accent_coverage), ["b"]);
        assert_eq!(accents.top_accent_attachment[0].value.to_i16(), 250);
        assert_eq!(names_of(&merged, info.extended_shape_coverage.as_ref().unwrap()), ["integral"]);
        let kerns = info.math_kern_info.as_ref().unwrap();
        assert_eq!(names_of(&merged, &kerns.math_kern_coverage), ["a"]);
        assert_eq!(kerns.math_kern_info_records.len(), 1);

        let v = &*table.math_variants;
        assert_eq!(v.min_connector_overlap.to_u16(), 20);
        assert_eq!(names_of(&merged, v.vert_glyph_coverage.as_ref().unwrap()), ["parenleft"]);
        let names = glyph_names(&merged).unwrap();
        let glyphs = |c: &MathGlyphConstruction| {
            c.math_glyph_variant_records.iter().map(|r| names[usize::from(r.variant_glyph.to_u16())].clone()).collect::<Vec<_>>()
        };
        assert_eq!(glyphs(&v.vert_glyph_constructions[0]), ["parenleft", "parenleft.size1"]);
        assert_eq!(names_of(&merged, v.horiz_glyph_coverage.as_ref().unwrap()), ["minus"]);
        assert_eq!(glyphs(&v.horiz_glyph_constructions[0]), ["minus"]);
    }

    fn base_table(tags: &[&[u8; 4]], scripts: &[(&[u8; 4], i16)]) -> Vec<u8> {
        let records = scripts
            .iter()
            .map(|(tag, low)| {
                let values = BaseValues::new(0, vec![BaseCoord::Format1(BaseCoordFormat1::new(*low)), BaseCoord::Format1(BaseCoordFormat1::new(0))]);
                BaseScriptRecord::new(OtTag::new(tag), BaseScript::new(Some(values), None, vec![]))
            })
            .collect();
        let tags = BaseTagList::new(tags.iter().map(|t| OtTag::new(t)).collect());
        write_fonts::dump_table(&Base::new(Some(Axis::new(Some(tags), BaseScriptList::new(records))), None)).unwrap()
    }

    #[test]
    fn base_tables_share_their_baselines_and_unite_their_scripts() {
        let mut one = first();
        one.insert(*b"BASE", base_table(&[b"ideo", b"romn"], &[(b"latn", -120), (b"DFLT", -120)]));
        let mut two = second();
        two.insert(*b"BASE", base_table(&[b"ideo", b"romn"], &[(b"hani", -100), (b"latn", -50)]));
        let merged = merge(&[one.clone(), two.clone()], &MergeOptions::default()).unwrap();
        let base: Base = read_fonts::tables::base::Base::read(FontData::new(merged.get(*b"BASE").unwrap())).unwrap().to_owned_table();
        let axis = base.horiz_axis.as_ref().unwrap();
        assert_eq!(axis.base_tag_list.as_ref().unwrap().baseline_tags, [OtTag::new(b"ideo"), OtTag::new(b"romn")], "kept once");
        let scripts: Vec<(OtTag, i16)> = axis
            .base_script_list
            .base_script_records
            .iter()
            .map(|r| match &*r.base_script.base_values.as_ref().unwrap().base_coords[0] {
                BaseCoord::Format1(f) => (r.base_script_tag, f.coordinate),
                other => panic!("{other:?}"),
            })
            .collect();
        assert_eq!(
            scripts,
            [(OtTag::new(b"DFLT"), -120), (OtTag::new(b"hani"), -100), (OtTag::new(b"latn"), -120)],
            "in tag order, the first font's latn"
        );

        // Layout base keeps the first font's table alone.
        let base_only = MergeOptions { layout: Some(LayoutMode::Base), ..MergeOptions::default() };
        assert_eq!(
            merge(&[one.clone(), two.clone()], &base_only).unwrap().get(*b"BASE"),
            Some(merge(&[one.clone(), second()], &base_only).unwrap().get(*b"BASE").unwrap())
        );

        two.insert(*b"BASE", base_table(&[b"romn"], &[(b"hani", -100)]));
        let e = merge(&[one, two], &MergeOptions::default()).unwrap_err();
        assert!(e.to_string().contains("different baselines"), "{e}");
    }
}

/// `font` with its `post` table replaced by its header at `version` followed by `rest`.
fn with_post(mut font: SfntFont, version: u32, rest: &[u8]) -> SfntFont {
    let mut post = font.get(*b"post").unwrap()[..32].to_vec();
    post[..4].copy_from_slice(&version.to_be_bytes());
    post.extend_from_slice(rest);
    font.insert(*b"post", post);
    font
}

fn codes(c: &[u16]) -> Vec<u8> {
    c.iter().flat_map(|v| v.to_be_bytes()).collect()
}

#[test]
fn post_versions_1_and_3_name_glyphs_as_fonttools_and_merge_to_their_header() {
    // fontTools 4.66.1 merge of fonts like these: a version 3 font takes its names from `cmap`
    // (U+0122 has no AGLFN name, so `uni0122`), and the result's `post` is the header alone.
    let one = built(&source("First Sans", &[("A", 0x41, 600.0), ("Gced", 0x122, 650.0)], false), CompileFlavour::TrueType);
    let names_one = glyph_names(&one).unwrap();
    let one = with_post(one, 0x0003_0000, b"");
    let two = second();
    let names_two = glyph_names(&two).unwrap();
    let result = merge_with_report(&[one, two], &MergeOptions::default()).unwrap();
    let mut expected: Vec<String> = names_one
        .iter()
        .map(|n| match n.as_str() {
            ".notdef" | "space" | "A" => n.clone(),
            _ => "uni0122".to_string(),
        })
        .collect();
    let taken = expected.clone();
    expected.extend(names_two.iter().map(|n| if taken.contains(n) { format!("{n}.1") } else { n.clone() }));
    let names: Vec<&str> = result.glyphs.iter().map(|g| g.name.as_str()).collect();
    assert_eq!(names, expected);
    let post = result.font.get(*b"post").unwrap();
    assert_eq!((post.len(), &post[..4]), (32, &[0, 3, 0, 0][..]));

    // Two version 1 fonts: standard Macintosh names, and a version 1 header.
    let (one, two) = (with_post(first(), 0x0001_0000, b""), with_post(second(), 0x0001_0000, b""));
    let result = merge_with_report(&[one, two], &MergeOptions::default()).unwrap();
    let names: Vec<&str> = result.glyphs.iter().map(|g| g.name.as_str()).collect();
    assert_eq!(names, [".notdef", ".null", "nonmarkingreturn", "space", ".notdef.1", ".null.1", "nonmarkingreturn.1", "space.1"]);
    let post = result.font.get(*b"post").unwrap();
    assert_eq!((post.len(), &post[..4]), (32, &[0, 1, 0, 0][..]));
}

#[test]
fn a_version_4_font_merges_to_version_4_with_a_code_per_merged_name() {
    // fontTools 4.66.1: font 1's codes name its glyphs `glyph00000`, `space`, `A` and `A.1`
    // (the repeated code); the result stores 0xFFFF for every name that is neither an Adobe
    // name nor `uniXXXX`, suffixed names included.
    let one = first();
    assert_eq!(glyph_names(&one).unwrap().len(), 4);
    let one = with_post(one, 0x0004_0000, &codes(&[0xFFFF, 0x20, 0x41, 0x41]));
    let two = second();
    let names_two = glyph_names(&two).unwrap();
    let result = merge_with_report(&[one, two], &MergeOptions::default()).unwrap();
    let mut expected = vec!["glyph00000".to_string(), "space".into(), "A".into(), "A.1".into()];
    let renamed = |n: &str| match n {
        "space" => "space.1".to_string(),
        "A" => "A.2".to_string(),
        n => n.to_string(),
    };
    expected.extend(names_two.iter().map(|n| renamed(n)));
    let names: Vec<&str> = result.glyphs.iter().map(|g| g.name.as_str()).collect();
    assert_eq!(names, expected);
    let mut expected_codes = vec![0xFFFF, 0x20, 0x41, 0xFFFF];
    expected_codes.extend(names_two.iter().map(|n| if n == "B" { 0x42 } else { 0xFFFF }));
    let post = result.font.get(*b"post").unwrap();
    assert_eq!(&post[..4], &[0, 4, 0, 0]);
    assert_eq!(post[32..], codes(&expected_codes));
}

#[test]
fn merged_post_names_keep_their_latin1_bytes() {
    // Font 1 stores glyph 3 under the one-byte name 0xE9, which fontTools reads and writes as
    // Latin-1.
    let mut rest = codes(&[4, 0, 3, 36, 258]);
    rest.extend_from_slice(&[1, 0xE9]);
    let one = with_post(first(), 0x0002_0000, &rest);
    let merged = merge(&[one, second()], &MergeOptions::default()).unwrap();
    let post = merged.get(*b"post").unwrap();
    let count = usize::from(u16::from_be_bytes([post[32], post[33]]));
    assert_eq!(u16::from_be_bytes([post[40], post[41]]), 258, "glyph 3 stores the first extra name");
    let strings = 34 + 2 * count;
    assert_eq!(post[strings..strings + 2], [1, 0xE9]);
}
