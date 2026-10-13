#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::collections::{BTreeMap, HashMap};

use fontcraft_compile::{Flavour as CompileFlavour, Options, compile};
use fontcraft_model::{Component, Font, Path, Shape};
use read_fonts::tables::glyf::Glyph;
use read_fonts::types::GlyphId;
use read_fonts::{FontRef, TableProvider};

use super::*;
use crate::TablesError;
use crate::reorder::{cff, glyph_names};
use crate::sfnt::{WriteOptions, read, write};
use crate::subset::core::Glyf;

/// .notdef, A, V, acute, Aacute (A + acute as components), parenleft and parenright.
fn source() -> Font {
    let mut f = Font::new("Subset Sans");
    for (name, cp, width) in [
        ("A", 0x41, 600.0),
        ("V", 0x56, 650.0),
        ("acute", 0xB4, 300.0),
        ("Aacute", 0xC1, 600.0),
        ("parenleft", 0x28, 300.0),
        ("parenright", 0x29, 310.0),
    ] {
        if f.glyph(name).is_none() {
            f.add_glyph(name, vec![cp], width).unwrap();
        }
        f.glyph_mut(name).unwrap().layers[0].width = width;
    }
    for (name, x) in [("A", 0.0), ("V", 10.0), ("acute", 50.0), ("parenleft", 20.0), ("parenright", 30.0)] {
        f.glyph_mut(name).unwrap().layers[0].shapes.push(Shape::Path(Path::rect(x, 0.0, x + 200.0, 700.0)));
    }
    {
        let mut c = f.glyph_mut("Aacute").unwrap();
        c.layers[0].shapes.push(Shape::Component(Component::new("A", 0.0, 0.0)));
        c.layers[0].shapes.push(Shape::Component(Component::new("acute", 200.0, 200.0)));
    }
    f
}

fn built(flavour: CompileFlavour) -> SfntFont {
    let f = source();
    let out = compile(&f, &f.default_target().unwrap(), flavour, &Options::default()).unwrap();
    read(&out.bytes, 0).unwrap()
}

fn gid_of(font: &SfntFont, name: &str) -> u32 {
    glyph_names(font).unwrap().iter().position(|n| n == name).unwrap() as u32
}

fn cmap_of(bytes: &[u8]) -> BTreeMap<u32, u32> {
    let f = FontRef::new(bytes).unwrap();
    let cmap = f.cmap().unwrap();
    let mut out = BTreeMap::new();
    for record in cmap.encoding_records() {
        for (u, g) in record.subtable(cmap.offset_data()).unwrap().iter() {
            out.insert(u, g.to_u32());
        }
    }
    out
}

/// The glyf records of `font`, one per glyph.
fn glyf_glyphs(font: &SfntFont) -> Vec<Vec<u8>> {
    let glyf = Glyf::read(font, crate::reorder::names::num_glyphs(font).unwrap()).unwrap().unwrap();
    (0..glyf.len()).map(|g| glyf.glyph(g).to_vec()).collect()
}

/// Replaces the glyf and loca tables of `font` with `glyphs` (long loca offsets).
fn set_glyf(font: &mut SfntFont, glyphs: &[Vec<u8>]) {
    let mut glyf_bytes = Vec::new();
    let mut loca = Vec::new();
    for g in glyphs {
        loca.extend_from_slice(&(glyf_bytes.len() as u32).to_be_bytes());
        glyf_bytes.extend_from_slice(g);
        while glyf_bytes.len() % 4 != 0 {
            glyf_bytes.push(0);
        }
    }
    loca.extend_from_slice(&(glyf_bytes.len() as u32).to_be_bytes());
    let mut head = font.get(*b"head").unwrap().to_vec();
    head[50..52].copy_from_slice(&1u16.to_be_bytes());
    font.insert(*b"head", head);
    font.insert(*b"glyf", glyf_bytes);
    font.insert(*b"loca", loca);
}

fn advance(bytes: &[u8], g: u32) -> u16 {
    FontRef::new(bytes).unwrap().hmtx().unwrap().advance(GlyphId::new(g)).unwrap()
}

