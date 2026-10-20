//! `cvar`: deltas against fontTools' for the same masters, and a variable export whose zones
//! differ between masters.

use fontcraft_hinting::settings::family_tt_zones;
use fontcraft_hinting::{Direction, HintSettings, NodeRef, TtInstruction, TtKind, TtPoint, ZoneRef, control_values, set_tt_instructions};
use fontcraft_model::{Axis, Font, Path, Shape};
use read_fonts::tables::cvar::Cvar;
use read_fonts::tables::variations::Tuple;
use read_fonts::types::F2Dot14;
use read_fonts::{FontData, FontRead};
use skrifa::FontRef;
use skrifa::raw::TableProvider;

use super::*;
use crate::var::{VariableOptions, compile_variable};

/// Each tuple of `bytes`: its peak and its deltas over `entries` control values.
fn tuples(bytes: &[u8], axis_count: u16, entries: usize) -> Vec<(Vec<f32>, Vec<i32>)> {
    let cvar = Cvar::read(FontData::new(bytes)).unwrap();
    let values = |t: Tuple| t.values.iter().map(|v| v.get().to_f32()).collect::<Vec<_>>();
    let mut out: Vec<(Vec<f32>, Vec<i32>)> = cvar
        .variation_data(axis_count)
        .unwrap()
        .tuples()
        .map(|t| {
            let mut dense = vec![0; entries];
            for d in t.deltas() {
                dense[usize::from(d.position)] = d.value;
            }
            (values(t.peak()), dense)
        })
        .collect();
    out.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
    out
}

/// The control values `bytes` gives at `location`, from `default`.
fn at(bytes: &[u8], default: &[i16], location: &[f32]) -> Vec<f64> {
    let cvar = Cvar::read(FontData::new(bytes)).unwrap();
    let coords: Vec<F2Dot14> = location.iter().map(|v| F2Dot14::from_f32(*v)).collect();
    let mut deltas = vec![0i32; default.len()];
    cvar.deltas(location.len() as u16, &coords, &mut deltas).unwrap();
    default.iter().zip(deltas).map(|(v, d)| f64::from(*v) + f64::from(d) / 65536.0).collect()
}

type Case = (u16, Vec<i16>, Vec<(Vec<f64>, Vec<i16>)>);

/// Masters as fontTools' `VariationModel.getDeltasAndSupports(round=round)` sees them in
/// `_merge_TTHinting`; the expected tuples are what it printed for each (fontTools 4, the
/// version fontmake 3.12.1 ships).
fn cases() -> Vec<(&'static str, Case, Vec<(Vec<f32>, Vec<i32>)>)> {
    vec![
        ("two masters", (1, vec![100, 50, -10], vec![(vec![1.0], vec![121, 50, -20])]), vec![(vec![1.0], vec![21, 0, -10])]),
        (
            "an intermediate master",
            (1, vec![100, 50], vec![(vec![0.5], vec![111, 60]), (vec![1.0], vec![121, 70])]),
            vec![(vec![0.5], vec![11, 10]), (vec![1.0], vec![21, 20])],
        ),
        (
            "a master equal to the default",
            (1, vec![100, 7], vec![(vec![-1.0], vec![80, 7]), (vec![1.0], vec![100, 7])]),
            vec![(vec![-1.0], vec![-20, 0])],
        ),
        (
            "half-unit deltas round to even",
            (2, vec![100, 10], vec![(vec![1.0, 0.0], vec![121, 10]), (vec![0.0, 1.0], vec![100, 13]), (vec![0.5, 0.5], vec![111, 13])]),
            vec![(vec![0.0, 1.0], vec![0, 3]), (vec![0.5, 0.5], vec![0, 2]), (vec![1.0, 0.0], vec![21, 0])],
        ),
    ]
}

#[test]
fn cvar_deltas_match_fonttools() {
    for (name, (axes, default, masters), expected) in cases() {
        let bytes = build_cvar(axes, &default, &masters, &mut Vec::new()).unwrap().unwrap();
        assert_eq!(tuples(&bytes, axes, default.len()), expected, "{name}");
        if name != "half-unit deltas round to even" {
            for (location, values) in &masters {
                let location: Vec<f32> = location.iter().map(|v| *v as f32).collect();
                let back: Vec<f64> = values.iter().map(|v| f64::from(*v)).collect();
                assert_eq!(at(&bytes, &default, &location), back, "{name}: every master's values come back");
            }
        }
    }
}

