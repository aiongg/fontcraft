//! The variation-model tools on a small variable font built in code. Expected values come from
//! fontTools 4.66 (`varLib.hvar`, `varLib.models`, `varLib.avar.plan`, `designspaceLib`) run on
//! the same font built with `fontBuilder` and on the same locations and document.

use read_fonts::tables::hvar::Hvar;
use read_fonts::{FontData, FontRead};
use write_fonts::tables::fvar::{AxisInstanceArrays, Fvar, VariationAxisRecord};
use write_fonts::tables::gvar::{GlyphDelta, GlyphDeltas, GlyphVariations, Gvar};
use write_fonts::tables::variations::Tent;
use write_fonts::types::{F2Dot14, Fixed, GlyphId, NameId, Tag as WTag};

use super::*;
use crate::instance::ivs::StoreDeltas;
use crate::sfnt::Flavour;

const WGHT: Tag = *b"wght";
const WDTH: Tag = *b"wdth";

fn be16(v: &mut Vec<u8>, x: u16) {
    v.extend_from_slice(&x.to_be_bytes());
}

/// `.notdef` (empty, advance 500); `A`, a stem 100–200 × 0–700 (advance 300); `B`, `A` placed at
/// (50, 0) (advance 350). On wght 100–400–900, at +1 A's right side and advance move +300, at −1
/// by −60; at +1 B's component moves (30, 10) and its advance +60, its left phantom point left out.
fn variable_font() -> SfntFont {
    let mut a = Vec::new();
    for v in [1i16, 100, 0, 200, 700] {
        be16(&mut a, v as u16);
    }
    be16(&mut a, 3);
    be16(&mut a, 0);
    a.extend_from_slice(&[1, 1, 1, 1]);
    for v in [100i16, 0, 100, 0, 0, 700, 0, -700] {
        be16(&mut a, v as u16);
    }
    let mut b = Vec::new();
    for v in [-1i16, 150, 0, 250, 700, 0x0003, 1, 50, 0] {
        be16(&mut b, v as u16);
    }
    let mut glyf = a.clone();
    glyf.extend_from_slice(&b);
    let mut loca = Vec::new();
    for v in [0u32, 0, a.len() as u32, (a.len() + b.len()) as u32] {
        loca.extend_from_slice(&v.to_be_bytes());
    }
    let mut head = vec![0u8; 54];
    head[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    head[12..16].copy_from_slice(&0x5F0F_3CF5u32.to_be_bytes());
    head[18..20].copy_from_slice(&1000u16.to_be_bytes());
    head[50..52].copy_from_slice(&1u16.to_be_bytes());
    let mut maxp = vec![0u8; 32];
    maxp[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    maxp[4..6].copy_from_slice(&3u16.to_be_bytes());
    let mut hhea = vec![0u8; 36];
    hhea[0..4].copy_from_slice(&0x0001_0000u32.to_be_bytes());
    hhea[34..36].copy_from_slice(&3u16.to_be_bytes());
    let mut hmtx = Vec::new();
    for (adv, lsb) in [(500u16, 0u16), (300, 100), (350, 150)] {
        be16(&mut hmtx, adv);
        be16(&mut hmtx, lsb);
    }
    // post 2.0 naming the glyphs with the standard names 0 (.notdef), 36 (A) and 37 (B).
    let mut post = vec![0u8; 32];
    post[0..4].copy_from_slice(&0x0002_0000u32.to_be_bytes());
    for v in [3u16, 0, 36, 37] {
        be16(&mut post, v);
    }
    let tent = |peak: f32| Tent::new(F2Dot14::from_f32(peak), None);
    let r = GlyphDelta::required;
    let o = GlyphDelta::optional;
    let gvar = Gvar::new(
        vec![
            GlyphVariations::new(GlyphId::new(0), vec![]),
            GlyphVariations::new(
                GlyphId::new(1),
                vec![
                    GlyphDeltas::new(vec![tent(1.0)], vec![r(0, 0), r(0, 0), r(300, 0), r(300, 0), r(0, 0), r(300, 0), r(0, 0), r(0, 0)]),
                    GlyphDeltas::new(vec![tent(-1.0)], vec![r(0, 0), r(0, 0), r(-60, 0), r(-60, 0), r(0, 0), r(-60, 0), r(0, 0), r(0, 0)]),
                ],
            ),
            GlyphVariations::new(GlyphId::new(2), vec![GlyphDeltas::new(vec![tent(1.0)], vec![r(30, 10), o(0, 0), r(60, 0), o(0, 0), o(0, 0)])]),
        ],
        1,
    )
    .unwrap();
    let axis =
        VariationAxisRecord::new(WTag::new(b"wght"), Fixed::from_f64(100.0), Fixed::from_f64(400.0), Fixed::from_f64(900.0), 0, NameId::new(256));
    let fvar = Fvar::new(AxisInstanceArrays::new(vec![axis], vec![]));

    let mut font = SfntFont::new(Flavour::TrueType);
    font.insert(*b"head", head);
    font.insert(*b"maxp", maxp);
    font.insert(*b"hhea", hhea);
    font.insert(*b"hmtx", hmtx);
    font.insert(*b"post", post);
    font.insert(*b"loca", loca);
    font.insert(*b"glyf", glyf);
    font.insert(*b"gvar", write_fonts::dump_table(&gvar).unwrap());
    font.insert(*b"fvar", write_fonts::dump_table(&fvar).unwrap());
    font
}

/// The advance delta of each glyph that the font's `HVAR` gives at `coords`.
fn hvar_deltas(font: &SfntFont, coords: &[f64]) -> Vec<f64> {
    let hvar = Hvar::read(FontData::new(font.get(*b"HVAR").unwrap())).unwrap();
    let deltas = StoreDeltas::from_store(&hvar.item_variation_store().unwrap(), coords).unwrap();
    (0..3u32)
        .map(|g| match hvar.advance_width_mapping() {
            Some(map) => {
                let i = map.unwrap().get(g).unwrap();
                deltas.get(i.outer, i.inner)
            }
            None => deltas.get(0, g as u16),
        })
        .collect()
}

#[test]
fn build_hvar_reads_advance_deltas_from_the_phantom_points() {
    let font = variable_font();
    let built = build_hvar_with(&font, &HvarOptions::default()).unwrap();
    assert_eq!((built.hvar, built.vvar), (TableAction::Built, TableAction::Skipped), "no vmtx, so no VVAR");
    assert_eq!(built.varied_glyphs, 2);
    // fontTools varLib.hvar on the same font, evaluated with VarStoreInstancer.
    assert_eq!(hvar_deltas(&built.font, &[1.0]), vec![0.0, 300.0, 60.0]);
    assert_eq!(hvar_deltas(&built.font, &[-1.0]), vec![0.0, -60.0, 0.0]);
    assert_eq!(hvar_deltas(&built.font, &[0.5]), vec![0.0, 150.0, 30.0]);
    assert_eq!(build_hvar(&font).unwrap().get(*b"HVAR"), built.font.get(*b"HVAR"));

    let again = build_hvar_with(&built.font, &HvarOptions::default());
    assert!(again.is_err(), "an existing HVAR is kept unless replaced, and then nothing is left to build");
    assert!(build_hvar_with(&built.font, &HvarOptions { vvar: true, replace: true }).is_ok());
    let mut no_gvar = font.clone();
    no_gvar.remove(*b"gvar");
    assert!(build_hvar_with(&no_gvar, &HvarOptions::default()).is_err());
}

#[test]
fn model_supports_of_masters_on_two_axes_match_fonttools() {
    // The axes in fontTools' sorted order (wdth before wght), so ties break the same way.
    let locations = vec![
        vec![(WDTH, 0.0), (WGHT, 0.0)],
        vec![(WGHT, 1.0)],
        vec![(WGHT, -1.0)],
        vec![(WDTH, 1.0)],
        vec![(WDTH, 1.0), (WGHT, 1.0)],
        vec![(WGHT, 0.5)],
    ];
    let model = describe_model(&locations).unwrap();
    let masters: Vec<usize> = model.iter().map(|s| s.master).collect();
    assert_eq!(masters, vec![0, 3, 2, 5, 1, 4], "fontTools' sorted locations");
    let regions: Vec<Vec<(Tag, [f64; 3])>> = model.iter().map(|s| s.region.clone()).collect();
    assert_eq!(
        regions,
        vec![
            vec![],
            vec![(WDTH, [0.0, 1.0, 1.0])],
            vec![(WGHT, [-1.0, -1.0, 0.0])],
            vec![(WGHT, [0.0, 0.5, 1.0])],
            vec![(WGHT, [0.5, 1.0, 1.0])],
            vec![(WDTH, [0.0, 1.0, 1.0]), (WGHT, [0.0, 1.0, 1.0])],
        ]
    );
    assert_eq!(model.get(5).map(|s| s.location.clone()), Some(vec![(WDTH, 1.0), (WGHT, 1.0)]));

    // Three masters on one axis.
    let three = describe_model(&[vec![(WGHT, 0.0)], vec![(WGHT, -1.0)], vec![(WGHT, 1.0)]]).unwrap();
    assert_eq!(
        three.iter().map(|s| s.region.clone()).collect::<Vec<_>>(),
        vec![vec![], vec![(WGHT, [-1.0, -1.0, 0.0])], vec![(WGHT, [0.0, 1.0, 1.0])]]
    );

    assert!(describe_model(&[vec![(WGHT, 1.0)]]).is_err(), "no default master");
    assert!(describe_model(&[vec![], vec![(WGHT, 1.0)], vec![(WGHT, 1.0)]]).is_err(), "two masters at one location");
    assert!(describe_model(&[vec![], vec![(WGHT, f64::NAN)]]).is_err());
    assert!(describe_model(&[vec![], vec![(WGHT, 1.0), (WGHT, 0.5)]]).is_err(), "an axis twice in one location");
}

#[test]
fn designspace_sources_are_normalised_through_the_axis_maps() {
    let ds = r#"<?xml version='1.0' encoding='UTF-8'?>
<designspace format="4.1">
  <axes>
    <axis tag="wght" name="Weight" minimum="100" maximum="900" default="400">
      <map input="100" output="20"/>
      <map input="400" output="80"/>
      <map input="900" output="200"/>
    </axis>
    <axis tag="wdth" name="Width" minimum="75" maximum="125" default="100"/>
  </axes>
  <!-- <source name="commented out"/> -->
  <sources>
    <source filename="a.ufo" name="Regular"><location><dimension name="Weight" xvalue="80"/><dimension name="Width" xvalue="100"/></location></source>
    <source filename="b.ufo" name="Bold"><location><dimension name="Weight" xvalue="200"/></location></source>
    <source filename="c.ufo" name="Light &amp; Condensed"><location><dimension name="Weight" xvalue="50"/><dimension name="Width" xvalue="75"/></location></source>
  </sources>
</designspace>"#;
    let read = designspace_locations(ds).unwrap();
    assert_eq!(read.axes, vec![(WGHT, "Weight".to_string()), (WDTH, "Width".to_string())], "sorted by name");
    let sources: Vec<(String, Vec<(Tag, f64)>)> = read.sources;
    // fontTools: [{Weight: 0, Width: 0}, {Weight: 1}, {Weight: -0.5, Width: -1}].
    assert_eq!(
        sources,
        vec![
            ("Regular".to_string(), vec![(WGHT, 0.0), (WDTH, 0.0)]),
            ("Bold".to_string(), vec![(WGHT, 1.0), (WDTH, 0.0)]),
            ("Light & Condensed".to_string(), vec![(WGHT, -0.5), (WDTH, -1.0)]),
        ]
    );
    assert!(designspace_locations("<designspace><axes/></designspace>").is_err(), "no sources");
    assert!(designspace_locations("<designspace><axes><axis name=\"Weight\"").is_err());
}

#[test]
fn plots_draw_one_panel_per_master() {
    let one = plot(&[vec![(WGHT, 0.0)], vec![(WGHT, -1.0)], vec![(WGHT, 1.0)]]).unwrap();
    assert!(one.starts_with("<svg"));
    assert_eq!(one.matches("class=\"panel\"").count(), 3);
    let two =
        plot_with(&[vec![(WDTH, 0.0), (WGHT, 0.0)], vec![(WGHT, 1.0)], vec![(WDTH, 1.0)], vec![(WDTH, 1.0), (WGHT, 1.0)]], &["Regular".into()], None)
            .unwrap();
    assert_eq!(two.matches("class=\"panel\"").count(), 4);
    assert!(two.contains(">Regular<"), "the default master is titled by its name");
    let sliced =
        plot_with(&[vec![(WDTH, 0.0), (WGHT, 0.0)], vec![(WGHT, 1.0)], vec![(WDTH, 1.0)], vec![(WDTH, 1.0), (WGHT, 1.0)]], &[], Some(&[WGHT]))
            .unwrap();
    assert_eq!(sliced.matches("class=\"panel\"").count(), 2, "masters off the plotted axis are left out");
    let three = [vec![], vec![(WGHT, 1.0)], vec![(WDTH, 1.0)], vec![(*b"opsz", 1.0)]];
    assert!(plot(&three).is_err(), "three axes need a choice");

    let font = build_hvar(&variable_font()).unwrap();
    let svg = plot_font(&font, None).unwrap();
    assert_eq!(svg.matches("class=\"panel\"").count(), 2, "the gvar regions +1 and -1 (HVAR has the same)");
}

/// fontTools `planWeightAxis(font.getGlyphSet, (100, 400, 900))` on the same font.
const PLANNED: [(f64, f64); 17] = [
    (-1.0, -1.0),
    (-0.8333333333333334, -0.8886019603036128),
    (-0.6666666666666667, -0.7586933722168839),
    (-0.5, -0.6087003665911999),
    (-0.33333333333333337, -0.43405134903328),
    (-0.16666666666666663, -0.23308526491756631),
    (0.0, 0.0),
    (0.1, 0.050812814924943835),
    (0.2, 0.11017257607122884),
    (0.3, 0.1769336863860632),
    (0.4, 0.25380300505568176),
    (0.5, 0.3416043465156008),
    (0.6, 0.4414650967742134),
    (0.7, 0.5551808477439983),
    (0.8, 0.6844987604503094),
    (0.9, 0.8318310289051524),
    (1.0, 1.0),
];

fn close(a: &[(f64, f64)], b: &[(f64, f64)], tolerance: f64) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| (x.0 - y.0).abs() <= tolerance && (x.1 - y.1).abs() <= tolerance)
}

