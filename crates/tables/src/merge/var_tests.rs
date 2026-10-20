//! Variable fonts merged as variable fonts, on two weight fonts compiled in code. The rule the
//! tests hold the merge to: instancing the merged font at a location gives what merging the
//! instances of the inputs at that location gives.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::collections::BTreeMap;
use std::sync::Arc;

use fontcraft_compile::Options;
use fontcraft_compile::var::{VariableOptions, compile_variable};
use fontcraft_model::{Axis, Component, Font, Glyph, GlyphTable, Instance, Layer, Master, Path, Shape};
use read_fonts::tables::glyf::Glyph as RGlyph;
use read_fonts::types::{F2Dot14, GlyphId};
use read_fonts::{FontData, FontRead, FontRef, TableProvider};
use write_fonts::from_obj::ToOwnedTable;
use write_fonts::tables::gpos::{Gpos, PairPos, PositionLookup};

use super::layout::{class_entries, coverage_glyphs};
use super::*;
use crate::instance::{AxisLimit, InstanceOptions, instantiate, named_instances};
use crate::reorder::glyph_names;
use crate::sfnt::{WriteOptions, read, write};

const WGHT: Tag = *b"wght";

fn layer(master: &str, width: f64, shapes: Vec<Shape>) -> Layer {
    let mut l = Layer::new(master, width);
    l.id = master.into();
    l.shapes = shapes;
    l
}

fn rect(x: f64, w: f64) -> Shape {
    Shape::Path(Path::rect(x, 0.0, x + w, 700.0))
}

fn glyph(name: &str, unicode: u32, layers: Vec<Layer>) -> Arc<Glyph> {
    let mut g = Glyph::new(name);
    g.unicodes = vec![unicode];
    g.layers = layers;
    Arc::new(g)
}

/// A weight font (Thin 100, the default, to Black 900) with `glyphs`, a kerning pair whose
/// value is `kern` (Thin, Black) and named instances at `instances` (name, weight).
fn weight_font(family: &str, glyphs: Vec<Arc<Glyph>>, kern: (&str, &str, f64, f64), instances: &[(&str, f64)]) -> SfntFont {
    let mut f = Font::new(family);
    f.axes = Arc::new(vec![Axis { name: "Weight".into(), tag: "wght".into(), ..Axis::default() }]);
    f.masters = Arc::new(vec![
        Master { id: "thin".into(), name: "Thin".into(), location: vec![100.0], ..Master::default() },
        Master { id: "black".into(), name: "Black".into(), location: vec![900.0], ..Master::default() },
    ]);
    f.glyphs = GlyphTable::from_glyphs(glyphs);
    let kerning = Arc::make_mut(&mut f.kerning);
    kerning.ltr.set("thin", kern.0, kern.1, kern.2);
    kerning.ltr.set("black", kern.0, kern.1, kern.3);
    f.instances = Arc::new(
        instances.iter().map(|(name, w)| Instance { name: (*name).into(), location: vec![*w], exports: true, ..Instance::default() }).collect(),
    );
    let built = compile_variable(&f, None, &Options::default(), &VariableOptions::default()).unwrap();
    read(&built.compiled.bytes, 0).unwrap()
}

/// `a` (a bar 100 → 300 wide), `o` (two bars) and `d` (`a` moved by 0 → 50, a composite), `a`
/// and `o` kerned −40 → −80.
fn one() -> SfntFont {
    let glyphs = vec![
        glyph("a", 0x61, vec![layer("thin", 300.0, vec![rect(0.0, 100.0)]), layer("black", 500.0, vec![rect(0.0, 300.0)])]),
        glyph(
            "o",
            0x6F,
            vec![layer("thin", 400.0, vec![rect(0.0, 80.0), rect(200.0, 80.0)]), layer("black", 700.0, vec![rect(0.0, 200.0), rect(400.0, 200.0)])],
        ),
        glyph(
            "d",
            0x64,
            vec![
                layer("thin", 300.0, vec![Shape::Component(Component::new("a", 0.0, 0.0))]),
                layer("black", 500.0, vec![Shape::Component(Component::new("a", 50.0, 0.0))]),
            ],
        ),
    ];
    weight_font("Var One", glyphs, ("a", "o", -40.0, -80.0), &[("Thin", 100.0), ("Black", 900.0)])
}

