use fontcraft_geom::{BezPath, Rect};
use fontcraft_model::{Anchor, Component, Guide, Node, NodeKind, Path, Shape};

use crate::clipboard::{self, ClipboardError, Fragment, FragmentGuide, FragmentShape, GuideScope};
use crate::{Placement, SvgError, bez_to_svg, glyph_name_for_file, looks_like_svg, read_svg, shapes_to_svg};

const UNSCALED: Placement = Placement::Unscaled { top: 0.0 };

fn svg(body: &str) -> String {
    format!(r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" viewBox="0 0 100 100">{body}</svg>"#)
}

fn points(p: &Path) -> Vec<(f64, f64)> {
    let mut v: Vec<(f64, f64)> = p.nodes.iter().filter(|n| n.on_curve()).map(|n| (n.x, n.y)).collect();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v
}

fn near(a: f64, b: f64, tolerance: f64) -> bool {
    (a - b).abs() <= tolerance
}

#[test]
fn a_rect_becomes_a_closed_path_with_y_flipped_and_its_fill_reported() {
    let out = read_svg(svg(r#"<rect x="10" y="20" width="30" height="40" fill="red"/>"#).as_bytes(), UNSCALED).unwrap();
    assert_eq!(out.paths.len(), 1);
    let p = &out.paths[0];
    assert!(p.closed);
    assert!(p.nodes.iter().all(|n| n.kind == NodeKind::Line && !n.smooth));
    assert_eq!(points(p), vec![(10.0, -60.0), (10.0, -20.0), (40.0, -60.0), (40.0, -20.0)]);
    assert_eq!(out.view_box, Some([0.0, 0.0, 100.0, 100.0]));
    assert_eq!(out.dropped.attributes.get("fill"), Some(&1));
}

#[test]
fn a_circle_fits_the_em_as_smooth_cubic_curves() {
    let doc = svg(r#"<circle cx="50" cy="50" r="50"/>"#);
    let out = read_svg(doc.as_bytes(), Placement::FitEm { units_per_em: 1000.0, ascender: 800.0 }).unwrap();
    assert_eq!(out.paths.len(), 1);
    let p = &out.paths[0];
    assert!(p.closed);
    assert!(p.nodes.iter().any(|n| n.kind == NodeKind::Curve));
    assert!(p.nodes.iter().all(|n| n.kind != NodeKind::Line && n.kind != NodeKind::QCurve));
    for n in p.nodes.iter().filter(|n| n.on_curve()) {
        assert!(near((n.x - 500.0).hypot(n.y - 300.0), 500.0, 0.5), "({}, {}) is not on the circle", n.x, n.y);
        assert!(n.smooth, "({}, {}) should be smooth", n.x, n.y);
    }
    let b = p.to_bez().unwrap();
    let r = fontcraft_geom::Shape::bounding_box(&b);
    assert!(near(r.x0, 0.0, 0.5) && near(r.x1, 1000.0, 0.5) && near(r.y0, -200.0, 0.5) && near(r.y1, 800.0, 0.5), "{r:?}");
}

#[test]
fn a_view_box_origin_is_the_top_left_of_the_em() {
    let doc = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="-50 -50 100 100"><rect x="-50" y="-50" width="100" height="100"/></svg>"#;
    let out = read_svg(doc.as_bytes(), Placement::FitEm { units_per_em: 1000.0, ascender: 800.0 }).unwrap();
    assert_eq!(points(&out.paths[0]), vec![(0.0, -200.0), (0.0, 800.0), (1000.0, -200.0), (1000.0, 800.0)]);
}

#[test]
fn arcs_in_path_data_become_cubic_curves() {
    let out = read_svg(svg(r#"<path d="M 0 0 A 50 50 0 0 1 100 0 Z"/>"#).as_bytes(), UNSCALED).unwrap();
    let p = &out.paths[0];
    assert!(p.closed);
    assert!(p.nodes.iter().filter(|n| n.kind == NodeKind::Curve).count() >= 2);
    let r = fontcraft_geom::Shape::bounding_box(&p.to_bez().unwrap());
    assert!(near(r.x0, 0.0, 0.01) && near(r.x1, 100.0, 0.01) && near(r.y1.abs().max(r.y0.abs()), 50.0, 0.1), "{r:?}");
    let ends: Vec<(f64, f64)> = p.nodes.iter().filter(|n| n.on_curve()).map(|n| (n.x, n.y)).collect();
    assert!(ends.contains(&(0.0, 0.0)) && ends.contains(&(100.0, 0.0)), "{ends:?}");
    // The joins between the arc's pieces are smooth; the corners at the chord are not.
    assert!(p.nodes.iter().any(|n| n.smooth));
}

#[test]
fn nested_transforms_and_use_references_are_applied() {
    let doc = svg(r##"<defs><rect id="r" x="0" y="0" width="10" height="5"/></defs>
        <g transform="translate(100 0) scale(2)"><g transform="rotate(90)"><use xlink:href="#r"/></g></g>
        <use href="#r" x="-20" y="-30"/>"##);
    let out = read_svg(doc.as_bytes(), UNSCALED).unwrap();
    assert_eq!(out.paths.len(), 2);
    assert_eq!(points(&out.paths[0]), vec![(90.0, -20.0), (90.0, 0.0), (100.0, -20.0), (100.0, 0.0)]);
    assert_eq!(points(&out.paths[1]), vec![(-20.0, 25.0), (-20.0, 30.0), (-10.0, 25.0), (-10.0, 30.0)]);
}

#[test]
fn a_stroked_path_keeps_its_centre_line_and_styling_and_text_are_reported() {
    let doc = svg(r#"<style>.a{fill:blue}</style>
        <path class="a" d="M0 0 L10 10" stroke="black" stroke-width="3" fill="none"/>
        <text x="0" y="50">Hi</text><image href="file:///etc/hostname" width="10" height="10"/>"#);
    let out = read_svg(doc.as_bytes(), UNSCALED).unwrap();
    assert_eq!(out.paths.len(), 1);
    let p = &out.paths[0];
    assert!(!p.closed);
    assert_eq!(p.nodes.iter().map(|n| (n.x, n.y)).collect::<Vec<_>>(), vec![(0.0, 0.0), (10.0, -10.0)]);
    for a in ["stroke", "stroke-width", "fill", "class"] {
        assert_eq!(out.dropped.attributes.get(a), Some(&1), "{a}");
    }
    for e in ["text", "image", "style"] {
        assert_eq!(out.dropped.elements.get(e), Some(&1), "{e}");
    }
    let summary = out.dropped.summary().unwrap();
    assert!(summary.contains("stroke") && summary.contains("text"), "{summary}");
}

fn curve_path() -> Path {
    let n = Node::new;
    Path::new(
        true,
        vec![n(0.0, 0.0, NodeKind::Curve), n(100.0, 0.0, NodeKind::Line), n(100.0, 50.5, NodeKind::OffCurve), n(0.0, 50.5, NodeKind::OffCurve)],
    )
}

fn shape_of(p: &Path) -> Vec<(f64, f64, NodeKind)> {
    p.nodes.iter().map(|n| (n.x, n.y, n.kind)).collect()
}

#[test]
fn a_layer_written_as_svg_reads_back_as_the_same_paths() {
    let rect = Path::rect(0.0, 0.0, 50.0, 60.0);
    let shapes = vec![Shape::Path(rect.clone()), Shape::Path(curve_path()), Shape::Component(Component::new("B", 200.0, 10.0))];
    let b_outline = |c: &Component| {
        (c.glyph == "B").then(|| {
            let mut o = Path::rect(0.0, 0.0, 20.0, 20.0).to_bez().unwrap();
            o.apply_affine(c.affine());
            o
        })
    };
    let text = shapes_to_svg(&shapes, &b_outline, None).unwrap();
    assert!(looks_like_svg(&text));
    assert!(text.contains(r#"viewBox="0 -60 220 60""#), "{text}");
    let back = read_svg(text.as_bytes(), UNSCALED).unwrap();
    assert!(back.dropped.is_empty(), "{:?}", back.dropped);
    let got: Vec<_> = back.paths.iter().map(shape_of).collect();
    let want = vec![shape_of(&rect), shape_of(&curve_path()), shape_of(&Path::rect(200.0, 10.0, 220.0, 30.0))];
    assert_eq!(got, want, "{text}");
}

#[test]
fn an_empty_outline_is_still_a_valid_document() {
    let text = bez_to_svg(&BezPath::new(), None);
    assert!(read_svg(text.as_bytes(), UNSCALED).unwrap().paths.is_empty(), "{text}");
    let framed = bez_to_svg(&BezPath::new(), Some(Rect::new(0.0, -200.0, 500.0, 800.0)));
    assert!(framed.contains(r#"viewBox="0 -800 500 1000""#), "{framed}");
}

fn sample_fragment() -> Fragment {
    let mut path = curve_path();
    path.nodes[0].smooth = true;
    let guide = Guide { x: 10.0, y: 500.0, angle: 90.0, name: Some("stem".into()), ..Guide::default() };
    Fragment {
        shapes: vec![FragmentShape::Path(path), FragmentShape::Component(Component::new("acutecomb", 120.0, 0.0))],
        anchors: vec![Anchor::new("top", 250.0, 700.0)],
        guides: vec![FragmentGuide { scope: GuideScope::Local, guide: guide.clone() }, FragmentGuide { scope: GuideScope::Global, guide }],
    }
}

#[test]
fn a_fragment_survives_encode_and_decode() {
    let f = sample_fragment();
    let text = clipboard::encode(&f).unwrap();
    assert!(clipboard::looks_like_fragment(&text) && !looks_like_svg(&text));
    assert!(text.starts_with(r#"{"format":"fontcraft-clipboard","version":1,"#), "{text}");
    assert_eq!(clipboard::decode(&text).unwrap(), f);
    let shapes: Vec<Shape> = f.shapes.into_iter().map(FragmentShape::into_shape).collect();
    assert!(matches!(shapes.as_slice(), [Shape::Path(_), Shape::Component(_)]));
    assert_eq!(clipboard::decode(r#"{"format":"fontcraft-clipboard","version":1}"#).unwrap(), Fragment::default());
}

#[test]
fn foreign_newer_and_damaged_fragments_are_refused() {
    let text = clipboard::encode(&sample_fragment()).unwrap();
    for n in 0..text.len() {
        if let Some(prefix) = text.get(..n) {
            assert!(clipboard::decode(prefix).is_err(), "{prefix}");
        }
    }
    assert_eq!(clipboard::decode("[1, 2]"), Err(ClipboardError::NotAFragment));
    assert_eq!(clipboard::decode(r#"{"format":"other","version":1}"#), Err(ClipboardError::NotAFragment));
    assert_eq!(clipboard::decode(r#"{"format":"fontcraft-clipboard","version":7}"#), Err(ClipboardError::TooNew(7)));
    assert!(matches!(clipboard::decode("not json"), Err(ClipboardError::NotJson(_))));
    let huge = r#"{"format":"fontcraft-clipboard","version":1,"anchors":[{"name":"a","x":1e300,"y":0}]}"#;
    assert!(matches!(clipboard::decode(huge), Err(ClipboardError::Invalid(_))));
    let overflow = r#"{"format":"fontcraft-clipboard","version":1,"anchors":[{"name":"a","x":1e400,"y":0}]}"#;
    assert!(clipboard::decode(overflow).is_err());
    let bad_path = r#"{"format":"fontcraft-clipboard","version":1,"shapes":[{"path":{"nodes":[{"x":0,"y":0,"type":"curve"}]}}]}"#;
    assert!(matches!(clipboard::decode(bad_path), Err(ClipboardError::Invalid(_))));
    // Unknown fields are skipped without recursion, however deep.
    let deep = format!(r#"{{"format":"fontcraft-clipboard","version":1,"x":{}{}}}"#, "[".repeat(100_000), "]".repeat(100_000));
    assert_eq!(clipboard::decode(&deep), Ok(Fragment::default()));
    let deep_shape = format!(r#"{{"format":"fontcraft-clipboard","version":1,"shapes":{}{}}}"#, "[".repeat(100_000), "]".repeat(100_000));
    assert!(clipboard::decode(&deep_shape).is_err());
    let many = format!(
        r#"{{"format":"fontcraft-clipboard","version":1,"anchors":[{}]}}"#,
        std::iter::repeat_n(r#"{"name":"a","x":0,"y":0}"#, clipboard::MAX_FRAGMENT_ANCHORS + 1).collect::<Vec<_>>().join(",")
    );
    assert!(matches!(clipboard::decode(&many), Err(ClipboardError::TooMany { what: "anchors", .. })));
    assert!(matches!(clipboard::decode(&" ".repeat(clipboard::MAX_FRAGMENT_BYTES + 1)), Err(ClipboardError::TooLarge { .. })));
}

#[test]
fn hostile_svg_fails_with_a_message() {
    let deep = svg(&format!("{}{}", "<g>".repeat(10_000), "</g>".repeat(10_000)));
    assert_eq!(read_svg(deep.as_bytes(), UNSCALED), Err(SvgError::TooDeep));

    let huge = svg(r#"<rect width="1e300" height="10"/>"#);
    assert_eq!(read_svg(huge.as_bytes(), UNSCALED), Err(SvgError::NumberOutOfRange));
    let overflow = svg(r#"<path d="M0 0 L 1e39 0 L 0 10 Z"/>"#);
    assert_eq!(read_svg(overflow.as_bytes(), UNSCALED), Err(SvgError::NumberOutOfRange));
    let placed_far = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 1 1"><rect width="10" height="10"/></svg>"#;
    assert_eq!(read_svg(placed_far.as_bytes(), Placement::FitEm { units_per_em: 1e6, ascender: 0.0 }), Err(SvgError::CoordinateOutOfRange));

    let laughs = r#"<?xml version="1.0"?>
<!DOCTYPE svg [
<!ENTITY a "lollollollollollollollollollol">
<!ENTITY b "&a;&a;&a;&a;&a;&a;&a;&a;&a;&a;">
<!ENTITY c "&b;&b;&b;&b;&b;&b;&b;&b;&b;&b;">
<!ENTITY d "&c;&c;&c;&c;&c;&c;&c;&c;&c;&c;">
<!ENTITY e "&d;&d;&d;&d;&d;&d;&d;&d;&d;&d;">
<!ENTITY f "&e;&e;&e;&e;&e;&e;&e;&e;&e;&e;">
<!ENTITY g "&f;&f;&f;&f;&f;&f;&f;&f;&f;&f;">
<!ENTITY h "&g;&g;&g;&g;&g;&g;&g;&g;&g;&g;">
<!ENTITY i "&h;&h;&h;&h;&h;&h;&h;&h;&h;&h;">
]>
<svg xmlns="http://www.w3.org/2000/svg"><desc>&i;</desc><rect width="10" height="10"/></svg>"#;
    assert!(matches!(read_svg(laughs.as_bytes(), UNSCALED), Err(SvgError::Parse(_))));

    // Each level uses the one below twice: 2^40 rectangles once expanded.
    let mut bomb = String::from(r#"<defs><rect id="l0" width="1" height="1"/>"#);
    for i in 1..=40 {
        bomb.push_str(&format!(r##"<g id="l{i}"><use href="#l{0}"/><use href="#l{0}"/></g>"##, i - 1));
    }
    bomb.push_str(r##"</defs><use href="#l40"/>"##);
    assert!(matches!(read_svg(svg(&bomb).as_bytes(), UNSCALED), Err(SvgError::TooManyExpanded | SvgError::TooDeep)));

    let looped = svg(r##"<g id="a"><use href="#a"/></g>"##);
    assert_eq!(read_svg(looped.as_bytes(), UNSCALED), Err(SvgError::TooDeep));

    let big = svg(&format!(r#"<path d="M0 0{}Z"/>"#, " L1 1".repeat(2_100_000)));
    assert!(matches!(read_svg(big.as_bytes(), UNSCALED), Err(SvgError::TooLarge { .. })));

    assert_eq!(read_svg(b"\x1f\x8b\x08\x00", UNSCALED), Err(SvgError::Compressed));
    assert_eq!(read_svg(b"\xff\xfe<svg/>", UNSCALED), Err(SvgError::NotUtf8));
    assert_eq!(read_svg(br#"<html xmlns="http://www.w3.org/1999/xhtml"/>"#, UNSCALED), Err(SvgError::NoSvgRoot));
    assert!(read_svg(svg("").as_bytes(), Placement::Unscaled { top: f64::NAN }).is_err());
}

#[test]
fn deep_markup_is_rejected_before_the_xml_parser_recurses() {
    let deep = format!("{}{}", "<g>".repeat(10_000), "</g>".repeat(10_000));
    let in_entity = format!(r#"<!DOCTYPE svg [<!ENTITY deep "{deep}">]><svg xmlns="http://www.w3.org/2000/svg">&deep;</svg>"#);
    assert_eq!(read_svg(in_entity.as_bytes(), UNSCALED), Err(SvgError::TooDeep));
    // A quote in a comment or an attribute value hides nothing from the depth check.
    let hidden = format!(r#"<!DOCTYPE svg [<!-- don't -->]><svg xmlns="http://www.w3.org/2000/svg"><g id="/>">{deep}</g></svg>"#);
    assert_eq!(read_svg(hidden.as_bytes(), UNSCALED), Err(SvgError::TooDeep));
    // Markup inside comments and CDATA is not nesting.
    let quiet = svg(&format!(r#"<!--{deep}--><g><![CDATA[{deep}]]><rect width="10" height="10"/></g>"#));
    assert_eq!(read_svg(quiet.as_bytes(), UNSCALED).unwrap().paths.len(), 1);
}

#[test]
fn truncated_svg_never_panics() {
    let doc = svg(r##"<g transform="matrix(1 0 0 -1 0 100)"><path d="M10 10 C 20 30 40 30 50 10 Q 60 0 70 10 A 5 5 0 1 1 80 10 Z" fill="red"/>
        <circle cx="50" cy="50" r="20"/><use href="#x"/></g><polygon id="x" points="0,0 10,0 5,8"/>"##);
    assert_eq!(read_svg(doc.as_bytes(), UNSCALED).unwrap().paths.len(), 4);
    for n in 0..doc.len() {
        let _ = read_svg(&doc.as_bytes()[..n], UNSCALED);
    }
}

#[test]
fn file_names_map_to_glyph_names() {
    assert_eq!(glyph_name_for_file("art/A.alt.svg"), "A.alt");
    assert_eq!(glyph_name_for_file("C:\\art\\a.SVG"), "a");
    assert_eq!(glyph_name_for_file(".svg"), ".svg");
}
