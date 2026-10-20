//! VARC in a subset. [`font`] builds the same glyph order, glyph records, conditions and store as
//! the fontTools script the expected values come from; they are pyftsubset's on that font, with
//! the fontTools of the corpus (4.66.2.dev0, which reads the current VARC layout). The output is
//! read back with read-fonts, so the layout is checked by a second reader.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::path::{Path as FsPath, PathBuf};

use fontcraft_compile::{Flavour, Options, compile};
use fontcraft_model::{Font, Path, Shape};
use read_fonts::tables::varc::{Condition, Varc};
use read_fonts::{FontData, FontRead};

use super::super::{SubsetOptions, SubsetResult, subset};
use crate::reorder::glyph_names;
use crate::sfnt::{SfntFont, read};

const NO: u32 = 0xFFFF_FFFF;

fn var32(v: u32) -> Vec<u8> {
    let mut out = Vec::new();
    super::put_u32var(&mut out, v);
    out
}

/// One run of 16-bit values.
fn packed(values: &[i16]) -> Vec<u8> {
    let mut out = vec![0x40 | (values.len() as u8 - 1)];
    values.iter().for_each(|v| out.extend(v.to_be_bytes()));
    out
}

fn index2(items: &[Vec<u8>]) -> Vec<u8> {
    let mut out = (items.len() as u32).to_be_bytes().to_vec();
    if items.is_empty() {
        return out;
    }
    out.push(2);
    let mut offset = 1u16;
    out.extend(offset.to_be_bytes());
    for item in items {
        offset += item.len() as u16;
        out.extend(offset.to_be_bytes());
    }
    items.iter().for_each(|i| out.extend(i));
    out
}

fn u24(v: usize) -> [u8; 3] {
    let b = (v as u32).to_be_bytes();
    [b[1], b[2], b[3]]
}

/// A component with translate x and y when `transform` is given.
fn component(
    gid: u16,
    cond: Option<u32>,
    axes: Option<(u32, &[i16])>,
    avar: Option<u32>,
    tvar: Option<u32>,
    transform: Option<(i16, i16)>,
) -> Vec<u8> {
    let mut flags = 0u32;
    let mut body = gid.to_be_bytes().to_vec();
    if let Some(c) = cond {
        flags |= 1 << 7;
        body.extend(var32(c));
    }
    if let Some((i, values)) = axes {
        flags |= 1 << 1;
        body.extend(var32(i));
        body.extend(packed(values));
    }
    if let Some(v) = avar {
        flags |= 1 << 2;
        body.extend(var32(v));
    }
    if let Some(v) = tvar {
        flags |= 1 << 3;
        body.extend(var32(v));
    }
    if let Some((x, y)) = transform {
        flags |= (1 << 4) | (1 << 5);
        body.extend(x.to_be_bytes());
        body.extend(y.to_be_bytes());
    }
    let mut out = var32(flags);
    out.extend(body);
    out
}

fn axis_condition(axis: u16, min: i16, max: i16) -> Vec<u8> {
    [1u16, axis, min as u16, max as u16].iter().flat_map(|v| v.to_be_bytes()).collect()
}

/// The condition list: 0 = axis 0 in 0.5..1, 1 = (value of delta set 0x2) and not (axis 1 in 0..1).
fn conditions() -> Vec<u8> {
    let c0 = axis_condition(0, 8192, 16384);
    let mut not = vec![0, 5];
    not.extend(u24(5));
    not.extend(axis_condition(1, 0, 16384));
    let mut value = vec![0, 2, 0, 0];
    value.extend(2u32.to_be_bytes());
    let mut and = vec![0, 3, 2];
    and.extend(u24(9));
    and.extend(u24(9 + value.len()));
    and.extend(value);
    and.extend(not);
    condition_list(&[c0, and])
}

fn condition_list(conditions: &[Vec<u8>]) -> Vec<u8> {
    let mut out = (conditions.len() as u32).to_be_bytes().to_vec();
    let mut at = 4 + 4 * conditions.len();
    for c in conditions {
        out.extend((at as u32).to_be_bytes());
        at += c.len();
    }
    conditions.iter().for_each(|c| out.extend(c));
    out
}

