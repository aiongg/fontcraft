use crate::decompile;
use fontcraft_compile::{Options, compile_ttf};
use fontcraft_geom::Shape as _;
use fontcraft_model::{Component, Font, Path, Shape, Transform};
use skrifa::MetadataProvider;
use skrifa::outline::DrawSettings;
use skrifa::prelude::{LocationRef, Size};
use skrifa::raw::types::{F2Dot14, GlyphId};
use write_fonts::FontBuilder;
use write_fonts::tables::avar::{Avar, AxisValueMap, SegmentMaps};
use write_fonts::tables::fvar::{AxisInstanceArrays, Fvar, InstanceRecord, VariationAxisRecord};
use write_fonts::tables::gvar::{GlyphDelta, GlyphDeltas, GlyphVariations, Gvar, Tent};
use write_fonts::tables::name::{Name, NameRecord};
use write_fonts::tables::stat::{AxisRecord, AxisValue as StatValue, AxisValueTableFlags, Stat};
use write_fonts::types::{Fixed, NameId, Tag};

use super::*;

/// A, B: rectangles; C: A as a component, 50 units to the right.
fn source() -> (Font, fontcraft_compile::Compiled) {
    let mut f = Font::new("Var Test");
    for name in ["A", "B"] {
        let mut g = f.glyph_mut(name).unwrap();
        g.layers[0].shapes.push(Shape::Path(Path::rect(60.0, 0.0, 560.0, 700.0)));
        g.layers[0].width = 620.0;
    }
    let mut c = f.glyph_mut("C").unwrap();
    c.layers[0].shapes.push(Shape::Component(Component {
        glyph: "A".into(),
        transform: Transform::Matrix([1.0, 0.0, 0.0, 1.0, 50.0, 0.0]),
        info: Default::default(),
        extra: Default::default(),
    }));
    c.layers[0].width = 700.0;
    drop(c);
    let compiled = compile_ttf(&f, &Options::default()).unwrap();
    (f, compiled)
}

fn tent(peak: f64) -> Vec<Tent> {
    vec![Tent::new(write_fonts::types::F2Dot14::from_f64(peak), None)]
}

/// Deltas for a rectangle: points right of x 300 move by `dx`, points above y 350 by `dy`, the
/// advance by `advance`. Every other point is left for the reader to infer.
fn rect_deltas(points: &[(i16, i16)], dx: i16, dy: i16, advance: i16) -> Vec<GlyphDelta> {
    let mut v: Vec<GlyphDelta> = points
        .iter()
        .enumerate()
        .map(|(i, (x, y))| {
            let (ddx, ddy) = (if *x > 300 { dx } else { 0 }, if *y > 350 { dy } else { 0 });
            if i % 2 == 0 { GlyphDelta::required(ddx, ddy) } else { GlyphDelta::optional(ddx, ddy) }
        })
        .collect();
    v.extend([GlyphDelta::required(0, 0), GlyphDelta::required(advance, 0), GlyphDelta::required(0, 0), GlyphDelta::required(0, 0)]);
    v
}

