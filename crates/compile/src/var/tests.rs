#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]
//! Behaviour tests of the variable compile on fonts built in code.

use std::sync::Arc;

use fontcraft_model::{
    Axis, Component, Font, Glyph, GlyphTable, Hoi, HoiAxis, HoiCenter, HoiCurve, HoiDirection, HoiTiming, Instance, Layer, Master, Node, NodeKind,
    Param, Path, Rare, Shape, Transform,
};
use skrifa::instance::Size;
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::raw::TableProvider;
use skrifa::raw::types::GlyphId;
use skrifa::{FontRef, MetadataProvider};

use super::*;

mod interpolatable;

fn rect(w: f64) -> Shape {
    Shape::Path(Path::rect(0.0, 0.0, w, 700.0))
}

/// A closed bowl with two cubic segments, `w` units wide.
fn bowl(w: f64) -> Shape {
    let n = |x: f64, y: f64, kind| Node::new(x, y, kind);
    Shape::Path(Path::new(
        true,
        vec![
            n(0.0, 0.0, NodeKind::Line),
            n(0.0, 400.0, NodeKind::OffCurve),
            n(w * 0.2, 700.0, NodeKind::OffCurve),
            n(w, 700.0, NodeKind::Curve),
            n(w, 300.0, NodeKind::OffCurve),
            n(w * 0.8, 0.0, NodeKind::OffCurve),
            n(w * 0.5, 0.0, NodeKind::Curve),
        ],
    ))
}

fn layer(master: &str, width: f64, shapes: Vec<Shape>) -> Layer {
    let mut l = Layer::new(master, width);
    l.id = master.into();
    l.shapes = shapes;
    l
}

fn glyph(name: &str, unicode: u32, layers: Vec<Layer>) -> Glyph {
    let mut g = Glyph::new(name);
    g.unicodes = vec![unicode];
    g.layers = layers;
    g
}

fn scaled(base: &str, s: f64) -> Shape {
    Shape::Component(Component {
        glyph: base.into(),
        transform: Transform::Decomposed { pos: (0.0, 0.0), scale: (s, s), angle: 0.0, slant: (0.0, 0.0) },
        ..Component::new(base, 0.0, 0.0)
    })
}

/// Thin (wght 100) and Black (900), with:
/// - `a`: a rectangle 100 or 300 wide, advances 300 and 500;
/// - `o`: a cubic bowl;
/// - `b`: like `a`, with an intermediate layer at 500 that is 250 wide;
/// - `c`: `a` scaled by 1 in Thin and 1.5 in Black;
/// - `d`: `a` moved by 0 or 50;
/// - `e`: only in Thin.
fn two_masters() -> Font {
    let mut f = Font::new("Var Test");
    f.axes = Arc::new(vec![Axis { name: "Weight".into(), tag: "wght".into(), ..Axis::default() }]);
    f.masters = Arc::new(vec![
        Master { id: "thin".into(), name: "Thin".into(), location: vec![100.0], ..Master::default() },
        Master { id: "black".into(), name: "Black".into(), location: vec![900.0], ..Master::default() },
    ]);
    let mut brace = layer("thin", 450.0, vec![rect(250.0)]);
    brace.id = "brace".into();
    brace.info.attrs.coordinates = vec![Some(500.0)];
    let glyphs = vec![
        glyph("a", 0x61, vec![layer("thin", 300.0, vec![rect(100.0)]), layer("black", 500.0, vec![rect(300.0)])]),
        glyph("o", 0x6F, vec![layer("thin", 300.0, vec![bowl(200.0)]), layer("black", 700.0, vec![bowl(600.0)])]),
        glyph("b", 0x62, vec![layer("thin", 300.0, vec![rect(100.0)]), layer("black", 500.0, vec![rect(300.0)]), brace]),
        glyph("c", 0x63, vec![layer("thin", 300.0, vec![scaled("a", 1.0)]), layer("black", 500.0, vec![scaled("a", 1.5)])]),
        glyph(
            "d",
            0x64,
            vec![
                layer("thin", 300.0, vec![Shape::Component(Component::new("a", 0.0, 0.0))]),
                layer("black", 500.0, vec![Shape::Component(Component::new("a", 50.0, 0.0))]),
            ],
        ),
        glyph("e", 0x65, vec![layer("thin", 300.0, vec![rect(120.0)])]),
    ];
    f.glyphs = GlyphTable::from_glyphs(glyphs.into_iter().map(Arc::new).collect());
    f
}

fn build(font: &Font, setting: Option<usize>) -> VariableFont {
    compile_variable(font, setting, &Options::default(), &VariableOptions::default()).unwrap()
}

#[derive(Default)]
struct Bounds(Option<(f32, f32, f32, f32)>);

impl Bounds {
    fn add(&mut self, x: f32, y: f32) {
        self.0 = Some(match self.0 {
            None => (x, y, x, y),
            Some((a, b, c, d)) => (a.min(x), b.min(y), c.max(x), d.max(y)),
        });
    }
}

impl OutlinePen for Bounds {
    fn move_to(&mut self, x: f32, y: f32) {
        self.add(x, y);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.add(x, y);
    }
    fn quad_to(&mut self, _: f32, _: f32, x: f32, y: f32) {
        self.add(x, y);
    }
    fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, x: f32, y: f32) {
        self.add(x, y);
    }
    fn close(&mut self) {}
}

/// The on-curve bounds and the advance of `name` at `wght` (and `CPHT` when given).
fn at(v: &VariableFont, name: &str, coords: &[(&str, f32)]) -> ((f32, f32, f32, f32), f32) {
    let font = FontRef::new(&v.compiled.bytes).unwrap();
    let gid = GlyphId::new(v.compiled.glyph_order.iter().position(|g| g == name).unwrap() as u32);
    let location = font.axes().location(coords.iter().copied());
    let mut pen = Bounds::default();
    font.outline_glyphs().get(gid).unwrap().draw(DrawSettings::unhinted(Size::unscaled(), &location), &mut pen).unwrap();
    let advance = font.glyph_metrics(Size::unscaled(), &location).advance_width(gid).unwrap();
    (pen.0.unwrap_or_default(), advance)
}

#[test]
fn two_masters_vary_outlines_and_advances() {
    let v = build(&two_masters(), None);
    let font = FontRef::new(&v.compiled.bytes).unwrap();
    let axis = font.fvar().unwrap().axes().unwrap()[0];
    assert_eq!((axis.min_value().to_f64(), axis.default_value().to_f64(), axis.max_value().to_f64()), (100.0, 100.0, 900.0));
    assert!(font.gvar().is_ok() && font.hvar().is_ok());
    assert_eq!(at(&v, "a", &[("wght", 100.0)]), ((0.0, 0.0, 100.0, 700.0), 300.0));
    assert_eq!(at(&v, "a", &[("wght", 900.0)]), ((0.0, 0.0, 300.0, 700.0), 500.0));
    assert_eq!(at(&v, "a", &[("wght", 500.0)]), ((0.0, 0.0, 200.0, 700.0), 400.0));
    // The bowl's ends follow both masters: its curves were converted with the same points.
    assert_eq!(at(&v, "o", &[("wght", 900.0)]).0.2, 600.0);
    assert_eq!(at(&v, "o", &[("wght", 100.0)]).0.2, 200.0);
    // A glyph with no Black layer does not vary.
    assert_eq!(at(&v, "e", &[("wght", 900.0)]), ((0.0, 0.0, 120.0, 700.0), 300.0));
    assert!(v.compiled.warnings.iter().all(|w| !w.contains("`e`")), "{:?}", v.compiled.warnings);
}

#[test]
fn an_intermediate_layer_is_a_source_of_its_own_glyph_only() {
    let v = build(&two_masters(), None);
    assert_eq!(at(&v, "b", &[("wght", 500.0)]), ((0.0, 0.0, 250.0, 700.0), 450.0));
    assert_eq!(at(&v, "a", &[("wght", 500.0)]).0.2, 200.0);
    assert_eq!(at(&v, "b", &[("wght", 900.0)]).0.2, 300.0);
}

