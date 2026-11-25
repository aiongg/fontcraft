use std::collections::HashMap;
use std::sync::Arc;

use fontcraft_model::plist::{Dict, Number, Str, Value, openstep::parse_value};
use fontcraft_model::{Component, Font, Node, NodeKind, Param, Path, Shape, Transform, UserData};
use skrifa::instance::{LocationRef, Size};
use skrifa::outline::{DrawSettings, OutlinePen};
use skrifa::raw::TableProvider;
use skrifa::raw::tables::glyf::{Anchor, Glyph as RawGlyph};
use skrifa::raw::types::GlyphId;
use skrifa::{FontRef, MetadataProvider};

use super::{ComponentOptions, DECOMPOSE_COMPONENTS, DECOMPOSE_GLYPHS, FILTERS_KEY};
use crate::{Compiled, Options, compile_ttf};

/// A glyph as `glyf` stores it: simple, or its components as (base, x scale, x offset, y offset).
#[derive(Clone, Debug, PartialEq)]
enum Stored {
    Empty,
    Simple,
    Composite(Vec<(String, f32, i16, i16)>),
}

fn component(base: &str, dx: f64, dy: f64) -> Shape {
    Shape::Component(Component::new(base, dx, dy))
}

fn scaled(base: &str, scale: f64) -> Shape {
    Shape::Component(Component { transform: Transform::Matrix([scale, 0.0, 0.0, scale, 0.0, 0.0]), ..Component::new(base, 0.0, 0.0) })
}

/// A and E are simple; B a plain composite of A; C overlapping components (E sits on A's corner);
/// D a scaled A; F a composite of the composite B; G the composites B and D; H mixes a contour
/// and a component; I a composite of the mixed H.
fn font() -> Font {
    let mut f = Font::new("Parts Sans");
    let glyphs: [(&str, Vec<Shape>); 9] = [
        ("A", vec![Shape::Path(Path::rect(60.0, 0.0, 560.0, 700.0))]),
        ("E", vec![Shape::Path(Path::rect(0.0, 0.0, 100.0, 100.0))]),
        ("B", vec![component("A", 10.0, 0.0)]),
        ("C", vec![component("A", 0.0, 0.0), component("E", 50.0, -50.0)]),
        ("D", vec![scaled("A", 0.5)]),
        ("F", vec![component("B", 100.0, 50.0)]),
        ("G", vec![component("B", 0.0, 0.0), component("D", 0.0, 0.0)]),
        ("H", vec![Shape::Path(Path::rect(0.0, 0.0, 40.0, 40.0)), component("A", 0.0, 0.0)]),
        ("I", vec![component("H", 5.0, 0.0)]),
    ];
    for (name, shapes) in glyphs {
        let mut g = f.glyph_mut(name).unwrap();
        g.layers[0].width = 600.0;
        g.layers[0].shapes = shapes;
    }
    f
}

fn number(v: i64) -> Value {
    Value::Number(Number::int(v))
}

fn font_param(f: &mut Font, name: &str, value: Value) {
    Arc::make_mut(&mut f.info).params.0.push(Param::new(name, value));
}

fn with_filters(text: &str) -> UserData {
    let mut d = Dict::new();
    d.insert(FILTERS_KEY, parse_value(text).unwrap());
    UserData(d)
}

/// Puts the filter list `text` (property-list text) in the first master's user data.
fn master_filters(f: &mut Font, text: &str) {
    Arc::make_mut(&mut f.masters)[0].user_data = with_filters(text);
}

fn build(f: &Font, components: ComponentOptions) -> Compiled {
    compile_ttf(f, &Options { remove_overlap: false, components, ..Default::default() }).unwrap()
}

fn stored(out: &Compiled) -> HashMap<String, Stored> {
    let font = FontRef::new(&out.bytes).unwrap();
    let (loca, glyf) = (font.loca(None).unwrap(), font.glyf().unwrap());
    let mut map = HashMap::new();
    for (i, name) in out.glyph_order.iter().enumerate() {
        let kind = match loca.get_glyf(GlyphId::new(i as u32), &glyf).unwrap() {
            None => Stored::Empty,
            Some(RawGlyph::Simple(_)) => Stored::Simple,
            Some(RawGlyph::Composite(c)) => Stored::Composite(
                c.components()
                    .map(|k| {
                        let Anchor::Offset { x, y } = k.anchor else { panic!("offsets only") };
                        (out.glyph_order[usize::from(k.glyph.to_u16())].clone(), k.transform.xx.to_f32(), x, y)
                    })
                    .collect(),
            ),
        };
        map.insert(name.clone(), kind);
    }
    map
}