/// `b` (a bar 120 → 320 wide) and `c`, which has an intermediate layer at 500 (so its `gvar`
/// has a tuple font one lacks), `b` and `c` kerned −20 → −100.
fn two() -> SfntFont {
    let mut brace = layer("thin", 450.0, vec![rect(0.0, 260.0)]);
    brace.id = "brace".into();
    brace.info.attrs.coordinates = vec![Some(500.0)];
    let glyphs = vec![
        glyph("b", 0x62, vec![layer("thin", 320.0, vec![rect(0.0, 120.0)]), layer("black", 520.0, vec![rect(0.0, 320.0)])]),
        glyph("c", 0x63, vec![layer("thin", 300.0, vec![rect(0.0, 100.0)]), layer("black", 500.0, vec![rect(0.0, 300.0)]), brace]),
    ];
    weight_font("Var Two", glyphs, ("b", "c", -20.0, -100.0), &[("Thin", 100.0), ("Medium", 500.0)])
}

fn at(font: &SfntFont, wght: f64) -> SfntFont {
    instantiate(font, &InstanceOptions { limits: vec![(WGHT, AxisLimit::Pin(wght))], ..InstanceOptions::default() }).unwrap()
}

/// Per glyph name: the outline (points of a simple glyph, components of a composite, by
/// component name) and the advance.
fn glyphs_of(font: &SfntFont) -> BTreeMap<String, (String, u16)> {
    let names = glyph_names(font).unwrap();
    let bytes = write(font, &WriteOptions::default()).unwrap();
    let f = FontRef::new(&bytes).unwrap();
    let (glyf, loca, hmtx) = (f.glyf().unwrap(), f.loca(None).unwrap(), f.hmtx().unwrap());
    let mut out = BTreeMap::new();
    for (g, name) in names.iter().enumerate() {
        let gid = GlyphId::new(g as u32);
        let outline = match loca.get_glyf(gid, &glyf).unwrap() {
            None => String::new(),
            Some(RGlyph::Simple(s)) => {
                let ends: Vec<u16> = s.end_pts_of_contours().iter().map(|e| e.get()).collect();
                let points: Vec<(i16, i16, bool)> = s.points().map(|p| (p.x, p.y, p.on_curve)).collect();
                format!("{ends:?} {points:?}")
            }
            Some(RGlyph::Composite(c)) => c
                .components()
                .map(|c| format!("{} {:?} {:?}", names[c.glyph.to_u32() as usize], c.anchor, c.transform))
                .collect::<Vec<_>>()
                .join("; "),
        };
        out.insert(name.clone(), (outline, hmtx.advance(gid).unwrap_or_default()));
    }
    out
}

/// Every kerning pair of the pair adjustment lookups (first value wins), by glyph names.
fn kerning(font: &SfntFont) -> BTreeMap<(String, String), i16> {
    let names = glyph_names(font).unwrap();
    let name = |g: u16| names[usize::from(g)].clone();
    let gpos: Gpos = read_fonts::tables::gpos::Gpos::read(FontData::new(font.get(*b"GPOS").unwrap())).unwrap().to_owned_table();
    let mut out = BTreeMap::new();
    for lookup in &gpos.lookup_list.lookups {
        let PositionLookup::Pair(l) = &**lookup else { continue };
        for st in &l.subtables {
            match &**st {
                PairPos::Format1(t) => {
                    for (g, set) in coverage_glyphs(&t.coverage).into_iter().zip(&t.pair_sets) {
                        for r in &set.pair_value_records {
                            out.entry((name(g.to_u16()), name(r.second_glyph.to_u16()))).or_insert(r.value_record1.x_advance.unwrap_or(0));
                        }
                    }
                }
                PairPos::Format2(t) => {
                    let (mut c1, mut c2) = (BTreeMap::new(), BTreeMap::new());
                    class_entries(&t.class_def1, &mut c1).unwrap();
                    class_entries(&t.class_def2, &mut c2).unwrap();
                    for g in coverage_glyphs(&t.coverage) {
                        let k1 = c1.get(&g.to_u16()).copied().unwrap_or(0);
                        for (g2, k2) in &c2 {
                            let v = t.class1_records[usize::from(k1)].class2_records[usize::from(*k2)].value_record1.x_advance.unwrap_or(0);
                            out.entry((name(g.to_u16()), name(*g2))).or_insert(v);
                        }
                    }
                }
            }
        }
    }
    out
}

fn pair(map: &BTreeMap<(String, String), i16>, a: &str, b: &str) -> i16 {
    map[&(a.to_string(), b.to_string())]
}

