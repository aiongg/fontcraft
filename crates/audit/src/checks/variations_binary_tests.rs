//! Behaviour tests of the binary `check.variations.*` checks on variable fonts built byte by byte.

use crate::{BinaryCtx, CHECKS, CheckGroup, CheckScope, CheckSpec, Config, LayerReport, NoServices, Severity, audit_binary, find};

#[derive(Clone)]
struct Axis {
    tag: [u8; 4],
    min: f64,
    default: f64,
    max: f64,
    name: u16,
}

#[derive(Clone)]
struct Inst {
    name: u16,
    ps: Option<u16>,
    coords: Vec<f64>,
}

#[derive(Clone)]
enum Val {
    One { axis: u16, flags: u16, name: u16, value: f64 },
    Linked { axis: u16, flags: u16, name: u16, value: f64, linked: f64 },
    Combo { flags: u16, name: u16, records: Vec<(u16, f64)> },
}

#[derive(Clone)]
struct StatSpec {
    /// Tag, name ID, ordering.
    axes: Vec<([u8; 4], u16, u16)>,
    values: Vec<Val>,
}

/// One gvar tuple of the glyph H: its peak (one value per axis, missing ones 0) and the deltas
/// of all eight points (four outline points, four phantom points).
#[derive(Clone)]
struct Tuple {
    peak: Vec<f64>,
    x: [i16; 8],
    y: [i16; 8],
}

/// A variable font with two glyphs (.notdef and H) and the tables the checks read.
#[derive(Clone)]
struct Spec {
    fvar: bool,
    axes: Vec<Axis>,
    instances: Vec<Inst>,
    names: Vec<(u16, String)>,
    stat: Option<StatSpec>,
    weight_class: u16,
    fs_selection: u16,
    avar: bool,
    hvar: bool,
    tuples: Vec<Tuple>,
    /// A kerning pair H H whose advance adjustment varies along this axis (index) by this delta.
    kern: Option<(usize, i8)>,
    /// Raw tables that replace or add to the built ones.
    raw: Vec<([u8; 4], Vec<u8>)>,
}

const ELIDABLE: u16 = 2;

impl Spec {
    fn clean() -> Self {
        let names = [
            (1, "Test"),
            (2, "Regular"),
            (4, "Test Regular"),
            (6, "Test-Regular"),
            (256, "Weight"),
            (257, "Thin"),
            (258, "Regular"),
            (259, "Bold"),
            (260, "Black"),
            (261, "ExtraLight"),
            (262, "Light"),
            (263, "Medium"),
            (264, "SemiBold"),
            (265, "ExtraBold"),
        ];
        let weights = [(100.0, 257), (200.0, 261), (300.0, 262), (400.0, 258), (500.0, 263), (600.0, 264), (700.0, 259), (800.0, 265), (900.0, 260)];
        Spec {
            fvar: true,
            axes: vec![Axis { tag: *b"wght", min: 100.0, default: 400.0, max: 900.0, name: 256 }],
            instances: [(257, 100.0), (258, 400.0), (259, 700.0), (260, 900.0)]
                .into_iter()
                .map(|(name, w)| Inst { name, ps: None, coords: vec![w] })
                .collect(),
            names: names.into_iter().map(|(id, s)| (id, s.to_string())).collect(),
            stat: Some(StatSpec {
                axes: vec![(*b"wght", 256, 0)],
                values: weights
                    .into_iter()
                    .map(|(value, name)| Val::One { axis: 0, flags: if value == 400.0 { ELIDABLE } else { 0 }, name, value })
                    .collect(),
            }),
            weight_class: 400,
            fs_selection: 0x0040,
            avar: true,
            hvar: true,
            tuples: vec![Tuple { peak: vec![1.0], x: [0, 0, 50, 50, 0, 0, 0, 0], y: [0; 8] }],
            kern: None,
            raw: Vec::new(),
        }
    }

    /// Adds an axis; every instance takes its default.
    fn add_axis(&mut self, tag: &[u8; 4], min: f64, default: f64, max: f64) {
        self.axes.push(Axis { tag: *tag, min, default, max, name: 256 });
        for inst in &mut self.instances {
            inst.coords.push(default);
        }
    }

    fn set_name(&mut self, id: u16, text: &str) {
        self.names.retain(|(i, _)| *i != id);
        self.names.push((id, text.to_string()));
    }

