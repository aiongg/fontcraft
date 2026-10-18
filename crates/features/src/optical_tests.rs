//! Behaviour tests of the optical bounds: offsets and guides give the `lfbd` and `rtbd` rules,
//! which compile to the single adjustments read back from `GPOS`.

use std::sync::Arc;

use fontcraft_fea::{CompileRequest, GlyphOrder, VirtualFiles};
use fontcraft_model::plist::{Dict, Number, Str, Value};
use fontcraft_model::{Axis, Font, Guide, Layer, LayerInfo, LayerInfoBox, Master, Path, Shape, Snippet};
use read_fonts::tables::gpos::{Gpos, PositionLookup, SinglePos};
use read_fonts::{FontData, FontRead};

use super::*;

fn offset(left: Value, right: Value) -> Value {
    let mut dict = Dict::new();
    dict.insert("left", left);
    dict.insert("right", right);
    Value::Dict(dict)
}

fn int(v: i64) -> Value {
    Value::Number(Number::int(v))
}

fn vertical(name: &str, x: f64) -> Guide {
    Guide { x, angle: 90.0, name: Some(name.into()), ..Default::default() }
}

/// Draws a box in `glyph`'s layer of `master`, with `guides`.
fn draw(f: &mut Font, glyph: &str, master: &str, guides: Vec<Guide>) {
    let mut g = f.glyph_mut(glyph).unwrap();
    if g.layer(master).is_none() {
        g.layers.push(Layer::new(master, 600.0));
    }
    let layer = g.layer_mut(master).unwrap();
    layer.shapes.push(Shape::Path(Path::rect(50.0, 0.0, 550.0, 700.0)));
    layer.info = LayerInfoBox::new(LayerInfo { guides, ..Default::default() });
}

/// `H`, `I` and `O` drawn, `O` with an `ol` guide at 35 and an `or` guide at 560 (width 600);
/// the master's offsets 20 and 15 (the right one written as a string).
fn font() -> Font {
    let mut f = Font::new("Bounds");
    Arc::make_mut(&mut f.masters)[0].params.set(OPTICAL_OFFSET, offset(int(20), Value::String(Str::new("15"))));
    draw(&mut f, "H", "m01", Vec::new());
    draw(&mut f, "I", "m01", Vec::new());
    draw(&mut f, "O", "m01", vec![vertical(LEFT_GUIDE, 35.0), vertical(RIGHT_GUIDE, 560.0)]);
    f
}

const ORDER: &[&str] = &[".notdef", "space", "A", "H", "I", "O"];

fn glyphs() -> Vec<(&'static str, &'static str)> {
    ORDER.iter().map(|n| (*n, *n)).collect()
}

fn code(entries: &[Snippet], tag: &str) -> String {
    entries.iter().find(|e| e.name == tag).map(|e| e.code.clone()).unwrap_or_default()
}

