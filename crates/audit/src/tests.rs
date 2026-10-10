use std::collections::HashSet;
use std::sync::Arc;

use fontcraft_model::{Component, Font, Glyph, Layer, Master, Node, NodeKind, Path, Shape};

use super::*;

fn line(x: f64, y: f64) -> Node {
    Node::new(x, y, NodeKind::Line)
}
fn curve(x: f64, y: f64) -> Node {
    Node::new(x, y, NodeKind::Curve)
}
fn smooth(x: f64, y: f64) -> Node {
    let mut n = curve(x, y);
    n.smooth = true;
    n
}
fn off(x: f64, y: f64) -> Node {
    Node::new(x, y, NodeKind::OffCurve)
}
fn qcurve(x: f64, y: f64) -> Node {
    Node::new(x, y, NodeKind::QCurve)
}
fn closed(nodes: Vec<Node>) -> Shape {
    Shape::Path(Path::new(true, nodes))
}
fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Shape {
    Shape::Path(Path::rect(x0, y0, x1, y1))
}

fn glyph(name: &str, unicode: Option<u32>, shapes: Vec<Shape>) -> Glyph {
    let mut g = Glyph::new(name);
    g.unicodes.extend(unicode);
    let mut layer = Layer::new("m01", 600.0);
    layer.shapes = shapes;
    g.layers.push(layer);
    g
}

/// A lowercase glyph with these shapes.
fn o(shapes: Vec<Shape>) -> Glyph {
    glyph("o", Some(0x6F), shapes)
}