#[test]
fn composites_keep_moving_components_and_decompose_scaled_ones() {
    let v = build(&two_masters(), None);
    let font = FontRef::new(&v.compiled.bytes).unwrap();
    let glyf = font.glyf().unwrap();
    let loca = font.loca(None).unwrap();
    let kind = |name: &str| {
        let gid = GlyphId::new(v.compiled.glyph_order.iter().position(|g| g == name).unwrap() as u32);
        match loca.get_glyf(gid, &glyf).unwrap() {
            Some(skrifa::raw::tables::glyf::Glyph::Composite(_)) => "composite",
            Some(skrifa::raw::tables::glyf::Glyph::Simple(_)) => "simple",
            None => "empty",
        }
    };
    assert_eq!(kind("d"), "composite");
    assert_eq!(kind("c"), "simple");
    assert_eq!(at(&v, "d", &[("wght", 500.0)]).0, (25.0, 0.0, 225.0, 700.0));
    assert_eq!(at(&v, "c", &[("wght", 900.0)]).0, (0.0, 0.0, 450.0, 1050.0));
}

#[test]
fn incompatible_glyphs_block_the_export() {
    let mut f = two_masters();
    f.glyph_mut("a").unwrap().layers[1].shapes.push(rect(10.0));
    let err = compile_variable(&f, None, &Options::default(), &VariableOptions::default()).unwrap_err();
    assert!(err.to_string().contains("not compatible") && err.to_string().contains('a'), "{err}");
}

#[test]
fn advance_deltas_past_16_bits_are_refused() {
    let mut f = two_masters();
    f.glyph_mut("a").unwrap().layers[1].width = 50_000.0;
    let err = compile_variable(&f, None, &Options::default(), &VariableOptions::default()).unwrap_err().to_string();
    assert!(err.contains("`a`") && err.contains("does not fit"), "{err}");
}

/// [`two_masters`] with a third master Regular at 400 and two variable font settings, the second
/// disabling Black.
fn with_settings() -> Font {
    let mut f = two_masters();
    Arc::make_mut(&mut f.masters).insert(1, Master { id: "regular".into(), name: "Regular".into(), location: vec![400.0], ..Master::default() });
    for i in 0..f.glyphs.len() {
        let mut g = f.glyphs.get_mut_at(i).unwrap();
        if let Some(thin) = g.layers.iter().find(|l| l.id == "thin").cloned()
            && let Some(black) = g.layers.iter().find(|l| l.id == "black").cloned()
        {
            let mut mid = black;
            mid.id = "regular".into();
            mid.master = "regular".into();
            mid.width = (mid.width + thin.width) / 2.0;
            g.layers.push(mid);
        }
    }
    f.glyph_mut("e").unwrap().layers.retain(|l| l.id == "thin");
    let setting = |name: &str| Instance { name: name.into(), kind: Some("variable".into()), ..Instance::default() };
    let mut reduced = setting("Reduced");
    reduced.params.0.push(Param::new("Disable Masters", fontcraft_model::plist::Value::String("Black".into())));
    f.instances = Arc::new(vec![setting("Full"), reduced]);
    f
}

#[test]
fn each_setting_is_a_font_and_disable_masters_narrows_it() {
    let f = with_settings();
    let targets = variable_targets(&f);
    assert_eq!(targets, vec![Some(0), Some(1)]);
    let full = build(&f, Some(0));
    let reduced = build(&f, Some(1));
    assert_ne!(full.file_stem, reduced.file_stem);
    let max = |v: &VariableFont| FontRef::new(&v.compiled.bytes).unwrap().fvar().unwrap().axes().unwrap()[0].max_value().to_f64();
    assert_eq!((max(&full), max(&reduced)), (900.0, 400.0));
    assert_eq!(at(&reduced, "a", &[("wght", 400.0)]).0.2, 300.0);
    assert_eq!(at(&full, "a", &[("wght", 900.0)]).0.2, 300.0);
}

#[test]
fn a_virtual_master_extends_an_axis_for_intermediate_layers() {
    let mut f = two_masters();
    Arc::make_mut(&mut f.axes).push(Axis { name: "Cap".into(), tag: "CPHT".into(), ..Axis::default() });
    for m in Arc::make_mut(&mut f.masters) {
        m.location.push(700.0);
    }
    let entry = fontcraft_interp::params::VirtualMasterEntry { coordinates: vec![("Cap".into(), 800.0)] };
    let params = fontcraft_interp::params::set_virtual_masters(&f.info.params, &[entry]).unwrap();
    Arc::make_mut(&mut f.info).params = params;
    {
        let mut b = f.glyph_mut("b").unwrap();
        b.layers.retain(|l| l.id != "brace");
        let mut tall = b.layers[0].clone();
        tall.id = "tall".into();
        tall.shapes = vec![Shape::Path(Path::rect(0.0, 0.0, 100.0, 800.0))];
        tall.info.attrs.coordinates = vec![Some(100.0), Some(800.0)];
        b.layers.push(tall);
    }
    let v = build(&f, None);
    assert_eq!(at(&v, "b", &[("wght", 100.0), ("CPHT", 800.0)]).0, (0.0, 0.0, 100.0, 800.0));
    assert_eq!(at(&v, "a", &[("wght", 100.0), ("CPHT", 800.0)]).0, (0.0, 0.0, 100.0, 700.0));
}

/// `two_masters` with an alternate layer of `a` in each master for wght 600 and up (a wider
/// rectangle); `c` and `d`, made of `a`, get alternate layers through their components.
fn with_alternates() -> Font {
    let mut f = two_masters();
    let mut glyphs: Vec<Glyph> = f.glyphs.iter().map(|g| (**g).clone()).collect();
    let a = glyphs.iter_mut().find(|g| g.name == "a").unwrap();
    for (master, w) in [("thin", 150.0), ("black", 350.0)] {
        let mut alt = layer(master, if master == "thin" { 300.0 } else { 500.0 }, vec![rect(w)]);
        alt.id = format!("{master}-alt");
        alt.info.attrs.axis_rules = vec![fontcraft_model::AxisRule { min: Some(600.0), max: None }];
        a.layers.push(alt);
    }
    f.glyphs = GlyphTable::from_glyphs(glyphs.into_iter().map(Arc::new).collect());
    f
}

/// The single substitutions `GSUB`'s feature variations put in place at normalized `wght`, by
/// name, with the tag of the feature they go under.
fn variation_subs(v: &VariableFont, wght: f32) -> Vec<(String, String, String)> {
    use skrifa::raw::tables::gsub::{SingleSubst, SubstitutionSubtables};
    use skrifa::raw::tables::layout::Condition;
    let font = FontRef::new(&v.compiled.bytes).unwrap();
    let gsub = font.gsub().unwrap();
    let fv = gsub.feature_variations().unwrap().unwrap();
    let names = &v.compiled.glyph_order;
    let mut out = Vec::new();
    for record in fv.feature_variation_records() {
        let holds = match record.condition_set(fv.offset_data()).transpose().unwrap() {
            None => true,
            Some(set) => set.conditions().iter().all(|c| match c.unwrap() {
                Condition::Format1AxisRange(r) => {
                    r.axis_index() == 0 && r.filter_range_min_value().to_f32() <= wght && wght <= r.filter_range_max_value().to_f32()
                }
                _ => false,
            }),
        };
        if !holds {
            continue;
        }
        let subst = record.feature_table_substitution(fv.offset_data()).unwrap().unwrap();
        for s in subst.substitutions() {
            let tag = gsub.feature_list().unwrap().feature_records()[usize::from(s.feature_index())].feature_tag().to_string();
            for index in s.alternate_feature(subst.offset_data()).unwrap().lookup_list_indices() {
                let lookup = gsub.lookup_list().unwrap().lookups().get(usize::from(index.get())).unwrap();
                let SubstitutionSubtables::Single(subtables) = lookup.subtables().unwrap() else { panic!("a single substitution") };
                let name = |g: u16| names[usize::from(g)].clone();
                for sub in subtables.iter() {
                    match sub.unwrap() {
                        SingleSubst::Format1(sub) => {
                            for gid in sub.coverage().unwrap().iter() {
                                let to = (i32::from(gid.to_u16()) + i32::from(sub.delta_glyph_id())) as u16;
                                out.push((tag.clone(), name(gid.to_u16()), name(to)));
                            }
                        }
                        SingleSubst::Format2(sub) => {
                            for (gid, to) in sub.coverage().unwrap().iter().zip(sub.substitute_glyph_ids()) {
                                out.push((tag.clone(), name(gid.to_u16()), name(to.get().to_u16())));
                            }
                        }
                    }
                }
            }
        }
        // The first record whose conditions hold is the one applied.
        break;
    }
    out
}