#[test]
fn text_keeps_components_mirrors_and_an_empty_notdef() {
    let font = built(CompileFlavour::TrueType);
    let (a, acute, aacute, paren_l, paren_r) =
        (gid_of(&font, "A"), gid_of(&font, "acute"), gid_of(&font, "Aacute"), gid_of(&font, "parenleft"), gid_of(&font, "parenright"));
    let options = SubsetOptions { text: "Á(".into(), ..SubsetOptions::default() };
    let result = subset(&font, &options).unwrap();
    let mut kept = vec![0, a, acute, aacute, paren_l, paren_r];
    kept.sort_unstable();
    assert_eq!(result.glyph_map.keys().copied().collect::<Vec<_>>(), kept, "components and the mirrored parenthesis join the set");
    assert_eq!(result.retained.len(), 6);
    assert_eq!(result.retained[0], ".notdef");

    let bytes = write(&result.font, &WriteOptions::default()).unwrap();
    let out = FontRef::new(&bytes).unwrap();
    assert_eq!(out.maxp().unwrap().num_glyphs(), 6);
    let new = |old: u32| result.glyph_map[&old];
    let cmap = cmap_of(&bytes);
    assert_eq!(cmap.get(&0xC1), Some(&new(aacute)));
    assert_eq!(cmap.get(&0x29), Some(&new(paren_r)), "the mirrored character is mapped");
    assert_eq!(cmap.get(&0x41), None, "A is kept as a component only, so U+0041 is not mapped");

    let glyf = out.glyf().unwrap();
    let loca = out.loca(None).unwrap();
    assert!(loca.get_glyf(GlyphId::new(0), &glyf).unwrap().is_none(), ".notdef loses its outline");
    let Some(Glyph::Composite(c)) = loca.get_glyf(GlyphId::new(new(aacute)), &glyf).unwrap() else { panic!("Aacute stays composite") };
    let parts: Vec<u32> = c.components().map(|c| c.glyph.to_u32()).collect();
    assert_eq!(parts, vec![new(a), new(acute)], "components point at the new glyph ids");
    assert_eq!(advance(&bytes, new(aacute)), 600);
    assert_eq!(advance(&bytes, new(paren_r)), 310);
    assert_eq!(out.post().unwrap().version().to_major_minor(), (3, 0), "glyph names go by default");
}

#[test]
fn glyph_ids_with_retain_gids_leave_empty_glyphs_in_place() {
    let font = built(CompileFlavour::TrueType);
    let v = gid_of(&font, "V");
    let options = SubsetOptions { gids: vec![v], retain_gids: true, ..SubsetOptions::default() };
    let result = subset(&font, &options).unwrap();
    assert_eq!(result.glyph_map.len(), v as usize + 1, "every glyph up to the last kept one stays");
    assert!(result.glyph_map.iter().all(|(o, n)| o == n));
    let bytes = write(&result.font, &WriteOptions::default()).unwrap();
    let out = FontRef::new(&bytes).unwrap();
    let (glyf, loca) = (out.glyf().unwrap(), out.loca(None).unwrap());
    for g in 1..v {
        assert!(loca.get_glyf(GlyphId::new(g), &glyf).unwrap().is_none(), "glyph {g} is emptied");
        assert_eq!(advance(&bytes, g), 0);
    }
    assert!(loca.get_glyf(GlyphId::new(v), &glyf).unwrap().is_some());
    assert_eq!(advance(&bytes, v), 650);
    assert_eq!(cmap_of(&bytes).get(&0x56), Some(&v), "a glyph requested by id keeps its code points");
}

#[test]
fn missing_glyphs_and_code_points_follow_the_options() {
    let font = built(CompileFlavour::TrueType);
    let missing_char = SubsetOptions { unicodes: vec![0x41, 0x4E00], ..SubsetOptions::default() };
    assert_eq!(subset(&font, &missing_char).unwrap().missing_unicodes, vec![0x4E00], "missing characters are skipped by default");
    let strict = SubsetOptions { ignore_missing_unicodes: false, ..missing_char };
    assert!(matches!(subset(&font, &strict), Err(TablesError::BadOptions(_))));
    let missing_glyph = SubsetOptions { glyphs: vec!["nosuch".into()], ..SubsetOptions::default() };
    assert!(matches!(subset(&font, &missing_glyph), Err(TablesError::BadOptions(_))), "missing glyphs are refused by default");
    let lenient = SubsetOptions { ignore_missing_glyphs: true, ..missing_glyph };
    assert_eq!(subset(&font, &lenient).unwrap().glyph_map.len(), 1, "only .notdef is left");
}