#[test]
fn identical_control_values_write_no_cvar() {
    let mut w = Vec::new();
    assert_eq!(build_cvar(1, &[100, 50], &[(vec![1.0], vec![100, 50]), (vec![-1.0], vec![100, 50])], &mut w), Ok(None));
    assert_eq!(build_cvar(1, &[100, 50], &[], &mut w), Ok(None));
    assert!(build_cvar(1, &[100, 50], &[(vec![1.0], vec![100])], &mut w).is_err(), "lengths that differ are refused");
    assert!(build_cvar(1, &[100], &[(vec![1.0], vec![i16::MAX])], &mut w).is_ok());
    assert!(build_cvar(1, &[i16::MIN], &[(vec![1.0], vec![i16::MAX])], &mut w).is_err(), "a delta past 16 bits is refused");
    assert!(w.is_empty(), "{w:?}");
}

#[test]
fn a_master_at_a_location_already_taken_adds_no_region_and_says_so_when_it_differs() {
    let mut w = Vec::new();
    assert_eq!(build_cvar(1, &[100, 50], &[(vec![0.0], vec![100, 50])], &mut w), Ok(None));
    assert!(w.is_empty(), "the same values as the default's: {w:?}");
    assert_eq!(build_cvar(1, &[100, 50], &[(vec![0.0], vec![130, 50])], &mut w), Ok(None));
    assert!(w.len() == 1 && w[0].contains("default master"), "{w:?}");
    let mut w = Vec::new();
    let bytes = build_cvar(1, &[100], &[(vec![1.0], vec![120]), (vec![1.0], vec![140])], &mut w).unwrap().unwrap();
    assert_eq!(tuples(&bytes, 1, 1), vec![(vec![1.0], vec![20])], "the first master at a location gives its region");
    assert!(w.len() == 1 && w[0].contains("earlier master"), "{w:?}");
}

/// A Snap of the top right node of a rectangle to its zone.
fn snap() -> TtInstruction {
    let kind = TtKind::Snap { zone: ZoneRef::Auto };
    TtInstruction {
        rounding: TtInstruction::default_rounding(&kind),
        kind,
        direction: Direction::Vertical,
        target: TtPoint::Node(NodeRef::new(0, 2)),
        origin: None,
        other: None,
        source: None,
    }
}

/// Two masters on a weight axis; Bold's cap height is 20 units higher. `H` snaps its top to the
/// cap-height zone.
fn two_masters(bold_cap_height: f64) -> Font {
    let mut f = Font::new("Cvar Sans");
    f.axes = std::sync::Arc::new(vec![Axis { name: "Weight".into(), tag: "wght".into(), ..Default::default() }]);
    let masters = std::sync::Arc::make_mut(&mut f.masters);
    masters[0].location = vec![100.0];
    let mut bold = masters[0].clone();
    bold.id = "m02".into();
    bold.name = "Bold".into();
    bold.location = vec![200.0];
    bold.metric_values[1].pos = bold_cap_height;
    masters.push(bold);
    {
        let mut h = f.glyph_mut("H").unwrap();
        h.layers[0].width = 600.0;
        h.layers[0].shapes.push(Shape::Path(Path::rect(50.0, 0.0, 150.0, 700.0)));
        set_tt_instructions(&mut h.layers[0], &[snap()]).unwrap();
    }
    for i in 0..f.glyphs.len() {
        let mut g = f.glyphs.get_mut_at(i).unwrap();
        let mut layer = g.layers[0].clone();
        layer.id = "m02".into();
        layer.master = "m02".into();
        layer.info.hints.clear();
        g.layers.push(layer);
    }
    f
}

/// [`two_masters`] with Bold's cap height at 720 and a third master, Medium, halfway between,
/// drawn like Regular; `medium_options` are its TTFAutohint options.
fn three_masters(medium_options: Option<&str>) -> Font {
    use fontcraft_model::Param;
    use fontcraft_model::plist::{Str, Value};
    let mut f = two_masters(720.0);
    let masters = std::sync::Arc::make_mut(&mut f.masters);
    let mut medium = masters[0].clone();
    medium.id = "m03".into();
    medium.name = "Medium".into();
    medium.location = vec![150.0];
    if let Some(text) = medium_options {
        medium.params.0.push(Param::new(fontcraft_hinting::settings::TTF_AUTOHINT_OPTIONS, Value::String(Str::new(text))));
    }
    masters.push(medium);
    for i in 0..f.glyphs.len() {
        let mut g = f.glyphs.get_mut_at(i).unwrap();
        let mut layer = g.layers[0].clone();
        layer.id = "m03".into();
        layer.master = "m03".into();
        layer.info.hints.clear();
        g.layers.push(layer);
    }
    f
}