#[test]
fn alternate_layers_become_glyphs_switched_by_rvrn() {
    let v = build(&with_alternates(), None);
    let order = &v.compiled.glyph_order;
    // The alternate glyphs follow the font's glyphs, in name order.
    assert_eq!(&order[order.len() - 3..], ["a.BRACKET.varAlt01", "c.BRACKET.varAlt01", "d.BRACKET.varAlt01"]);
    assert!(!v.compiled.warnings.iter().any(|w| w.contains("alternate")), "{:?}", v.compiled.warnings);
    // Each alternate glyph varies between its own layers: 150 wide in Thin, 350 in Black.
    assert_eq!(at(&v, "a.BRACKET.varAlt01", &[("wght", 100.0)]).0.2, 150.0);
    assert_eq!(at(&v, "a.BRACKET.varAlt01", &[("wght", 900.0)]).0.2, 350.0);
    assert_eq!(at(&v, "a", &[("wght", 900.0)]).0.2, 300.0);
    // A composite's alternate glyph uses the component's alternate glyph (moved by 50 in Black).
    assert_eq!(at(&v, "d.BRACKET.varAlt01", &[("wght", 900.0)]).0.2, 400.0);

    // wght 600 is 0.625 normalized: below it nothing is substituted, from it on both glyphs are.
    assert!(variation_subs(&v, 0.5).is_empty());
    let want = |p: &str| ("rvrn".to_string(), p.to_string(), format!("{p}.BRACKET.varAlt01"));
    assert_eq!(variation_subs(&v, 0.625), vec![want("a"), want("c"), want("d")]);
    assert_eq!(variation_subs(&v, 1.0), vec![want("a"), want("c"), want("d")]);

    // The Feature for Feature Variations parameter picks the feature.
    let mut f = with_alternates();
    Arc::make_mut(&mut f.info).params.0.push(Param::new(featvar::FEATURE_PARAMETER, fontcraft_model::plist::Value::String("rclt".into())));
    assert_eq!(variation_subs(&build(&f, None), 1.0)[0].0, "rclt");
}

/// A comma-separated feature list (the designspace lib key fontTools reads, which glyphsLib fills
/// from the parameter): every feature gets the record's lookups, in one record per region, as
/// fontmake builds the CustomFeatures source.
#[test]
fn a_feature_list_gives_each_feature_the_variation_lookups() {
    let mut f = with_alternates();
    Arc::make_mut(&mut f.info).params.0.push(Param::new(featvar::FEATURE_PARAMETER, fontcraft_model::plist::Value::String("salt, rclt,salt".into())));
    let v = build(&f, None);
    let font = FontRef::new(&v.compiled.bytes).unwrap();
    let gsub = font.gsub().unwrap();
    let tags: Vec<String> = gsub.feature_list().unwrap().feature_records().iter().map(|r| r.feature_tag().to_string()).collect();
    assert_eq!(tags, ["rclt", "salt"]);
    assert_eq!(gsub.lookup_list().unwrap().lookup_count(), 1);
    assert_eq!(gsub.feature_variations().unwrap().unwrap().feature_variation_records().len(), 1);
    let mut subs = variation_subs(&v, 1.0);
    subs.sort();
    let want = |tag: &str| ["a", "c", "d"].map(|p| (tag.to_string(), p.to_string(), format!("{p}.BRACKET.varAlt01")));
    assert_eq!(subs, [want("rclt"), want("salt")].concat());
    assert!(variation_subs(&v, 0.5).is_empty());
}

#[test]
fn alternate_glyphs_join_feature_code_and_kerning() {
    use fontcraft_features::evaluate::PairEvaluator;
    use fontcraft_model::Snippet;
    let mut f = with_alternates();
    Arc::make_mut(&mut f.features).features.push(Snippet { name: "ss01".into(), code: "sub o by b;".into(), ..Default::default() });
    let kerning = Arc::make_mut(&mut f.kerning);
    for master in ["thin", "black"] {
        kerning.ltr.set(master, "a", "o", -40.0);
    }
    let v = build(&f, None);
    let font = FontRef::new(&v.compiled.bytes).unwrap();
    let gsub = font.gsub().unwrap();
    let tags: Vec<String> = gsub.feature_list().unwrap().feature_records().iter().map(|r| r.feature_tag().to_string()).collect();
    assert!(tags.contains(&"ss01".to_string()) && tags.contains(&"rvrn".to_string()), "{tags:?}");
    // The code's lookup and the variation lookup share the lookup list, rvrn's first.
    assert_eq!(gsub.lookup_list().unwrap().lookup_count(), 2);
    assert_eq!(variation_subs(&v, 1.0).len(), 3);

    let gid = |n: &str| v.compiled.glyph_order.iter().position(|g| g == n).unwrap() as u16;
    let eval = PairEvaluator::new(&v.compiled.bytes).unwrap();
    let dflt = skrifa::raw::types::Tag::new(b"DFLT");
    assert_eq!(eval.value(dflt, gid("a"), gid("o")), -40);
    assert_eq!(eval.value(dflt, gid("a.BRACKET.varAlt01"), gid("o")), -40);
}

#[test]
fn touching_alternate_ranges_switch_as_the_oracles_do() {
    // `o` switches from wght 300 to 600 and `a` from 600 on. At 600 both ranges hold; the
    // oracles' records give `o` there, and the first record that holds is the one applied.
    let mut f = with_alternates();
    let mut glyphs: Vec<Glyph> = f.glyphs.iter().map(|g| (**g).clone()).collect();
    let o = glyphs.iter_mut().find(|g| g.name == "o").unwrap();
    for (master, w) in [("thin", 300.0), ("black", 700.0)] {
        let mut alt = layer(master, w, vec![bowl(w - 50.0)]);
        alt.id = format!("{master}-alt");
        alt.info.attrs.axis_rules = vec![fontcraft_model::AxisRule { min: Some(300.0), max: Some(600.0) }];
        o.layers.push(alt);
    }
    f.glyphs = GlyphTable::from_glyphs(glyphs.into_iter().map(Arc::new).collect());
    let v = build(&f, None);
    let names = |subs: Vec<(String, String, String)>| subs.into_iter().map(|s| s.1).collect::<Vec<_>>();
    assert_eq!(names(variation_subs(&v, 0.4)), ["o"]);
    assert_eq!(names(variation_subs(&v, 0.625)), ["o"]);
    assert_eq!(names(variation_subs(&v, 0.75)), ["a", "c", "d"]);
}

fn build_cff2(font: &Font, setting: Option<usize>) -> VariableFont {
    let var = VariableOptions { flavour: VarFlavour::Cff2, ..VariableOptions::default() };
    compile_variable(font, setting, &Options::default(), &var).unwrap()
}

/// Bounds of the drawn outline, curves sampled, so quadratic and cubic outlines of one shape
/// compare.
#[derive(Default)]
struct Drawn {
    bounds: Bounds,
    at: (f32, f32),
}

impl OutlinePen for Drawn {
    fn move_to(&mut self, x: f32, y: f32) {
        self.bounds.add(x, y);
        self.at = (x, y);
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.move_to(x, y);
    }
    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        let (x0, y0) = self.at;
        for i in 1..=32 {
            let t = i as f32 / 32.0;
            let u = 1.0 - t;
            self.bounds.add(u * u * x0 + 2.0 * u * t * cx + t * t * x, u * u * y0 + 2.0 * u * t * cy + t * t * y);
        }
        self.at = (x, y);
    }
    fn curve_to(&mut self, ax: f32, ay: f32, bx: f32, by: f32, x: f32, y: f32) {
        let (x0, y0) = self.at;
        for i in 1..=32 {
            let t = i as f32 / 32.0;
            let u = 1.0 - t;
            let (p, q, r, s) = (u * u * u, 3.0 * u * u * t, 3.0 * u * t * t, t * t * t);
            self.bounds.add(p * x0 + q * ax + r * bx + s * x, p * y0 + q * ay + r * by + s * y);
        }
        self.at = (x, y);
    }
    fn close(&mut self) {}
}