#[test]
fn no_hinting_drops_instructions_and_the_hinting_tables() {
    let mut font = built(CompileFlavour::TrueType);
    let a = gid_of(&font, "A") as usize;
    // Give A an instruction (PUSHB[0] 1) and the font a prep table.
    let glyphs = glyf_glyphs(&font);
    let mut with = glyphs.clone();
    let simple = &glyphs[a];
    let contours = i16::from_be_bytes([simple[0], simple[1]]) as usize;
    let at = 10 + 2 * contours;
    let mut patched = simple[..at].to_vec();
    patched.extend_from_slice(&[0, 2, 0xB0, 1]);
    patched.extend_from_slice(&simple[at + 2..]);
    with[a] = patched;
    set_glyf(&mut font, &with);
    font.insert(*b"prep", vec![0xB0, 1]);

    let keep = subset(&font, &SubsetOptions { text: "A".into(), ..SubsetOptions::default() }).unwrap();
    assert!(keep.font.get(*b"prep").is_some());
    let bytes = write(&keep.font, &WriteOptions::default()).unwrap();
    let out = FontRef::new(&bytes).unwrap();
    let new_a = GlyphId::new(keep.glyph_map[&(a as u32)]);
    let Some(Glyph::Simple(g)) = out.loca(None).unwrap().get_glyf(new_a, &out.glyf().unwrap()).unwrap() else { panic!() };
    assert_eq!(g.instructions(), &[0xB0, 1]);

    let strip = subset(&font, &SubsetOptions { text: "A".into(), hinting: false, ..SubsetOptions::default() }).unwrap();
    assert!(strip.font.get(*b"prep").is_none());
    assert!(strip.dropped.iter().any(|d| d.tag == *b"prep"));
    let bytes = write(&strip.font, &WriteOptions::default()).unwrap();
    let out = FontRef::new(&bytes).unwrap();
    let Some(Glyph::Simple(g)) = out.loca(None).unwrap().get_glyf(new_a, &out.glyf().unwrap()).unwrap() else { panic!() };
    assert!(g.instructions().is_empty());
    assert_eq!(g.num_points(), 4, "the outline survives");
    assert_eq!(out.maxp().unwrap().max_function_defs(), Some(0));
}

#[test]
fn cff_closes_over_seac_and_keeps_the_notdef_width() {
    let mut font = built(CompileFlavour::Cff);
    let n = crate::reorder::names::num_glyphs(&font).unwrap();
    let (a, acute, aacute) = (gid_of(&font, "A"), gid_of(&font, "acute"), gid_of(&font, "Aacute"));
    // Aacute as `width 0 0 65 194 endchar`: an accented glyph built from A and acute.
    let table = cff::parse(font.get(*b"CFF ").unwrap(), n).unwrap();
    let (_, nominal) = table.widths(aacute as usize);
    let mut seac = Vec::new();
    for v in [600.0 - nominal.unwrap() as f64, 0.0, 0.0, 65.0, 194.0] {
        seac.extend(cs::encode_number(v));
    }
    seac.push(14);
    let identity: Vec<u16> = (0..n as u16).collect();
    let rebuilt = table.rebuild_with(&identity, None, &HashMap::from([(aacute as usize, seac)]), false).unwrap();
    let original = font.clone();
    let table = cff::parse(original.get(*b"CFF ").unwrap(), n).unwrap();
    font.insert(*b"CFF ", rebuilt);

    let result = subset(&font, &SubsetOptions { text: "Á".into(), ..SubsetOptions::default() }).unwrap();
    let mut kept = vec![0, a, acute, aacute];
    kept.sort_unstable();
    assert_eq!(result.glyph_map.keys().copied().collect::<Vec<_>>(), kept);
    let out_cff = cff::parse(result.font.get(*b"CFF ").unwrap(), 4).unwrap();
    assert_eq!(out_cff.glyph_names(), [".notdef", "A", "acute", "Aacute"]);
    let notdef = out_cff.charstring(0).unwrap();
    assert_eq!(notdef.last(), Some(&14), ".notdef is reduced to its width and endchar");
    let budget = &mut cs::MAX_FONT_CHARSTRING_OPS.clone();
    let (local, global) = out_cff.subrs(0);
    let scan = cs::scan(notdef, local, global, budget).unwrap();
    let (local0, global0) = table.subrs(0);
    assert_eq!(scan.width, cs::scan(table.charstring(0).unwrap(), local0, global0, budget).unwrap().width, "the width stays");
    let bytes = write(&result.font, &WriteOptions::default()).unwrap();
    assert_eq!(cmap_of(&bytes).get(&0xC1), Some(&3));
    assert_eq!(advance(&bytes, 3), 600);

    let retained = subset(&font, &SubsetOptions { gids: vec![a], retain_gids: true, hinting: false, ..SubsetOptions::default() }).unwrap();
    let out_cff = cff::parse(retained.font.get(*b"CFF ").unwrap(), a as usize + 1).unwrap();
    for g in 1..a as usize {
        assert_eq!(out_cff.charstring(g), Some(&[14u8][..]), "glyph {g} becomes endchar");
    }
}

#[test]
fn layout_tables_are_dropped_with_a_reason() {
    let mut font = built(CompileFlavour::TrueType);
    font.insert(*b"GSUB", vec![0, 1, 0, 0, 0, 0, 0, 0, 0, 0]);
    font.insert(*b"DSIG", vec![0, 0, 0, 1, 0, 0, 0, 0]);
    font.insert(*b"zzzz", vec![1, 2, 3]);
    let result = subset(&font, &SubsetOptions { text: "A".into(), ..SubsetOptions::default() }).unwrap();
    let reasons: BTreeMap<Tag, &str> = result.dropped.iter().map(|d| (d.tag, d.reason.as_str())).collect();
    assert!(reasons[b"GSUB"].contains("not built"));
    assert_eq!(reasons[b"DSIG"], "dropped by the options");
    assert!(reasons.contains_key(b"zzzz"));
    let kept = subset(&font, &SubsetOptions { text: "A".into(), passthrough_tables: true, ..SubsetOptions::default() }).unwrap();
    assert_eq!(kept.font.get(*b"zzzz"), Some(&[1u8, 2, 3][..]));
}