fn run_with(font: &Font, g: &Glyph, checks: &[&'static CheckSpec], config: &Config) -> LayerReport {
    let ctx = AuditContext::new(font);
    audit_layer(&ctx, g, &g.layers[0], checks, config)
}

fn run(g: &Glyph, id: &str) -> LayerReport {
    let spec = find(id).unwrap_or_else(|| panic!("no check {id}"));
    run_with(&Font::new("Test"), g, &[spec], &Config::default())
}

/// An "o": outer contour counter-clockwise, counter clockwise, nodes at the extremes, smooth,
/// balanced handles, on the baseline and the x-height.
fn clean_o() -> Glyph {
    let outer = closed(vec![
        smooth(250.0, 0.0),
        off(388.0, 0.0),
        off(500.0, 112.0),
        smooth(500.0, 250.0),
        off(500.0, 388.0),
        off(388.0, 500.0),
        smooth(250.0, 500.0),
        off(112.0, 500.0),
        off(0.0, 388.0),
        smooth(0.0, 250.0),
        off(0.0, 112.0),
        off(112.0, 0.0),
    ]);
    let inner = closed(vec![
        smooth(250.0, 100.0),
        off(167.0, 100.0),
        off(100.0, 167.0),
        smooth(100.0, 250.0),
        off(100.0, 333.0),
        off(167.0, 400.0),
        smooth(250.0, 400.0),
        off(333.0, 400.0),
        off(400.0, 333.0),
        smooth(400.0, 250.0),
        off(400.0, 167.0),
        off(333.0, 100.0),
    ]);
    o(vec![outer, inner])
}

/// For every check, a layer it must report.
fn fixtures() -> Vec<(&'static str, Glyph)> {
    let mut clockwise = Path::rect(0.0, 0.0, 100.0, 100.0);
    clockwise.nodes.reverse();
    vec![
        ("check.outlines.overlap", o(vec![rect(0.0, 0.0, 100.0, 100.0), rect(50.0, 50.0, 150.0, 150.0)])),
        (
            "check.outlines.coincidentPoints",
            o(vec![closed(vec![line(0.0, 0.0), line(100.0, 0.0), line(100.0, 0.0), line(100.0, 100.0), line(0.0, 100.0)])]),
        ),
        (
            "check.outlines.flatCurve",
            o(vec![closed(vec![line(0.0, 0.0), off(30.0, 0.0), off(60.0, 0.0), curve(100.0, 0.0), line(100.0, 100.0), line(0.0, 100.0)])]),
        ),
        (
            "check.outlines.collinearLines",
            o(vec![closed(vec![line(0.0, 0.0), line(50.0, 0.0), line(100.0, 0.0), line(100.0, 100.0), line(0.0, 100.0)])]),
        ),
        ("check.outlines.tinyContour", o(vec![rect(0.0, 0.0, 4.0, 4.0)])),
        ("check.outlines.noContours", o(vec![])),
        ("check.outlines.pathDirection", o(vec![Shape::Path(clockwise)])),
        ("check.outlines.alignmentMiss", o(vec![rect(0.0, 1.0, 100.0, 300.0)])),
        ("check.outlines.jaggySegments", o(vec![closed(vec![line(0.0, 0.0), line(100.0, 0.0), line(0.0, 10.0)])])),
        ("check.outlines.almostOrthogonalLine", o(vec![closed(vec![line(0.0, 0.0), line(100.0, 0.0), line(101.0, 200.0), line(0.0, 200.0)])])),
        (
            "check.outlines.shortSegments",
            o(vec![closed(vec![line(0.0, 0.0), line(100.0, 0.0), off(100.0, 50.0), off(50.0, 100.0), curve(0.0, 100.0), line(0.0, 98.0)])]),
        ),
        ("check.outlines.overlappingSegments", o(vec![rect(0.0, 0.0, 100.0, 100.0), rect(100.0, 0.0, 200.0, 100.0)])),
        (
            "check.outlines.mixedCurveTypes",
            o(vec![closed(vec![line(0.0, 0.0), off(50.0, -50.0), off(100.0, -50.0), curve(100.0, 0.0), off(100.0, 50.0), qcurve(0.0, 100.0)])]),
        ),
        (
            "check.outlines.openCorners",
            o(vec![closed(vec![line(0.0, 0.0), line(110.0, 0.0), line(100.0, -10.0), line(100.0, 100.0), line(0.0, 100.0)])]),
        ),
        ("check.outlines.missingExtremes", o(vec![closed(vec![line(0.0, 0.0), off(0.0, 100.0), off(100.0, 100.0), curve(100.0, 0.0)])])),
        ("check.outlines.inflections", o(vec![closed(vec![line(0.0, 0.0), off(50.0, 100.0), off(50.0, -100.0), curve(100.0, 0.0)])])),
        (
            "check.outlines.almostStraightCurve",
            o(vec![closed(vec![line(0.0, 0.0), off(30.0, 1.0), off(70.0, 1.0), curve(100.0, 0.0), line(100.0, 100.0), line(0.0, 100.0)])]),
        ),
        (
            "check.outlines.almostOrthogonalHandle",
            o(vec![closed(vec![line(0.0, 0.0), off(50.0, 1.0), off(100.0, 50.0), curve(100.0, 100.0), line(0.0, 100.0)])]),
        ),
        ("check.outlines.zeroLengthHandle", o(vec![closed(vec![line(0.0, 0.0), off(0.0, 0.0), off(70.0, 50.0), curve(100.0, 0.0)])])),
        (
            "check.outlines.nearCoincidentPoints",
            o(vec![closed(vec![line(0.0, 0.0), line(100.0, 0.0), line(100.0, 0.5), line(100.0, 100.0), line(0.0, 100.0)])]),
        ),
        ("check.outlines.duplicateContour", o(vec![rect(0.0, 0.0, 100.0, 100.0), rect(0.0, 0.0, 100.0, 100.0)])),
        ("check.outlines.openPath", o(vec![Shape::Path(Path::new(false, vec![line(0.0, 0.0), line(100.0, 0.0), line(100.0, 100.0)]))])),
        ("check.outlines.strayPoint", o(vec![closed(vec![line(10.0, 10.0)])])),
        ("check.outlines.zeroAreaContour", o(vec![closed(vec![line(0.0, 0.0), line(100.0, 0.0)])])),
        ("check.outlines.invalidSegmentStructure", o(vec![closed(vec![line(0.0, 0.0), off(50.0, 50.0), curve(100.0, 0.0)])])),
        ("check.outlines.nodesOffGrid", o(vec![rect(0.0, 0.0, 100.5, 100.0)])),
        ("check.outlines.coordinatesOutOfRange", o(vec![rect(0.0, 0.0, 40_000.0, 100.0)])),
        ("check.outlines.nearAlignmentZone", o(vec![closed(vec![line(0.0, 0.0), line(200.0, 0.0), line(100.0, 517.0)])])),
        (
            "check.outlines.kinkedSmoothNode",
            o(vec![closed(vec![
                line(0.0, 0.0),
                off(0.0, 60.0),
                off(40.0, 100.0),
                smooth(100.0, 100.0),
                off(160.0, 120.0),
                off(200.0, 60.0),
                curve(200.0, 0.0),
            ])]),
        ),
        (
            "check.outlines.unflaggedSmoothNode",
            o(vec![closed(vec![
                line(0.0, 0.0),
                off(0.0, 60.0),
                off(40.0, 100.0),
                curve(100.0, 100.0),
                off(160.0, 100.0),
                off(200.0, 60.0),
                curve(200.0, 0.0),
            ])]),
        ),
        (
            "check.outlines.unbalancedHandles",
            o(vec![closed(vec![line(0.0, 0.0), off(0.0, 90.0), off(95.0, 100.0), curve(100.0, 100.0), line(100.0, 0.0)])]),
        ),
        ("check.outlines.crossedHandles", o(vec![closed(vec![line(0.0, 0.0), off(100.0, 100.0), off(0.0, 100.0), curve(100.0, 0.0)])])),
        (
            "check.outlines.curvatureBreak",
            o(vec![closed(vec![
                line(0.0, 0.0),
                off(0.0, 55.0),
                off(45.0, 100.0),
                smooth(100.0, 100.0),
                off(110.0, 100.0),
                off(200.0, 50.0),
                curve(200.0, 0.0),
            ])]),
        ),
        ("check.outlines.startPointPosition", o(vec![closed(vec![line(100.0, 0.0), line(100.0, 100.0), line(0.0, 100.0), line(0.0, 0.0)])])),
    ]
}

#[test]
fn every_check_reports_its_fixture() {
    let fixtures = fixtures();
    let covered: HashSet<&str> = fixtures.iter().map(|(id, _)| *id).collect();
    for spec in CHECKS {
        assert!(covered.contains(spec.id), "{} has no fixture", spec.id);
    }
    for (id, g) in &fixtures {
        let r = run(g, id);
        assert!(r.skipped.is_empty(), "{id} was skipped: {:?}", r.skipped);
        assert!(!r.findings.is_empty(), "{id} reports nothing on its fixture");
        for f in &r.findings {
            assert_eq!(f.check, *id);
            assert!(find(id).is_some_and(|c| c.codes.contains(&f.code)), "{id} emits `{}`, which its registry entry does not list", f.code);
            assert_eq!(f.location.glyph, "o");
            assert_eq!(f.location.layer, "m01");
            assert!(!f.message.is_empty());
        }
    }
}

#[test]
fn the_clean_glyph_passes_every_check() {
    let g = clean_o();
    let checks: Vec<&'static CheckSpec> = CHECKS.iter().collect();
    let r = run_with(&Font::new("Test"), &g, &checks, &Config::default());
    assert!(r.findings.is_empty(), "{:#?}", r.findings);
    assert!(r.skipped.is_empty(), "{:?}", r.skipped);
}

#[test]
fn findings_carry_addresses_and_fixes() {
    let r = run(&o(vec![rect(0.0, 0.0, 100.0, 100.0), rect(50.0, 50.0, 150.0, 150.0)]), "check.outlines.overlap");
    let f = &r.findings[0];
    assert_eq!(f.location.shapes, vec![0, 1]);
    assert_eq!(f.message, "Shapes 1 and 2 cross.", "messages count from 1; addresses from 0");
    assert!(f.location.point.is_some());
    assert_eq!(f.fix.map(|x| x.command), Some(Some("path.removeOverlap")));

    // One retracted handle is reported without a fix; both retracted can be tidied into a line.
    let one = run(&o(vec![closed(vec![line(0.0, 0.0), off(0.0, 0.0), off(70.0, 50.0), curve(100.0, 0.0)])]), "check.outlines.zeroLengthHandle");
    assert_eq!(one.findings[0].location.points, vec![[0, 1]]);
    assert_eq!(one.findings[0].message, "Handle 2 of path 1 sits on its node.");
    assert!(one.findings[0].fix.is_none());
    let both = run(
        &o(vec![closed(vec![line(0.0, 0.0), off(0.0, 0.0), off(100.0, 0.0), curve(100.0, 0.0), line(50.0, 80.0)])]),
        "check.outlines.zeroLengthHandle",
    );
    assert_eq!(both.findings[0].code, "both-retracted");
    assert!(both.findings[0].fix.is_some());
}

#[test]
fn near_misses_are_not_reported() {
    // A space has no contours on purpose, whatever its name.
    for (name, u) in [("space", Some(0x20)), ("nbspace", None), ("uni2009", None), ("space.tf", None)] {
        assert!(run(&glyph(name, u, vec![]), "check.outlines.noContours").findings.is_empty(), "{name}");
    }
    // A corner between two lines is not collinear; a hole the right way round is fine.
    assert!(run(&o(vec![rect(0.0, 0.0, 100.0, 100.0)]), "check.outlines.collinearLines").findings.is_empty());
    let mut hole = Path::rect(20.0, 20.0, 80.0, 80.0);
    hole.nodes.reverse();
    let nested = o(vec![rect(0.0, 0.0, 100.0, 100.0), Shape::Path(hole)]);
    assert!(run(&nested, "check.outlines.pathDirection").findings.is_empty());
    assert!(run(&nested, "check.outlines.overlap").findings.is_empty());
    // The same hole running the same way as its outer contour overlaps it.
    let same = o(vec![rect(0.0, 0.0, 100.0, 100.0), rect(20.0, 20.0, 80.0, 80.0)]);
    assert_eq!(run(&same, "check.outlines.overlap").findings[0].code, "nested-same-direction");
    assert_eq!(run(&same, "check.outlines.pathDirection").findings[0].code, "counter-same-direction");
    // Points on a metric line, or far from one, are fine.
    assert!(run(&o(vec![rect(0.0, 0.0, 100.0, 500.0)]), "check.outlines.alignmentMiss").findings.is_empty());
    assert!(run(&o(vec![rect(0.0, 10.0, 100.0, 400.0)]), "check.outlines.alignmentMiss").findings.is_empty());
}

#[test]
fn heuristics_are_skipped_on_variable_sources() {
    let mut font = Font::new("Test");
    let second = Master { id: "m02".into(), name: "Bold".into(), ..Default::default() };
    Arc::make_mut(&mut font.masters).push(second);
    let g = o(vec![closed(vec![line(0.0, 0.0), line(100.0, 0.0), line(0.0, 10.0)])]);
    let spec = find("check.outlines.jaggySegments").unwrap();
    let r = run_with(&font, &g, &[spec], &Config::default());
    assert!(r.findings.is_empty());
    assert_eq!(r.skipped.len(), 1);
}

#[test]
fn thresholds_are_validated_and_used() {
    let mut config = Config::default();
    assert!(config.set("check.outlines.tinyContour", "minArea", f64::NAN).is_err());
    assert!(config.set("check.outlines.tinyContour", "minArea", -1.0).is_err());
    assert!(config.set("check.outlines.tinyContour", "nope", 1.0).is_err());
    assert!(config.set("check.outlines.nope", "minArea", 1.0).is_err());
    assert_eq!(config, Config::default(), "a failed set changes nothing");
    config.set("check.outlines.tinyContour", "minArea", 10.0).unwrap();
    assert_eq!(config.get("check.outlines.tinyContour", "minArea"), Some(10.0));
    let spec = find("check.outlines.tinyContour").unwrap();
    let r = run_with(&Font::new("Test"), &o(vec![rect(0.0, 0.0, 4.0, 4.0)]), &[spec], &config);
    assert!(r.findings.is_empty(), "16 square units is not below 10");
}

#[test]
fn profiles_select_their_checks() {
    assert_eq!(select(Some(Profile::Outlines), &[], &[]).len(), 28);
    assert_eq!(select(Some(Profile::Universal), &[], &[]).len(), 29);
    assert_eq!(select(Some(Profile::Strict), &[], &[]).len(), 34);
    assert!(select(Some(Profile::Opentype), &[], &[]).is_empty());
    let open: Vec<&str> = select(None, &["check.outlines.open*"], &[]).iter().map(|c| c.id).collect();
    assert_eq!(open, vec!["check.outlines.openCorners", "check.outlines.openPath"]);
    assert_eq!(select(None, &["check.outlines.*"], &["check.outlines.open*"]).len(), 32);
}

#[test]
fn registry_matches_the_inventory() {
    let text = include_str!("../../../spec/inventory/21-qa-checks.toml");
    let mut rows = Vec::new();
    for block in text.split("[[") {
        let get = |key: &str| {
            let prefix = format!("{key} = \"");
            block.lines().find_map(|l| l.strip_prefix(prefix.as_str()).and_then(|r| r.strip_suffix('"')).map(str::to_string))
        };
        if let (Some(id), Some(severity), Some(milestone)) = (get("id"), get("severity"), get("milestone"))
            && id.starts_with("check.outlines.")
            && milestone == "M3"
        {
            rows.push((id, severity));
        }
    }
    assert_eq!(rows.len(), CHECKS.len());
    let mut ids = HashSet::new();
    for (id, severity) in rows {
        let spec = find(&id).unwrap_or_else(|| panic!("{id} is not in the registry"));
        assert_eq!(spec.severity.as_str(), severity, "{id}");
        assert!(ids.insert(id));
    }
    for spec in CHECKS {
        for t in spec.thresholds {
            assert!(t.min <= t.default && t.default <= t.max, "{} {}", spec.id, t.name);
        }
    }
}

#[test]
fn collapse_summarises_capped_checks() {
    let finding = |check: &'static str| Finding {
        check,
        code: "x",
        severity: Severity::Warn,
        message: "m".into(),
        location: Location::default(),
        data: BTreeMap::new(),
        fix: None,
    };
    let mut all: Vec<Finding> = (0..150).map(|_| finding("check.outlines.collinearLines")).collect();
    all.extend((0..150).map(|_| finding("check.outlines.openPath")));
    all.extend((0..5).map(|_| finding("check.outlines.inflections")));
    let out = collapse(all, DEFAULT_MAX_RESULTS);
    let summary: Vec<&Finding> = out.iter().filter(|f| f.check == "check.outlines.collinearLines").collect();
    assert_eq!(summary.len(), 1);
    assert_eq!(summary[0].code, "too-many-results");
    assert_eq!(summary[0].data.get("count"), Some(&150.0));
    assert_eq!(out.iter().filter(|f| f.check == "check.outlines.openPath").count(), 150, "not capped");
    assert_eq!(out.iter().filter(|f| f.check == "check.outlines.inflections").count(), 5);
}

