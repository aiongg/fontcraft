use fontcraft_compile::{Options, compile_ttf};
use fontcraft_model::{Font, NodeKind, Path, Shape};

use super::*;

fn sample() -> Font {
    let mut f = Font::new("Back Again");
    f.glyph_mut("A").unwrap().layers[0].shapes.push(Shape::Path(Path::rect(60.0, 0.0, 560.0, 700.0)));
    f.glyph_mut("A").unwrap().layers[0].width = 620.0;
    f
}

#[test]
fn a_compiled_font_reads_back() {
    let bytes = compile_ttf(&sample(), &Options::default()).unwrap().bytes;
    assert_eq!(sniff(&bytes), Some("TrueType"));
    let d = decompile(&bytes, None).unwrap();
    let f = d.font;
    assert_eq!(f.source.format, fontcraft_model::SourceFormat::Binary);
    assert_eq!(f.family_name, "Back Again");
    assert_eq!(f.default_master().unwrap().name, "Regular");
    let a = f.glyph("A").unwrap();
    assert_eq!((a.unicodes.clone(), a.layers[0].width), (vec![0x41], 620.0));
    let Shape::Path(p) = &a.layers[0].shapes[0] else { panic!("a path") };
    assert_eq!(p.nodes.len(), 4);
    assert!(p.nodes.iter().all(|n| n.kind == NodeKind::Line));
    let m = f.default_master().unwrap();
    assert_eq!(f.metric_pos(m, fontcraft_model::metric::ASCENDER, 0.0), 800.0);
    // The glyph order is recorded for a later export.
    assert!(f.extra.get(Family::Glyphs, "customParameters").is_some());
}

#[test]
fn quadratic_contours_stay_quadratic() {
    // A cubic oval compiles to quadratic curves; reading them back must not turn them cubic.
    let mut f = sample();
    let n = Node::new;
    let (cx, cy, r, k) = (300.0, 300.0, 250.0, 0.5523);
    f.glyph_mut("O").unwrap().layers[0].shapes.push(Shape::Path(Path::new(
        true,
        vec![
            n(cx + r, cy, NodeKind::Curve),
            n(cx + r, cy + r * k, NodeKind::OffCurve),
            n(cx + r * k, cy + r, NodeKind::OffCurve),
            n(cx, cy + r, NodeKind::Curve),
            n(cx - r * k, cy + r, NodeKind::OffCurve),
            n(cx - r, cy + r * k, NodeKind::OffCurve),
            n(cx - r, cy, NodeKind::Curve),
            n(cx - r, cy - r * k, NodeKind::OffCurve),
            n(cx - r * k, cy - r, NodeKind::OffCurve),
            n(cx, cy - r, NodeKind::Curve),
            n(cx + r * k, cy - r, NodeKind::OffCurve),
            n(cx + r, cy - r * k, NodeKind::OffCurve),
        ],
    )));
    let bytes = compile_ttf(&f, &Options::default()).unwrap().bytes;
    let back = decompile(&bytes, None).unwrap().font;
    let Shape::Path(p) = &back.glyph("O").unwrap().layers[0].shapes[0] else { panic!() };
    // Quadratic: on-curve nodes ending quadratic segments, or (when the converter left every
    // on-curve point implied, as cu2qu does on a symmetric oval) off-curve nodes only.
    assert!(p.nodes.iter().any(|n| n.kind == NodeKind::QCurve) || p.nodes.iter().all(|n| n.kind == NodeKind::OffCurve));
    assert!(!p.nodes.iter().any(|n| n.kind == NodeKind::Curve));
    let area = |b: fontcraft_geom::BezPath| fontcraft_geom::Shape::area(&b).abs();
    let before = area(f.outline("O", "m01").unwrap());
    let after = area(back.outline("O", "m01").unwrap());
    assert!((before - after).abs() / before < 0.01, "{before} vs {after}");
}

#[test]
fn collections_need_a_face() {
    let one = compile_ttf(&sample(), &Options::default()).unwrap().bytes;
    // A minimal collection header pointing at the same font twice.
    let mut ttc = b"ttcf\x00\x01\x00\x00\x00\x00\x00\x02".to_vec();
    let base = 12 + 8;
    ttc.extend_from_slice(&(base as u32).to_be_bytes());
    ttc.extend_from_slice(&(base as u32).to_be_bytes());
    // Offsets inside the member font are relative to the collection, so shift them.
    let mut member = one.clone();
    let tables = u16::from_be_bytes([member[4], member[5]]) as usize;
    for i in 0..tables {
        let at = 12 + 16 * i + 8;
        let off = u32::from_be_bytes(member[at..at + 4].try_into().unwrap()) + base as u32;
        member[at..at + 4].copy_from_slice(&off.to_be_bytes());
    }
    ttc.extend_from_slice(&member);
    assert!(matches!(decompile(&ttc, None), Err(DecompileError::ChooseFace { count: 2 })));
    assert!(matches!(decompile(&ttc, Some(5)), Err(DecompileError::NoSuchFace { face: 5 })));
    assert_eq!(decompile(&ttc, Some(1)).unwrap().font.family_name, "Back Again");
}

#[test]
fn damaged_fonts_are_errors_not_panics() {
    let good = compile_ttf(&sample(), &Options::default()).unwrap().bytes;
    for cut in (0..good.len()).step_by(37) {
        let _ = decompile(good.get(..cut).unwrap_or_default(), None);
    }
    for i in (0..good.len()).step_by(11) {
        let mut bad = good.clone();
        bad[i] = bad[i].wrapping_add(97);
        let _ = decompile(&bad, None);
    }
    for junk in [&b""[..], b"wOFF", b"wOFF\0\0\0\0\0\0\0\0\xff\xff", b"ttcf\0\0\0\0", b"OTTO"] {
        assert!(decompile(junk, None).is_err());
    }
}
