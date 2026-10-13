#![allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]
//! Behaviour tests of the variable compile on fonts built in code.

use std::sync::Arc;

use fontcraft_model::{Axis, Component, Font, Glyph, GlyphTable, Instance, Layer, Master, Node, NodeKind, Param, Path, Shape, Transform};
use skrifa::instance::Size;
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::raw::TableProvider;
use skrifa::raw::types::GlyphId;
use skrifa::{FontRef, MetadataProvider};

use super::*;

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
    let glyph = Cff2Glyph { name: "c".into(), vsindex: 0, path: vec![BlendOp::MoveTo((v(0.0), v(0.0))), curve] };
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
    // One value with more regions than the stack holds cannot be written.
    let err = fontcraft_cff::cff2::write_cff2(&many_regions(600)).unwrap_err();
    assert!(matches!(err, fontcraft_cff::cff2::Cff2Error::StackOverflow { .. }), "{err}");
}