/// The (glyph id, x placement, x advance) of every glyph the single adjustments of `tag` move.
fn single_positions(gpos: &[u8], tag: &[u8; 4]) -> Vec<(u16, i16, i16)> {
    let table = Gpos::read(FontData::new(gpos)).unwrap();
    let features = table.feature_list().unwrap();
    let lookups = table.lookup_list().unwrap();
    let mut out = Vec::new();
    for record in features.feature_records().iter().filter(|r| r.feature_tag() == read_fonts::types::Tag::new(tag)) {
        let feature = record.feature(features.offset_data()).unwrap();
        for index in feature.lookup_list_indices() {
            let PositionLookup::Single(lookup) = lookups.lookups().get(usize::from(index.get())).unwrap() else {
                panic!("a single adjustment lookup");
            };
            for subtable in lookup.subtables().iter() {
                match subtable.unwrap() {
                    SinglePos::Format1(t) => {
                        let v = t.value_record();
                        for gid in t.coverage().unwrap().iter() {
                            out.push((gid.to_u16(), v.x_placement().unwrap_or(0), v.x_advance().unwrap_or(0)));
                        }
                    }
                    SinglePos::Format2(t) => {
                        let records = t.value_records();
                        for (i, gid) in t.coverage().unwrap().iter().enumerate() {
                            let v = records.get(i).unwrap();
                            out.push((gid.to_u16(), v.x_placement().unwrap_or(0), v.x_advance().unwrap_or(0)));
                        }
                    }
                }
            }
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

#[test]
fn optical_bounds_from_offsets_and_guides_compile_to_lfbd_and_rtbd() {
    let f = font();
    let mut warnings = Vec::new();
    let entries = bounds_features(&f, &glyphs(), BoundsTarget::Master("m01"), &mut warnings);
    assert!(warnings.is_empty(), "{warnings:?}");
    // Every glyph with an outline (`.notdef` too) takes the global offset; the empty `space`
    // and `A` take none; the guides override it for `O`.
    assert_eq!(code(&entries, LEFT_FEATURE), "pos .notdef <-20 0 -20 0>;\npos H <-20 0 -20 0>;\npos I <-20 0 -20 0>;\npos O <-35 0 -35 0>;\n");
    assert_eq!(code(&entries, RIGHT_FEATURE), "pos .notdef <0 0 -15 0>;\npos H <0 0 -15 0>;\npos I <0 0 -15 0>;\npos O <0 0 -40 0>;\n");
    assert!(entries.iter().all(|e| e.automatic));

    // Read back from the compiled tables.
    let text: String = entries.iter().map(|e| format!("feature {0} {{\n{1}}} {0};\n", e.name, e.code)).collect();
    let files = VirtualFiles::new("features.fea", &format!("languagesystem DFLT dflt;\n{text}")).unwrap();
    let order = GlyphOrder::new(ORDER.iter().map(|g| g.to_string()).collect()).unwrap();
    let compiled = fontcraft_fea::compile(&CompileRequest::new(&files, &order)).unwrap_or_else(|d| panic!("{text}\n{d:?}"));
    let gpos = compiled.gpos.unwrap();
    assert_eq!(single_positions(&gpos, b"lfbd"), vec![(0, -20, -20), (3, -20, -20), (4, -20, -20), (5, -35, -35)]);
    assert_eq!(single_positions(&gpos, b"rtbd"), vec![(0, 0, -15), (3, 0, -15), (4, 0, -15), (5, 0, -40)]);
}

#[test]
fn a_guide_only_glyph_and_a_slanted_guide_give_bounds() {
    let mut f = Font::new("Bounds");
    // No offsets: only the guides count. A guide at 80° through (100, 500) crosses the baseline
    // at 100 − 500·cot 80° ≈ 11.8; a horizontal guide named `or` is not read.
    let slanted = Guide { x: 100.0, y: 500.0, angle: 80.0, name: Some(LEFT_GUIDE.into()), ..Default::default() };
    let flat = Guide { x: 300.0, y: 200.0, angle: 0.0, name: Some(RIGHT_GUIDE.into()), ..Default::default() };
    {
        let mut space = f.glyph_mut("space").unwrap();
        space.layers[0].info = LayerInfoBox::new(LayerInfo { guides: vec![slanted, flat], ..Default::default() });
    }
    draw(&mut f, "H", "m01", Vec::new());
    let mut warnings = Vec::new();
    let entries = bounds_features(&f, &glyphs(), BoundsTarget::Master("m01"), &mut warnings);
    assert_eq!(code(&entries, LEFT_FEATURE), "pos space <-12 0 -12 0>;\n");
    assert!(code(&entries, RIGHT_FEATURE).is_empty(), "{entries:?}");

    // Neither offsets nor guides: no entry.
    let plain = Font::new("Plain");
    assert!(bounds_features(&plain, &glyphs(), BoundsTarget::Master("m01"), &mut warnings).is_empty());

    // A bound beyond a value record's range leaves the glyph out with a warning.
    let mut wide = font();
    Arc::make_mut(&mut wide.masters)[0].params.set(OPTICAL_OFFSET, offset(int(40_000), int(0)));
    let entries = bounds_features(&wide, &glyphs(), BoundsTarget::Master("m01"), &mut warnings);
    assert_eq!(code(&entries, LEFT_FEATURE), "pos O <-35 0 -35 0>;\n");
    assert!(warnings.iter().any(|w| w.contains("optical bounds of H")), "{warnings:?}");
}

#[test]
fn variable_font_bounds_that_differ_between_masters_are_variable_values() {
    let mut f = font();
    f.axes = Arc::new(vec![Axis { name: "Weight".into(), tag: "wght".into(), ..Default::default() }]);
    {
        let masters = Arc::make_mut(&mut f.masters);
        masters[0].location = vec![100.0];
        masters[0].params.set(OPTICAL_OFFSET, offset(int(20), int(15)));
        let mut bold = Master { id: "m02".into(), name: "Bold".into(), location: vec![900.0], ..Default::default() };
        bold.params.set(OPTICAL_OFFSET, offset(int(30), int(15)));
        masters.push(bold);
    }
    draw(&mut f, "H", "m02", Vec::new());
    let mut warnings = Vec::new();
    let entries = bounds_features(&f, &[("H", "H")], BoundsTarget::Variable, &mut warnings);
    assert!(warnings.is_empty(), "{warnings:?}");
    assert_eq!(code(&entries, LEFT_FEATURE), "pos H <(wght=100:-20 wght=900:-30) 0 (wght=100:-20 wght=900:-30) 0>;\n");
    assert_eq!(code(&entries, RIGHT_FEATURE), "pos H <0 0 -15 0>;\n");
}

#[test]
fn code_that_writes_a_bounds_feature_keeps_it() {
    let generated = bounds_features(&font(), &glyphs(), BoundsTarget::Master("m01"), &mut Vec::new());
    // A disabled hand-written lfbd entry still wins; a UFO prefix with an rtbd block too.
    let mut entries = Features::default();
    entries.features.push(Snippet { name: LEFT_FEATURE.into(), code: "pos H <-5 0 -5 0>;".into(), disabled: true, ..Default::default() });
    entries.prefixes.push(Snippet { code: "feature rtbd { pos H <0 0 -5 0>; } rtbd;".into(), ..Default::default() });
    add_missing(&mut entries, generated.clone());
    assert_eq!(entries.features.len(), 1);

    let mut empty = Features::default();
    add_missing(&mut empty, generated);
    let names: Vec<&str> = empty.features.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, [LEFT_FEATURE, RIGHT_FEATURE]);
}
