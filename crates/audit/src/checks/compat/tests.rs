//! Behaviour tests of the master-compatibility checks. The expected scores, nodes and statistics
//! were produced by fontTools 4.66.1 (`varLib.interpolatable.test_gen` and the statistics pens)
//! on the same outlines, drawn as UFO contours with the first on-curve point as the start.

use std::sync::Arc;

use fontcraft_model::{Anchor, Axis, Component, Font, Glyph, Instance, Layer, Master, Node, NodeKind, Path, Shape};

use super::*;
use crate::{AuditContext, AuditServices, CompatibilityReport, Config, LayerReport, NoServices, Unavailable, audit_glyph, audit_source, find};

const L: NodeKind = NodeKind::Line;
const C: NodeKind = NodeKind::Curve;
const O: NodeKind = NodeKind::OffCurve;

type Pts = Vec<(f64, f64, NodeKind)>;

fn shape(points: &Pts) -> Shape {
    Shape::Path(Path::new(true, points.iter().map(|&(x, y, k)| Node::new(x, y, k)).collect()))
}

fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Pts {
    vec![(x0, y0, L), (x1, y0, L), (x1, y1, L), (x0, y1, L)]
}

/// Four cubic arcs, starting at the right-hand extreme; `rot` rotates the node list.
fn ellipse(cx: f64, cy: f64, rx: f64, ry: f64, rot: usize) -> Pts {
    let k = 0.5523;
    let mut v = vec![
        (cx + rx, cy, C),
        (cx + rx, cy + ry * k, O),
        (cx + rx * k, cy + ry, O),
        (cx, cy + ry, C),
        (cx - rx * k, cy + ry, O),
        (cx - rx, cy + ry * k, O),
        (cx - rx, cy, C),
        (cx - rx, cy - ry * k, O),
        (cx - rx * k, cy - ry, O),
        (cx, cy - ry, C),
        (cx + rx * k, cy - ry, O),
        (cx + rx, cy - ry * k, O),
    ];
    v.rotate_left(rot);
    v
}

/// A contour whose first node is smooth, with its incoming handle at `prev` and its outgoing one
/// at `next`.
fn kinky(prev: (f64, f64), next: (f64, f64)) -> Pts {
    vec![(0.0, 0.0, C), (next.0, next.1, O), (300.0, 100.0, O), (300.0, 200.0, C), (-100.0, 200.0, L), (-100.0, 100.0, O), (prev.0, prev.1, O)]
}

/// Regular (wght 400, the origin) and Bold (wght 700).
fn font() -> Font {
    let mut font = Font::new("Test");
    // The fixture's own glyphs only: the new font's alphabet would shadow them.
    font.glyphs.retain(|_| false);
    font.axes = Arc::new(vec![Axis { name: "Weight".into(), tag: "wght".into(), ..Default::default() }]);
    let masters = Arc::make_mut(&mut font.masters);
    if let Some(m) = masters.first_mut() {
        m.location = vec![400.0];
    }
    masters.push(Master { id: "m02".into(), name: "Bold".into(), location: vec![700.0], ..Default::default() });
    font
}

fn layer(master: &str, shapes: Vec<Shape>) -> Layer {
    let mut l = Layer::new(master, 600.0);
    l.shapes = shapes;
    l
}

fn glyph(name: &str, a: Vec<Shape>, b: Vec<Shape>) -> Glyph {
    let mut g = Glyph::new(name);
    g.layers.push(layer("m01", a));
    g.layers.push(layer("m02", b));
    g
}

fn run_glyph(font: &Font, name: &str, id: &str, config: &Config) -> LayerReport {
    let ctx = AuditContext::new(font);
    let g = font.glyph(name).unwrap_or_else(|| panic!("no glyph {name}"));
    let gctx = GlyphCtx::new(&ctx, g, &NoServices);
    audit_glyph(&gctx, &[find(id).unwrap_or_else(|| panic!("no check {id}"))], config)
}

