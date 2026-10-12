//! Expected statistics come from fontTools' `pens.statisticsPen` (4.66.1) run on a CFF font with
//! the same outlines, built with fontTools' FontBuilder; its em-scaled values are multiplied
//! back to font units (em 1000). Expected bounds were checked with `tx -mtx -1` on that font.

use std::sync::Arc;

use fontcraft_model::{Component, Font, Path, Shape};

use super::*;

/// Equal within 1e-6, relative above 1. The expected values are the exact fractions the
/// statistics pen's rounded output stands for.
fn close(a: f64, b: f64, what: &str) {
    assert!((a - b).abs() < 1e-6 * b.abs().max(1.0), "{what}: {a} against {b}");
}

/// The bulge of the oracle font: one cubic from (0, 0) to (100, 0), closed by a line.
fn bulge() -> Vec<Contour> {
    let p = |x, y| pt(x, y);
    vec![Contour {
        start: p(0.0, 0.0),
        segs: vec![Seg::Cubic(p(0.0, 0.0), p(0.0, 100.0), p(100.0, 100.0), p(100.0, 0.0)), Seg::Line(p(100.0, 0.0), p(0.0, 0.0))],
    }]
}

/// The parallelogram of the oracle font, counter-clockwise.
fn parallelogram() -> Vec<Contour> {
    let pts = [(0.0, 0.0), (100.0, 0.0), (150.0, 200.0), (50.0, 200.0)];
    let mut segs = Vec::new();
    for i in 0..4 {
        let (a, b) = (pts[i], pts[(i + 1) % 4]);
        segs.push(Seg::Line(pt(a.0, a.1), pt(b.0, b.1)));
    }
    vec![Contour { start: pt(0.0, 0.0), segs }]
}

#[test]
fn metrics_curve_bounds_are_exact_and_statistics_match_the_statistics_pen() {
    assert_eq!(exact_bounds(&bulge()), Some([0.0, 0.0, 100.0, 75.0]), "the curve's top, not its control points");
    let s = moment_statistics(&bulge());
    close(s.area, -6000.0, "area (clockwise)");
    close(s.mean[0], 50.0, "meanX");
    close(s.mean[1], 225.0 / 7.0, "meanY");
    close(s.variance[0], 147_500.0 / 231.0, "varianceX");
    close(s.variance[1], 19_375.0 / 49.0, "varianceY");
    assert_eq!((s.correlation, s.slant), (0.0, 0.0));

    let s = moment_statistics(&parallelogram());
    close(s.area, 20000.0, "area");
    close(s.mean[0], 75.0, "meanX");
    close(s.mean[1], 100.0, "meanY");
    close(s.variance[0], 3125.0 / 3.0, "varianceX");
    close(s.variance[1], 10_000.0 / 3.0, "varianceY");
    close(s.covariance, 2500.0 / 3.0, "covariance");
    close(s.correlation, 1.0 / 5f64.sqrt(), "correlation");
    close(s.slant, 0.25, "slant");

    // StatisticsControlPen counts the start again when a closing line is drawn.
    let c = control_statistics(&bulge());
    close(c.area, -10000.0, "control area");
    assert_eq!((c.mean, c.variance), ([40.0, 40.0], [3000.0, 3000.0]));
    close(c.covariance, 500.0, "control covariance");
    close(c.slant, 1.0 / 6.0, "control slant");
    let c = control_statistics(&parallelogram());
    close(c.area, 20000.0, "control area");
    close(c.mean[0], 60.0, "control meanX");
    close(c.variance[1], 12000.0, "control varianceY");
    close(c.correlation, 4.0 / 51f64.sqrt(), "control correlation");
    close(c.slant, 1.0 / 3.0, "control slant");
}