/// Regions 0 (axis 0), 1 (axis 1) and 2 (both); delta set list 0 on regions 0 and 1 with three
/// delta sets, list 1 on region 2 with one.
fn store() -> Vec<u8> {
    let axis = |i: u16| -> Vec<u8> { [i, 0, 16384, 16384].iter().flat_map(|v| v.to_be_bytes()).collect() };
    let region = |axes: &[u16]| -> Vec<u8> {
        let mut out = (axes.len() as u16).to_be_bytes().to_vec();
        let mut at = 2 + 4 * axes.len();
        for _ in axes {
            out.extend((at as u32).to_be_bytes());
            at += 8;
        }
        axes.iter().for_each(|a| out.extend(axis(*a)));
        out
    };
    let regions = [region(&[0]), region(&[1]), region(&[0, 1])];
    let mut list = 3u32.to_be_bytes().to_vec();
    let mut at = 4 + 12;
    for r in &regions {
        list.extend((at as u32).to_be_bytes());
        at += r.len();
    }
    regions.iter().for_each(|r| list.extend(r));
    let data = |regions: &[u16], items: &[&[i16]]| -> Vec<u8> {
        let mut out = vec![1];
        out.extend((regions.len() as u16).to_be_bytes());
        regions.iter().for_each(|r| out.extend(r.to_be_bytes()));
        out.extend((out.len() as u32 + 4).to_be_bytes());
        out.extend(index2(&items.iter().map(|i| packed(i)).collect::<Vec<_>>()));
        out
    };
    let d0 = data(&[0, 1], &[&[1, 2], &[3, 4, 5, 6], &[7, 8]]);
    let d1 = data(&[2], &[&[9, 10]]);
    let head = 2 + 4 + 2 + 8;
    let mut out = vec![0, 1];
    out.extend((head as u32).to_be_bytes());
    out.extend(2u16.to_be_bytes());
    out.extend(((head + list.len()) as u32).to_be_bytes());
    out.extend(((head + list.len() + d0.len()) as u32).to_be_bytes());
    out.extend(list);
    out.extend(d0);
    out.extend(d1);
    out
}

/// A VARC table from its parts; `None` parts get a null offset.
fn table(coverage: &[u16], store: Option<Vec<u8>>, conditions: Option<Vec<u8>>, axis_indices: Option<Vec<u8>>, glyphs: Vec<u8>) -> Vec<u8> {
    let mut cov = vec![0, 1];
    cov.extend((coverage.len() as u16).to_be_bytes());
    coverage.iter().for_each(|g| cov.extend(g.to_be_bytes()));
    let parts = [Some(cov), store, conditions, axis_indices, Some(glyphs)];
    let mut out = 0x0001_0000u32.to_be_bytes().to_vec();
    let mut at = 24;
    for p in &parts {
        match p {
            Some(p) => {
                out.extend((at as u32).to_be_bytes());
                at += p.len();
            }
            None => out.extend(0u32.to_be_bytes()),
        }
    }
    parts.iter().flatten().for_each(|p| out.extend(p));
    out
}

/// `.notdef`, `A` (B moved by delta sets and E), `B`, `C`, `D` (C under a condition) and `E`
/// (no code point); B, C and E are rectangles.
fn font() -> SfntFont {
    let mut f = Font::new("Varc Subset");
    f.glyphs.retain(|g| g.name == ".notdef");
    for (i, (name, cps)) in [("A", vec![0x41]), ("B", vec![0x42]), ("C", vec![0x43]), ("D", vec![0x44]), ("E", vec![])].into_iter().enumerate() {
        f.add_glyph(name, cps, 500.0).unwrap();
        if matches!(name, "B" | "C" | "E") {
            let x = 10.0 * (i + 1) as f64;
            f.glyph_mut(name).unwrap().layers[0].shapes.push(Shape::Path(Path::rect(x, 0.0, x + 200.0, 700.0)));
        }
    }
    let out = compile(&f, &f.default_target().unwrap(), Flavour::TrueType, &Options::default()).unwrap();
    let mut font = read(&out.bytes, 0).unwrap();
    for tag in [*b"GDEF", *b"GSUB", *b"GPOS", *b"kern"] {
        font.remove(tag);
    }
    let [a, b, c, d, e] = ["A", "B", "C", "D", "E"].map(|n| gid(&font, n));
    let records = vec![
        [
            component(b, Some(1), Some((1, &[8192, -16384])), Some(0x1), Some(0x1_0000), Some((10, 20))),
            component(e, None, None, None, None, Some((-5, 0))),
        ]
        .concat(),
        component(c, Some(0), Some((0, &[16384])), Some(0), None, None),
    ];
    let axis_indices = index2(&[packed(&[0]), packed(&[0, 1])]);
    font.insert(*b"VARC", table(&[a, d], Some(store()), Some(conditions()), Some(axis_indices), index2(&records)));
    font
}