#[test]
fn plan_avar_places_weights_as_fonttools_does() {
    let font = variable_font();
    let plan = plan_avar_with(&font, &PlanOptions::default()).unwrap();
    assert_eq!(plan.axes.len(), 1, "only the weight axis exists");
    let wght = plan.axes.first().unwrap();
    assert!(close(&wght.normalized, &PLANNED, 1e-9), "{:?}", wght.normalized);
    assert_eq!(wght.mapping.get(4).map(|p| p.0), Some(300.0));
    assert!((wght.mapping.get(4).unwrap().1 - 269.784595290016).abs() < 1e-7);
    assert!(wght.designspace.starts_with("    <axis tag=\"wght\" name=\"Weight\" minimum=\"100\" default=\"400\" maximum=\"900\">\n"));
    assert!(wght.designspace.contains("      <map input=\"300\" output=\"269.785\"/>\n"), "{}", wght.designspace);
    assert!(wght.designspace.ends_with("    </axis>"));
    // The avar table holds the map, quantised to F2Dot14.
    let written = crate::variations::segment_maps(&plan.font).unwrap();
    let (_, map) = written.first().unwrap();
    assert!(close(map, &PLANNED, 0.5 / 16384.0 + 1e-12), "{map:?}");
    assert_eq!(plan_avar(&font).unwrap(), vec![(WGHT, wght.normalized.clone())]);

    // fontTools with weights 300 600, 4 samples and the glyphs A and B:2.
    let options = PlanOptions {
        weight: AxisPlan { values: Some(vec![300.0, 600.0]), ..AxisPlan::default() },
        samples: Some(4),
        glyphs: Some(vec![("A".into(), 1.0), ("B".into(), 2.0)]),
        ..PlanOptions::default()
    };
    let plan = plan_avar_with(&font, &options).unwrap();
    let expected = [(-1.0, -1.0), (-0.33333333333333337, -0.43865616711987176), (0.0, 0.0), (0.4, 0.24703370886408244), (1.0, 1.0)];
    assert!(close(&plan.axes.first().unwrap().normalized, &expected, 1e-9), "{:?}", plan.axes);
    assert!((plan.axes.first().unwrap().mapping.get(3).unwrap().1 - 523.5168544320412).abs() < 1e-7);

    let unknown = PlanOptions { glyphs: Some(vec![("Z".into(), 1.0)]), ..PlanOptions::default() };
    assert!(plan_avar_with(&font, &unknown).is_err(), "a glyph the font lacks");
    let disordered = PlanOptions { weight: AxisPlan { design_limits: Some((900.0, 400.0, 100.0)), ..AxisPlan::default() }, ..PlanOptions::default() };
    assert!(plan_avar_with(&font, &disordered).is_err());
    let too_many = PlanOptions { samples: Some(MAX_SAMPLES_FOR_TESTS + 1), ..PlanOptions::default() };
    assert!(plan_avar_with(&font, &too_many).is_err());
    let identity = PlanOptions { weight: AxisPlan { values: Some(vec![]), ..AxisPlan::default() }, ..PlanOptions::default() };
    let plan = plan_avar_with(&font, &identity).unwrap();
    assert!(plan.axes.first().unwrap().mapping.is_empty() && plan.font.get(*b"avar").is_none(), "nothing to place, no avar");
    assert!(plan_plot(&plan.axes).starts_with("<svg"));
}