/// The sampled bounds and the advance of `name` at `wght`.
fn drawn_at(v: &VariableFont, name: &str, wght: f32) -> ((f32, f32, f32, f32), f32) {
    let font = FontRef::new(&v.compiled.bytes).unwrap();
    let gid = GlyphId::new(v.compiled.glyph_order.iter().position(|g| g == name).unwrap() as u32);
    let location = font.axes().location([("wght", wght)]);
    let mut pen = Drawn::default();
    // At one pixel per unit rather than unscaled, which truncates CFF coordinates to whole units.
    let size = Size::new(f32::from(font.head().unwrap().units_per_em()));
    font.outline_glyphs().get(gid).unwrap().draw(DrawSettings::unhinted(size, &location), &mut pen).unwrap();
    let advance = font.glyph_metrics(Size::unscaled(), &location).advance_width(gid).unwrap();
    (pen.bounds.0.unwrap_or_default(), advance)
}

/// Every glyph of `cff2` drawn like the TrueType build of the same source at each of
/// `locations` (the TrueType curves are approximations).
fn assert_like_truetype(tt: &VariableFont, cff2: &VariableFont, locations: &[f32]) {
    assert_eq!(tt.compiled.glyph_order, cff2.compiled.glyph_order);
    for name in &tt.compiled.glyph_order {
        for &w in locations {
            let ((a, b), (c, d)) = (drawn_at(tt, name, w), drawn_at(cff2, name, w));
            // The CFF2 reader truncates coordinates to whole units and the TrueType reader rounds them,
            // so a point half a unit off a whole number can read one unit apart.
            let close = [a.0 - c.0, a.1 - c.1, a.2 - c.2, a.3 - c.3, b - d].iter().all(|e| e.abs() <= 1.0);
            assert!(close, "`{name}` at {w}: TrueType {a:?} {b}, CFF2 {c:?} {d}");
        }
    }
}

#[test]
fn cff2_fonts_draw_like_the_truetype_build() {
    let f = two_masters();
    let cff2 = build_cff2(&f, None);
    let font = FontRef::new(&cff2.compiled.bytes).unwrap();
    assert!(font.cff2().is_ok() && font.hvar().is_ok() && font.fvar().is_ok());
    assert!(font.glyf().is_err() && font.gvar().is_err() && font.loca(None).is_err() && font.cff().is_err());
    assert_eq!(font.maxp().unwrap().version(), skrifa::raw::types::Version16Dot16::VERSION_0_5);
    // A CFF2 table holds no names, so `post` does.
    assert_eq!(font.post().unwrap().version(), skrifa::raw::types::Version16Dot16::VERSION_2_0);
    assert_like_truetype(&build(&f, None), &cff2, &[100.0, 300.0, 500.0, 700.0, 900.0]);
    // The intermediate layer of `b` and the moved component of `d`.
    assert_eq!(drawn_at(&cff2, "b", 500.0), ((0.0, 0.0, 250.0, 700.0), 450.0));
    assert_eq!(drawn_at(&cff2, "d", 500.0).0, (25.0, 0.0, 225.0, 700.0));
}

#[test]
fn cff2_with_three_masters_an_intermediate_layer_and_disabled_masters() {
    let f = with_settings();
    let (tt, cff2) = (build(&f, Some(0)), build_cff2(&f, Some(0)));
    assert_like_truetype(&tt, &cff2, &[100.0, 250.0, 400.0, 450.0, 500.0, 650.0, 900.0]);
    let reduced = build_cff2(&f, Some(1));
    let max = FontRef::new(&reduced.compiled.bytes).unwrap().fvar().unwrap().axes().unwrap()[0].max_value().to_f64();
    assert_eq!(max, 400.0);
    assert_eq!(drawn_at(&reduced, "a", 400.0).0.2, 300.0);
    assert_like_truetype(&build(&f, Some(1)), &reduced, &[100.0, 250.0, 400.0]);
}

#[test]
fn cff2_incompatible_glyphs_block_the_export() {
    let mut f = two_masters();
    f.glyph_mut("a").unwrap().layers[1].shapes.push(rect(10.0));
    let var = VariableOptions { flavour: VarFlavour::Cff2, ..VariableOptions::default() };
    let err = compile_variable(&f, None, &Options::default(), &var).unwrap_err();
    assert!(err.to_string().contains("not compatible"), "{err}");
}

/// A one-axis `CFF2` font of one curve glyph whose every coordinate varies over `regions`
/// regions, each peaking at a different point of the axis.
fn many_regions(regions: usize) -> fontcraft_cff::cff2::Cff2Font {
    use fontcraft_cff::cff2::{Blend, BlendOp, Cff2Font, Cff2Glyph, RegionAxis};
    let peaks: Vec<f64> = (1..=regions).map(|i| i as f64 / regions as f64).collect();
    let region = |i: usize| {
        let peak = peaks[i];
        let start = if i == 0 { 0.0 } else { peaks[i - 1] };
        let end = peaks.get(i + 1).copied().unwrap_or(1.0);
        vec![RegionAxis { start, peak, end }]
    };
    // Each coordinate moves by 1 unit per region; at a peak only that region applies.
    let v = |d: f64| Blend { default: d, deltas: vec![1.0; regions] };
    let curve = BlendOp::CurveTo((v(0.0), v(100.0)), (v(50.0), v(200.0)), (v(100.0), v(100.0)));
    let glyph = Cff2Glyph { name: "c".into(), vsindex: 0, path: vec![BlendOp::MoveTo((v(0.0), v(0.0))), curve], hints: None };
    Cff2Font {
        axis_count: 1,
        regions: (0..regions).map(region).collect(),
        region_lists: vec![(0..regions as u16).collect()],
        glyphs: vec![Cff2Glyph { name: ".notdef".into(), ..Cff2Glyph::default() }, glyph],
        round_tolerance: 0.01,
        ..Cff2Font::default()
    }
}

#[test]
fn cff2_blends_past_the_stack_limit_are_split() {
    use skrifa::instance::LocationRef;
    use skrifa::raw::types::F2Dot14;
    // 6 blended operands × 101 values each is past the 513-operand stack.
    let built = fontcraft_cff::cff2::write_cff2(&many_regions(100)).unwrap();
    let mut b = write_fonts::FontBuilder::new();
    b.add_raw(skrifa::raw::types::Tag::new(b"CFF2"), built.table);
    b.add_table(&write_fonts::tables::maxp::Maxp { num_glyphs: 2, ..Default::default() }).unwrap();
    b.add_table(&write_fonts::tables::head::Head { units_per_em: 1000, ..Default::default() }).unwrap();
    // The outline reader wants horizontal metrics too.
    b.add_table(&write_fonts::tables::hhea::Hhea { number_of_h_metrics: 2, ..Default::default() }).unwrap();
    let metric = write_fonts::tables::hmtx::LongMetric { advance: 500, side_bearing: 0 };
    b.add_table(&write_fonts::tables::hmtx::Hmtx::new(vec![metric.clone(), metric], Vec::new())).unwrap();
    let bytes = b.build();
    let font = FontRef::new(&bytes).unwrap();
    let coords = [F2Dot14::from_f32(0.5)];
    let mut pen = Bounds::default();
    let glyph = font.outline_glyphs().get(GlyphId::new(1)).unwrap();
    glyph.draw(DrawSettings::unhinted(Size::unscaled(), LocationRef::new(&coords)), &mut pen).unwrap();
    // At a peak one region applies: every coordinate moves by 1.
    assert_eq!(pen.0.unwrap(), (1.0, 1.0, 101.0, 101.0));
    // A region list with more regions than one blend can hold is refused before any glyph is
    // encoded.
    let err = fontcraft_cff::cff2::write_cff2(&many_regions(600)).unwrap_err();
    assert!(matches!(err, fontcraft_cff::cff2::Cff2Error::TooManyBlendRegions(600)), "{err}");
}