fn composite(parts: &[(&str, f32, i16, i16)]) -> Stored {
    Stored::Composite(parts.iter().map(|&(b, s, x, y)| (b.to_string(), s, x, y)).collect())
}

/// The points skrifa draws for a glyph, composites resolved.
#[derive(Default)]
struct Points(Vec<(f32, f32)>);
impl OutlinePen for Points {
    fn move_to(&mut self, x: f32, y: f32) {
        self.0.push((x, y));
    }
    fn line_to(&mut self, x: f32, y: f32) {
        self.0.push((x, y));
    }
    fn quad_to(&mut self, cx: f32, cy: f32, x: f32, y: f32) {
        self.0.extend([(cx, cy), (x, y)]);
    }
    fn curve_to(&mut self, _: f32, _: f32, _: f32, _: f32, x: f32, y: f32) {
        self.0.push((x, y));
    }
    fn close(&mut self) {}
}

fn drawn(out: &Compiled, name: &str) -> Vec<(f32, f32)> {
    let font = FontRef::new(&out.bytes).unwrap();
    let gid = GlyphId::new(out.glyph_order.iter().position(|n| n == name).unwrap() as u32);
    let mut pen = Points::default();
    font.outline_glyphs().get(gid).unwrap().draw(DrawSettings::unhinted(Size::unscaled(), LocationRef::default()), &mut pen).unwrap();
    pen.0
}

#[test]
fn overlapping_and_transformed_components_stay_components_whatever_the_keep_parameters_say() {
    let default = build(&font(), ComponentOptions::default());
    let glyphs = stored(&default);
    assert_eq!(glyphs["C"], composite(&[("A", 1.0, 0, 0), ("E", 1.0, 50, -50)]));
    assert_eq!(glyphs["D"], composite(&[("A", 0.5, 0, 0)]));
    assert_eq!(glyphs["F"], composite(&[("B", 1.0, 100, 50)]), "nesting is kept by default");
    let mut f = font();
    font_param(&mut f, "Keep Overlapping Components", number(1));
    font_param(&mut f, "Keep Transformed Components", number(1));
    assert_eq!(build(&f, ComponentOptions::default()).bytes, default.bytes, "the two parameters change nothing");
}

#[test]
fn the_decompose_components_parameter_and_option_decompose_every_composite() {
    let default = build(&font(), ComponentOptions::default());
    let mut f = font();
    font_param(&mut f, DECOMPOSE_COMPONENTS, number(1));
    let by_param = build(&f, ComponentOptions::default());
    let glyphs = stored(&by_param);
    for name in ["A", "B", "C", "D", "F", "G", "H", "I"] {
        assert_eq!(glyphs[name], Stored::Simple, "{name}");
    }
    for name in ["B", "C", "D", "F", "G", "I"] {
        assert_eq!(drawn(&by_param, name), drawn(&default, name), "{name} draws as its composite did");
    }
    let by_option = build(&font(), ComponentOptions { decompose_all: true, ..Default::default() });
    assert_eq!(by_option.bytes, by_param.bytes);
    let mut off = font();
    font_param(&mut off, DECOMPOSE_COMPONENTS, number(0));
    assert_eq!(build(&off, ComponentOptions::default()).bytes, default.bytes);
}

#[test]
fn the_decompose_glyphs_parameter_decomposes_the_listed_glyphs_only() {
    let list = Value::Array(fontcraft_model::plist::Array {
        items: vec![Value::String(Str::from("B")), Value::String(Str::from("D"))],
        ..Default::default()
    });
    let mut f = font();
    font_param(&mut f, DECOMPOSE_GLYPHS, list);
    let out = build(&f, ComponentOptions::default());
    let glyphs = stored(&out);
    assert_eq!((&glyphs["B"], &glyphs["D"]), (&Stored::Simple, &Stored::Simple));
    assert_eq!(glyphs["C"], composite(&[("A", 1.0, 0, 0), ("E", 1.0, 50, -50)]));
    assert_eq!(glyphs["F"], composite(&[("B", 1.0, 100, 50)]), "a composite of a decomposed glyph keeps it as a component");
    let mut as_text = font();
    font_param(&mut as_text, DECOMPOSE_GLYPHS, Value::String(Str::from("B, D")));
    assert_eq!(build(&as_text, ComponentOptions::default()).bytes, out.bytes);
}