fn run_source(font: &Font, id: &str, services: &dyn AuditServices) -> LayerReport {
    let ctx = AuditContext::new(font);
    let sctx = SourceCtx { audit: &ctx, services };
    audit_source(&sctx, &[find(id).unwrap_or_else(|| panic!("no check {id}"))], &Config::default())
}

/// The engine's answer until M9: the structural report of this module.
struct Structural;

impl AuditServices for Structural {
    fn compatibility(&self, font: &Font) -> Result<CompatibilityReport, Unavailable> {
        Ok(structural_report(font))
    }
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 * (1.0 + b.abs())
}

fn nodes(points: &Pts) -> Path {
    Path::new(true, points.iter().map(|&(x, y, k)| Node::new(x, y, k)).collect())
}

#[test]
fn contour_statistics_match_the_reference() {
    let cases = [
        (
            kinky((-10.0, 0.0), (200.0, 0.0)),
            [247.8911051248108, 91.7445464760355, 116.19396334611957, 199.29665157700234, 105.21497681279197, 25.990179710325982],
            [265.5183609470351, 73.75, 75.0, 335.16520446746006, 177.28105208558367, 40.217569591008115],
        ),
        (
            ellipse(300.0, 300.0, 200.0, 100.0, 0),
            [250.69897590536746, 300.00000000000017, 300.00000000000006, 200.02884592671222, 100.01442296335574, 0.0],
            [268.2957107372386, 315.38461538461536, 300.0, 329.50909108504476, 155.13586690382078, 0.0],
        ),
    ];
    for (points, green, control) in cases {
        let item = path_item(0, &nodes(&points));
        for (got, want) in item.green.iter().zip(green) {
            assert!(close(*got, want), "green {:?} against {green:?}", item.green);
        }
        for (got, want) in item.control.iter().zip(control) {
            assert!(close(*got, want), "control {:?} against {control:?}", item.control);
        }
    }
}

const HEURISTICS: [&str; 4] =
    ["check.variations.contourOrder", "check.variations.wrongStartPoint", "check.variations.kink", "check.variations.underweight"];

/// Every heuristic finding of the four oracle cases: glyph, check, node, score.
#[test]
fn heuristic_tests_match_the_reference() {
    let mut font = font();
    let cases: [(&str, Vec<Pts>, Vec<Pts>); 4] = [
        (
            "order",
            vec![rect(0.0, 0.0, 100.0, 100.0), rect(200.0, 0.0, 500.0, 300.0)],
            vec![rect(200.0, 0.0, 520.0, 320.0), rect(0.0, 0.0, 110.0, 110.0)],
        ),
        ("start", vec![ellipse(300.0, 300.0, 200.0, 100.0, 0)], vec![ellipse(300.0, 300.0, 220.0, 110.0, 3)]),
        ("kink", vec![kinky((-10.0, 0.0), (200.0, 0.0))], vec![kinky((-187.9385, -68.404), (9.396926, 3.420201))]),
        ("square", vec![rect(0.0, 0.0, 100.0, 100.0)], {
            let mut r = rect(0.0, 0.0, 100.0, 100.0);
            r.rotate_left(2);
            vec![r]
        }),
    ];
    for (name, a, b) in &cases {
        font.glyphs.push(glyph(name, a.iter().map(shape).collect(), b.iter().map(shape).collect()));
    }
    let expected: Vec<(&str, &str, Option<usize>, f64)> = vec![
        ("order", "check.variations.contourOrder", None, 0.0031393383240762994),
        ("start", "check.variations.wrongStartPoint", Some(9), 0.03246637912571271),
        ("start", "check.variations.underweight", None, 0.5022727272727269),
        ("kink", "check.variations.kink", Some(0), 0.6427794548062457),
        ("square", "check.variations.wrongStartPoint", Some(2), 0.0),
        ("square", "check.variations.underweight", None, 0.0),
    ];
    for (name, ..) in &cases {
        for id in HEURISTICS {
            let r = run_glyph(&font, name, id, &Config::default());
            assert!(r.skipped.is_empty(), "{name} {id}: {:?}", r.skipped);
            let want: Vec<_> = expected.iter().filter(|e| e.0 == *name && e.1 == id).collect();
            assert_eq!(r.findings.len(), want.len(), "{name} {id}: {:#?}", r.findings);
            for (f, w) in r.findings.iter().zip(want) {
                assert!(close(f.data["score"], w.3), "{name} {id}: score {} against {}", f.data["score"], w.3);
                assert_eq!(f.data.get("node").map(|n| *n as usize), w.2, "{name} {id}");
                assert_eq!(f.location.glyph, *name);
                assert_eq!(f.location.layer, "m02");
                assert_eq!(f.location.masters, vec!["m01".to_string(), "m02".to_string()]);
                if let Some(n) = w.2 {
                    assert_eq!(f.location.points, vec![[0, n]]);
                }
            }
        }
    }
    // The contour-order result proposes the swapped order and names both shapes as moving.
    let r = run_glyph(&font, "order", "check.variations.contourOrder", &Config::default());
    assert_eq!(r.findings[0].location.shapes, vec![1, 0]);
    assert!(r.findings[0].message.contains("2, 1"), "{}", r.findings[0].message);
    // A tolerance of 0 reports nothing; a low kinkiness hides the kink.
    let mut config = Config::default();
    config.set("check.variations.contourOrder", "tolerance", 0.0).unwrap();
    assert!(run_glyph(&font, "order", "check.variations.contourOrder", &config).findings.is_empty());
    config.set("check.variations.kink", "kinkiness", 0.01).unwrap();
    assert!(run_glyph(&font, "kink", "check.variations.kink", &config).findings.is_empty());
}

