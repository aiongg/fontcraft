//! Interpolatable masters (M2.9) of the fonts built in code.

use skrifa::raw::tables::glyf::Glyph as GlyfGlyph;

use super::*;
use crate::var::interpolatable::{MasterFont, MasterSource, compile_masters, master_sources};

/// Per glyph, its structure: on-curve flags and contour sizes, or component bases and offsets.
fn structure(font: &MasterFont, name: &str) -> (Vec<bool>, Vec<u16>, Vec<(u16, i16)>) {
    let parsed = FontRef::new(&font.compiled.bytes).unwrap();
    let gid = font.compiled.glyph_order.iter().position(|g| g == name).unwrap();
    match parsed.loca(None).unwrap().get_glyf(GlyphId::new(gid as u32), &parsed.glyf().unwrap()).unwrap() {
        None => Default::default(),
        Some(GlyfGlyph::Simple(g)) => {
            (g.points().map(|p| p.on_curve).collect(), g.end_pts_of_contours().iter().map(|e| e.get()).collect(), Vec::new())
        }
        Some(GlyfGlyph::Composite(g)) => {
            let offset = |anchor| match anchor {
                skrifa::raw::tables::glyf::Anchor::Offset { x, .. } => x,
                skrifa::raw::tables::glyf::Anchor::Point { .. } => panic!("a point anchor"),
            };
            (Vec::new(), Vec::new(), g.components().map(|c| (c.glyph.to_u16(), offset(c.anchor))).collect())
        }
    }
}

fn advance(font: &MasterFont, name: &str) -> u16 {
    let parsed = FontRef::new(&font.compiled.bytes).unwrap();
    let gid = font.compiled.glyph_order.iter().position(|g| g == name).unwrap();
    parsed.hmtx().unwrap().advance(GlyphId::new(gid as u32)).unwrap()
}

fn tags(font: &MasterFont) -> Vec<String> {
    FontRef::new(&font.compiled.bytes).unwrap().table_directory().table_records().iter().map(|r| r.tag().to_string()).collect()
}

#[test]
fn masters_share_their_points_and_intermediate_layers_become_sparse_fonts() {
    let font = two_masters();
    let fonts = compile_masters(&font, Flavour::TrueType, &Options::default()).unwrap();
    let source = |master: &str, at: Option<&str>| MasterSource { master: master.into(), intermediate: at.map(str::to_string) };
    let sources = [source("thin", None), source("black", None), source("thin", Some("{500}"))];
    assert_eq!(fonts.iter().map(|f| f.source.clone()).collect::<Vec<_>>(), sources);
    assert_eq!(master_sources(&font).unwrap(), sources, "the list without compiling is the same");

    let [thin, black, sparse] = fonts.as_slice() else { panic!("{} fonts", fonts.len()) };
    assert_eq!(thin.compiled.glyph_order, black.compiled.glyph_order);
    for name in &thin.compiled.glyph_order {
        let (a, b) = (structure(thin, name), structure(black, name));
        assert_eq!((&a.0, &a.1), (&b.0, &b.1), "{name}");
    }
    // The bowl's curves, the scaled composite drawn as contours, the moved one kept.
    assert!(structure(thin, "o").0.iter().any(|on| !on));
    assert!(structure(thin, "c").2.is_empty());
    assert_eq!(structure(thin, "d").2.iter().map(|c| c.1).collect::<Vec<_>>(), [0]);
    assert_eq!(structure(black, "d").2.iter().map(|c| c.1).collect::<Vec<_>>(), [50]);
    assert_eq!((advance(thin, "a"), advance(black, "a")), (300, 500));
    // A glyph without a layer in Black is drawn there as the variable font draws it.
    assert_eq!(structure(black, "e"), structure(thin, "e"));

    assert_eq!(sparse.compiled.glyph_order, [".notdef", "b"]);
    assert_eq!((advance(sparse, ".notdef"), advance(sparse, "b")), (65_535, 450));
    assert_eq!(structure(sparse, "b").0, structure(thin, "b").0);
    assert_eq!(tags(sparse), ["glyf", "head", "hmtx", "loca", "maxp", "post"]);
}

#[test]
fn cubic_masters_draw_the_same_commands_without_subroutines() {
    #[derive(Default, Debug, PartialEq)]
    struct Commands(String);
    impl OutlinePen for Commands {
        fn move_to(&mut self, _: f32, _: f32) {
            self.0.push('M');
        }
        fn line_to(&mut self, _: f32, _: f32) {
            self.0.push('L');
        }
        fn quad_to(&mut self, _: f32, _: f32, _: f32, _: f32) {
            self.0.push('Q');
        }
        fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, _: f32, _: f32) {
            self.0.push('C');
        }
        fn close(&mut self) {
            self.0.push('Z');
        }
    }
    let commands = |font: &MasterFont, name: &str| {
        let parsed = FontRef::new(&font.compiled.bytes).unwrap();
        let gid = font.compiled.glyph_order.iter().position(|g| g == name).unwrap();
        let mut pen = Commands::default();
        let glyph = parsed.outline_glyphs().get(GlyphId::new(gid as u32)).unwrap();
        glyph.draw(DrawSettings::unhinted(Size::unscaled(), skrifa::instance::LocationRef::default()), &mut pen).unwrap();
        pen
    };
    let fonts = compile_masters(&two_masters(), Flavour::Cff, &Options::default()).unwrap();
    let [thin, black, sparse] = fonts.as_slice() else { panic!("{} fonts", fonts.len()) };
    for name in &thin.compiled.glyph_order {
        assert_eq!(commands(thin, name), commands(black, name), "{name}");
    }
    assert!(commands(thin, "o").0.contains('C'));
    assert_eq!(commands(thin, "d").0, commands(thin, "a").0, "a CFF composite is drawn as contours");
    assert_eq!(commands(sparse, "b"), commands(thin, "b"));
    assert_eq!(tags(sparse), ["CFF ", "head", "hmtx", "maxp"]);
    let parsed = FontRef::new(&thin.compiled.bytes).unwrap();
    let cff = parsed.table_data(skrifa::Tag::new(b"CFF ")).unwrap();
    let set = fontcraft_cff::subr::charstrings_of_table(cff.as_bytes(), false).unwrap();
    assert!(set.global_subrs.is_empty() && set.local_subrs.iter().all(Vec::is_empty));
}

#[test]
fn a_glyph_that_cannot_interpolate_is_refused_by_name() {
    let mut font = two_masters();
    font.glyphs.push(glyph("x", 0x78, vec![layer("thin", 300.0, vec![rect(100.0)]), layer("black", 500.0, vec![bowl(300.0)])]));
    let e = compile_masters(&font, Flavour::TrueType, &Options::default()).unwrap_err();
    assert!(matches!(&e, CompileError::Incompatible(names) if names == "x"), "{e}");
}