/// The source compiled, then given a weight axis 300–400–700 with an `avar` point (550 → 625
/// in design units), named instances, STAT labels and these variations:
/// - A: −1 narrows it by 50, +1 widens it by 100 (advance too);
/// - B: as A, plus a region peaking at 0.5 that only B uses, raising its top by 80;
/// - C: its component moves −20 at −1 and +30 at +1; the advance follows A's.
fn variable_font() -> Vec<u8> {
    let (_, compiled) = source();
    let base = FontRef::new(&compiled.bytes).unwrap();
    let glyf = base.glyf().unwrap();
    let loca = base.loca(None).unwrap();
    let mut variations = Vec::new();
    for (gid, name) in compiled.glyph_order.iter().enumerate() {
        let id = write_fonts::types::GlyphId::new(gid as u32);
        let deltas = match (name.as_str(), loca.get(id, &glyf)) {
            ("A" | "B", Some(LocaGlyph::Glyph(RawGlyph::Simple(s)))) => {
                let points: Vec<(i16, i16)> = s.points().map(|p| (p.x, p.y)).collect();
                let mut d = vec![
                    GlyphDeltas::new(tent(-1.0), rect_deltas(&points, -50, 0, -50)),
                    GlyphDeltas::new(tent(1.0), rect_deltas(&points, 100, 0, 100)),
                ];
                if name == "B" {
                    let mid = Tent::new(
                        write_fonts::types::F2Dot14::from_f64(0.5),
                        Some((write_fonts::types::F2Dot14::from_f64(0.0), write_fonts::types::F2Dot14::from_f64(1.0))),
                    );
                    d.push(GlyphDeltas::new(vec![mid], rect_deltas(&points, 0, 80, 0)));
                }
                d
            }
            ("C", Some(LocaGlyph::Glyph(RawGlyph::Composite(_)))) => {
                let phantoms =
                    |adv| [GlyphDelta::required(0, 0), GlyphDelta::required(adv, 0), GlyphDelta::required(0, 0), GlyphDelta::required(0, 0)];
                let mut low = vec![GlyphDelta::required(-20, 0)];
                low.extend(phantoms(-50));
                let mut high = vec![GlyphDelta::required(30, 0)];
                high.extend(phantoms(100));
                vec![GlyphDeltas::new(tent(-1.0), low), GlyphDeltas::new(tent(1.0), high)]
            }
            ("C", _) => panic!("C compiles to a composite glyph"),
            _ => Vec::new(),
        };
        variations.push(GlyphVariations::new(id, deltas));
    }
    let gvar = Gvar::new(variations, 1).unwrap();
    let wght = Tag::new(b"wght");
    let instance = |name: u16, user: f64| InstanceRecord {
        subfamily_name_id: NameId::new(name),
        flags: 0,
        coordinates: vec![Fixed::from_f64(user)],
        post_script_name_id: None,
    };
    let fvar = Fvar::new(AxisInstanceArrays::new(
        vec![VariationAxisRecord::new(wght, Fixed::from_f64(300.0), Fixed::from_f64(400.0), Fixed::from_f64(700.0), 0, NameId::new(256))],
        vec![instance(258, 300.0), instance(2, 400.0), instance(259, 550.0), instance(257, 700.0)],
    ));
    let f2 = write_fonts::types::F2Dot14::from_f64;
    let avar = Avar::new(vec![SegmentMaps::new(vec![
        AxisValueMap::new(f2(-1.0), f2(-1.0)),
        AxisValueMap::new(f2(0.0), f2(0.0)),
        AxisValueMap::new(f2(0.5), f2(0.75)),
        AxisValueMap::new(f2(1.0), f2(1.0)),
    ])]);
    let record = |id: u16, s: &str| NameRecord::new(3, 1, 0x409, NameId::new(id), write_fonts::OffsetMarker::new(s.to_string()));
    let name = Name::new(vec![
        record(1, "Var Test"),
        record(2, "Regular"),
        record(256, "Weight"),
        record(257, "Bold"),
        record(258, "Light"),
        record(259, "Medium"),
    ]);
    let stat = Stat::new(
        vec![AxisRecord::new(wght, NameId::new(256), 0)],
        vec![
            StatValue::format_1(0, AxisValueTableFlags::empty(), NameId::new(257), Fixed::from_f64(700.0)),
            StatValue::format_3(0, AxisValueTableFlags::ELIDABLE_AXIS_VALUE_NAME, NameId::new(2), Fixed::from_f64(400.0), Fixed::from_f64(700.0)),
        ],
        NameId::new(2),
    );
    let mut builder = FontBuilder::new();
    builder.add_table(&fvar).unwrap().add_table(&avar).unwrap().add_table(&gvar).unwrap().add_table(&name).unwrap().add_table(&stat).unwrap();
    builder.copy_missing_tables(base);
    builder.build()
}

/// The bounds skrifa draws a glyph with at `coords`, and its advance there.
fn drawn(bytes: &[u8], gid: u32, coords: &[F2Dot14]) -> (Option<fontcraft_geom::Rect>, f64) {
    let font = FontRef::new(bytes).unwrap();
    let id = GlyphId::new(gid);
    let mut pen = crate::CubicPen::default();
    if let Some(outline) = font.outline_glyphs().get(id) {
        outline.draw(DrawSettings::unhinted(Size::unscaled(), LocationRef::new(coords)), &mut pen).unwrap();
    }
    let bounds = pen.finish().iter().map(|p| p.to_bez().unwrap().bounding_box()).reduce(|a, b| a.union(b));
    let advance = font.glyph_metrics(Size::unscaled(), LocationRef::new(coords)).advance_width(id).unwrap();
    (bounds, f64::from(advance).round())
}