#[test]
fn overweight_reports_a_halfway_contour_heavier_than_both_masters() {
    let mut font = font();
    // 100 × 10 and 10 × 100 enclose 1,000 units each; halfway, 55 × 55 encloses 3,025.
    font.glyphs.push(glyph("bar", vec![shape(&rect(0.0, 0.0, 100.0, 10.0))], vec![shape(&rect(0.0, 0.0, 10.0, 100.0))]));
    let r = run_glyph(&font, "bar", "check.variations.overweight", &Config::default());
    assert_eq!(r.findings.len(), 1, "{:#?}", r.findings);
    assert!(close(r.findings[0].data["score"], 1000.0 / 3025.0));
    assert!(run_glyph(&font, "bar", "check.variations.underweight", &Config::default()).findings.is_empty());
}

#[test]
fn structural_checks_report_each_mismatch_once() {
    let mut font = font();
    let o = |x: f64| shape(&rect(x, 0.0, x + 100.0, 100.0));
    font.glyphs.push(glyph("A", vec![o(0.0)], vec![o(0.0)]));
    font.glyphs.push(glyph("O", vec![o(0.0), o(200.0)], vec![o(0.0)]));
    font.glyphs.push(glyph("n", vec![o(0.0)], vec![shape(&vec![(0.0, 0.0, L), (100.0, 0.0, L), (100.0, 100.0, L)])]));
    font.glyphs.push(glyph("t", vec![o(0.0)], vec![shape(&vec![(0.0, 0.0, L), (30.0, 0.0, O), (60.0, 0.0, O), (100.0, 100.0, C)])]));
    let mut aacute = glyph("Aacute", vec![Shape::Component(Component::new("A", 0.0, 0.0))], vec![]);
    aacute.layers[0].shapes.push(Shape::Component(Component::new("acute", 0.0, 0.0)));
    aacute.layers[1].shapes.push(Shape::Component(Component::new("A", 0.0, 0.0)));
    font.glyphs.push(aacute);
    let mut acute = Glyph::new("acute");
    acute.layers.push(layer("m01", vec![o(0.0)]));
    font.glyphs.push(acute);
    // Present in two of three masters, so the third master lacks it.
    let masters = Arc::make_mut(&mut font.masters);
    masters.push(Master { id: "m03".into(), name: "Light".into(), location: vec![300.0], ..Default::default() });
    for g in ["A", "O", "n", "t", "Aacute", "acute"] {
        if g == "acute" {
            continue;
        }
        let mut gm = font.glyphs.get_mut(g).unwrap();
        let first = gm.layers[0].clone();
        let mut light = first;
        light.id = "m03".into();
        light.master = "m03".into();
        gm.layers.push(light);
    }
    let mut a = font.glyphs.get_mut("A").unwrap();
    a.layers[0].anchors.push(Anchor::new("top", 50.0, 100.0));
    a.layers[2].anchors.push(Anchor::new("top", 50.0, 100.0));
    drop(a);
    let mut sparse = Glyph::new("sparse");
    sparse.layers.push(layer("m01", vec![o(0.0)]));
    sparse.layers.push(layer("m02", vec![o(0.0)]));
    font.glyphs.push(sparse);

    let expect = [
        ("check.variations.glyphMissingInMaster", "missing", "sparse", ["m01", "m03"]),
        ("check.variations.pathCount", "path-count", "O", ["m01", "m02"]),
        ("check.variations.nodeCount", "node-count", "n", ["m01", "m02"]),
        ("check.variations.componentMismatch", "component-mismatch", "Aacute", ["m01", "m02"]),
        ("check.variations.anchorMismatch", "anchor-mismatch", "A", ["m01", "m02"]),
    ];
    for (id, code, glyph, masters) in expect {
        let r = run_source(&font, id, &Structural);
        assert_eq!(r.findings.len(), 1, "{id}: {:#?}", r.findings);
        let f = &r.findings[0];
        assert_eq!((f.code, f.location.glyph.as_str()), (code, glyph), "{id}");
        assert_eq!(f.location.masters, masters.map(str::to_string).to_vec(), "{id}");
        assert!(!f.message.is_empty());
    }
    // Three nodes change kind (line against two off-curve nodes and a curve).
    let r = run_source(&font, "check.variations.nodeTypeMismatch", &Structural);
    assert_eq!(r.findings.len(), 3, "{:#?}", r.findings);
    assert!(r.findings.iter().all(|f| f.location.glyph == "t" && f.location.shapes == vec![0]));
    assert_eq!(r.findings[0].location.points, vec![[0, 1]]);
    // A sparse glyph can be exempted.
    let mut config = Config::default();
    config.set("check.variations.glyphMissingInMaster", "ignoreMissing", 1.0).unwrap();
    let ctx = AuditContext::new(&font);
    let sctx = SourceCtx { audit: &ctx, services: &Structural };
    assert!(audit_source(&sctx, &[find("check.variations.glyphMissingInMaster").unwrap()], &config).findings.is_empty());
    // Without the service the structural checks skip with the reason.
    let r = run_source(&font, "check.variations.pathCount", &NoServices);
    assert!(r.findings.is_empty());
    assert_eq!(r.skipped.len(), 1);
}