#[test]
fn the_decompose_components_filter_reaches_the_glyphs_it_names() {
    let mut only = font();
    master_filters(&mut only, "({name = decomposeComponents; include = (C);})");
    let glyphs = stored(&build(&only, ComponentOptions::default()));
    assert_eq!(glyphs["C"], Stored::Simple);
    assert_eq!(glyphs["B"], composite(&[("A", 1.0, 10, 0)]));

    let mut except = font();
    master_filters(&mut except, "({name = \"Decompose Components\"; exclude = (C);})");
    let glyphs = stored(&build(&except, ComponentOptions::default()));
    assert_eq!(glyphs["C"], composite(&[("A", 1.0, 0, 0), ("E", 1.0, 50, -50)]));
    assert_eq!((&glyphs["B"], &glyphs["D"], &glyphs["F"]), (&Stored::Simple, &Stored::Simple, &Stored::Simple));
}

#[test]
fn transformed_components_are_decomposed_when_the_filter_or_the_option_asks() {
    let default = stored(&build(&font(), ComponentOptions::default()));
    let mut f = font();
    master_filters(&mut f, "({name = decomposeTransformedComponents;})");
    let glyphs = stored(&build(&f, ComponentOptions::default()));
    assert_eq!(glyphs["D"], Stored::Simple);
    assert_eq!(glyphs["B"], default["B"], "a moved component is not a transformed one");
    assert_eq!(glyphs["C"], default["C"]);
    assert_eq!(glyphs["G"], composite(&[("B", 1.0, 0, 0), ("D", 1.0, 0, 0)]), "G's own components are not transformed");

    let off = stored(&build(&f, ComponentOptions { decompose_transformed: Some(false), ..Default::default() }));
    assert_eq!(off["D"], default["D"], "the option overrides the source");
    let on = stored(&build(&font(), ComponentOptions { decompose_transformed: Some(true), ..Default::default() }));
    assert_eq!(on["D"], Stored::Simple);

    // A designspace keeps the list in the font's user data.
    let mut font_level = font();
    Arc::make_mut(&mut font_level.info).user_data = with_filters("({name = decomposeTransformedComponents;})");
    assert_eq!(stored(&build(&font_level, ComponentOptions::default()))["D"], Stored::Simple);
}

#[test]
fn flattening_leaves_composites_one_level_deep_with_transforms_combined() {
    let mut f = font();
    master_filters(&mut f, "({name = flattenComponents;})");
    let glyphs = stored(&build(&f, ComponentOptions::default()));
    assert_eq!(glyphs["F"], composite(&[("A", 1.0, 110, 50)]));
    assert_eq!(glyphs["G"], composite(&[("A", 1.0, 10, 0), ("A", 0.5, 0, 0)]));
    assert_eq!(glyphs["B"], composite(&[("A", 1.0, 10, 0)]));
    assert_eq!(glyphs["I"], composite(&[("H", 1.0, 5, 0)]), "a mixed glyph is simple once written, so it stays a component");

    let by_option = build(&font(), ComponentOptions { flatten: Some(true), ..Default::default() });
    assert_eq!(stored(&by_option), glyphs);
    let off = stored(&build(&f, ComponentOptions { flatten: Some(false), ..Default::default() }));
    assert_eq!(off["F"], composite(&[("B", 1.0, 100, 50)]));
}

#[test]
fn transformed_components_are_decomposed_before_the_rest_is_flattened() {
    let mut f = font();
    master_filters(&mut f, "({name = flattenComponents;}, {name = decomposeTransformedComponents;})");
    let glyphs = stored(&build(&f, ComponentOptions::default()));
    assert_eq!(glyphs["D"], Stored::Simple);
    assert_eq!(glyphs["G"], composite(&[("A", 1.0, 10, 0), ("D", 1.0, 0, 0)]), "D is simple when G is flattened");
}

