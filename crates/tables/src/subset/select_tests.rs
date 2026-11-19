//! The reference converter's glyph selection in a subset (`select`), on fonts built here.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::collections::BTreeSet;

use fontcraft_cff::PathOp;
use fontcraft_cff::cid::{CidFont, CidGlyph, FontDict, Ros};
use fontcraft_compile::{Flavour as CompileFlavour, Options, compile};
use fontcraft_model::{Component, Font, Path, Shape};

use super::*;
use crate::TablesError;
use crate::convert::{FontDictFilter, glyph_cids};
use crate::sfnt::{Flavour, read};

/// .notdef, A, V, acute, Aacute (A + acute as components), parenleft and parenright.
fn truetype() -> SfntFont {
    let mut f = Font::new("Select Sans");
    for (name, cp) in [("A", 0x41), ("V", 0x56), ("acute", 0xB4), ("Aacute", 0xC1), ("parenleft", 0x28), ("parenright", 0x29)] {
        if f.glyph(name).is_none() {
            f.add_glyph(name, vec![cp], 500.0).unwrap();
        }
    }
    for name in ["A", "V", "acute", "parenleft", "parenright"] {
        f.glyph_mut(name).unwrap().layers[0].shapes.push(Shape::Path(Path::rect(0.0, 0.0, 200.0, 700.0)));
    }
    {
        let mut c = f.glyph_mut("Aacute").unwrap();
        c.layers[0].shapes.push(Shape::Component(Component::new("A", 0.0, 0.0)));
        c.layers[0].shapes.push(Shape::Component(Component::new("acute", 200.0, 200.0)));
    }
    let out = compile(&f, &f.default_target().unwrap(), CompileFlavour::TrueType, &Options::default()).unwrap();
    read(&out.bytes, 0).unwrap()
}

fn with_filter(filter: GlyphFilter) -> SubsetOptions {
    SubsetOptions { filter, ..SubsetOptions::default() }
}

fn kept(font: &SfntFont, options: &SubsetOptions) -> BTreeSet<String> {
    subset(font, options).unwrap().retained.into_iter().collect()
}

fn names(list: &[&str]) -> BTreeSet<String> {
    list.iter().map(|s| s.to_string()).collect()
}

fn gids(font: &SfntFont, list: &[&str]) -> BTreeSet<u32> {
    let all = crate::reorder::glyph_names(font).unwrap();
    list.iter().map(|n| all.iter().position(|a| a == n).unwrap() as u32).collect()
}

#[test]
fn excluded_and_goadb_glyphs_narrow_the_selection_before_the_closure() {
    let font = truetype();
    // Alone, an exclusion starts from every glyph; a component of a kept glyph comes back.
    let excluded = kept(&font, &with_filter(GlyphFilter { exclude: gids(&font, &[".notdef", "A", "V"]), ..GlyphFilter::default() }));
    assert_eq!(excluded, names(&[".notdef", "A", "acute", "Aacute", "parenleft", "parenright"]));
    // With other selections, only what they chose is narrowed.
    let mut options = with_filter(GlyphFilter { exclude: gids(&font, &["V"]), ..GlyphFilter::default() });
    options.glyphs = vec!["V".into(), "parenleft".into()];
    assert_eq!(kept(&font, &options), names(&[".notdef", "parenleft"]));
    // A GOADB keeps the glyphs it names in either column.
    let goadb = with_filter(GlyphFilter { goadb: Some(names(&["V", "parenright", "zero"])), ..GlyphFilter::default() });
    assert_eq!(kept(&font, &goadb), names(&[".notdef", "V", "parenright"]));
    let mut options = goadb.clone();
    options.unicodes = vec![0x41, 0x56];
    assert_eq!(kept(&font, &options), names(&[".notdef", "V"]));
}

#[test]
fn a_random_share_repeats_for_a_font_and_keeps_at_least_one_glyph() {
    let font = truetype();
    let share =
        |millionths: u32, seed: Option<u64>| with_filter(GlyphFilter { random: Some(RandomShare { millionths, seed }), ..GlyphFilter::default() });
    // Half of the glyphs, rounded; none still chooses one.
    let n = crate::reorder::glyph_names(&font).unwrap().len();
    let plan = closure::plan(&font, &share(500_000, None)).unwrap();
    assert_eq!(plan.requested.len(), n.div_ceil(2));
    assert_eq!(plan, closure::plan(&font, &share(500_000, None)).unwrap(), "the seed taken from the font repeats");
    assert_eq!(closure::plan(&font, &share(0, Some(7))).unwrap().requested.len(), 1);
    assert_eq!(closure::plan(&font, &share(1_000_000, Some(7))).unwrap().requested.len(), n);
    // Different seeds pick different glyphs somewhere among a few tries.
    let picks: BTreeSet<Vec<u32>> =
        (0..8).map(|seed| closure::plan(&font, &share(300_000, Some(seed))).unwrap().requested.into_iter().collect()).collect();
    assert!(picks.len() > 1);
}