#[test]
fn masters_are_compared_with_their_neighbour_toward_the_default() {
    let mut font = font();
    let masters = Arc::make_mut(&mut font.masters);
    masters.push(Master { id: "m03".into(), name: "Light".into(), location: vec![300.0], ..Default::default() });
    masters.push(Master { id: "m04".into(), name: "Black".into(), location: vec![900.0], ..Default::default() });
    let r = |x: f64| shape(&rect(x, 0.0, x + 100.0, 100.0));
    let mut g = Glyph::new("x");
    for m in ["m01", "m02", "m03"] {
        g.layers.push(layer(m, vec![r(0.0)]));
    }
    g.layers.push(layer("m04", vec![r(0.0), r(200.0)]));
    font.glyphs.push(g);
    let report = structural_report(&font);
    assert_eq!(report.default_master, "m01");
    let x: Vec<&CompatIssue> = report.issues.iter().filter(|i| i.glyph == "x").collect();
    assert_eq!(x.len(), 1, "{x:#?}");
    assert_eq!(x[0].masters, ["m02".to_string(), "m04".to_string()], "Black is compared with Bold, not with Regular");
    assert_eq!(report.compatible.get("x"), Some(&false));
}

#[test]
fn an_open_path_is_reported_and_stops_the_other_tests() {
    let mut font = font();
    let mut open = nodes(&rect(0.0, 0.0, 100.0, 100.0));
    open.closed = false;
    font.glyphs.push(glyph("o", vec![shape(&rect(0.0, 0.0, 100.0, 100.0))], vec![Shape::Path(open)]));
    let r = run_glyph(&font, "o", "check.variations.openPathInMaster", &Config::default());
    assert_eq!(r.findings.len(), 1);
    assert_eq!((r.findings[0].location.layer.as_str(), r.findings[0].location.shapes.clone()), ("m02", vec![0]));
    assert!(r.findings[0].fix.is_some());
    for id in HEURISTICS {
        let r = run_glyph(&font, "o", id, &Config::default());
        assert!(r.findings.is_empty() && r.skipped.is_empty(), "{id}: {r:#?}");
    }
}