#[test]
fn metrics_truetype_contours_get_implied_points() {
    let off = |x: f64, y: f64| TtPoint { x, y, on: false };
    let on = |x: f64, y: f64| TtPoint { x, y, on: true };
    // Four off-curve points: a rounded square through the implied midpoints.
    let all_off = tt_contour(&[off(100.0, 100.0), off(-100.0, 100.0), off(-100.0, -100.0), off(100.0, -100.0)]).unwrap();
    assert_eq!(all_off.start, pt(100.0, 0.0), "the start is halfway between the last and the first point");
    assert_eq!(all_off.segs.len(), 4);
    assert_eq!(exact_bounds(std::slice::from_ref(&all_off)), Some([-100.0, -100.0, 100.0, 100.0]));
    close(moment_statistics(std::slice::from_ref(&all_off)).area, 100_000.0 / 3.0, "area");

    // A contour that starts off-curve starts drawing at its first on-curve point, and its
    // closing line is drawn once.
    let c = tt_contour(&[off(50.0, 100.0), on(100.0, 0.0), on(0.0, 0.0)]).unwrap();
    assert_eq!(c.start, pt(100.0, 0.0));
    assert_eq!(c.segs, vec![Seg::Line(pt(100.0, 0.0), pt(0.0, 0.0)), Seg::Quad(pt(0.0, 0.0), pt(50.0, 100.0), pt(100.0, 0.0))]);
    assert_eq!(exact_bounds(&[c]), Some([0.0, 0.0, 100.0, 50.0]));
}

fn sample() -> Font {
    let mut f = Font::new("Metrics Sample");
    f.glyph_mut("A").unwrap().layers[0].shapes.push(Shape::Path(Path::rect(60.0, 0.0, 560.0, 700.0)));
    f.glyph_mut("A").unwrap().layers[0].width = 620.0;
    f.glyph_mut("C").unwrap().layers[0].shapes.push(Shape::Component(Component::new("A", 10.0, 20.0)));
    f.glyph_mut("C").unwrap().layers[0].width = 640.0;
    f.glyph_mut("V").unwrap().layers[0].shapes.push(Shape::Path(Path::rect(0.0, 0.0, 500.0, 700.0)));
    Arc::make_mut(&mut f.kerning).ltr.set("m01", "A", "V", -50.0);
    f
}

fn measured(flavour: fontcraft_compile::Flavour, options: &MetricsOptions) -> MetricsReport {
    let f = sample();
    let bytes = fontcraft_compile::compile(&f, &f.default_target().unwrap(), flavour, &fontcraft_compile::Options::default()).unwrap().bytes;
    report(&crate::sfnt::read(&bytes, 0).unwrap(), options).unwrap()
}

#[test]
fn font_report_metrics_measures_compiled_fonts() {
    let options = MetricsOptions { kerning: true, statistics: true, ..MetricsOptions::default() };
    for (flavour, sign) in [(fontcraft_compile::Flavour::TrueType, -1.0), (fontcraft_compile::Flavour::Cff, 1.0)] {
        let r = measured(flavour, &options);
        assert_eq!(r.units_per_em, 1000);
        let a = r.glyphs.iter().find(|g| g.name == "A").unwrap();
        assert_eq!((a.unicode, a.advance, a.bounds), (Some(0x41), 620.0, Some([60.0, 0.0, 560.0, 700.0])));
        assert_eq!((a.left_side_bearing, a.right_side_bearing), (60.0, 60.0));
        close(a.area.unwrap(), sign * 350_000.0, "the area's sign follows the contour direction");
        // Exact up to rounding: the shared tolerance is relative, too loose at these magnitudes.
        let [cx, cy] = a.centroid.unwrap();
        assert!((cx - 310.0).abs() < 1e-6 && (cy - 350.0).abs() < 1e-6, "centroid ({cx}, {cy})");
        let c = r.glyphs.iter().find(|g| g.name == "C").unwrap();
        assert_eq!(c.bounds, Some([70.0, 20.0, 570.0, 720.0]), "the component is placed");
        let space = r.glyphs.iter().find(|g| g.name == "space").unwrap();
        assert_eq!((space.bounds, space.right_side_bearing), (None, 250.0));
        assert_eq!(r.kerning, vec![KernPair { left: "A".into(), right: "V".into(), value: -50.0 }]);
        let fb = r.font_bounds.as_ref().unwrap();
        assert_eq!((fb.bounds, fb.names[2].as_str(), fb.names[3].as_str()), ([0.0, 0.0, 570.0, 720.0], "C", "C"));
        assert!(r.font_statistics.is_some());
    }
}