fn assert_near(a: Option<fontcraft_geom::Rect>, b: Option<fontcraft_geom::Rect>, what: &str) {
    match (a, b) {
        (None, None) => {}
        (Some(a), Some(b)) => {
            let far = [(a.x0, b.x0), (a.y0, b.y0), (a.x1, b.x1), (a.y1, b.y1)].iter().any(|(p, q)| (p - q).abs() > 1.0);
            assert!(!far, "{what}: {a:?} vs {b:?}");
        }
        _ => panic!("{what}: {a:?} vs {b:?}"),
    }
}

fn bounds_of(b: fontcraft_geom::BezPath) -> Option<fontcraft_geom::Rect> {
    (!b.elements().is_empty()).then(|| b.bounding_box())
}

#[test]
fn a_variable_font_opens_into_masters_and_intermediate_layers() {
    let bytes = variable_font();
    let d = decompile(&bytes, None).unwrap();
    assert!(!d.warnings.iter().any(|w| w.starts_with(DEFAULT_ONLY)), "{:?}", d.warnings);
    assert!(!d.warnings.iter().any(|w| w.contains("gvar") || w.contains("fvar")), "{:?}", d.warnings);
    let f = d.font;

    // The axis, its user range and its STAT labels.
    assert_eq!(f.axes.len(), 1);
    let axis = &f.axes[0];
    assert_eq!((axis.tag.as_str(), axis.name.as_str(), axis.hidden), ("wght", "Weight", false));
    assert_eq!(axis.range, Some(AxisRange { minimum: 300.0, default: 400.0, maximum: 700.0 }));
    let labels: Vec<_> = axis.labels.iter().map(|l| (l.name.as_str(), l.user_value, l.elidable, l.linked_user_value)).collect();
    assert_eq!(labels, vec![("Bold", 700.0, false, None), ("Regular", 400.0, true, Some(700.0))]);
    assert_eq!(axis.ordering, Some(0));

    // The avar map as the Axis Mappings parameter: user 550 sits at design 625.
    assert_eq!(fontcraft_model::axes::mapping_parameter(&f, "wght"), Some(vec![(300.0, 300.0), (400.0, 400.0), (550.0, 625.0), (700.0, 700.0)]));

    // The default master first and named as the origin, then one master per axis end.
    let masters: Vec<_> = f.masters.iter().map(|m| (m.id.as_str(), m.name.as_str(), m.location.clone())).collect();
    assert_eq!(masters, vec![("m01", "Regular", vec![400.0]), ("m02", "Light", vec![300.0]), ("m03", "Bold", vec![700.0])]);
    assert_eq!(f.info.params.get(fontcraft_model::axes::ORIGIN).and_then(Value::as_str), Some("m01"));
    let instances: Vec<_> = f.instances.iter().map(|i| (i.name.as_str(), i.location.clone())).collect();
    assert_eq!(instances, vec![("Light", vec![300.0]), ("Regular", vec![400.0]), ("Medium", vec![625.0]), ("Bold", vec![700.0])]);

    // Master layers, with the inferred points and advances varied.
    let a = f.glyph("A").unwrap();
    assert_eq!(a.layers.iter().map(|l| (l.id.as_str(), l.width)).collect::<Vec<_>>(), vec![("m01", 620.0), ("m02", 570.0), ("m03", 720.0)]);
    let right = |l: &Layer| match l.shapes.first() {
        Some(Shape::Path(p)) => p.nodes.iter().map(|n| n.x).fold(f64::MIN, f64::max),
        _ => f64::NAN,
    };
    assert_eq!((right(&a.layers[1]), right(&a.layers[2])), (510.0, 660.0));
    let c = f.glyph("C").unwrap();
    let Some(Shape::Component(comp)) = c.layers[2].shapes.first() else { panic!("C keeps its component: {:?}", c.layers[2].shapes) };
    assert_eq!((comp.glyph.as_str(), comp.transform, c.layers[2].width), ("A", Transform::Matrix([1.0, 0.0, 0.0, 1.0, 80.0, 0.0]), 800.0));

    // The region only B uses becomes an intermediate layer of B at design 550.
    assert_eq!(a.layers.len(), 3);
    let b = f.glyph("B").unwrap();
    assert_eq!(b.layers.len(), 4);
    let brace = &b.layers[3];
    assert_eq!(brace.info.attrs.coordinates, vec![Some(550.0)]);
    // Equally near the default and the Bold master: the first, the default.
    assert_eq!(brace.master, "m01");
    assert_eq!(brace.width, 670.0);

    // Every layer draws as the font does at its location.
    let order = {
        let font = FontRef::new(&bytes).unwrap();
        (0..f.glyphs.len() as u32).map(|g| font.glyph_names().get(GlyphId::new(g)).map(|n| n.as_str().to_string())).collect::<Vec<_>>()
    };
    let at = |bits: i16| vec![F2Dot14::from_bits(bits)];
    for (gid, g) in f.glyphs.iter().enumerate() {
        assert_eq!(order.get(gid).cloned().flatten().as_deref(), Some(g.name.as_str()));
        for (master, bits) in [("m01", 0), ("m02", -16384), ("m03", 16384)] {
            let (bounds, advance) = drawn(&bytes, gid as u32, &at(bits));
            assert_near(bounds_of(f.outline(&g.name, master).unwrap()), bounds, &format!("{} {master}", g.name));
            assert_eq!(g.layers.iter().find(|l| l.id == master).unwrap().width, advance, "{} {master}", g.name);
        }
    }
    let gid_b = f.glyphs.position("B").unwrap() as u32;
    let (bounds, advance) = drawn(&bytes, gid_b, &at(8192));
    let Some(Shape::Path(p)) = brace.shapes.first() else { panic!("a path") };
    assert_near(Some(p.to_bez().unwrap().bounding_box()), bounds, "B at 0.5");
    assert_eq!(brace.width, advance);
}