fn square(x: f64) -> Vec<PathOp> {
    vec![PathOp::MoveTo((x, 0.0)), PathOp::LineTo((x + 300.0, 0.0)), PathOp::LineTo((x + 300.0, 300.0)), PathOp::LineTo((x, 300.0))]
}

/// A CID-keyed CFF font: CIDs 0 and 1 in Font DICT 0, CID 5 in Font DICT 1.
fn two_dict_cid_font() -> SfntFont {
    let mut font = SfntFont::new(Flavour::Cff);
    let mut head = vec![0u8; 54];
    head[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    head[12..16].copy_from_slice(&0x5F0F_3CF5u32.to_be_bytes());
    head[18..20].copy_from_slice(&1000u16.to_be_bytes());
    let mut hhea = vec![0u8; 36];
    hhea[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    hhea[34..36].copy_from_slice(&3u16.to_be_bytes());
    let mut maxp = vec![0u8; 6];
    maxp[0..4].copy_from_slice(&0x0000_5000u32.to_be_bytes());
    maxp[4..6].copy_from_slice(&3u16.to_be_bytes());
    let hmtx: Vec<u8> = [500u16, 0, 500, 0, 500, 0].iter().flat_map(|v| v.to_be_bytes()).collect();
    let mut post = vec![0u8; 32];
    post[0..4].copy_from_slice(&0x0003_0000u32.to_be_bytes());
    let glyph = |cid: u32, fd: usize, path: Vec<PathOp>| CidGlyph {
        cid,
        fd,
        glyph: fontcraft_cff::Glyph { name: format!("cid{cid}"), advance: 500.0, path, hints: None },
    };
    let dict = |name: &str| FontDict { name: name.into(), private: fontcraft_cff::PrivateDict::default(), widths: fontcraft_cff::Widths::Optimise };
    let cid = CidFont {
        name: "Select-CID".into(),
        top: fontcraft_cff::TopDict::default(),
        ros: Ros { registry: "Adobe".into(), ordering: "Identity".into(), supplement: 0 },
        cid_count: None,
        font_dicts: vec![dict("Select-Zero"), dict("Select-One")],
        glyphs: vec![glyph(0, 0, Vec::new()), glyph(1, 0, square(0.0)), glyph(5, 1, square(50.0))],
        round_tolerance: 0.5,
        specialise: true,
    };
    for (tag, table) in [(*b"head", head), (*b"hhea", hhea), (*b"maxp", maxp), (*b"hmtx", hmtx), (*b"post", post)] {
        font.insert(tag, table);
    }
    font.insert(*b"CFF ", fontcraft_cff::cid::build(&cid).unwrap().table);
    font
}

#[test]
fn a_font_dict_selection_keeps_one_font_dict_of_a_cid_font() {
    let font = two_dict_cid_font();
    let dicts = |keep: Option<&[u16]>, exclude: &[u16]| {
        with_filter(GlyphFilter {
            font_dicts: Some(FontDictFilter { keep: keep.map(|k| k.iter().copied().collect()), exclude: exclude.iter().copied().collect() }),
            ..GlyphFilter::default()
        })
    };
    let kept_one = subset(&font, &dicts(Some(&[1]), &[])).unwrap().font;
    assert_eq!(glyph_cids(&kept_one).unwrap(), Some(vec![0, 5]), ".notdef stays");
    let without_one = subset(&font, &dicts(None, &[1])).unwrap().font;
    assert_eq!(glyph_cids(&without_one).unwrap(), Some(vec![0, 1]));
    let cff = crate::reorder::cff::parse(without_one.get(*b"CFF ").unwrap(), 2).unwrap();
    assert_eq!(cff.font_dict_of_glyphs().1, 1, "the Font DICT no kept glyph uses is dropped");
    // A Font DICT the font lacks, and a font without Font DICTs, are refused.
    assert!(matches!(subset(&font, &dicts(Some(&[2]), &[])), Err(TablesError::BadOptions(_))));
    assert!(matches!(subset(&truetype(), &dicts(Some(&[0]), &[])), Err(TablesError::BadOptions(_))));
}