#[test]
fn cff2_stem_blends_past_the_stack_limit_leave_the_glyph_unhinted_with_a_warning() {
    use fontcraft_cff::cff2::{Blend, BlendStem, Cff2Hints};
    // With 500 regions a blended stem edge takes 501 operands: after the 12 plain operands of
    // the six fixed stems before it in its operator, it does not fit the 513-operand stack.
    let mut font = many_regions(500);
    let fixed = |edge: f64| BlendStem { edge: Blend::fixed(edge), width: Blend::fixed(20.0) };
    let mut h_stems: Vec<BlendStem> = (0..6).map(|k| fixed(100.0 * f64::from(k))).collect();
    h_stems.push(BlendStem { edge: Blend { default: 700.0, deltas: vec![1.0; 500] }, width: Blend::fixed(20.0) });
    font.glyphs[1].hints = Some(Cff2Hints { h_stems, ..Cff2Hints::default() });
    let mut warnings = Vec::new();
    let built = cff2::write(&font, &mut warnings).unwrap();
    assert_eq!(built.unhinted, vec!["c".to_string()]);
    assert!(warnings.iter().any(|w| w.starts_with("glyph `c`") && w.contains("without hints")), "{warnings:?}");
    assert_eq!(warnings.len(), 1, "{warnings:?}");

    // The same stems over 4 regions fit: the glyph keeps its hints, without a warning.
    let mut font = many_regions(4);
    let mut h_stems: Vec<BlendStem> = (0..6).map(|k| fixed(100.0 * f64::from(k))).collect();
    h_stems.push(BlendStem { edge: Blend { default: 700.0, deltas: vec![1.0; 4] }, width: Blend::fixed(20.0) });
    font.glyphs[1].hints = Some(Cff2Hints { h_stems, ..Cff2Hints::default() });
    let mut warnings = Vec::new();
    assert!(cff2::write(&font, &mut warnings).unwrap().unhinted.is_empty());
    assert!(warnings.is_empty(), "{warnings:?}");
}

#[test]
fn cff2_sources_without_an_outline_each_are_refused() {
    let master = |l: f64| (vec![l], fontcraft_cff::PrivateDict::default());
    let glyph = cff2::CubicSources {
        glyph: "a".into(),
        locations: vec![vec![0.0], vec![1.0]],
        paths: vec![vec![fontcraft_geom::BezPath::new()]],
        layers: Vec::new(),
        samples: 0,
    };
    let err = cff2::cff2_font(1, &[glyph], &[master(0.0), master(1.0)], 1000, None, &mut Vec::new()).unwrap_err();
    assert!(matches!(&err, CompileError::Glyph { glyph, .. } if glyph == "a"), "{err}");
}

#[test]
fn cff2_composites_of_many_repeated_components_export_quickly() {
    // `m` holds 60000 components cycling over 1000 empty glyphs, and 400 glyphs hold `m`: the
    // walk over each composite's components must not compare every name with every other.
    let mut f = two_masters();
    let mut glyphs: Vec<Glyph> = f.glyphs.iter().map(|g| (**g).clone()).collect();
    let both = |shapes: Vec<Shape>| vec![layer("thin", 300.0, shapes.clone()), layer("black", 500.0, shapes)];
    for i in 0..1000 {
        glyphs.push(glyph(&format!("e{i}"), 0xE000 + i, both(Vec::new())));
    }
    let parts: Vec<Shape> = (0..60_000).map(|i| Shape::Component(Component::new(format!("e{}", i % 1000), 0.0, 0.0))).collect();
    glyphs.push(glyph("m", 0xF000, both(parts)));
    for i in 0..400 {
        glyphs.push(glyph(&format!("t{i}"), 0xF100 + i, both(vec![Shape::Component(Component::new("m", 0.0, 0.0))])));
    }
    f.glyphs = GlyphTable::from_glyphs(glyphs.into_iter().map(Arc::new).collect());
    let started = std::time::Instant::now();
    let v = build_cff2(&f, None);
    assert!(started.elapsed() < std::time::Duration::from_secs(20), "took {:?}", started.elapsed());
    assert!(v.compiled.glyph_order.iter().any(|g| g == "t399"));
}

#[derive(Default)]
struct StemSink {
    h: Vec<(i32, i32)>,
    v: Vec<(i32, i32)>,
    masks: usize,
}

impl skrifa::raw::ps::cs::CommandSink for StemSink {
    fn move_to(&mut self, _: skrifa::raw::types::Fixed, _: skrifa::raw::types::Fixed) {}
    fn line_to(&mut self, _: skrifa::raw::types::Fixed, _: skrifa::raw::types::Fixed) {}
    fn curve_to(
        &mut self,
        _: skrifa::raw::types::Fixed,
        _: skrifa::raw::types::Fixed,
        _: skrifa::raw::types::Fixed,
        _: skrifa::raw::types::Fixed,
        _: skrifa::raw::types::Fixed,
        _: skrifa::raw::types::Fixed,
    ) {
    }
    fn close(&mut self) {}
    fn hstem(&mut self, y: skrifa::raw::types::Fixed, dy: skrifa::raw::types::Fixed) {
        self.h.push((y.to_i32(), dy.to_i32()));
    }
    fn vstem(&mut self, x: skrifa::raw::types::Fixed, dx: skrifa::raw::types::Fixed) {
        self.v.push((x.to_i32(), dx.to_i32()));
    }
    fn hint_mask(&mut self, _: &[u8]) {
        self.masks += 1;
    }
}

/// Stems as (lower, upper) edges.
type Stems = Vec<(i32, i32)>;

/// The horizontal and vertical stems of `name` in the `CFF2` table of `v` at `wght`, and its
/// number of hint masks.
fn cff2_stems(v: &VariableFont, name: &str, wght: f32) -> (Stems, Stems, usize) {
    let font = FontRef::new(&v.compiled.bytes).unwrap();
    let table = font.table_data(skrifa::raw::types::Tag::new(b"CFF2")).unwrap();
    let cff = skrifa::raw::ps::cff::CffFontRef::new_cff2(table.as_bytes(), None).unwrap();
    let location = font.axes().location([("wght", wght)]);
    let coords = location.coords();
    let subfont = cff.subfont(0, coords).unwrap();
    let gid = GlyphId::new(v.compiled.glyph_order.iter().position(|g| g == name).unwrap() as u32);
    let mut sink = StemSink::default();
    cff.evaluate_charstring(&subfont, gid, coords, &mut sink).unwrap();
    (sink.h, sink.v, sink.masks)
}

#[test]
fn cff2_private_dict_has_the_family_blues_of_each_master() {
    let mut f = two_masters();
    let zones = |top: f64, low: f64| {
        fontcraft_model::plist::openstep::parse_value(&format!(
            "({{pos = {top}; size = 12;}}, {{pos = 0; size = -12;}}, {{pos = {low}; size = -12;}})"
        ))
        .unwrap()
    };
    let masters = Arc::make_mut(&mut f.masters);
    masters[0].params.0.push(Param::new("Family Alignment Zones", zones(700.0, -200.0)));
    masters[1].params.0.push(Param::new("Family Alignment Zones", zones(720.0, -210.0)));
    let v = build_cff2(&f, None);
    let font = FontRef::new(&v.compiled.bytes).unwrap();
    let table = font.table_data(skrifa::raw::types::Tag::new(b"CFF2")).unwrap();
    let cff = skrifa::raw::ps::cff::CffFontRef::new_cff2(table.as_bytes(), None).unwrap();
    let pairs = |b: &skrifa::raw::ps::hinting::Blues| -> Vec<(i32, i32)> { b.values().iter().map(|(a, b)| (a.to_i32(), b.to_i32())).collect() };
    let at = |wght: f32| {
        let location = font.axes().location([("wght", wght)]);
        let (_, params) = cff.subfont_hinted(0, location.coords()).unwrap();
        (pairs(&params.family_blues), pairs(&params.family_other_blues), params.blue_fuzz.to_i32())
    };
    assert_eq!(at(100.0), (vec![(-12, 0), (700, 712)], vec![(-212, -200)], 1));
    assert_eq!(at(900.0), (vec![(-12, 0), (720, 732)], vec![(-222, -210)], 1));
}