fn gid(font: &SfntFont, name: &str) -> u16 {
    glyph_names(font).unwrap().iter().position(|n| n == name).unwrap() as u16
}

fn run(font: &SfntFont, options: SubsetOptions) -> SubsetResult {
    subset(font, &options).unwrap()
}

fn text(t: &str) -> SubsetOptions {
    SubsetOptions { text: t.into(), ..SubsetOptions::default() }
}

#[derive(Debug, PartialEq)]
struct Comp {
    gid: u32,
    condition: Option<u32>,
    axes: Option<u32>,
    values: Vec<i32>,
    axis_var: u32,
    transform_var: u32,
    translate: (f32, f32),
}

fn comp(gid: u32, condition: Option<u32>, axes: Option<u32>, values: &[i32], axis_var: u32, transform_var: u32, translate: (f32, f32)) -> Comp {
    Comp { gid, condition, axes, values: values.to_vec(), axis_var, transform_var, translate }
}

/// The VARC table of `font` as read-fonts reads it.
#[derive(Debug, PartialEq)]
struct Described {
    coverage: Vec<u16>,
    records: Vec<Vec<Comp>>,
    axis_indices: Vec<Vec<i32>>,
    conditions: Vec<String>,
    /// Region indices and delta sets of each delta set list.
    store: Vec<(Vec<u16>, Vec<Vec<i32>>)>,
    /// The axes of each region.
    regions: Vec<Vec<u16>>,
}

fn describe_condition(c: &Condition<'_>) -> String {
    let list = |items: Vec<Condition<'_>>| items.iter().map(describe_condition).collect::<Vec<_>>().join(", ");
    match c {
        Condition::Format1AxisRange(c) => {
            format!("axis {} {} {}", c.axis_index(), c.filter_range_min_value().to_f32(), c.filter_range_max_value().to_f32())
        }
        Condition::Format2VariableValue(c) => format!("value {} {:#x}", c.default_value(), c.var_index()),
        Condition::Format3And(c) => format!("and({})", list(c.conditions().iter().map(Result::unwrap).collect())),
        Condition::Format4Or(c) => format!("or({})", list(c.conditions().iter().map(Result::unwrap).collect())),
        Condition::Format5Negate(c) => format!("not({})", describe_condition(&c.condition().unwrap())),
    }
}

fn describe(font: &SfntFont) -> Option<Described> {
    let varc = Varc::read(FontData::new(font.get(*b"VARC")?)).unwrap();
    let coverage: Vec<u16> = varc.coverage().unwrap().iter().map(|g| g.to_u16()).collect();
    let records = (0..coverage.len())
        .map(|i| {
            varc.glyph(i)
                .unwrap()
                .components()
                .map(|c| {
                    let c = c.unwrap();
                    Comp {
                        gid: c.gid().to_u32(),
                        condition: c.condition_index(),
                        axes: c.axis_indices_index(),
                        values: c.axis_values().map(|v| v.iter().collect()).unwrap_or_default(),
                        axis_var: c.axis_values_var_index().unwrap_or(NO),
                        transform_var: c.transform_var_index().unwrap_or(NO),
                        translate: (c.transform().translate_x(), c.transform().translate_y()),
                    }
                })
                .collect()
        })
        .collect();
    let axis_indices = match varc.axis_indices_list() {
        Some(list) => (0..list.unwrap().count() as usize).map(|i| varc.axis_indices(i).unwrap().iter().collect()).collect(),
        None => Vec::new(),
    };
    let conditions = match varc.condition_list() {
        Some(list) => list.unwrap().conditions().iter().map(|c| describe_condition(&c.unwrap())).collect(),
        None => Vec::new(),
    };
    let (mut store, mut regions) = (Vec::new(), Vec::new());
    if let Some(s) = varc.multi_var_store() {
        let s = s.unwrap();
        for data in s.variation_data().iter() {
            let data = data.unwrap();
            let count = data.delta_sets().unwrap().count() as usize;
            store.push((
                data.region_indices().iter().map(|r| r.get()).collect(),
                (0..count).map(|i| data.delta_set(i).unwrap().iter().collect()).collect(),
            ));
        }
        for r in s.region_list().unwrap().regions().iter() {
            regions.push(r.unwrap().axis_coordinates().iter().map(|a| a.unwrap().axis_index()).collect());
        }
    }
    Some(Described { coverage, records, axis_indices, conditions, store, regions })
}

