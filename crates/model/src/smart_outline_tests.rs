//! Smart components in outline resolution ([`crate::Font::outline`]).
#![allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::unreachable)]

use super::*;
use crate::{
    ComponentInfo, ComponentInfoBox, GlyphInfo, GlyphInfoBox, LayerAttrs, LayerInfo, LayerInfoBox, PartProperty, Path, SmartPole, SmartValue,
};
use fontcraft_geom::{PathEl, Point, Rect, Shape as _};

fn pole_info(property: &str, top: bool) -> LayerInfoBox {
    LayerInfoBox::new(LayerInfo {
        attrs: LayerAttrs { smart_poles: vec![SmartPole { property: property.into(), top }], ..Default::default() },
        ..Default::default()
    })
}

/// A font with the smart glyph `_part.bar`: one property running from `bottom` to `top`, a
/// bottom pole layer (the master layer) holding a `bottom` × 100 rectangle and a top pole layer
/// holding a `top` × 100 one.
fn bar_font(property: &str, bottom: f64, top: f64) -> Font {
    let mut font = Font::new("Smart");
    let mut part = Glyph::new("_part.bar");
    part.info = GlyphInfoBox::new(GlyphInfo {
        parts: vec![PartProperty {
            name: property.into(),
            bottom_value: bottom,
            top_value: top,
            bottom_name: None,
            top_name: None,
            extra: Default::default(),
        }],
        ..Default::default()
    });
    let (low, high) = if property == "Width" { (bottom, top) } else { (200.0, 400.0) };
    for (id, end, width) in [("m01", false, low), ("wide", true, high)] {
        let mut l = Layer::new("m01", 600.0);
        l.id = id.into();
        l.shapes.push(Shape::Path(Path::rect(0.0, 0.0, width, 100.0)));
        l.info = pole_info(property, end);
        part.layers.push(l);
    }
    font.glyphs.push(part);
    font
}

/// `user` holding one `_part.bar` component at (`dx`, 0) with `value` for `property` (none when
/// `None`), after a 10 × 10 square when `square_first`.
fn add_user(font: &mut Font, property: &str, value: Option<f64>, dx: f64, square_first: bool) {
    let mut c = Component::new("_part.bar", dx, 0.0);
    if let Some(value) = value {
        c.info = ComponentInfoBox::new(ComponentInfo { smart_values: vec![SmartValue { property: property.into(), value }], ..Default::default() });
    }
    let mut user = Glyph::new("user");
    let mut l = Layer::new("m01", 600.0);
    l.shapes.push(Shape::Component(c));
    if square_first {
        l.shapes.push(Shape::Path(Path::rect(-50.0, -50.0, -40.0, -40.0)));
    }
    user.layers.push(l);
    font.glyphs.push(user);
}

fn bounds(font: &Font) -> Rect {
    font.outline("user", "m01").unwrap().bounding_box()
}

/// A Width value between the poles interpolates, one beyond them extrapolates
/// (`smart.valuesExtrapolate`), no value draws the master layer's pole, and the component's
/// offset applies (`smart.axesAreLocalToGlyph`).
#[test]
fn outlines_draw_smart_components_interpolated() {
    for (value, width) in [(Some(400.0), 400.0), (Some(700.0), 700.0), (Some(100.0), 100.0), (None, 200.0)] {
        let mut font = bar_font("Width", 200.0, 600.0);
        add_user(&mut font, "Width", value, 10.0, false);
        assert_eq!(bounds(&font), Rect::new(10.0, 0.0, 10.0 + width, 100.0), "{value:?}");
        // Callers that draw one component see the same outline.
        let Shape::Component(c) = &font.glyph("user").unwrap().layers[0].shapes[0] else { unreachable!() };
        assert_eq!(font.component_outline(c, "m01").unwrap().bounding_box(), Rect::new(10.0, 0.0, 10.0 + width, 100.0));
    }
}

/// A property running from 0 to 1 switches between its two pole layers (`smart.zeroOneRangeIsToggle`).
#[test]
fn a_zero_one_property_switches_between_its_layers() {
    for (value, width) in [(0.0, 200.0), (1.0, 400.0)] {
        let mut font = bar_font("Serif", 0.0, 1.0);
        add_user(&mut font, "Serif", Some(value), 0.0, false);
        assert_eq!(bounds(&font), Rect::new(0.0, 0.0, width, 100.0), "{value}");
    }
}

/// The drawn smart component comes after the layer's other shapes, as the reference compilers
/// order them.
#[test]
fn smart_components_come_after_the_other_shapes() {
    let mut font = bar_font("Width", 200.0, 600.0);
    add_user(&mut font, "Width", Some(400.0), 0.0, true);
    let outline = font.outline("user", "m01").unwrap();
    assert_eq!(outline.elements().first(), Some(&PathEl::MoveTo(Point::new(-50.0, -50.0))));
}

/// Pole layers that do not match node for node cannot be interpolated: the component is drawn as
/// a plain component of the master layer.
#[test]
fn a_smart_component_that_cannot_be_interpolated_is_drawn_plain() {
    let mut font = bar_font("Width", 200.0, 600.0);
    font.glyph_mut("_part.bar").unwrap().layers[1].shapes.push(Shape::Path(Path::rect(0.0, 200.0, 50.0, 250.0)));
    add_user(&mut font, "Width", Some(400.0), 0.0, false);
    assert_eq!(bounds(&font), Rect::new(0.0, 0.0, 200.0, 100.0));
}

/// A smart glyph whose pole layers use it again resolves without recursing forever.
#[test]
fn a_smart_glyph_that_uses_itself_has_an_outline() {
    let mut font = bar_font("Width", 200.0, 600.0);
    for l in &mut font.glyph_mut("_part.bar").unwrap().layers {
        l.shapes.push(Shape::Component(Component::new("_part.bar", 0.0, 0.0)));
    }
    add_user(&mut font, "Width", Some(400.0), 0.0, false);
    // The self-reference makes the plain fallback a cycle, reported as an error, never a hang.
    assert!(font.outline("user", "m01").is_err());
}