    fn instance(&mut self, name: u16) -> &mut Inst {
        self.instances.iter_mut().find(|i| i.name == name).unwrap()
    }

    fn stat(&mut self) -> &mut StatSpec {
        self.stat.as_mut().unwrap()
    }

    fn build(&self) -> Vec<u8> {
        let mut tables: Vec<([u8; 4], Vec<u8>)> = vec![
            (*b"name", self.name()),
            (*b"OS/2", self.os2()),
            (*b"head", head()),
            (*b"hhea", hhea()),
            (*b"hmtx", hmtx()),
            (*b"maxp", maxp()),
            (*b"loca", loca()),
            (*b"glyf", glyf()),
            (*b"cmap", cmap()),
        ];
        if self.fvar {
            tables.push((*b"fvar", self.fvar_table()));
            if !self.tuples.is_empty() {
                tables.push((*b"gvar", self.gvar()));
            }
            if self.avar {
                tables.push((*b"avar", self.avar_table()));
            }
            if self.hvar {
                tables.push((*b"HVAR", self.hvar_table()));
            }
            if let Some((axis, delta)) = self.kern {
                tables.push((*b"GDEF", self.gdef(axis, delta)));
                tables.push((*b"GPOS", gpos()));
            }
        }
        if let Some(stat) = &self.stat {
            tables.push((*b"STAT", stat_table(stat)));
        }
        for (tag, data) in &self.raw {
            tables.retain(|(t, _)| t != tag);
            tables.push((*tag, data.clone()));
        }
        sfnt(tables)
    }

    fn fvar_table(&self) -> Vec<u8> {
        let mut w = W::default();
        let has_ps = self.instances.iter().any(|i| i.ps.is_some());
        let n = self.axes.len() as u16;
        w.u16(1).u16(0).u16(16).u16(2).u16(n).u16(20).u16(self.instances.len() as u16);
        w.u16(4 + 4 * n + if has_ps { 2 } else { 0 });
        for a in &self.axes {
            w.tag(&a.tag).fixed(a.min).fixed(a.default).fixed(a.max).u16(0).u16(a.name);
        }
        for i in &self.instances {
            w.u16(i.name).u16(0);
            for c in &i.coords {
                w.fixed(*c);
            }
            if has_ps {
                w.u16(i.ps.unwrap_or(0xFFFF));
            }
        }
        w.0
    }

    fn name(&self) -> Vec<u8> {
        let mut w = W::default();
        let n = self.names.len() as u16;
        w.u16(0).u16(n).u16(6 + 12 * n);
        let mut strings = Vec::new();
        for (id, text) in &self.names {
            let bytes: Vec<u8> = text.encode_utf16().flat_map(u16::to_be_bytes).collect();
            w.u16(3).u16(1).u16(0x409).u16(*id).u16(bytes.len() as u16).u16(strings.len() as u16);
            strings.extend(bytes);
        }
        w.0.extend(strings);
        w.0
    }

    fn os2(&self) -> Vec<u8> {
        let mut t = vec![0u8; 96];
        t[0..2].copy_from_slice(&4u16.to_be_bytes());
        t[4..6].copy_from_slice(&self.weight_class.to_be_bytes());
        t[6..8].copy_from_slice(&5u16.to_be_bytes());
        t[62..64].copy_from_slice(&self.fs_selection.to_be_bytes());
        t
    }

    fn avar_table(&self) -> Vec<u8> {
        let mut w = W::default();
        w.u16(1).u16(0).u16(0).u16(self.axes.len() as u16);
        for _ in &self.axes {
            w.u16(3).f2(-1.0).f2(-1.0).f2(0.0).f2(0.0).f2(1.0).f2(1.0);
        }
        w.0
    }

    fn hvar_table(&self) -> Vec<u8> {
        let mut w = W::default();
        // Header, then an item variation store with no regions and one zero row per glyph.
        w.u16(1).u16(0).u32(20).u32(0).u32(0).u32(0);
        w.u16(1).u32(12).u16(1).u32(16);
        w.u16(self.axes.len() as u16).u16(0);
        w.u16(2).u16(0).u16(0);
        w.0
    }

