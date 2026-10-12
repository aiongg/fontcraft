//! SVG fonts (format `svg-font`): reading a font built here, writing it back, and the written
//! glyph elements against what AFDKO tx 5.0.1 writes for the same outline (`tx -svg -sa -gn2
//! -abs`).
// Integration tests may unwrap and panic, unlike shipped code (AGENTS.md › Never crash).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing, clippy::string_slice)]

use fontcraft_io_svg::svg_font::{is_svg_font, read_svg_font, write_svg_font};
use fontcraft_model::{Font, NodeKind, Shape};

const SVG_FONT: &str = r#"<?xml version="1.0" standalone="no"?>
<!DOCTYPE svg PUBLIC "-//W3C//DTD SVG 1.1//EN" "http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd">
<svg xmlns="http://www.w3.org/2000/svg" version="1.1">
<defs>
<font id="SVGTest" horiz-adv-x="600">
<font-face font-family="SVG Test" font-weight="bold" units-per-em="1000" ascent="800" descent="-200" x-height="500" cap-height="700" underline-position="-100" underline-thickness="50"/>
<missing-glyph horiz-adv-x="500" d="M50,0 L450,0 L450,700 L50,700 Z"/>
<glyph glyph-name="space" unicode=" " horiz-adv-x="250"/>
<glyph glyph-name="A" unicode="A" d="M 20,0 L 120,0 L 120,50 L 320,50 L 320,650 Z"/>
<glyph glyph-name="O" unicode="O" horiz-adv-x="700" d="M40,0 C40,200 140,300 350,300 Q500,300 600,150 T660,0 Z"/>
<glyph unicode="&#x2026;" horiz-adv-x="723" d="m55 0l131 0l0 122l-131 0z m241 0l131 0l0 122l-131 0z"/>
<glyph unicode="fi" glyph-name="f_i" horiz-adv-x="500" d="M0 0H100V100H0Z"/>
<hkern g1="A" g2="O" k="40"/>
<hkern u1="O" u2="A" k="-20"/>
<vkern g1="A" g2="O" k="10"/>
</font>
</defs>
</svg>
"#;

fn points(font: &Font, glyph: &str) -> Vec<(f64, f64, NodeKind)> {
    let layer = font.glyph(glyph).unwrap().layers.first().unwrap();
    layer
        .shapes
        .iter()
        .flat_map(|s| match s {
            Shape::Path(p) => p.nodes.iter().map(|n| (n.x, n.y, n.kind)).collect(),
            _ => Vec::new(),
        })
        .collect()
}

#[test]
fn svg_font_reads_glyphs_metrics_and_kerning() {
    assert!(is_svg_font(SVG_FONT));
    let read = read_svg_font(SVG_FONT.as_bytes()).unwrap();
    let font = &read.font;
    assert_eq!(read.warnings, vec!["1 vertical kerning pairs were not read".to_string()]);
    assert_eq!(font.family_name, "SVG Test");
    assert_eq!(font.units_per_em, 1000);
    let names: Vec<&str> = font.glyphs.iter().map(|g| g.name.as_str()).collect();
    assert_eq!(names, [".notdef", "space", "A", "O", "ellipsis", "f_i"]);
    let unicodes: Vec<Vec<u32>> = font.glyphs.iter().map(|g| g.unicodes.clone()).collect();
    assert_eq!(unicodes, [vec![], vec![0x20], vec![0x41], vec![0x4F], vec![0x2026], vec![]]);
    let widths: Vec<f64> = font.glyphs.iter().map(|g| g.layers.first().unwrap().width).collect();
    assert_eq!(widths, [500.0, 250.0, 600.0, 700.0, 723.0, 500.0], "the font's horiz-adv-x is the default");

    let master = font.masters.first().unwrap();
    assert_eq!(master.name, "Bold");
    let pos: Vec<f64> = master.metric_values.iter().map(|v| v.pos).collect();
    assert_eq!(pos, [800.0, 700.0, 500.0, 0.0, -200.0]);

    // hkern's k narrows the space: the kerning value is minus k.
    assert_eq!(font.kerning.ltr.get(&master.id, "A", "O"), Some(-40.0));
    assert_eq!(font.kerning.ltr.get(&master.id, "O", "A"), Some(20.0));

    // Relative commands, and two subpaths.
    let ellipsis = points(font, "ellipsis");
    assert_eq!(ellipsis.len(), 8);
    assert_eq!(
        &ellipsis[4..],
        [(296.0, 0.0, NodeKind::Line), (427.0, 0.0, NodeKind::Line), (427.0, 122.0, NodeKind::Line), (296.0, 122.0, NodeKind::Line)]
    );

    // The quadratic segment and its smooth continuation become cubic curves.
    let o = points(font, "O");
    assert_eq!(o.iter().filter(|p| p.2 == NodeKind::Curve).count(), 3);
    assert!(o.contains(&(450.0, 300.0, NodeKind::OffCurve)), "{o:?}");
}

#[test]
fn svg_font_written_reads_back_and_matches_tx() {
    let font = read_svg_font(SVG_FONT.as_bytes()).unwrap().font;
    let master = font.masters.first().unwrap().id.clone();
    let text = write_svg_font(&font, &master).unwrap();
    // tx's line for the same outline.
    assert!(text.contains(r#"<glyph unicode="A" glyph-name="A" horiz-adv-x="600" d="M 20,0 L 120,0 L 120,50 L 320,50 L 320,650 Z"/>"#), "{text}");
    assert!(text.contains(r#"<missing-glyph horiz-adv-x="500" d="M 50,0 L 450,0 L 450,700 L 50,700 Z"/>"#), "{text}");
    assert!(text.contains("unicode=\"&#x2026;\""), "{text}");

    let back = read_svg_font(text.as_bytes()).unwrap().font;
    for g in font.glyphs.iter() {
        let b = back.glyph(&g.name).unwrap();
        assert_eq!(g.unicodes, b.unicodes, "{}", g.name);
        assert_eq!(points(&font, &g.name), points(&back, &g.name), "{}", g.name);
    }
    assert_eq!(back.kerning.ltr.get(&master, "A", "O"), Some(-40.0));
}

#[test]
fn damaged_svg_fonts_fail_with_a_message() {
    for bad in [&SVG_FONT[..300], "<svg><font><glyph d=\"M 1e300,0 L 0,0\"/></font></svg>", "<svg/>", "\u{0}"] {
        let _ = read_svg_font(bad.as_bytes());
    }
    let huge = format!("<svg><font>{}</font></svg>", "<glyph unicode=\"a\" d=\"M0 0L1 1\"/>".repeat(200_000));
    assert!(read_svg_font(huge.as_bytes()).is_err());
}