#[test]
fn designspace_checks_find_shared_locations_and_extrapolated_instances() {
    let mut font = font();
    let masters = Arc::make_mut(&mut font.masters);
    masters.push(Master { id: "m03".into(), name: "Bold Copy".into(), location: vec![700.0], ..Default::default() });
    let mut g = glyph("a", vec![shape(&rect(0.0, 0.0, 100.0, 100.0))], vec![shape(&rect(0.0, 0.0, 120.0, 100.0))]);
    let mut brace = layer("m01", vec![shape(&rect(0.0, 0.0, 110.0, 100.0))]);
    brace.id = "brace".into();
    brace.info.attrs.coordinates = vec![Some(400.0)];
    g.layers.push(brace);
    font.glyphs.push(g);
    let r = run_source(&font, "check.variations.mastersShareLocation", &NoServices);
    let codes: Vec<(&str, &str)> = r.findings.iter().map(|f| (f.code, f.location.glyph.as_str())).collect();
    assert_eq!(codes, vec![("masters-share-location", ""), ("layer-shares-location", "a")], "{:#?}", r.findings);
    assert_eq!(r.findings[0].location.masters, vec!["m02".to_string(), "m03".to_string()]);

    let instances = Arc::make_mut(&mut font.instances);
    instances.push(Instance { name: "Regular".into(), location: vec![400.0], ..Default::default() });
    instances.push(Instance { name: "Black".into(), location: vec![900.0], ..Default::default() });
    instances.push(Instance { name: "Hidden".into(), location: vec![100.0], exports: false, ..Default::default() });
    let r = run_source(&font, "check.variations.instanceOutsideAxisRange", &NoServices);
    assert_eq!(r.findings.len(), 1, "{:#?}", r.findings);
    let f = &r.findings[0];
    assert_eq!((f.location.instance.as_deref(), f.location.axis.as_deref()), (Some("Black"), Some("wght")));
    assert_eq!((f.data["value"], f.data["maximum"]), (900.0, 700.0));
}

#[test]
fn single_master_sources_and_hostile_outlines_report_nothing_and_never_panic() {
    let mut single = Font::new("Test");
    single.glyphs.push({
        let mut g = Glyph::new("a");
        g.layers.push(layer("m01", vec![shape(&rect(0.0, 0.0, 10.0, 10.0))]));
        g
    });
    for spec in crate::CHECKS.iter().filter(|c| c.id.starts_with("check.variations.")) {
        let r = match spec.scope {
            crate::CheckScope::Glyph => run_glyph(&single, "a", spec.id, &Config::default()),
            _ => run_source(&single, spec.id, &Structural),
        };
        assert!(r.findings.is_empty() && r.skipped.is_empty(), "{}: {r:#?}", spec.id);
    }

    let mut font = font();
    let nan = vec![(f64::NAN, 0.0, L), (1.0, 0.0, L), (1.0, 1.0, L)];
    font.glyphs.push(glyph("nan", vec![shape(&nan)], vec![shape(&nan)]));
    let degenerate = vec![(5.0, 5.0, L), (5.0, 5.0, L), (5.0, 5.0, O), (5.0, 5.0, O), (5.0, 5.0, C)];
    font.glyphs.push(glyph("dot", vec![shape(&degenerate)], vec![shape(&degenerate)]));
    let offs = vec![(0.0, 0.0, O), (100.0, 0.0, O), (100.0, 100.0, O)];
    font.glyphs.push(glyph("offs", vec![shape(&offs)], vec![shape(&offs)]));
    font.glyphs.push(glyph("empty", vec![Shape::Path(Path::new(true, Vec::new()))], vec![Shape::Path(Path::new(true, Vec::new()))]));
    let ids: Vec<&str> =
        crate::CHECKS.iter().filter(|c| c.id.starts_with("check.variations.") && c.scope == crate::CheckScope::Glyph).map(|c| c.id).collect();
    for name in ["nan", "dot", "offs", "empty"] {
        for id in &ids {
            let r = run_glyph(&font, name, id, &Config::default());
            for f in &r.findings {
                assert!(f.data.values().all(|v| v.is_finite()), "{name} {id}: {f:?}");
            }
        }
    }
    assert_eq!(run_glyph(&font, "nan", "check.variations.kink", &Config::default()).skipped.len(), 1);
}