const MAX_SAMPLES_FOR_TESTS: usize = super::plan::MAX_SAMPLES;

#[test]
fn glyph_deltas_are_charged_once_however_many_components_use_the_glyph() {
    use super::glyphset::{GlyphSet, MAX_DRAW_WORK, Work};
    const POINTS: usize = 1_000;
    const TUPLES: usize = 0x0FFF;
    const COMPONENTS: usize = 300;
    // Glyph 1: one contour of 1,000 points at the origin. Glyph 2: 300 copies of glyph 1.
    let mut s = Vec::new();
    for v in [1u16, 0, 0, 0, 0, (POINTS - 1) as u16, 0] {
        be16(&mut s, v);
    }
    let mut left = POINTS;
    while left > 0 {
        let run = left.min(256);
        s.extend_from_slice(&[0x39, (run - 1) as u8]);
        left -= run;
    }
    let mut c = Vec::new();
    for v in [0xFFFFu16, 0, 0, 0, 0] {
        be16(&mut c, v);
    }
    for i in 0..COMPONENTS {
        be16(&mut c, if i + 1 < COMPONENTS { 0x22 } else { 0x02 });
        be16(&mut c, 1);
        c.extend_from_slice(&[0, 0]);
    }
    let mut glyf = s.clone();
    glyf.extend_from_slice(&c);
    let mut loca = Vec::new();
    for v in [0u32, 0, s.len() as u32, (s.len() + c.len()) as u32] {
        loca.extend_from_slice(&v.to_be_bytes());
    }
    // gvar: 4,095 tuples peaking at wght +1 on glyph 1, each with a zero delta for every point.
    let all_zero = |out: &mut Vec<u8>| {
        let mut left = POINTS + 4;
        while left > 0 {
            let run = left.min(64);
            out.push(0x80 | (run - 1) as u8);
            left -= run;
        }
    };
    let mut tuple_data = Vec::new();
    all_zero(&mut tuple_data);
    all_zero(&mut tuple_data);
    let mut data = Vec::new();
    be16(&mut data, 0x8000 | TUPLES as u16);
    be16(&mut data, (4 + 6 * TUPLES) as u16);
    for _ in 0..TUPLES {
        for v in [tuple_data.len() as u16, 0x8000, 0x4000] {
            be16(&mut data, v);
        }
    }
    data.push(0);
    for _ in 0..TUPLES {
        data.extend_from_slice(&tuple_data);
    }
    let mut gvar = Vec::new();
    for v in [1u16, 0, 1, 0] {
        be16(&mut gvar, v);
    }
    let offsets = 4 * 4;
    gvar.extend_from_slice(&(20u32 + offsets).to_be_bytes());
    for v in [3u16, 1] {
        be16(&mut gvar, v);
    }
    gvar.extend_from_slice(&(20u32 + offsets).to_be_bytes());
    for v in [0u32, 0, data.len() as u32, data.len() as u32] {
        gvar.extend_from_slice(&v.to_be_bytes());
    }
    gvar.extend_from_slice(&data);
    let mut font = variable_font();
    font.insert(*b"glyf", glyf);
    font.insert(*b"loca", loca);
    font.insert(*b"gvar", gvar);

    let set = GlyphSet::new(&font, 1).unwrap();
    let mut work = Work(MAX_DRAW_WORK);
    set.at(&[1.0]).unwrap().glyph(1, &mut work).unwrap();
    let simple = MAX_DRAW_WORK - work.0;
    assert!(simple >= TUPLES * (POINTS + 4), "every tuple that applies is charged per point: {simple}");
    // Decoding the deltas again for each component would cost 300 times as much.
    let mut work = Work(MAX_DRAW_WORK);
    set.at(&[1.0]).unwrap().glyph(2, &mut work).unwrap();
    let composite = MAX_DRAW_WORK - work.0;
    assert!(composite < 2 * simple, "{composite} against {simple}");
}