#[test]
fn cff2_blends_manual_hints_that_follow_their_nodes() {
    use fontcraft_hinting::{Edge, NodeRef, PsHint, set_ps_hints};
    let node = |p, n| Edge::Node(NodeRef::new(p, n));
    let hinted = |glyph: &str, hints: &[PsHint]| {
        let mut f = two_masters();
        let mut g = f.glyph_mut(glyph).unwrap();
        set_ps_hints(&mut g.layers[0], hints).unwrap();
        drop(g);
        let options = Options { subroutinize: Some(false), ..Options::default() };
        compile_variable(&f, None, &options, &VariableOptions { flavour: VarFlavour::Cff2, ..VariableOptions::default() }).unwrap()
    };
    // `a` is a box 100 units wide in Thin and 300 in Black: a stem from its bottom-left node to
    // 80, a stem across its width and a top ghost on its top-left node.
    let main = [PsHint::stem(true, node(0, 0), Edge::Value(80.0)), PsHint::stem(false, node(0, 0), node(0, 1)), PsHint::ghost(true, node(0, 3))];
    let v = hinted("a", &main);
    for (wght, width) in [(100.0, 100), (500.0, 200), (900.0, 300)] {
        assert_eq!(cff2_stems(&v, "a", wght), (vec![(0, 80), (700, 680)], vec![(0, width)], 0), "at {wght}");
    }
    // The other glyphs' charstrings are those of the unhinted font.
    let plain = hinted("o", &[]);
    let charstrings = |v: &VariableFont| {
        let font = FontRef::new(&v.compiled.bytes).unwrap();
        let table = font.table_data(skrifa::raw::types::Tag::new(b"CFF2")).unwrap();
        fontcraft_cff::subr::charstrings_of_table(table.as_bytes(), true).unwrap().charstrings
    };
    let a = v.compiled.glyph_order.iter().position(|g| g == "a").unwrap();
    for (gid, (x, y)) in charstrings(&v).iter().zip(charstrings(&plain)).enumerate() {
        assert_eq!(gid == a, *x != y, "glyph {gid}");
    }

    // A fixed stem from 150 to 200 overlaps the box's stem only in Black, so Black needs hint
    // masks that Thin does not: the masters' hints differ in structure and are left out.
    let fixed = PsHint::stem(false, Edge::Value(150.0), Edge::Value(200.0));
    let v = hinted("a", &[PsHint::stem(false, node(0, 0), node(0, 1)), fixed]);
    assert_eq!(cff2_stems(&v, "a", 100.0), (Vec::new(), Vec::new(), 0));
    assert!(v.compiled.warnings.iter().any(|w| w.starts_with("glyph `a`: its PostScript hints differ in structure")), "{:?}", v.compiled.warnings);
}

#[test]
fn cff2_blends_the_hints_of_intermediate_layers() {
    use fontcraft_hinting::{Edge, NodeRef, PsHint, set_ps_hints};
    let node = |p, n| Edge::Node(NodeRef::new(p, n));
    let hints = |top: f64| [PsHint::stem(true, node(0, 0), Edge::Value(top)), PsHint::stem(false, node(0, 0), node(0, 1))];
    let build = |brace: Option<f64>| {
        let mut f = two_masters();
        let mut g = f.glyph_mut("b").unwrap();
        set_ps_hints(&mut g.layers[0], &hints(80.0)).unwrap();
        if let Some(top) = brace {
            set_ps_hints(&mut g.layers[2], &hints(top)).unwrap();
        }
        drop(g);
        let options = Options { subroutinize: Some(false), ..Options::default() };
        compile_variable(&f, None, &options, &VariableOptions { flavour: VarFlavour::Cff2, ..VariableOptions::default() }).unwrap()
    };
    let dropped = |v: &VariableFont| v.compiled.warnings.iter().any(|w| w.starts_with("glyph `b`"));
    // `b` is 100 wide in Thin, 250 in its intermediate layer at 500 and 300 in Black. Without
    // hints of its own, the intermediate layer takes Thin's through their nodes.
    let v = build(None);
    assert!(!dropped(&v), "{:?}", v.compiled.warnings);
    for (wght, width) in [(100.0, 100), (300.0, 175), (500.0, 250), (900.0, 300)] {
        assert_eq!(cff2_stems(&v, "b", wght), (vec![(0, 80)], vec![(0, width)], 0), "at {wght}");
    }
    // Its own hints in the same structure blend with the masters'.
    let v = build(Some(120.0));
    assert!(!dropped(&v), "{:?}", v.compiled.warnings);
    for (wght, top) in [(100.0, 80), (300.0, 100), (500.0, 120), (700.0, 100), (900.0, 80)] {
        assert_eq!(cff2_stems(&v, "b", wght).0, vec![(0, top)], "at {wght}");
    }
}

#[test]
fn cff2_blends_the_hints_of_a_sparse_alternate_layer() {
    use fontcraft_hinting::{Edge, NodeRef, PsHint, set_ps_hints};
    // `a`'s alternate layers (150 wide in Thin, 350 in Black) plus an intermediate alternate
    // layer at wght 750, 400 wide: a source of the alternate glyph only. Thin's alternate layer
    // carries the hints; the other layers take them through their nodes.
    let mut f = with_alternates();
    let mut a = f.glyph_mut("a").unwrap();
    let stem = PsHint::stem(false, Edge::Node(NodeRef::new(0, 0)), Edge::Node(NodeRef::new(0, 1)));
    let thin_alt = a.layers.iter_mut().find(|l| l.id == "thin-alt").unwrap();
    set_ps_hints(thin_alt, &[stem]).unwrap();
    let mut brace = layer("thin", 450.0, vec![rect(400.0)]);
    brace.id = "brace-alt".into();
    brace.info.attrs.coordinates = vec![Some(750.0)];
    brace.info.attrs.axis_rules = vec![fontcraft_model::AxisRule { min: Some(600.0), max: None }];
    a.layers.push(brace);
    drop(a);
    let options = Options { subroutinize: Some(false), ..Options::default() };
    let v = compile_variable(&f, None, &options, &VariableOptions { flavour: VarFlavour::Cff2, ..VariableOptions::default() }).unwrap();
    assert!(!v.compiled.warnings.iter().any(|w| w.contains("without hints")), "{:?}", v.compiled.warnings);
    for (wght, width) in [(100.0, 150), (750.0, 400), (900.0, 350)] {
        assert_eq!(cff2_stems(&v, "a.BRACKET.varAlt01", wght).1, vec![(0, width)], "at {wght}");
    }
}

#[test]
fn cff2_autohints_the_sources_of_a_glyph_together() {
    // `s`: two bars whose vertical stems overlap in Thin, so its hints need masks, and lie apart
    // in Black, which hinted alone would need none: hinted one by one the masters' hints would
    // differ in structure.
    let mut f = two_masters();
    let list = |text: &str| fontcraft_model::plist::openstep::parse_value(text).unwrap();
    for m in Arc::make_mut(&mut f.masters).iter_mut() {
        m.zones = vec![fontcraft_model::AlignmentZone { pos: 0.0, size: -10.0 }, fontcraft_model::AlignmentZone { pos: 700.0, size: 10.0 }];
        m.params.0.push(Param::new("postscriptStemSnapH", list("(80)")));
        m.params.0.push(Param::new("postscriptStemSnapV", list("(80)")));
    }
    let bars = |x: f64| vec![Shape::Path(Path::rect(100.0, 0.0, 180.0, 300.0)), Shape::Path(Path::rect(x, 400.0, x + 80.0, 700.0))];
    let mut glyphs: Vec<Glyph> = f.glyphs.iter().map(|g| (**g).clone()).collect();
    glyphs.push(glyph("s", 0x73, vec![layer("thin", 400.0, bars(140.0)), layer("black", 500.0, bars(300.0))]));
    f.glyphs = GlyphTable::from_glyphs(glyphs.into_iter().map(Arc::new).collect());
    let options = Options { subroutinize: Some(false), hinting: crate::HintingOptions { autohint: true, strip: false }, ..Options::default() };
    let v = compile_variable(&f, None, &options, &VariableOptions { flavour: VarFlavour::Cff2, ..VariableOptions::default() }).unwrap();
    assert!(!v.compiled.warnings.iter().any(|w| w.contains("without hints") || w.contains("autohinted together")), "{:?}", v.compiled.warnings);
    let (thin_h, thin_v, thin_masks) = cff2_stems(&v, "s", 100.0);
    let (black_h, black_v, black_masks) = cff2_stems(&v, "s", 900.0);
    assert_eq!(thin_v, vec![(100, 180), (140, 220)]);
    assert_eq!(black_v, vec![(100, 180), (300, 380)]);
    assert_eq!(thin_h.len(), black_h.len());
    assert!(thin_masks > 0 && thin_masks == black_masks, "{thin_masks} and {black_masks} masks");
    // The middle of the design space has Thin's structure with the stems in between.
    assert_eq!(cff2_stems(&v, "s", 500.0).1, vec![(100, 180), (220, 300)]);
}