#[test]
fn instancing_the_merged_variable_font_equals_merging_the_instances() {
    let (one, two) = (one(), two());
    let report = merge_with_report(&[one.clone(), two.clone()], &MergeOptions::default()).unwrap();
    let merged = report.font;
    for tag in [b"fvar", b"gvar", b"HVAR", b"GDEF", b"GPOS"] {
        assert!(merged.get(*tag).is_some(), "the merged font has no {}", tag_str(*tag));
    }
    assert!(report.warnings.iter().all(|w| !w.contains("variable")), "{:?}", report.warnings);
    let gdef = read_fonts::tables::gdef::Gdef::read(FontData::new(merged.get(*b"GDEF").unwrap())).unwrap();
    assert!(gdef.item_var_store().is_some(), "the kerning varies through the merged GDEF store");

    for wght in [100.0, 300.0, 500.0, 650.0, 900.0] {
        let instanced = at(&merged, wght);
        let separately = merge(&[at(&one, wght), at(&two, wght)], &MergeOptions::default()).unwrap();
        assert_eq!(glyphs_of(&instanced), glyphs_of(&separately), "outlines and advances at wght {wght}");
        assert_eq!(kerning(&instanced), kerning(&separately), "kerning at wght {wght}");
    }
    // The two fonts' kerning varies by different amounts, so each font's devices must reach
    // its own deltas in the merged store.
    let (thin, black) = (kerning(&at(&merged, 100.0)), kerning(&at(&merged, 900.0)));
    assert_eq!((pair(&thin, "a", "o"), pair(&black, "a", "o")), (-40, -80));
    assert_eq!((pair(&thin, "b", "c"), pair(&black, "b", "c")), (-20, -100));
    // `c`'s intermediate layer: its own tuple survives the merged shared tuple list.
    assert_eq!(glyphs_of(&at(&merged, 500.0))["c"].1, 450);
}

#[test]
fn merged_advance_variations_follow_each_glyph() {
    let (one, two) = (one(), two());
    let merged = merge(&[one.clone(), two.clone()], &MergeOptions::default()).unwrap();
    let delta = |font: &SfntFont, name: &str, coord: f32| -> f64 {
        let gid = glyph_names(font).unwrap().iter().position(|n| n == name).unwrap();
        let bytes = write(font, &WriteOptions::default()).unwrap();
        let hvar = FontRef::new(&bytes).unwrap().hvar().unwrap();
        hvar.advance_delta(GlyphId::new(gid as u32), &[F2Dot14::from_f32(coord)]).unwrap().to_f64()
    };
    for (input, names) in [(&one, ["a", "o", "d"].as_slice()), (&two, ["b", "c"].as_slice())] {
        for name in names {
            for coord in [0.25, 1.0] {
                assert_eq!(delta(&merged, name, coord), delta(input, name, coord), "{name} at {coord}");
            }
        }
    }
}

#[test]
fn named_instances_are_combined_and_their_names_copied() {
    let merged = merge(&[one(), two()], &MergeOptions::default()).unwrap();
    let instances = named_instances(&merged).unwrap();
    let list: Vec<(Option<String>, f64)> = instances.iter().map(|i| (i.subfamily_name.clone(), i.coordinates[0].1)).collect();
    assert_eq!(
        list,
        [(Some("Thin".to_string()), 100.0), (Some("Black".to_string()), 900.0), (Some("Medium".to_string()), 500.0)],
        "font 2's Thin is at font 1's Thin coordinates, so only its Medium is added"
    );
}