#[test]
fn many_distinct_advance_regions_are_counted_in_linear_time_and_refused() {
    // 20 glyphs without outlines, each with 4,095 tuples whose private intermediate regions all
    // differ: 81,900 regions, more than a store can list. Comparing each new region with every
    // earlier one could not finish.
    const GLYPHS: usize = 20;
    const TUPLES: usize = 0x0FFF;
    // The right phantom point moves by 1; every other delta is zero.
    let tuple_data = [0x03u8, 0, 1, 0, 0, 0x83];
    let mut gvar = Vec::new();
    for v in [1u16, 0, 1, 0] {
        be16(&mut gvar, v);
    }
    let header = 20 + 4 * (GLYPHS as u32 + 1);
    gvar.extend_from_slice(&header.to_be_bytes());
    for v in [GLYPHS as u16, 1] {
        be16(&mut gvar, v);
    }
    gvar.extend_from_slice(&header.to_be_bytes());
    let per_glyph = 4 + 10 * TUPLES + 1 + tuple_data.len() * TUPLES;
    for g in 0..=GLYPHS {
        gvar.extend_from_slice(&((g * per_glyph) as u32).to_be_bytes());
    }
    for g in 0..GLYPHS {
        be16(&mut gvar, 0x8000 | TUPLES as u16);
        be16(&mut gvar, (4 + 10 * TUPLES) as u16);
        for k in 0..TUPLES {
            let n = g * TUPLES + k;
            let (start, peak) = (-((n % 16_384) as i16), 1 + (n / 16_384) as i16);
            for v in [tuple_data.len() as u16, 0xC000, peak as u16, start as u16, 16_384] {
                be16(&mut gvar, v);
            }
        }
        gvar.push(0);
        for _ in 0..TUPLES {
            gvar.extend_from_slice(&tuple_data);
        }
    }
    let mut font = variable_font();
    let mut maxp = font.get(*b"maxp").unwrap().to_vec();
    maxp[4..6].copy_from_slice(&(GLYPHS as u16).to_be_bytes());
    font.insert(*b"maxp", maxp);
    font.insert(*b"loca", vec![0u8; 4 * (GLYPHS + 1)]);
    font.insert(*b"gvar", gvar);
    let r = build_hvar_with(&font, &HvarOptions::default());
    assert!(matches!(r, Err(TablesError::TooLarge { what: "the distinct regions of the advance deltas", .. })), "{r:?}");
}