    fn gvar(&self) -> Vec<u8> {
        let axes = self.axes.len();
        let mut data = W::default();
        let n = self.tuples.len() as u16;
        data.u16(n).u16(4 + n * (4 + 2 * axes as u16));
        let mut serialized = W::default();
        for t in &self.tuples {
            let mut one = W::default();
            one.0.push(0);
            for deltas in [&t.x, &t.y] {
                one.0.push(0x40 | 7);
                for d in deltas.iter() {
                    one.i16(*d);
                }
            }
            data.u16(one.0.len() as u16).u16(0x8000 | 0x2000);
            for i in 0..axes {
                data.f2(t.peak.get(i).copied().unwrap_or(0.0));
            }
            serialized.0.extend(one.0);
        }
        data.0.extend(serialized.0);
        if data.0.len() % 2 == 1 {
            data.0.push(0);
        }
        let header = 20 + 4 * 3;
        let mut w = W::default();
        w.u16(1).u16(0).u16(axes as u16).u16(0).u32(header).u16(2).u16(1).u32(header);
        w.u32(0).u32(0).u32(data.0.len() as u32);
        w.0.extend(data.0);
        w.0
    }

    fn gdef(&self, axis: usize, delta: i8) -> Vec<u8> {
        let axes = self.axes.len();
        let mut w = W::default();
        w.u16(1).u16(3).u16(0).u16(0).u16(0).u16(0).u16(0).u32(18);
        let regions = 12;
        let data = regions + 4 + 6 * axes as u32;
        w.u16(1).u32(regions).u16(1).u32(data);
        w.u16(axes as u16).u16(1);
        for i in 0..axes {
            let peak = if i == axis { 1.0 } else { 0.0 };
            w.f2(0.0).f2(peak).f2(peak);
        }
        w.u16(1).u16(0).u16(1).u16(0);
        w.0.push(delta as u8);
        w.0
    }
}

fn gpos() -> Vec<u8> {
    let mut w = W::default();
    w.u16(1).u16(0).u16(10).u16(12).u16(14);
    w.u16(0);
    w.u16(0);
    w.u16(1).u16(4);
    w.u16(2).u16(0).u16(1).u16(8);
    // PairPosFormat1: H followed by H, x advance -20 with a variation index.
    w.u16(1).u16(12).u16(0x0044).u16(0).u16(1).u16(18);
    w.u16(1).u16(1).u16(1);
    w.u16(1).u16(1).i16(-20).u16(8);
    w.u16(0).u16(0).u16(0x8000);
    w.0
}

fn stat_table(stat: &StatSpec) -> Vec<u8> {
    let mut w = W::default();
    let axes = stat.axes.len() as u32;
    let count = stat.values.len() as u16;
    let values_at = if count == 0 { 0 } else { 20 + 8 * axes };
    w.u16(1).u16(1).u16(8).u16(axes as u16).u32(20).u16(count).u32(values_at).u16(2);
    for (tag, name, ordering) in &stat.axes {
        w.tag(tag).u16(*name).u16(*ordering);
    }
    let mut bodies = W::default();
    let mut offsets = Vec::new();
    for v in &stat.values {
        offsets.push(2 * count + bodies.0.len() as u16);
        match v {
            Val::One { axis, flags, name, value } => {
                bodies.u16(1).u16(*axis).u16(*flags).u16(*name).fixed(*value);
            }
            Val::Linked { axis, flags, name, value, linked } => {
                bodies.u16(3).u16(*axis).u16(*flags).u16(*name).fixed(*value).fixed(*linked);
            }
            Val::Combo { flags, name, records } => {
                bodies.u16(4).u16(records.len() as u16).u16(*flags).u16(*name);
                for (axis, value) in records {
                    bodies.u16(*axis).fixed(*value);
                }
            }
        }
    }
    for o in offsets {
        w.u16(o);
    }
    w.0.extend(bodies.0);
    w.0
}