#[test]
fn cff2_exports_are_subroutinised_and_draw_as_without_subroutines() {
    // Thirty glyphs in three shapes: whole charstrings and the rectangle's segments repeat.
    let mut f = two_masters();
    let mut glyphs: Vec<Glyph> = f.glyphs.iter().map(|g| (**g).clone()).collect();
    for i in 0..30u32 {
        let w = 200.0 + f64::from(i % 3) * 10.0;
        let layers = vec![layer("thin", 300.0, vec![bowl(w), rect(50.0)]), layer("black", 700.0, vec![bowl(w * 3.0), rect(80.0)])];
        glyphs.push(glyph(&format!("o{i}"), 0xE000 + i, layers));
    }
    f.glyphs = GlyphTable::from_glyphs(glyphs.into_iter().map(Arc::new).collect());
    let var = VariableOptions { flavour: VarFlavour::Cff2, ..VariableOptions::default() };
    let plain = compile_variable(&f, None, &Options { subroutinize: Some(false), ..Options::default() }, &var).unwrap();
    let subroutinised = build_cff2(&f, None);
    let table =
        |v: &VariableFont| FontRef::new(&v.compiled.bytes).unwrap().table_data(skrifa::raw::types::Tag::new(b"CFF2")).unwrap().as_bytes().to_vec();
    let (without, with) = (table(&plain), table(&subroutinised));
    assert!(with.len() < without.len(), "{} bytes with subroutines, {} without", with.len(), without.len());
    let set = fontcraft_cff::subr::charstrings_of_table(&with, true).unwrap();
    assert!(!set.global_subrs.is_empty() || set.local_subrs.iter().any(|s| !s.is_empty()));
    assert!(fontcraft_cff::subr::charstrings_of_table(&without, true).unwrap().global_subrs.is_empty());
    for name in &subroutinised.compiled.glyph_order {
        for w in [100.0, 300.0, 500.0, 900.0] {
            assert_eq!(drawn_at(&plain, name, w), drawn_at(&subroutinised, name, w), "`{name}` at {w}");
        }
    }
}

#[test]
fn open_corners_go_only_when_every_master_has_them() {
    let eared = |ear: f64| {
        let n = |x, y| Node::new(x, y, NodeKind::Line);
        Shape::Path(Path::new(true, vec![n(0.0, 0.0), n(100.0, 0.0), n(100.0, 100.0 + ear), n(100.0 + ear, 100.0), n(0.0, 100.0)]))
    };
    let with_black = |ear: f64| {
        let mut f = two_masters();
        let mut glyphs: Vec<Arc<Glyph>> = f.glyphs.as_arcs().to_vec();
        glyphs[0] = Arc::new(glyph("a", 0x61, vec![layer("thin", 300.0, vec![eared(40.0)]), layer("black", 500.0, vec![eared(ear)])]));
        f.glyphs = GlyphTable::from_glyphs(glyphs);
        build(&f, None)
    };
    let both = with_black(60.0);
    for w in [100.0, 500.0, 900.0] {
        assert_eq!(at(&both, "a", &[("wght", w)]).0, (0.0, 0.0, 100.0, 100.0));
    }
    // The black master's ear is larger than its sides, so it is part of the shape there.
    let one = with_black(150.0);
    assert_eq!(at(&one, "a", &[("wght", 100.0)]).0, (0.0, 0.0, 140.0, 140.0));
    assert_eq!(at(&one, "a", &[("wght", 900.0)]).0, (0.0, 0.0, 250.0, 250.0));
    assert!(one.compiled.warnings.iter().any(|w| w.contains("open corners were kept") && w.contains(": a")), "{:?}", one.compiled.warnings);
}

/// Thin and Black with vertical metrics: typographic ascender 800 and 860, descender -200 and
/// -240.
/// - `A`: its own vertical width (1000, 1100) and origin (20 and 50 below the ascender);
/// - `B`: neither;
/// - `C`: a composite of both, with neither;
/// - `D`: like `A`, with an intermediate layer at 500 (1010, 90 below);
/// - `E`: like `B`, with an intermediate layer at 500.
fn vertical_font() -> Font {
    use fontcraft_model::plist::{Number, Value};
    let mut f = two_masters();
    let param = |name: &str, v: i64| Param::new(name, Value::Number(Number::int(v)));
    Arc::make_mut(&mut f.info).params.0.extend([param("vheaVertAscender", 500), param("vheaVertDescender", -500), param("vheaVertLineGap", 0)]);
    for (m, (ascender, descender)) in Arc::make_mut(&mut f.masters).iter_mut().zip([(800, -200), (860, -240)]) {
        m.params.0.extend([param("typoAscender", ascender), param("typoDescender", descender)]);
    }
    let own = |mut l: Layer, width: f64, origin: f64| {
        l.info.vert_width = Some(width);
        l.info.vert_origin = Some(origin);
        l
    };
    let brace = |mut l: Layer| {
        l.id = "brace".into();
        l.info.attrs.coordinates = vec![Some(500.0)];
        l
    };
    let (thin, black) = (|| layer("thin", 600.0, vec![rect(500.0)]), || layer("black", 640.0, vec![rect(560.0)]));
    let pair = |w: f64, dx: f64, master: &str| {
        layer(master, w, vec![Shape::Component(Component::new("A", 0.0, 0.0)), Shape::Component(Component::new("B", dx, 0.0))])
    };
    let glyphs = vec![
        glyph("A", 0x41, vec![own(thin(), 1000.0, 20.0), own(black(), 1100.0, 50.0)]),
        glyph("B", 0x42, vec![thin(), black()]),
        glyph("C", 0x43, vec![pair(900.0, 300.0, "thin"), pair(980.0, 340.0, "black")]),
        glyph("D", 0x44, vec![own(thin(), 1000.0, 20.0), own(black(), 1100.0, 50.0), brace(own(thin(), 1010.0, 90.0))]),
        glyph("E", 0x45, vec![thin(), black(), brace(thin())]),
    ];
    f.glyphs = GlyphTable::from_glyphs(glyphs.into_iter().map(Arc::new).collect());
    f
}

// The numbers are fontc 1.0.0's and fontmake 3.12.1's on this source (`gvar`, `VVAR`, `VORG`),
// but for the vertical origins of glyphs without an intermediate layer in a CFF2 font (see
// docs/spec/40-compile.md § 4a).
#[test]
fn vertical_metrics_vary_through_vvar_and_the_phantom_points() {
    use skrifa::raw::types::F2Dot14;
    let gid = |v: &VariableFont, name: &str| GlyphId::new(v.compiled.glyph_order.iter().position(|g| g == name).unwrap() as u32);
    // Per glyph: the deltas of the advance, the vertical origin and the bottom at 500 and at 900.
    let expected = [
        (".notdef", [(50.0, 30.0, -20.0), (100.0, 60.0, -40.0)]),
        ("A", [(50.0, 15.0, -35.0), (100.0, 30.0, -70.0)]),
        ("B", [(50.0, 30.0, -20.0), (100.0, 60.0, -40.0)]),
        ("C", [(50.0, 30.0, -20.0), (100.0, 60.0, -40.0)]),
        ("D", [(10.0, -70.0, -80.0), (100.0, 30.0, -70.0)]),
        ("E", [(0.0, 0.0, 0.0), (100.0, 60.0, -40.0)]),
    ];
    let coords = [[F2Dot14::from_f32(0.5)], [F2Dot14::from_f32(1.0)]];

    let f = vertical_font();
    let tt = build(&f, None);
    let font = FontRef::new(&tt.compiled.bytes).unwrap();
    let (vmtx, vvar, gvar) = (font.vmtx().unwrap(), font.vvar().unwrap(), font.gvar().unwrap());
    let (glyf, loca) = (font.glyf().unwrap(), font.loca(None).unwrap());
    assert!(font.vhea().is_ok() && font.vorg().is_err() && vvar.v_org_mapping().is_none());
    for (name, deltas) in expected {
        let g = gid(&tt, name);
        assert_eq!(vmtx.advance(g), Some(1000), "{name}");
        for (at, (height, top, bottom)) in coords.iter().zip(deltas) {
            assert_eq!(vvar.advance_delta(g, at).unwrap().to_f64(), height, "{name} advance at {at:?}");
            let phantoms = gvar.phantom_point_deltas(&glyf, &loca, at, g).unwrap();
            assert_eq!((phantoms[2].y.to_f64(), phantoms[3].y.to_f64()), (top, bottom), "{name} phantom points at {at:?}");
        }
    }

    let cff2 = build_cff2(&f, None);
    let font = FontRef::new(&cff2.compiled.bytes).unwrap();
    let (vvar, vorg) = (font.vvar().unwrap(), font.vorg().unwrap());
    assert!(font.vhea().is_ok() && font.gvar().is_err());
    for (name, deltas) in expected {
        let g = gid(&cff2, name);
        assert_eq!(vorg.vertical_origin_y(g), if matches!(name, "A" | "D") { 780 } else { 800 }, "{name}");
        for (at, (height, top, _)) in coords.iter().zip(deltas) {
            assert_eq!(vvar.advance_delta(g, at).unwrap().to_f64(), height, "{name} CFF2 advance at {at:?}");
            assert_eq!(vvar.v_origin_y_delta(g, at).unwrap().to_f64(), top, "{name} CFF2 origin at {at:?}");
        }
    }

    // Without vertical metrics there is no table and the phantom points stay put.
    let plain = build(&two_masters(), None);
    let font = FontRef::new(&plain.compiled.bytes).unwrap();
    assert!(font.vvar().is_err() && font.vmtx().is_err());
    let (glyf, loca) = (font.glyf().unwrap(), font.loca(None).unwrap());
    let phantoms = font.gvar().unwrap().phantom_point_deltas(&glyf, &loca, &coords[1], gid(&plain, "a")).unwrap();
    assert_eq!((phantoms[2].y.to_f64(), phantoms[3].y.to_f64()), (0.0, 0.0));
}