#[test]
fn fonts_with_different_axes_or_a_static_font_are_refused() {
    let narrow = {
        let mut f = Font::new("Narrow");
        f.axes = Arc::new(vec![Axis { name: "Weight".into(), tag: "wght".into(), ..Axis::default() }]);
        f.masters = Arc::new(vec![
            Master { id: "thin".into(), name: "Thin".into(), location: vec![100.0], ..Master::default() },
            Master { id: "bold".into(), name: "Bold".into(), location: vec![700.0], ..Master::default() },
        ]);
        f.glyphs = GlyphTable::from_glyphs(vec![glyph(
            "x",
            0x78,
            vec![layer("thin", 300.0, vec![rect(0.0, 100.0)]), layer("bold", 500.0, vec![rect(0.0, 300.0)])],
        )]);
        read(&compile_variable(&f, None, &Options::default(), &VariableOptions::default()).unwrap().compiled.bytes, 0).unwrap()
    };
    let err = merge(&[one(), narrow.clone()], &MergeOptions::default()).unwrap_err().to_string();
    assert!(err.contains("different axes") && err.contains("100–100–700"), "{err}");

    let fixed = at(&two(), 400.0);
    let err = merge(&[one(), fixed.clone()], &MergeOptions::default()).unwrap_err().to_string();
    assert!(err.contains("font 2 is static"), "{err}");

    // Without `fvar` the fonts merge as their default outlines.
    let options = MergeOptions { drop_tables: vec![*b"fvar"], ..MergeOptions::default() };
    let report = merge_with_report(&[one(), narrow], &options).unwrap();
    assert!(report.font.get(*b"fvar").is_none() && report.font.get(*b"gvar").is_none());
    assert!(report.warnings.iter().any(|w| w.contains("is variable")), "{:?}", report.warnings);
    // The static result keeps the default kerning, without the variation store or the devices
    // that would use it.
    if let Some(gdef) = report.font.get(*b"GDEF") {
        assert!(read_fonts::tables::gdef::Gdef::read(FontData::new(gdef)).unwrap().item_var_store().is_none());
    }
    assert_eq!(pair(&kerning(&report.font), "a", "o"), pair(&kerning(&one()), "a", "o"));
    let gpos: Gpos = read_fonts::tables::gpos::Gpos::read(FontData::new(report.font.get(*b"GPOS").unwrap())).unwrap().to_owned_table();
    for lookup in &gpos.lookup_list.lookups {
        let PositionLookup::Pair(l) = &**lookup else { continue };
        for st in &l.subtables {
            let records: Vec<_> = match &**st {
                PairPos::Format1(t) => t.pair_sets.iter().flat_map(|s| s.pair_value_records.iter().map(|r| &r.value_record1)).collect(),
                PairPos::Format2(t) => t.class1_records.iter().flat_map(|c| c.class2_records.iter().map(|r| &r.value_record1)).collect(),
            };
            assert!(records.iter().all(|r| r.x_advance_device.as_ref().is_none()), "a kerning value keeps its device");
        }
    }
}

/// `font` with `count` named instances, each with its own subfamily name ID (from 1000) and name
/// "I<n>", at weights 100 + n / 100 for n from `first`.
fn with_instances(font: &SfntFont, first: u32, count: u16) -> SfntFont {
    use write_fonts::tables::fvar::{Fvar, InstanceRecord};
    use write_fonts::tables::name::{Name, NameRecord};
    use write_fonts::types::{Fixed, NameId};
    let mut fvar: Fvar = read_fonts::tables::fvar::Fvar::read(FontData::new(font.get(*b"fvar").unwrap())).unwrap().to_owned_table();
    let mut name: Name = read_fonts::tables::name::Name::read(FontData::new(font.get(*b"name").unwrap())).unwrap().to_owned_table();
    fvar.axis_instance_arrays.instances = (0..count)
        .map(|k| InstanceRecord {
            subfamily_name_id: NameId::new(1000 + k),
            flags: 0,
            coordinates: vec![Fixed::from_f64(100.0 + f64::from(first + u32::from(k)) / 100.0)],
            post_script_name_id: None,
        })
        .collect();
    for k in 0..count {
        name.name_record.push(NameRecord::new(3, 1, 0x409, NameId::new(1000 + k), format!("I{}", first + u32::from(k)).into()));
    }
    name.name_record.sort();
    let mut out = font.clone();
    out.insert(*b"fvar", write_fonts::dump_table(&fvar).unwrap());
    out.insert(*b"name", write_fonts::dump_table(&name).unwrap());
    out
}

#[test]
fn many_named_instances_merge_without_rescanning_the_names() {
    // 12,000 instances in each font, every name of the second copied: rebuilding the name
    // index for each copy would compare hundreds of millions of records.
    let count = 12_000u16;
    let merged = merge(&[with_instances(&one(), 0, count), with_instances(&two(), 20_000, count)], &MergeOptions::default()).unwrap();
    let fvar = read_fonts::tables::fvar::Fvar::read(FontData::new(merged.get(*b"fvar").unwrap())).unwrap();
    let instances = fvar.instances().unwrap();
    assert_eq!(instances.len(), 2 * usize::from(count));
    let last = instances.get(instances.len() - 1).unwrap().subfamily_name_id;
    let name = read_fonts::tables::name::Name::read(FontData::new(merged.get(*b"name").unwrap())).unwrap();
    let strings: Vec<String> =
        name.name_record().iter().filter(|r| r.name_id() == last).map(|r| r.string(name.string_data()).unwrap().to_string()).collect();
    assert_eq!(strings, [format!("I{}", 20_000 + u32::from(count) - 1)]);

    // An instance at coordinates the first font already names is left out.
    let merged = merge(&[with_instances(&one(), 0, 3), with_instances(&two(), 1, 3)], &MergeOptions::default()).unwrap();
    assert_eq!(named_instances(&merged).unwrap().len(), 4);
}