#[test]
fn plans_with_many_pins_and_values_are_refused_before_measuring() {
    // 9,999 stretches between 10,000 pins, each holding a target value: copying the pins'
    // measurements into every stretch, and measuring each pin, would take the whole budget of
    // draws many times over.
    let font = variable_font();
    let pins: Vec<(f64, f64)> = (0..10_000).map(|i| 100.0 + 0.08 * f64::from(i)).map(|v| (v, v)).collect();
    let values: Vec<f64> = pins.windows(2).map(|w| (w[0].0 + w[1].0) / 2.0).collect();
    for samples in [0, MAX_SAMPLES_FOR_TESTS] {
        let options = PlanOptions {
            weight: AxisPlan { values: Some(values.clone()), pins: pins.clone(), ..AxisPlan::default() },
            samples: Some(samples),
            ..PlanOptions::default()
        };
        let r = plan_avar_with(&font, &options);
        assert!(matches!(r, Err(TablesError::TooLarge { what: "the measurements of one plan", .. })), "{samples} samples: {r:?}");
    }
    // A few pins and values stay well inside the budget.
    let options = PlanOptions {
        weight: AxisPlan { values: Some(vec![250.0, 650.0]), pins: vec![(300.0, 300.0), (600.0, 600.0)], ..AxisPlan::default() },
        samples: Some(MAX_SAMPLES_FOR_TESTS),
        ..PlanOptions::default()
    };
    assert!(plan_avar_with(&font, &options).is_ok());
}