#[test]
fn components_take_part_in_the_overlap_test() {
    let mut font = Font::new("Test");
    font.glyphs.push(glyph("bar", None, vec![rect(0.0, 0.0, 100.0, 300.0)]));
    let g = o(vec![rect(50.0, 100.0, 200.0, 200.0), Shape::Component(Component::new("bar", 0.0, 0.0))]);
    let spec = find("check.outlines.overlap").unwrap();
    let r = run_with(&font, &g, &[spec], &Config::default());
    assert_eq!(r.findings.len(), 1);
    assert_eq!(r.findings[0].location.shapes, vec![0, 1]);
    let apart = o(vec![rect(150.0, 100.0, 200.0, 200.0), Shape::Component(Component::new("bar", 0.0, 0.0))]);
    assert!(run_with(&font, &apart, &[spec], &Config::default()).findings.is_empty());
}

/// A deterministic stream of numbers in [-1, 1).
fn noise(seed: &mut u64) -> f64 {
    *seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
    ((*seed >> 11) as f64 / (1u64 << 53) as f64) * 2.0 - 1.0
}

#[test]
fn hostile_layers_do_not_panic() {
    let mut font = Font::new("Test");
    font.glyphs.push(glyph("loop", None, vec![Shape::Component(Component::new("loop", 0.0, 0.0))]));
    font.glyphs.push(glyph("bar", None, vec![rect(0.0, 0.0, 100.0, 300.0)]));
    let mut seed = 7u64;
    let mut random = Vec::new();
    for i in 0..15_000 {
        let (x, y) = (noise(&mut seed) * 1000.0, noise(&mut seed) * 1000.0);
        random.push(match i % 5 {
            1 | 2 => off(x, y),
            3 => curve(x, y),
            _ => line(x, y),
        });
    }
    let mut many = Vec::new();
    for _ in 0..30 {
        let nodes: Vec<Node> = (0..300)
            .map(|i| {
                let (x, y) = (noise(&mut seed) * 500.0, noise(&mut seed) * 500.0);
                if i % 3 == 0 {
                    line(x, y)
                } else if i % 3 == 1 {
                    off(x, y)
                } else {
                    qcurve(x, y)
                }
            })
            .collect();
        many.push(closed(nodes));
    }
    let layers: Vec<(&str, Vec<Shape>)> = vec![
        ("nan", vec![closed(vec![line(f64::NAN, 0.0), line(100.0, f64::NAN), off(f64::INFINITY, 1.0), curve(0.0, f64::NEG_INFINITY)])]),
        ("huge", vec![rect(-1e300, -1e300, 1e300, 1e300), rect(0.0, 0.0, 1e6, 1e6), rect(0.0, 0.0, 9e5, 1e-300)]),
        ("empty", vec![closed(vec![]), Shape::Path(Path::new(false, vec![]))]),
        (
            "off-only",
            vec![
                closed(vec![off(0.0, 0.0)]),
                Shape::Path(Path::new(false, vec![off(0.0, 0.0), off(1.0, 1.0)])),
                closed(vec![off(0.0, 0.0), off(100.0, 0.0), off(50.0, 80.0)]),
            ],
        ),
        (
            "malformed",
            vec![
                closed(vec![off(0.0, 0.0), off(1.0, 0.0), off(2.0, 0.0), off(3.0, 0.0), off(4.0, 0.0), curve(5.0, 5.0)]),
                closed(vec![qcurve(0.0, 0.0), qcurve(10.0, 0.0)]),
                Shape::Path(Path::new(false, vec![off(0.0, 0.0), line(1.0, 1.0), off(2.0, 2.0)])),
            ],
        ),
        ("single-curve", vec![closed(vec![curve(0.0, 0.0)]), closed(vec![curve(0.0, 0.0), off(10.0, 10.0), off(20.0, 0.0)])]),
        ("degenerate", vec![closed(vec![line(0.0, 0.0); 50]), closed(vec![line(0.0, 0.0), off(0.0, 0.0), off(0.0, 0.0), curve(0.0, 0.0)])]),
        (
            "components",
            vec![
                Shape::Component(Component::new("missing", 0.0, 0.0)),
                Shape::Component(Component::new("loop", 0.0, 0.0)),
                Shape::Component(Component::new("bar", f64::NAN, f64::INFINITY)),
            ],
        ),
        ("random", vec![closed(random)]),
        ("many", many),
        ("too-many-nodes", vec![closed((0..MAX_LAYER_NODES + 1).map(|i| line(i as f64, 0.0)).collect())]),
    ];
    let checks: Vec<&'static CheckSpec> = CHECKS.iter().collect();
    let mut loose = Config::default();
    let mut strict = Config::default();
    for spec in CHECKS {
        for t in spec.thresholds {
            loose.set(spec.id, t.name, t.max).unwrap();
            strict.set(spec.id, t.name, t.min).unwrap();
        }
    }
    for (name, shapes) in layers {
        let g = o(shapes);
        for config in [&Config::default(), &loose, &strict] {
            let r = run_with(&font, &g, &checks, config);
            for f in &r.findings {
                assert!(
                    f.code == TOO_MANY_RESULTS || find(f.check).is_some_and(|c| c.codes.contains(&f.code)),
                    "{name}: {} emits unlisted `{}`",
                    f.check,
                    f.code
                );
            }
            for spec in CHECKS {
                assert!(r.findings.iter().filter(|f| f.check == spec.id).count() <= MAX_FINDINGS_PER_CHECK, "{name} {}", spec.id);
            }
            if name == "too-many-nodes" {
                assert!(r.findings.is_empty());
                assert_eq!(r.skipped.len(), CHECKS.len());
            }
            for f in &r.findings {
                assert!(f.location.point.is_none_or(|p| p.iter().all(|v| v.is_finite())), "{name}: {f:?}");
                assert!(f.data.values().all(|v| v.is_finite()), "{name}: {f:?}");
            }
        }
    }
}