fn new(r: &SubsetResult, font: &SfntFont, name: &str) -> u32 {
    r.glyph_map[&u32::from(gid(font, name))]
}

#[test]
fn varc_subset_keeps_the_used_records_conditions_axis_indices_and_delta_sets() {
    let font = font();
    let r = run(&font, text("A"));
    assert_eq!(r.retained, [".notdef", "A", "B", "E"], "E has no code point: only A's record reaches it");
    let (a, b, e) = (new(&r, &font, "A"), new(&r, &font, "B"), new(&r, &font, "E"));
    assert_eq!(
        describe(&r.font).unwrap(),
        Described {
            coverage: vec![a as u16],
            records: vec![vec![
                comp(b, Some(0), Some(0), &[8192, -16384], 0x0, 0x1_0000, (10.0, 20.0)),
                comp(e, None, None, &[], NO, NO, (-5.0, 0.0))
            ]],
            axis_indices: vec![vec![0, 1]],
            conditions: vec!["and(value 0 0x1, not(axis 1 0 1))".into()],
            store: vec![(vec![0, 1], vec![vec![3, 4, 5, 6], vec![7, 8]]), (vec![2], vec![vec![9, 10]])],
            regions: vec![vec![0], vec![1], vec![0, 1]],
        }
    );

    // D drops the second delta set list and the region only it used.
    let r = run(&font, text("D"));
    assert_eq!(r.retained, [".notdef", "C", "D"]);
    let (c, d) = (new(&r, &font, "C"), new(&r, &font, "D"));
    let expected = Described {
        coverage: vec![d as u16],
        records: vec![vec![comp(c, Some(0), Some(0), &[16384], 0x0, NO, (0.0, 0.0))]],
        axis_indices: vec![vec![0]],
        conditions: vec!["axis 0 0.5 1".into()],
        store: vec![(vec![0, 1], vec![vec![1, 2]])],
        regions: vec![vec![0], vec![1]],
    };
    assert_eq!(describe(&r.font).unwrap(), expected);

    // Under retain_gids the records keep the input's glyph ids.
    let r = run(&font, SubsetOptions { retain_gids: true, ..text("D") });
    let (c, d) = (u32::from(gid(&font, "C")), gid(&font, "D"));
    let described = describe(&r.font).unwrap();
    assert_eq!(described.coverage, [d]);
    assert_eq!(described.records[0][0].gid, c);
    assert_eq!(described.store, expected.store);

    // No kept glyph has a record: the table goes.
    let r = run(&font, text("B"));
    assert!(r.font.get(*b"VARC").is_none());
    assert!(r.dropped.iter().any(|t| &t.tag == b"VARC" && t.reason == "empty after subsetting"), "{:?}", r.dropped);
}

#[test]
fn damaged_varc_tables_are_refused_without_panicking() {
    let font = font();
    let varc = font.get(*b"VARC").unwrap().to_vec();
    for cut in 0..varc.len() {
        let mut damaged = font.clone();
        damaged.insert(*b"VARC", varc[..cut].to_vec());
        let _ = subset(&damaged, &text("A"));
    }

    // NOT conditions nested past the limit.
    let mut nested = axis_condition(0, 0, 16384);
    for _ in 0..=super::MAX_CONDITION_DEPTH {
        let mut not = vec![0, 5];
        not.extend(u24(5));
        not.extend(nested);
        nested = not;
    }
    let mut deep = font.clone();
    let (a, b) = (gid(&font, "A"), gid(&font, "B"));
    let record = component(b, Some(0), None, None, None, None);
    deep.insert(*b"VARC", table(&[a], None, Some(condition_list(&[nested])), None, index2(&[record])));
    assert!(subset(&deep, &text("A")).is_err());

    // A component naming a delta set the store does not have.
    let mut dangling = font.clone();
    let record = component(b, None, None, None, Some(0x5_0000), None);
    dangling.insert(*b"VARC", table(&[a], Some(store()), None, None, index2(&[record])));
    assert!(subset(&dangling, &text("A")).is_err());
}