#[test]
fn a_master_with_dehint_on_is_left_out_of_cvar() {
    let autohint = Options { hinting: crate::HintingOptions { autohint: true, ..Default::default() }, ..Options::default() };
    let peaks = |f: &Font| {
        let var = compile_variable(f, None, &autohint, &VariableOptions::default()).unwrap();
        let font = FontRef::new(&var.compiled.bytes).unwrap();
        let entries = font.cvt().unwrap().len();
        let cvar = font.table_data(skrifa::Tag::new(b"cvar")).unwrap_or_else(|| panic!("a cvar table: {:?}", var.compiled.warnings));
        tuples(cvar.as_bytes(), 1, entries).into_iter().map(|(peak, _)| peak).collect::<Vec<_>>()
    };
    assert_eq!(peaks(&three_masters(None)), vec![vec![0.5], vec![1.0]], "Medium's zones differ from the interpolated ones");
    assert_eq!(peaks(&three_masters(Some("--dehint"))), vec![vec![1.0]], "Medium has no control values; Bold still varies them");
}

#[test]
fn a_variable_font_varies_its_zones_with_cvar() {
    let f = two_masters(720.0);
    let var = compile_variable(&f, None, &Options::default(), &VariableOptions::default()).unwrap();
    let out = &var.compiled;
    assert!(!out.warnings.iter().any(|w| w.contains("cvar")), "{:?}", out.warnings);
    let font = FontRef::new(&out.bytes).unwrap();
    let cvt: Vec<i16> = font.cvt().unwrap().iter().map(|v| v.get()).collect();
    let family = family_tt_zones(&f).unwrap();
    let regular = control_values(&HintSettings::for_master(&f, "m01").unwrap(), &family).unwrap().1;
    let bold = control_values(&HintSettings::for_master(&f, "m02").unwrap(), &family).unwrap().1;
    assert_eq!(cvt, regular);
    assert_ne!(regular, bold);
    let cvar = font.table_data(skrifa::Tag::new(b"cvar")).expect("a cvar table").as_bytes().to_vec();
    let back: Vec<f64> = bold.iter().map(|v| f64::from(*v)).collect();
    assert_eq!(at(&cvar, &cvt, &[1.0]), back, "the Bold master's zones");
    let moved: Vec<i32> = regular.iter().zip(&bold).map(|(r, b)| i32::from(*b) - i32::from(*r)).collect();
    assert_eq!(tuples(&cvar, 1, cvt.len()), vec![(vec![1.0], moved.clone())]);
    assert!(moved.iter().filter(|d| **d == 20).count() == 2, "the cap-height zone's position and fitted row: {moved:?}");

    let same = compile_variable(&two_masters(700.0), None, &Options::default(), &VariableOptions::default()).unwrap();
    let font = FontRef::new(&same.compiled.bytes).unwrap();
    assert!(font.table_data(skrifa::Tag::new(b"cvt ")).is_some());
    assert!(font.table_data(skrifa::Tag::new(b"cvar")).is_none(), "identical control values write no cvar");
}

/// The glyph program of `name` in `out`.
fn program(out: &crate::Compiled, name: &str) -> Vec<u8> {
    use skrifa::raw::tables::glyf::Glyph as RawGlyph;
    let font = FontRef::new(&out.bytes).unwrap();
    let gid = skrifa::GlyphId::new(u32::try_from(out.glyph_order.iter().position(|n| n == name).unwrap()).unwrap());
    match font.loca(None).unwrap().get_glyf(gid, &font.glyf().unwrap()).unwrap() {
        Some(RawGlyph::Simple(s)) => s.instructions().to_vec(),
        _ => Vec::new(),
    }
}

#[test]
fn an_alternate_layer_of_the_main_master_brings_its_own_instructions() {
    let mut f = two_masters(700.0);
    {
        let mut h = f.glyph_mut("H").unwrap();
        for master in ["m01", "m02"] {
            let mut alt = h.layers[0].clone();
            alt.id = format!("{master}-alt");
            alt.master = master.into();
            alt.shapes = vec![Shape::Path(Path::rect(50.0, 0.0, 250.0, 700.0))];
            alt.info.hints.clear();
            alt.info.attrs.axis_rules = vec![fontcraft_model::AxisRule { min: Some(150.0), max: None }];
            if master == "m01" {
                set_tt_instructions(&mut alt, &[snap()]).unwrap();
            }
            h.layers.push(alt);
        }
    }
    let var = compile_variable(&f, None, &Options::default(), &VariableOptions::default()).unwrap();
    let out = &var.compiled;
    let alternate = out.glyph_order.iter().find(|n| n.starts_with("H.")).expect("an alternate glyph").clone();
    assert!(!program(out, "H").is_empty(), "{:?}", out.warnings);
    assert!(!program(out, &alternate).is_empty(), "the main master's alternate layer is the alternate glyph's main-master layer: {:?}", out.warnings);
}