/// A 700-high rectangle `right` wide whose bottom right node keeps `curve` on the weight axis.
fn curved(right: f64, curve: Option<HoiCurve>) -> Shape {
    let mut nodes: Vec<Node> =
        [(0.0, 0.0), (right, 0.0), (right, 700.0), (0.0, 700.0)].into_iter().map(|(x, y)| Node::new(x, y, NodeKind::Line)).collect();
    if let Some(curve) = curve {
        let mut info = (*nodes[1].info).clone();
        info.hoi = Hoi(vec![HoiAxis { axis: "wght".into(), curve }]);
        nodes[1].info = Rare::new(info);
    }
    Shape::Path(Path::new(true, nodes))
}

/// [`two_masters`] with `a` bent by `curve` (so its composites `c`, decomposed, and `d` are too).
fn with_curve(curve: Option<HoiCurve>) -> Font {
    let mut f = two_masters();
    f.glyph_mut("a").unwrap().layers[0].shapes = vec![curved(100.0, curve)];
    f.glyph_mut("a").unwrap().layers[1].shapes = vec![curved(300.0, None)];
    f
}

type Box4 = (f64, f64, f64, f64);

/// The bounds of the nodes of `a` in a static instance at `wght`.
fn instance_bounds(f: &Font, wght: f64) -> Box4 {
    let designspace = fontcraft_interp::Designspace::new(f).unwrap();
    let sources = fontcraft_interp::GlyphSources::of(f, &designspace, "a").unwrap();
    let options = fontcraft_interp::InterpOptions::default();
    let layer = fontcraft_interp::instance::interpolate_layer(f, &sources, &fontcraft_interp::Location(vec![wght]), &options).unwrap();
    let Some(Shape::Path(p)) = layer.shapes.first() else { panic!("no path") };
    p.nodes.iter().fold((f64::MAX, f64::MAX, f64::MIN, f64::MIN), |b, n| (b.0.min(n.x), b.1.min(n.y), b.2.max(n.x), b.3.max(n.y)))
}

/// The widest difference between drawn bounds and expected ones.
fn off(drawn: (f32, f32, f32, f32), expected: Box4) -> f64 {
    let pairs = [(drawn.0, expected.0), (drawn.1, expected.1), (drawn.2, expected.2), (drawn.3, expected.3)];
    pairs.into_iter().map(|(a, b)| (f64::from(a) - b).abs()).fold(0.0, f64::max)
}

#[test]
fn interpolation_curves_are_followed_along_their_span() {
    let amplitudes = HoiTiming { lower_time: Some(0.6), upper_time: Some(0.2), lower_amplitude: Some(-40.0), upper_amplitude: Some(-70.0) };
    let curves = [
        HoiCurve::Point { x: 450.0, y: -120.0 },
        HoiCurve::Timing(amplitudes),
        HoiCurve::Rotation { center: Some(HoiCenter::At { x: 200.0, y: 0.0 }), direction: Some(HoiDirection::CounterClockwise) },
    ];
    for curve in curves {
        let f = with_curve(Some(curve.clone()));
        let (tt, cff2) = (build(&f, None), build_cff2(&f, None));
        assert!(instance_bounds(&f, 500.0).1 < -40.0, "{curve:?} bends the outline");
        for k in 0..=32u8 {
            let wght = 100.0 + 25.0 * f32::from(k);
            let expected = instance_bounds(&f, f64::from(wght));
            for (what, v) in [("TrueType", &tt), ("CFF2", &cff2)] {
                let drawn = drawn_at(v, "a", wght).0;
                assert!(off(drawn, expected) <= 1.05, "{what} {curve:?} at {wght}: {drawn:?} against {expected:?}");
                // `d` moves `a` by up to 50; `c` scales it by up to a half.
                let s = f64::from(k) / 32.0;
                let moved = (expected.0 + 50.0 * s, expected.1, expected.2 + 50.0 * s, expected.3);
                assert!(off(drawn_at(v, "d", wght).0, moved) <= 1.55, "{what} {curve:?}: the composite at {wght}");
                let m = 1.0 + 0.5 * s;
                let scaled = (expected.0 * m, expected.1 * m, expected.2 * m, expected.3 * m);
                assert!(off(drawn_at(v, "c", wght).0, scaled) <= 2.5, "{what} {curve:?}: the decomposed composite at {wght}");
            }
        }
    }
}

#[test]
fn curves_that_bend_nothing_add_nothing_and_advances_keep_their_own_regions() {
    let table = |v: &VariableFont, tag: &[u8; 4]| {
        let font = FontRef::new(&v.compiled.bytes).unwrap();
        font.table_data(skrifa::raw::types::Tag::new(tag)).unwrap().as_bytes().to_vec()
    };
    let plain = build(&with_curve(None), None);
    let straight = build(&with_curve(Some(HoiCurve::Timing(HoiTiming::default()))), None);
    let bent = build(&with_curve(Some(HoiCurve::Point { x: 200.0, y: -120.0 })), None);
    for tag in [b"gvar", b"HVAR", b"glyf"] {
        assert_eq!(table(&plain, tag), table(&straight, tag), "{}", String::from_utf8_lossy(tag));
    }
    assert_ne!(table(&plain, b"gvar"), table(&bent, b"gvar"));
    assert_eq!(table(&plain, b"HVAR"), table(&bent, b"HVAR"), "advances vary with the glyph's own layers");
}

#[test]
fn a_curve_fades_along_the_other_axes_as_in_instances() {
    let mut f = two_masters();
    let axis = |name: &str, tag: &str| Axis { name: name.into(), tag: tag.into(), ..Axis::default() };
    f.axes = Arc::new(vec![axis("Weight", "wght"), axis("Width", "wdth")]);
    let master = |id: &str, location: [f64; 2]| Master { id: id.into(), name: id.into(), location: location.to_vec(), ..Master::default() };
    f.masters = Arc::new(vec![
        master("thin", [100.0, 0.0]),
        master("black", [900.0, 0.0]),
        master("wide", [100.0, 100.0]),
        master("wideblack", [900.0, 100.0]),
    ]);
    let curve = HoiCurve::Point { x: 200.0, y: -120.0 };
    let a = glyph(
        "a",
        0x61,
        vec![
            layer("thin", 300.0, vec![curved(100.0, Some(curve))]),
            layer("black", 500.0, vec![curved(300.0, None)]),
            layer("wide", 400.0, vec![curved(200.0, None)]),
            layer("wideblack", 700.0, vec![curved(500.0, None)]),
        ],
    );
    f.glyphs = GlyphTable::from_glyphs(vec![Arc::new(a)]);
    let v = build(&f, None);
    for (wdth, bottom) in [(0.0, -120.0), (25.0, -90.0), (50.0, -60.0), (100.0, 0.0)] {
        let (drawn, _) = at(&v, "a", &[("wght", 500.0), ("wdth", wdth)]);
        assert!((drawn.1 - bottom).abs() <= 1.05, "at width {wdth}: {drawn:?}");
    }
}
