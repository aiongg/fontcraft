use fontcraft_geom::{Affine, BezPath, Point};
use fontcraft_model::{Font, Path, Shape};

use crate::*;

/// A font with `ring` (an outer square with a counter drawn the other way round) and `bar`,
/// on a 1000-unit em with ascender 800 and descender −200.
fn font() -> Font {
    let mut font = Font::new("Render");
    let master = font.masters.first().map(|m| m.id.clone()).unwrap();
    font.add_glyph("ring", vec![], 600.0).unwrap();
    font.add_glyph("bar", vec![], 300.0).unwrap();
    {
        let mut o = font.glyph_mut("ring").unwrap();
        let layer = o.layer_mut(&master).unwrap();
        layer.shapes.push(Shape::Path(Path::rect(50.0, 0.0, 550.0, 700.0)));
        let mut counter = Path::rect(150.0, 100.0, 450.0, 600.0);
        counter.nodes.reverse();
        layer.shapes.push(Shape::Path(counter));
    }
    {
        let mut i = font.glyph_mut("bar").unwrap();
        i.layer_mut(&master).unwrap().shapes.push(Shape::Path(Path::rect(100.0, 0.0, 200.0, 700.0)));
    }
    font
}

#[test]
fn a_line_fills_by_the_nonzero_rule_and_is_reproducible() {
    let font = font();
    // One pixel per unit, no padding: the image is 900 × 1000, the baseline at y = 800.
    let options = LineOptions { scale: Some(1.0), padding: 0, ..Default::default() };
    let image = render_line(&font, &["ring", "bar"], &options).unwrap();
    assert_eq!((image.width, image.height), (900, 1000));
    let at = |x: u32, y_font: u32| image.pixel(x, 800 - y_font).unwrap();
    assert_eq!(at(100, 50), [0, 0, 0, 255], "inside the outer contour");
    assert_eq!(at(300, 350), [255, 255, 255, 255], "inside the counter");
    assert_eq!(at(20, 350), [255, 255, 255, 255], "left of the glyph");
    assert_eq!(at(600 + 150, 350), [0, 0, 0, 255], "the second glyph starts at the first one's advance");
    assert_eq!(render_line(&font, &["ring", "bar"], &options).unwrap(), image, "the same input gives the same bytes");
    let png = image.png().unwrap();
    assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
}

#[test]
fn a_layer_renders_at_a_size_in_pixels_per_em() {
    let font = font();
    let glyph = font.glyph("bar").unwrap();
    let image = render_layer(&font, glyph, None, &LineOptions { size: Some(100.0), padding: 2, ..Default::default() }).unwrap();
    assert_eq!((image.width, image.height), (34, 104));
    assert!(render_layer(&font, glyph, Some("no such layer"), &LineOptions::default()).is_err());
}

#[test]
fn bad_sizes_and_paths_are_errors_not_crashes() {
    let font = font();
    let bad = |options: LineOptions| render_line(&font, &["ring"], &options).unwrap_err();
    assert!(matches!(bad(LineOptions { scale: Some(f64::NAN), ..Default::default() }), RenderError::BadNumber(_)));
    assert!(matches!(bad(LineOptions { scale: Some(-1.0), ..Default::default() }), RenderError::BadNumber(_)));
    assert!(matches!(bad(LineOptions { scale: Some(1000.0), ..Default::default() }), RenderError::BadNumber(_)));
    assert!(matches!(bad(LineOptions { scale: Some(60.0), ..Default::default() }), RenderError::TooLarge { .. }));
    assert_eq!(render_line(&font, &[], &LineOptions::default()), Err(RenderError::NoGlyphs));
    assert!(render_line(&font, &["nope"], &LineOptions::default()).is_err());
    let many = vec!["bar"; MAX_LINE_GLYPHS + 1];
    assert_eq!(render_line(&font, &many, &LineOptions::default()), Err(RenderError::TooManyGlyphs));
    // Points that are not numbers are skipped, the rest still draws.
    let mut path = BezPath::new();
    path.move_to(Point::new(f64::NAN, 0.0));
    path.line_to(Point::new(10.0, f64::INFINITY));
    path.close_path();
    path.move_to(Point::new(0.0, 0.0));
    path.line_to(Point::new(8.0, 0.0));
    path.line_to(Point::new(8.0, 8.0));
    path.close_path();
    let image = render_path(&path, Affine::IDENTITY, 10, 10, &Style::default()).unwrap();
    assert_eq!(image.pixel(6, 2), Some([0, 0, 0, 255]));
    assert_eq!(render_path(&path, Affine::IDENTITY, 0, 10, &Style::default()), Err(RenderError::Empty));
    assert!(matches!(render_path(&path, Affine::IDENTITY, 20_000, 10, &Style::default()), Err(RenderError::TooLarge { .. })));
    assert_eq!(Image::from_rgba_premultiplied(2, 2, vec![0; 15]), Err(RenderError::BadPixels));
}