fn corpus() -> PathBuf {
    std::env::var_os("FONTCRAFT_CORPUS").map_or_else(|| FsPath::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus"), PathBuf::from)
}

/// The VARC test fonts of the fontTools corpus, subset as pyftsubset subsets them (the counts of
/// the first two cases are fontTools' own `test_varComposite`).
#[test]
fn corpus_varc_fonts_subset_as_pyftsubset_does() {
    let dir = corpus().join("fonttools/Tests/ttLib/data");
    if !dir.is_dir() {
        eprintln!("skipped: no corpus at {}", dir.display());
        return;
    }
    let open = |name: &str| read(&std::fs::read(dir.join(name)).unwrap(), 0).unwrap();
    let unicodes = |u: u32| SubsetOptions { unicodes: vec![u], ..SubsetOptions::default() };
    let gids = |g: u32| SubsetOptions { gids: vec![g], ..SubsetOptions::default() };

    let ac = open("varc-ac00-ac01.ttf");
    let r = run(&ac, unicodes(0xAC00));
    assert_eq!(r.retained.len(), 6);
    let v = describe(&r.font).unwrap();
    assert_eq!(v.coverage, [1, 2, 4]);
    let gids_of = |v: &Described| {
        v.records.iter().map(|r| r.iter().map(|c| (c.gid, c.axes, c.axis_var, c.transform_var)).collect::<Vec<_>>()).collect::<Vec<_>>()
    };
    assert_eq!(gids_of(&v), [vec![(2, None, NO, NO), (4, None, NO, NO)], vec![(3, Some(0), 0, 1)], vec![(5, Some(1), NO, NO)]]);
    assert_eq!(v.axis_indices, [vec![2, 3, 4], vec![4]]);
    assert_eq!(v.store.iter().map(|(r, items)| (r.clone(), items.len())).collect::<Vec<_>>(), [(vec![0, 1, 2], 2)]);
    assert_eq!(v.regions.len(), 3);

    let r = run(&ac, unicodes(0xAC01));
    assert_eq!(r.retained.len(), 8);
    let v = describe(&r.font).unwrap();
    assert_eq!(v.axis_indices.len(), 2);
    assert_eq!(v.store.iter().map(|(_, items)| items.len()).collect::<Vec<_>>(), [5]);
    assert_eq!(v.regions.len(), 3);

    // Only the record that does not use the condition: the condition list goes.
    let cond = open("varc-ac01-conditional.ttf");
    let r = run(&cond, gids(6));
    assert_eq!(r.retained.len(), 3);
    let v = describe(&r.font).unwrap();
    assert_eq!((v.coverage.clone(), gids_of(&v)), (vec![1], vec![vec![(2, Some(0), 0, 1)]]));
    assert!(v.conditions.is_empty());
    assert_eq!(v.axis_indices, [vec![2, 3, 4]]);
    assert_eq!(v.store.iter().map(|(r, items)| (r.clone(), items.len())).collect::<Vec<_>>(), [(vec![0, 1, 2], 2)]);
    let r = run(&cond, unicodes(0xAC01));
    assert_eq!(describe(&r.font).unwrap().conditions, ["axis 0 0.5 1"]);

    // One glyph of 6868: its delta set list keeps five delta sets and ten of 39 regions,
    // renumbered in their old order.
    let cjk = open("varc-6868.ttf");
    let r = run(&cjk, gids(5));
    assert_eq!(r.retained.len(), 4);
    let v = describe(&r.font).unwrap();
    assert_eq!((v.coverage.clone(), gids_of(&v)), (vec![2], vec![vec![(3, Some(0), NO, 0), (1, Some(1), 1, 2), (1, Some(2), 3, 4)]]));
    assert_eq!(v.axis_indices, [vec![2], vec![2, 5, 6, 10, 11, 13, 14], vec![2, 3, 6, 8, 10, 11, 12, 14]]);
    assert_eq!(v.store.iter().map(|(r, items)| (r.clone(), items.len())).collect::<Vec<_>>(), [(vec![0, 1, 7, 2, 8, 3, 9, 4, 5, 6], 5)]);
    assert_eq!(v.regions.len(), 10);
}