#[test]
fn nothing_kept_is_an_error_not_a_crash() {
    let font = built(CompileFlavour::TrueType);
    let options = SubsetOptions { notdef_glyph: false, ..SubsetOptions::default() };
    assert!(subset(&font, &options).is_err());
}

#[test]
fn a_component_past_the_glyph_count_is_an_error() {
    let mut font = built(CompileFlavour::TrueType);
    let aacute = gid_of(&font, "Aacute") as usize;
    let mut glyphs = glyf_glyphs(&font);
    // The first component's glyph id follows the 10-byte header and the 2-byte flags.
    glyphs[aacute][12..14].copy_from_slice(&0xFFFFu16.to_be_bytes());
    set_glyf(&mut font, &glyphs);
    let err = subset(&font, &SubsetOptions { text: "Á".into(), retain_gids: true, ..SubsetOptions::default() }).unwrap_err();
    assert!(err.to_string().contains("past the glyph count"), "{err}");
}

#[test]
fn a_format_4_cmap_mapping_u_ffff_stays_format_4() {
    let mut f = source();
    f.add_glyph("uniFFFF", vec![0xFFFF], 500.0).unwrap();
    let out = compile(&f, &f.default_target().unwrap(), CompileFlavour::TrueType, &Options::default()).unwrap();
    let font = read(&out.bytes, 0).unwrap();
    let formats = |bytes: &[u8]| -> Vec<u16> {
        let f = FontRef::new(bytes).unwrap();
        let cmap = f.cmap().unwrap();
        cmap.encoding_records().iter().map(|r| r.subtable(cmap.offset_data()).unwrap().format()).collect()
    };
    assert!(formats(&out.bytes).iter().all(|f| *f == 4), "the input maps U+FFFF in format 4");
    let result = subset(&font, &SubsetOptions { unicodes: vec![0x41, 0xFFFF], ..SubsetOptions::default() }).unwrap();
    let bytes = write(&result.font, &WriteOptions::default()).unwrap();
    assert!(formats(&bytes).iter().all(|f| *f == 4));
    assert_eq!(cmap_of(&bytes).get(&0xFFFF), Some(&result.glyph_map[&gid_of(&font, "uniFFFF")]));
}

#[test]
fn fan_out_subroutines_over_many_glyphs_hit_the_font_budget() {
    // Global subroutine k calls k + 1 four times, eight levels deep: about 2^18 operations
    // per glyph, under the per-glyph cap, and past the font's budget over 300 glyphs.
    let mut f = Font::new("Fan Out");
    for i in 0..300u32 {
        f.add_glyph(&format!("g{i}"), vec![0x100 + i], 500.0).unwrap();
    }
    let out = compile(&f, &f.default_target().unwrap(), CompileFlavour::Cff, &Options::default()).unwrap();
    let mut font = read(&out.bytes, 0).unwrap();
    let n = crate::reorder::names::num_glyphs(&font).unwrap();
    let call = |k: usize| [cs::encode_number(k as f64 - 107.0), vec![29]].concat();
    let mut subrs: Vec<Vec<u8>> = (0..8).map(|k| [call(k + 1).repeat(4), vec![11]].concat()).collect();
    subrs.push(vec![11]);
    let items: Vec<&[u8]> = subrs.iter().map(Vec::as_slice).collect();
    let glyph = [call(0), vec![14]].concat();
    let replace: HashMap<usize, Vec<u8>> = (0..n).map(|g| (g, glyph.clone())).collect();
    let table = cff::parse(font.get(*b"CFF ").unwrap(), n).unwrap();
    let rebuilt = table.with_global_subrs(&items, &replace).unwrap();
    font.insert(*b"CFF ", rebuilt);

    let one = closure::plan(&font, &SubsetOptions { gids: vec![1], ..SubsetOptions::default() }).unwrap();
    assert!(one.charstring_ops_left < cs::MAX_FONT_CHARSTRING_OPS - (1 << 18), "one glyph runs every call");
    let err = closure::plan(&font, &SubsetOptions { all_glyphs: true, ..SubsetOptions::default() }).unwrap_err();
    assert!(matches!(err, TablesError::TooLarge { limit: cs::MAX_FONT_CHARSTRING_OPS, .. }), "{err}");
}
