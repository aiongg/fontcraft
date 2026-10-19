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
    assert!(matches!(merge(&[one.clone(), cff], &MergeOptions::default()), Err(TablesError::Unsupported(_))));

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
    assert_eq!(maxima, [4, 1, 0, 0, 0, 0], "rectangles only, no composites");
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