#[test]
fn malformed_filter_lists_warn_and_change_nothing() {
    let default = build(&font(), ComponentOptions::default());
    let mut not_a_list = font();
    master_filters(&mut not_a_list, "flattenComponents");
    let out = build(&not_a_list, ComponentOptions::default());
    assert_eq!(out.bytes, default.bytes);
    assert!(out.warnings.iter().any(|w| w.contains(FILTERS_KEY) && w.contains("not a list")), "{:?}", out.warnings);

    let mut both = font();
    master_filters(&mut both, "({name = decomposeComponents; include = (B); exclude = (C);})");
    let out = build(&both, ComponentOptions::default());
    assert_eq!(out.bytes, default.bytes);
    assert!(out.warnings.iter().any(|w| w.contains("both an include and an exclude")), "{:?}", out.warnings);

    let mut elsewhere = font();
    master_filters(&mut elsewhere, "({name = decomposeComponents; namespace = my.filters;}, {name = propagateAnchors;})");
    let out = build(&elsewhere, ComponentOptions::default());
    assert_eq!(out.bytes, default.bytes, "another namespace's filter and other filters are not component options");
}

/// A 100 × 100 square whose top right corner is open by `ear` units on both sides.
fn eared(ear: f64) -> Shape {
    let n = |x, y| Node::new(x, y, NodeKind::Line);
    Shape::Path(Path::new(true, vec![n(0.0, 0.0), n(100.0, 0.0), n(100.0, 100.0 + ear), n(100.0 + ear, 100.0), n(0.0, 100.0)]))
}

/// Whether the ear's tip is left in `A`, and in `B`, a composite of it 10 units to the right.
fn ears(f: &Font, options: ComponentOptions) -> (bool, bool) {
    let out = build(f, options.clone());
    let target = f.default_target().unwrap();
    let cff = crate::compile(f, &target, crate::Flavour::Cff, &Options { remove_overlap: false, components: options, ..Default::default() }).unwrap();
    let found = |out: &Compiled, name: &str, tip: (f32, f32)| drawn(out, name).contains(&tip);
    let ears = (found(&out, "A", (140.0, 100.0)), found(&out, "B", (150.0, 100.0)));
    assert_eq!(ears, (found(&cff, "A", (140.0, 100.0)), found(&cff, "B", (150.0, 100.0))), "CFF is built from the same outlines");
    ears
}

#[test]
fn invisible_open_corners_are_erased_where_the_source_asks() {
    let mut f = font();
    f.glyph_mut("A").unwrap().layers[0].shapes = vec![eared(40.0)];
    assert_eq!(ears(&f, ComponentOptions::default()), (false, false));
    assert!(drawn(&build(&f, ComponentOptions::default()), "A").contains(&(100.0, 100.0)), "the corner is one point at the crossing");
    assert_eq!(ears(&f, ComponentOptions { erase_open_corners: Some(false), ..Default::default() }), (true, true));
    // An ear larger than what stays visible next to it is part of the shape.
    let mut large = f.clone();
    large.glyph_mut("A").unwrap().layers[0].shapes = vec![eared(150.0)];
    assert!(drawn(&build(&large, ComponentOptions::default()), "A").contains(&(250.0, 100.0)));

    // A UFO keeps them unless its filter list names the filter, for the glyphs it names.
    f.source.format = fontcraft_model::SourceFormat::Ufo;
    assert_eq!(ears(&f, ComponentOptions::default()), (true, true));
    assert_eq!(ears(&f, ComponentOptions { erase_open_corners: Some(true), ..Default::default() }), (false, false));
    master_filters(&mut f, "({name = eraseOpenCorners; namespace = \"glyphsLib.filters\"; pre = 1;})");
    assert_eq!(ears(&f, ComponentOptions::default()), (false, false));
    master_filters(&mut f, "({name = eraseOpenCorners; namespace = \"glyphsLib.filters\"; pre = 1; exclude = (A);})");
    assert_eq!(ears(&f, ComponentOptions::default()), (true, true));
    // In a .glyphs source a filter list without the filter turns it off.
    f.source.format = fontcraft_model::SourceFormat::Glyphs;
    master_filters(&mut f, "({name = flattenComponents;})");
    assert!(ears(&f, ComponentOptions::default()).0);
}
