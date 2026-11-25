//! Pinned component tests.
#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]

use super::*;
use crate::{Component, Glyph, Layer, Path};
use fontcraft_geom::Shape as _;

/// Two masters; `bar` is 100 wide in the first and 300 in the second, `wide` holds a `bar`, and
/// `host` holds a `wide` pinned to the second master on its first layer, an unpinned one on its
/// second.
fn font() -> Font {
    let mut font = Font::new("Pinned");
    let mut second = font.masters[0].clone();
    second.id = "m02".into();
    std::sync::Arc::make_mut(&mut font.masters).push(second);
    let glyph = |font: &mut Font, name: &str, shapes: [Shape; 2]| {
        font.add_glyph(name, Vec::new(), 600.0).unwrap();
        let mut g = font.glyph_mut(name).unwrap();
        g.layers.clear();
        for (master, shape) in ["m01", "m02"].into_iter().zip(shapes) {
            let mut l = Layer::new(master, 600.0);
            l.shapes = vec![shape];
            g.layers.push(l);
        }
    };
    let comp = |base: &str, pin: Option<&str>| {
        let mut c = Component::new(base, 10.0, 0.0);
        c.info.master_id = pin.map(str::to_string);
        Shape::Component(c)
    };
    glyph(&mut font, "bar", [100.0, 300.0].map(|w| Shape::Path(Path::rect(0.0, 0.0, w, 50.0))));
    glyph(&mut font, "wide", [comp("bar", None), comp("bar", None)]);
    glyph(&mut font, "host", [comp("wide", Some("m02")), comp("wide", None)]);
    font
}

fn width(font: &Font, glyph: &str, master: &str) -> f64 {
    font.outline(glyph, master).unwrap().bounding_box().width()
}

/// A pinned component is drawn from its own master through its nested components; a pin to a
/// master the base glyph does not have is as no pin; a pin to a layer reads that layer.
#[test]
fn a_pinned_component_is_drawn_from_its_own_master() {
    let mut font = font();
    assert_eq!((width(&font, "wide", "m01"), width(&font, "host", "m01"), width(&font, "host", "m02")), (100.0, 300.0, 300.0));
    let pin = |font: &mut Font, to: &str| {
        let mut g = font.glyph_mut("host").unwrap();
        let Shape::Component(c) = &mut g.layers[0].shapes[0] else { panic!() };
        c.info.master_id = Some(to.into());
    };
    pin(&mut font, "gone");
    assert_eq!(width(&font, "host", "m01"), 100.0);
    {
        let mut g: crate::GlyphMut<'_> = font.glyph_mut("wide").unwrap();
        let g: &mut Glyph = &mut g;
        let mut l = Layer::new("m01", 600.0);
        l.id = "backup".into();
        l.shapes = vec![Shape::Path(Path::rect(0.0, 0.0, 70.0, 50.0))];
        g.layers.push(l);
    }
    pin(&mut font, "backup");
    assert_eq!(width(&font, "host", "m01"), 70.0);
}

/// Export writes the pinned component as paths on every layer of the glyph, so the two masters
/// keep the same structure, and leaves glyphs without one alone.
#[test]
fn pinned_components_are_written_as_paths_on_every_layer() {
    let font = font();
    let (drawn, report) = draw_pinned_components(&font);
    assert_eq!((report.drawn, report.kept.len()), (2, 0), "{:?}", report.kept);
    let g = drawn.glyph("host").unwrap();
    for l in &g.layers {
        let [Shape::Path(p)] = l.shapes.as_slice() else { panic!("{:?}", l.shapes) };
        assert_eq!(p.nodes.len(), 4);
    }
    assert_eq!((width(&drawn, "host", "m01"), width(&drawn, "host", "m02")), (300.0, 300.0));
    assert!(matches!(drawn.glyph("wide").unwrap().layers[0].shapes[0], Shape::Component(_)));
    // The compiler's view draws it too when the font was not prepared.
    let resolved = font.resolve(&crate::Target::Master("m01".into())).unwrap();
    let host = resolved.glyphs.iter().find(|g| g.name == "host").unwrap();
    assert!(host.components.is_empty());
    assert_eq!(host.contours[0].bounding_box().width(), 300.0);
}

/// A glyph that reaches itself through a pin ends with a message, not a crash.
#[test]
fn a_pinned_cycle_is_kept_with_a_reason() {
    let mut font = font();
    {
        let mut g = font.glyph_mut("bar").unwrap();
        g.layers[1].shapes = vec![Shape::Component(Component::new("host", 0.0, 0.0))];
    }
    let (_, report) = draw_pinned_components(&font);
    assert!(report.kept.iter().any(|k| k.contains("nest")), "{:?}", report.kept);
}