fn head() -> Vec<u8> {
    let mut t = vec![0u8; 54];
    t[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    t[12..16].copy_from_slice(&0x5F0F_3CF5u32.to_be_bytes());
    t[18..20].copy_from_slice(&1000u16.to_be_bytes());
    t[40..42].copy_from_slice(&200i16.to_be_bytes());
    t[42..44].copy_from_slice(&700i16.to_be_bytes());
    t[48..50].copy_from_slice(&2i16.to_be_bytes());
    t[50..52].copy_from_slice(&1i16.to_be_bytes());
    t
}

fn hhea() -> Vec<u8> {
    let mut t = vec![0u8; 36];
    t[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    t[4..6].copy_from_slice(&800i16.to_be_bytes());
    t[6..8].copy_from_slice(&(-200i16).to_be_bytes());
    t[10..12].copy_from_slice(&500u16.to_be_bytes());
    t[18..20].copy_from_slice(&1i16.to_be_bytes());
    t[34..36].copy_from_slice(&2u16.to_be_bytes());
    t
}

fn hmtx() -> Vec<u8> {
    let mut w = W::default();
    w.u16(500).i16(0).u16(300).i16(100);
    w.0
}

fn maxp() -> Vec<u8> {
    let mut w = W::default();
    w.u32(0x0000_5000).u16(2);
    w.0
}

/// H: one rectangle from (100, 0) to (200, 700), coordinates as 16-bit deltas.
fn glyf() -> Vec<u8> {
    let mut w = W::default();
    w.i16(1).i16(100).i16(0).i16(200).i16(700).u16(3).u16(0);
    w.0.extend([1, 1, 1, 1]);
    for x in [100, 0, 100, 0] {
        w.i16(x);
    }
    for y in [0, 700, 0, -700] {
        w.i16(y);
    }
    w.0.extend([0, 0]);
    w.0
}

fn loca() -> Vec<u8> {
    let mut w = W::default();
    w.u32(0).u32(0).u32(glyf().len() as u32);
    w.0
}

/// One format 4 subtable mapping H to glyph 1.
fn cmap() -> Vec<u8> {
    let mut w = W::default();
    w.u16(0).u16(1).u16(3).u16(1).u32(12);
    w.u16(4).u16(32).u16(0).u16(4).u16(4).u16(1).u16(0);
    w.u16(0x48).u16(0xFFFF).u16(0);
    w.u16(0x48).u16(0xFFFF);
    w.i16(1 - 0x48).i16(1);
    w.u16(0).u16(0);
    w.0
}

fn sfnt(mut tables: Vec<([u8; 4], Vec<u8>)>) -> Vec<u8> {
    tables.sort_by_key(|a| a.0);
    let mut w = W::default();
    w.u32(0x0001_0000).u16(tables.len() as u16).u16(0).u16(0).u16(0);
    let start = 12 + 16 * tables.len();
    let mut body: Vec<u8> = Vec::new();
    for (tag, data) in &tables {
        w.tag(tag).u32(0).u32((start + body.len()) as u32).u32(data.len() as u32);
        body.extend(data);
        while !body.len().is_multiple_of(4) {
            body.push(0);
        }
    }
    w.0.extend(body);
    w.0
}

#[derive(Default)]
struct W(Vec<u8>);

impl W {
    fn u16(&mut self, v: u16) -> &mut Self {
        self.0.extend(v.to_be_bytes());
        self
    }
    fn i16(&mut self, v: i16) -> &mut Self {
        self.0.extend(v.to_be_bytes());
        self
    }
    fn u32(&mut self, v: u32) -> &mut Self {
        self.0.extend(v.to_be_bytes());
        self
    }
    fn tag(&mut self, t: &[u8; 4]) -> &mut Self {
        self.0.extend(t);
        self
    }
    fn fixed(&mut self, v: f64) -> &mut Self {
        self.0.extend(((v * 65536.0).round() as i32).to_be_bytes());
        self
    }
    fn f2(&mut self, v: f64) -> &mut Self {
        self.0.extend(((v * 16384.0).round() as i16).to_be_bytes());
        self
    }
}

fn variation_checks() -> Vec<&'static CheckSpec> {
    CHECKS.iter().filter(|c| c.group == CheckGroup::Variations && c.scope == CheckScope::Binary).collect()
}

fn run(bytes: &[u8], checks: &[&'static CheckSpec]) -> LayerReport {
    let services = NoServices;
    let ctx = BinaryCtx::new("f0", bytes, 0, &services).unwrap();
    audit_binary(&ctx, checks, &Config::default())
}

fn run_one(spec: &Spec, id: &str) -> LayerReport {
    let check = find(id).unwrap_or_else(|| panic!("no check {id}"));
    run(&spec.build(), &[check])
}

fn codes(r: &LayerReport) -> Vec<&'static str> {
    r.findings.iter().map(|f| f.code).collect()
}

#[test]
fn the_variation_table_checks_are_registered() {
    let checks = variation_checks();
    assert_eq!(checks.len(), 24);
    for c in checks {
        assert!(c.preconditions.iter().all(|p| matches!(p, crate::Precondition::Variable | crate::Precondition::Table(_))), "{}", c.id);
    }
}

#[test]
fn a_clean_variable_font_passes_every_variation_check() {
    let r = run(&Spec::clean().build(), &variation_checks());
    let serious: Vec<_> = r.findings.iter().filter(|f| f.severity != Severity::Info).collect();
    assert!(serious.is_empty(), "{serious:#?}");
    let skipped: Vec<&str> = r.skipped.iter().map(|s| s.check).collect();
    assert_eq!(skipped, vec!["check.variations.duplexedAxisReflow"], "{:?}", r.skipped);
    let info = codes(&r);
    assert_eq!(info, vec!["stat-axis-order"], "STAT's axis order is reported as information");
}

#[test]
fn a_static_font_skips_every_variation_check() {
    let mut spec = Spec::clean();
    spec.fvar = false;
    spec.stat = None;
    let r = run(&spec.build(), &variation_checks());
    assert!(r.findings.is_empty(), "{:#?}", r.findings);
    assert_eq!(r.skipped.len(), 24);
}

type Fault = fn(&mut Spec);

fn case(check: &'static str, code: &'static str, fault: Fault) -> (&'static str, &'static str, Fault) {
    (check, code, fault)
}

#[test]
fn each_variation_check_reports_its_fault() {
    let cases: Vec<(&str, &str, Fault)> = vec![
        case("axisRanges", "wght-out-of-range", |s| s.instance(260).coords[0] = 1001.0),
        case("axisRanges", "invalid-ital-range", |s| s.add_axis(b"ital", 0.0, 0.0, 2.0)),
        case("axisRanges", "unusual-slnt-range", |s| s.add_axis(b"slnt", 0.0, 0.0, 12.0)),
        case("regularInstanceCoordinates", "wght-not-400", |s| s.instance(258).coords[0] = 450.0),
        case("regularInstanceCoordinates", "no-regular-instance", |s| s.set_name(258, "Book")),
        case("boldInstanceCoordinate", "no-bold-instance", |s| s.set_name(259, "Heavy")),
        case("boldInstanceCoordinate", "wght-not-700", |s| s.instance(259).coords[0] = 690.0),
        case("foundryAxisTags", "invalid-foundry-defined-tag-first-letter", |s| s.add_axis(b"xOPQ", 0.0, 0.0, 100.0)),
        case("foundryAxisTags", "invalid-foundry-defined-tag-chars", |s| s.add_axis(b"Xopq", 0.0, 0.0, 100.0)),
        case("foundryAxisTags", "foundry-defined-similar-registered-name", |s| s.add_axis(b"WDTH", 50.0, 100.0, 100.0)),
        case("unsupportedItalAxis", "unsupported-ital", |s| s.add_axis(b"ital", 0.0, 0.0, 1.0)),
        case("axisHasNoVariation", "axis-has-no-variation", |s| s.add_axis(b"GRAD", 0.0, 0.0, 0.0)),
        case("axisOrder", "axes-incorrect-order", |s| {
            s.stat = None;
            s.add_axis(b"wdth", 75.0, 100.0, 100.0);
        }),
        case("axisOrder", "custom-axes", |s| {
            s.stat = None;
            s.add_axis(b"XOPQ", 0.0, 0.0, 100.0);
        }),
        case("instanceRecords", "different-size-instance-records", |s| s.instance(258).ps = Some(6)),
        case("instanceRecords", "repeated-instance-record", |s| {
            let bold = s.instance(259).clone();
            s.instances.push(bold);
        }),
        case("duplicateInstanceNames", "duplicate-instance-names", |s| s.instance(260).name = 259),
        case("duplicateInstanceNames", "name-record-not-found", |s| s.instance(260).name = 300),
        case("instancesInOrder", "instances-not-in-order", |s| s.instances.swap(2, 3)),
        case("instanceNameIds", "invalid-subfamily-nameid", |s| s.instance(260).name = 100),
        case("instanceNameIds", "missing-name", |s| s.instance(260).name = 300),
        case("instanceNameIds", "invalid-axis-nameid", |s| s.axes[0].name = 3),
        case("instanceNameIds", "invalid-postscript-nameid", |s| s.instance(259).ps = Some(7)),
        case("defaultInstanceNames", "invalid-default-instance-subfamily-name", |s| s.set_name(258, "Book")),
        case("defaultInstanceNames", "invalid-default-instance-postscript-name", |s| s.instance(258).ps = Some(270)),
        case("weightClassMatchesDefault", "bad-weight-class", |s| s.weight_class = 500),
        case("statAxisRecords", "missing-axis-records", |s| s.add_axis(b"wdth", 75.0, 100.0, 100.0)),
        case("statItalAxis", "missing-ital-axis", |s| s.fs_selection = 0x0001),
        case("statItalAxis", "no-stat", |s| {
            s.fs_selection = 0x0001;
            s.stat = None;
        }),
        case("statItalAxis", "wrong-ital-axis-linkedvalue", |s| {
            s.stat().axes.push((*b"ital", 256, 1));
            s.stat().values.push(Val::One { axis: 1, flags: ELIDABLE, name: 258, value: 0.0 });
        }),
        case("statItalicNaming", "bad-italic", |s| s.set_name(257, "Thin Italic")),
        case("statCoversInstances", "missing-STAT-table", |s| s.stat = None),
        case("statCoversInstances", "ambiguous-axis-value", |s| s.stat().values.push(Val::One { axis: 0, flags: 0, name: 259, value: 700.0 })),
        case("statCoversInstances", "missing-fvar-instance-axis-value", |s| s.instance(259).coords[0] = 650.0),
        case("statValueQuality", "duplicate-axis-value", |s| s.stat().values.push(Val::One { axis: 0, flags: 0, name: 259, value: 700.0 })),
        case("statValueQuality", "regular-not-elidable", |s| {
            for v in &mut s.stat().values {
                if let Val::One { flags, value, .. } = v
                    && *value == 400.0
                {
                    *flags = 0;
                }
            }
        }),
        case("statValueQuality", "missing-axis-value", |s| s.stat().values.retain(|v| !matches!(v, Val::One { value, .. } if *value == 200.0))),
        case("statValueQuality", "axis-order-invalid", |s| s.stat().axes.push((*b"opsz", 256, 1))),
        case("statValueQuality", "no-axis-values", |s| s.stat().values.clear()),
        case("avarMissing", "missing-avar", |s| s.avar = false),
        case("hvarMissing", "lacks-HVAR", |s| s.hvar = false),
        case("duplexedAxisReflow", "axis-causes-reflow", |s| {
            s.add_axis(b"GRAD", -200.0, 0.0, 150.0);
            s.tuples.push(Tuple { peak: vec![0.0, 1.0], x: [0, 0, 0, 0, 0, 20, 0, 0], y: [0; 8] });
        }),
        case("duplexedAxisReflow", "kern-causes-reflow", |s| {
            s.add_axis(b"GRAD", -200.0, 0.0, 150.0);
            s.kern = Some((1, 20));
        }),
        case("staticInstantiation", "unreadable-table", |s| s.raw.push((*b"HVAR", vec![0, 1]))),
        case("customAxisRange", "invalid-range", |s| s.add_axis(b"ytlc", 0.0, 500.0, 1200.0)),
    ];
    for (check, code, fault) in cases {
        let id = format!("check.variations.{check}");
        let clean = run_one(&Spec::clean(), &id);
        assert!(!codes(&clean).contains(&code), "{id} reports {code} on the clean font");
        let mut spec = Spec::clean();
        fault(&mut spec);
        let r = run_one(&spec, &id);
        assert!(r.skipped.is_empty(), "{id} skipped on its fault: {:?}", r.skipped);
        assert!(codes(&r).contains(&code), "{id} does not report {code}: {:#?}", r.findings);
        let spec = find(&id).unwrap();
        for f in &r.findings {
            assert!(spec.codes.contains(&f.code), "{id} emits `{}`, which its registry entry does not list", f.code);
            assert_eq!(f.location.font.as_deref(), Some("f0"));
            assert!(!f.message.is_empty());
        }
    }
}

#[test]
fn slant_direction_follows_the_lean_of_h() {
    let leaning = |min: f64, max: f64, peak: f64| {
        let mut s = Spec::clean();
        s.add_axis(b"slnt", min, 0.0, max);
        // The top of H moves right by 100 at the peak.
        s.tuples.push(Tuple { peak: vec![0.0, peak], x: [0, 100, 100, 0, 0, 0, 0, 0], y: [0; 8] });
        codes(&run_one(&s, "check.variations.slantDirection"))
    };
    assert!(leaning(-12.0, 0.0, -1.0).is_empty(), "negative slant leaning right is the registered direction");
    assert_eq!(leaning(0.0, 12.0, 1.0), vec!["positive-value-for-clockwise-lean"]);
}

#[test]
fn findings_carry_table_level_addresses() {
    let mut spec = Spec::clean();
    spec.weight_class = 500;
    spec.instance(259).coords[0] = 690.0;
    let r = run_one(&spec, "check.variations.weightClassMatchesDefault");
    let f = &r.findings[0];
    assert_eq!((f.location.table.as_deref(), f.location.field.as_deref()), (Some("OS/2"), Some("usWeightClass")));
    assert_eq!(f.fix.map(|x| x.id), Some("patchTable"));
    let r = run_one(&spec, "check.variations.boldInstanceCoordinate");
    let f = &r.findings[0];
    assert_eq!((f.location.instance.as_deref(), f.location.axis.as_deref()), (Some("Bold"), Some("wght")));
    let mut italic = Spec::clean();
    italic.add_axis(b"ital", 0.0, 0.0, 1.0);
    let r = run_one(&italic, "check.variations.unsupportedItalAxis");
    assert_eq!(r.findings[0].severity, Severity::Fail);
}

#[test]
fn italic_names_on_slnt_are_allowed_when_asked() {
    let mut spec = Spec::clean();
    spec.add_axis(b"slnt", -12.0, 0.0, 0.0);
    spec.stat().axes.push((*b"slnt", 256, 1));
    spec.stat().values.push(Val::Linked { axis: 1, flags: 0, name: 270, value: -12.0, linked: 0.0 });
    spec.set_name(270, "Italic");
    let bytes = spec.build();
    let check = find("check.variations.statItalicNaming").unwrap();
    assert_eq!(codes(&run(&bytes, &[check])), vec!["bad-italic"]);
    let mut config = Config::default();
    config.set(check.id, "allowOnSlnt", 1.0).unwrap();
    let services = NoServices;
    let ctx = BinaryCtx::new("f0", &bytes, 0, &services).unwrap();
    assert!(audit_binary(&ctx, &[check], &config).findings.is_empty());
}

#[test]
fn a_format_4_value_covers_a_whole_instance() {
    let mut spec = Spec::clean();
    spec.instance(259).coords[0] = 650.0;
    spec.stat().values.push(Val::Combo { flags: 0, name: 259, records: vec![(0, 650.0)] });
    assert!(run_one(&spec, "check.variations.statCoversInstances").findings.is_empty());
}

#[test]
fn hostile_variation_tables_do_not_panic() {
    let junk: Vec<([u8; 4], Vec<u8>)> = vec![
        (*b"fvar", vec![0, 1, 0, 0, 0, 16, 0, 2, 0xFF, 0xFF]),
        (*b"fvar", vec![0, 1, 0, 0, 0, 16, 0, 2, 0, 1, 0, 20, 0xFF, 0xFF, 0, 4]),
        (*b"STAT", vec![0, 1, 0, 1, 0, 8, 0xFF, 0xFF, 0, 0, 0, 20, 0xFF, 0xFF, 0, 0, 0, 30]),
        (*b"gvar", vec![0, 1, 0, 0, 0, 1, 0xFF, 0xFF, 0, 0, 0, 0]),
        (*b"name", vec![0, 0, 0xFF, 0xFF, 0, 6]),
        (*b"GDEF", vec![0, 1, 0, 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 18]),
        (*b"GPOS", vec![0, 1, 0, 0, 0, 10, 0, 12, 0, 14, 0, 0, 0, 0, 0xFF, 0xFF]),
        (*b"glyf", vec![0xFF; 7]),
        (*b"OS/2", vec![0, 4]),
    ];
    let checks = variation_checks();
    for (tag, data) in junk {
        let mut spec = Spec::clean();
        spec.add_axis(b"GRAD", -1.0, 0.0, 1.0);
        spec.kern = Some((1, 5));
        spec.raw.push((tag, data));
        let r = run(&spec.build(), &checks);
        for f in &r.findings {
            assert!(
                find(f.check).is_some_and(|c| c.codes.contains(&f.code)),
                "{}: {} emits unlisted `{}`",
                String::from_utf8_lossy(&tag),
                f.check,
                f.code
            );
        }
    }
}