#[test]
fn damaged_variable_fonts_are_errors_not_panics() {
    let good = variable_font();
    for i in (0..good.len()).step_by(7) {
        let mut bad = good.clone();
        bad[i] = bad[i].wrapping_add(97);
        let _ = decompile(&bad, None);
        let _ = open_variable(&bad, None);
    }
    for cut in (0..good.len()).step_by(53) {
        let _ = open_variable(good.get(..cut).unwrap_or_default(), None);
    }
}

#[test]
fn a_static_font_opens_as_the_static_reader_opens_it() {
    let (_, compiled) = source();
    let a = open_variable(&compiled.bytes, None).unwrap().font;
    let b = decompile(&compiled.bytes, None).unwrap().font;
    assert_eq!(a, b);
    assert_eq!(a.masters.len(), 1);
}

/// The measure of this package: compile, open, compile again, and the two fonts draw the same
/// at every master and in between.
#[test]
#[ignore = "needs the variable compile (M9.5)"]
fn a_reopened_variable_font_compiles_to_the_same_outlines_and_advances() {
    let bytes = variable_font();
    let opened = decompile(&bytes, None).unwrap().font;
    let again = fontcraft_compile::var::compile_variable(&opened, None, &Options::default(), &Default::default()).unwrap().compiled;
    let one = FontRef::new(&bytes).unwrap();
    let two = FontRef::new(&again.bytes).unwrap();
    let names_one: Vec<String> =
        (0..one.maxp().unwrap().num_glyphs() as u32).map(|g| one.glyph_names().get(GlyphId::new(g)).unwrap().as_str().to_string()).collect();
    for user in [300.0, 350.0, 400.0, 475.0, 550.0, 625.0, 700.0] {
        let loc_one = one.axes().location([("wght", user)]);
        let loc_two = two.axes().location([("wght", user)]);
        for (gid, name) in names_one.iter().enumerate() {
            let Some(gid_two) = again.glyph_order.iter().position(|n| n == name) else { panic!("{name} is compiled again") };
            let (b1, a1) = drawn(&bytes, gid as u32, loc_one.coords());
            let (b2, a2) = drawn(&again.bytes, gid_two as u32, loc_two.coords());
            assert_near(b1, b2, &format!("{name} at {user}"));
            assert_eq!(a1, a2, "{name} at {user}");
        }
    }
}