#[test]
fn the_matching_is_the_cheapest() {
    let mut seed = 11u64;
    let mut next = || {
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        ((seed >> 33) % 1000) as f64
    };
    fn permutations(n: usize) -> Vec<Vec<usize>> {
        if n == 0 {
            return vec![Vec::new()];
        }
        let mut out = Vec::new();
        for p in permutations(n - 1) {
            for i in 0..=p.len() {
                let mut q = p.clone();
                q.insert(i, n - 1);
                out.push(q);
            }
        }
        out
    }
    for n in 1..=6 {
        for _ in 0..20 {
            let g: Vec<Vec<f64>> = (0..n).map(|_| (0..n).map(|_| next()).collect()).collect();
            let cost = |m: &[usize]| m.iter().enumerate().map(|(i, &j)| g[i][j]).sum::<f64>();
            let best = permutations(n).iter().map(|p| cost(p)).fold(f64::INFINITY, f64::min);
            let m = min_cost_matching(&g).unwrap();
            assert_eq!(cost(&m), best, "{g:?}");
        }
    }
    assert!(min_cost_matching(&[vec![f64::NAN]]).is_none());
}

#[test]
fn a_glyph_too_costly_to_compare_is_skipped_with_a_reason() {
    // 64 masters with 256 contours each: four matchings of 256 contours for each of 63 pairs,
    // in each of the five tests, before the work budget.
    let mut font = font();
    let masters = Arc::make_mut(&mut font.masters);
    for i in 3..=64 {
        masters.push(Master { id: format!("m{i:02}"), name: format!("M{i}"), location: vec![400.0 + f64::from(i) * 5.0], ..Default::default() });
    }
    let ids: Vec<String> = font.masters.iter().map(|m| m.id.clone()).collect();
    let mut g = Glyph::new("many");
    for (k, id) in ids.iter().enumerate() {
        let shapes = (0..256).map(|i| shape(&rect(f64::from(i) * 20.0 + k as f64, 0.0, f64::from(i) * 20.0 + 10.0, 10.0))).collect();
        g.layers.push(layer(id, shapes));
    }
    font.glyphs.push(g);
    let ctx = AuditContext::new(&font);
    let gctx = GlyphCtx::new(&ctx, font.glyph("many").unwrap(), &NoServices);
    let heuristics = ["contourOrder", "wrongStartPoint", "kink", "underweight", "overweight"];
    let checks: Vec<_> = heuristics.iter().map(|id| find(&format!("check.variations.{id}")).unwrap_or_else(|| panic!("no check {id}"))).collect();
    let report = audit_glyph(&gctx, &checks, &Config::default());
    assert!(report.findings.is_empty(), "{:?}", report.findings);
    assert_eq!(report.skipped.len(), heuristics.len(), "{:?}", report.skipped);
    assert!(report.skipped.iter().all(|s| s.reason.contains("steps")), "{:?}", report.skipped);
    // The open-path test still runs on the same prepared glyph.
    let open = audit_glyph(&gctx, &[find("check.variations.openPathInMaster").unwrap()], &Config::default());
    assert!(open.findings.is_empty() && open.skipped.is_empty(), "{open:?}");
}