#[test]
fn font_report_metrics_selects_glyphs_and_writes_text_forms() {
    let options =
        MetricsOptions { glyphs: vec!["A,C".into(), "V".into()], exclude_glyphs: vec!["C".into()], kerning: true, ..MetricsOptions::default() };
    let r = measured(fontcraft_compile::Flavour::Cff, &options);
    let names: Vec<&str> = r.glyphs.iter().map(|g| g.name.as_str()).collect();
    assert_eq!(names, ["A", "V"]);
    let text = to_text(&r, &TextOptions { sidebearings: true, aggregate_bbox: true, ..TextOptions::default() });
    assert!(text.contains("glyph[2] {A,0x0041,620,{60,0,560,700},60,60}"), "{text}");
    assert!(text.contains("bbox  {0,0,560,700}"), "{text}");
    assert!(text.contains("kern {A,V,-50}"), "{text}");
    let afm = to_afm(&r, &TextOptions::default());
    assert!(afm.contains("StartCharMetrics 2\n"), "the count of glyphs written: {afm}");
    assert!(afm.contains("C 65 ; WX 620 ; N A ; B 60 0 560 700 ;"), "{afm}");
    assert!(afm.contains("KPX A V -50"), "{afm}");
    let csv = to_csv(&r.rescaled(2000.0), &TextOptions::default());
    assert!(csv.contains("2,A,U+0041,1240,120,0,1120,1400"), "{csv}");

    let all = measured(fontcraft_compile::Flavour::Cff, &MetricsOptions { glyphs: vec!["0-2".into(), "U+0043".into()], ..MetricsOptions::default() });
    let names: Vec<&str> = all.glyphs.iter().map(|g| g.name.as_str()).collect();
    assert_eq!(names, [".notdef", "space", "A", "C"]);
    let f = sample();
    let bytes = fontcraft_compile::compile_ttf(&f, &fontcraft_compile::Options::default()).unwrap().bytes;
    let font = crate::sfnt::read(&bytes, 0).unwrap();
    assert!(matches!(report(&font, &MetricsOptions { glyphs: vec!["nosuch".into()], ..MetricsOptions::default() }), Err(TablesError::BadOptions(_))));
    assert!(matches!(report(&font, &MetricsOptions { glyphs: vec!["70000".into()], ..MetricsOptions::default() }), Err(TablesError::BadOptions(_))));
}

#[test]
fn font_report_metrics_refuses_damaged_fonts_without_panicking() {
    let f = sample();
    let bytes = fontcraft_compile::compile_ttf(&f, &fontcraft_compile::Options::default()).unwrap().bytes;
    let font = crate::sfnt::read(&bytes, 0).unwrap();
    let mut no_head = font.clone();
    no_head.remove(*b"head");
    assert!(report(&no_head, &MetricsOptions::default()).is_err());
    // A cut glyf table: the glyphs it no longer holds are reported with a warning.
    let mut cut = font.clone();
    let glyf = cut.get(*b"glyf").unwrap().to_vec();
    cut.insert(*b"glyf", glyf[..glyf.len() / 2].to_vec());
    let r = report(&cut, &MetricsOptions { statistics: true, kerning: true, ..MetricsOptions::default() }).unwrap();
    assert!(!r.warnings.is_empty());
    // A loca that points every glyph at garbage.
    let mut garbage = font;
    let n = garbage.get(*b"glyf").unwrap().len();
    garbage.insert(*b"glyf", vec![0xFF; n]);
    let _ = report(&garbage, &MetricsOptions { statistics: true, control_box: true, ..MetricsOptions::default() });
}
