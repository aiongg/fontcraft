//! Smart components on the canvas: the handles of the selected ones and the sizes their box knobs set
//! (`smart.handlesFromPropertyAnchors`, `smart.widthHeightFollowBoundingBox`).
//!
//! A handle is the anchor of a smart glyph's property, by the property's exact name, when it lies elsewhere at the two
//! ends of the property's range. [`SmartCache`] resolves handles and sizes once per document revision and selection;
//! the Select tool gets them in its scene (`tools::apply`), and [`paint`] draws the handles as grey dots. Smart values
//! are set on the foreground, so the background's components have neither. A locked component has no handles.

use std::collections::BTreeSet;

use egui::Shape as Paint;
use fontcraft_engine::Session;
use fontcraft_engine::model::smart::component_anchors;
use fontcraft_engine::model::{Component, Font, Glyph, Layer, LayerSide, Shape, SmartValue};
use fontcraft_geom::Point;
use fontcraft_tools::{Selection, SmartAxis, SmartHandle, SmartSize};

use super::layers::PaintCx;
use super::paint::screen;
use super::tools::tokens::ToolTokens;
use super::tools::{Target, active_layer, target};
use crate::app::ViewState;

/// Most selected components whose handles are resolved; each takes three interpolations per property.
const MAX_HANDLE_COMPONENTS: usize = 64;
/// Most properties of one smart component that handles and sizes are resolved for.
const MAX_SMART_PARTS: usize = 32;
/// Two handle positions closer than this, in font units, are the same one: the anchor does not move with the property.
const SAME_POSITION: f64 = 1e-6;

/// What the handles and sizes were resolved from.
#[derive(Clone, Debug, PartialEq)]
struct Key {
    doc: Option<(u64, u64)>,
    target: Target,
    shapes: BTreeSet<usize>,
    /// One component is selected and nothing else: the only selection whose box knobs set sizes.
    alone: bool,
}

/// The handles and sizes of the selected smart components, kept while what they come from stays the same.
#[derive(Clone, Debug, Default)]
pub struct SmartCache {
    key: Option<Key>,
    pub handles: Vec<SmartHandle>,
    pub sizes: Vec<SmartSize>,
}

impl SmartCache {
    /// Brings the handles and sizes up to date with the edited layer and `selection`.
    pub fn update(&mut self, session: &Session, view: &ViewState, selection: &Selection) {
        let key = match target(session, view) {
            Some(target) if target.side == LayerSide::Foreground && !selection.shapes.is_empty() => {
                Key { doc: session.active().map(|d| (d.id, d.revision)), target, shapes: selection.shapes.clone(), alone: selection.len() == 1 }
            }
            _ => {
                *self = SmartCache::default();
                return;
            }
        };
        if self.key.as_ref() == Some(&key) {
            return;
        }
        (self.handles, self.sizes) = match session.font() {
            Ok(font) => match active_layer(font, view) {
                Some(layer) => (handles(font, layer, &key.shapes), if key.alone { sizes(font, layer, &key.shapes) } else { Vec::new() }),
                None => Default::default(),
            },
            Err(_) => Default::default(),
        };
        self.key = Some(key);
    }
}

/// The smart component at index `shape` of `shapes`, with its smart glyph.
fn smart<'f>(font: &'f Font, shapes: &'f [Shape], shape: usize) -> Option<(&'f Component, &'f Glyph)> {
    let Some(Shape::Component(c)) = shapes.get(shape) else { return None };
    let base = font.glyph(&c.glyph)?;
    (!base.info.parts.is_empty()).then_some((c, &**base))
}

/// Whether one of the `selected` of `shapes` is a smart component.
pub fn any_smart(font: &Font, shapes: &[Shape], selected: &BTreeSet<usize>) -> bool {
    selected.iter().any(|&i| smart(font, shapes, i).is_some())
}

/// `c` with its property `name` at `value`.
fn at_value(c: &Component, name: &str, value: f64) -> Component {
    let mut probe = c.clone();
    probe.info.smart_values.retain(|v| v.property != name);
    probe.info.smart_values.push(SmartValue { property: name.to_string(), value });
    probe
}

fn handles(font: &Font, layer: &Layer, shapes: &BTreeSet<usize>) -> Vec<SmartHandle> {
    let mut out = Vec::new();
    for &shape in shapes.iter().filter(|&&i| smart(font, &layer.shapes, i).is_some()).take(MAX_HANDLE_COMPONENTS) {
        let Some((c, base)) = smart(font, &layer.shapes, shape).filter(|(c, _)| !c.info.locked) else { continue };
        for part in base.info.parts.iter().take(MAX_SMART_PARTS) {
            let position = |probe: &Component| -> Option<Point> {
                let anchors = component_anchors(font, probe, &layer.master).ok()??;
                let a = anchors.iter().find(|a| a.name == part.name)?;
                let p = c.affine() * Point::new(a.x, a.y);
                (p.x.is_finite() && p.y.is_finite()).then_some(p)
            };
            let (min, max) = (part.bottom_value.min(part.top_value), part.bottom_value.max(part.top_value));
            let end = |value: f64| position(&at_value(c, &part.name, value));
            let (Some(at), Some(at_min), Some(at_max)) = (position(c), end(min), end(max)) else { continue };
            if (at_max - at_min).hypot() < SAME_POSITION {
                continue;
            }
            out.push(SmartHandle { shape, property: part.name.clone(), at, at_min, at_max, min, max });
        }
    }
    out
}

/// The `Width` and `Height` properties (the names matched exactly) with the component's extent at each end of them,
/// measured as the box of a component is: from its nodes and handles.
fn sizes(font: &Font, layer: &Layer, shapes: &BTreeSet<usize>) -> Vec<SmartSize> {
    let mut out = Vec::new();
    for &shape in shapes.iter().take(1) {
        let Some((c, base)) = smart(font, &layer.shapes, shape) else { continue };
        let axis = |name: &str, horizontal: bool| -> Option<SmartAxis> {
            let part = base.info.parts.iter().take(MAX_SMART_PARTS).find(|p| p.name == name)?;
            let (min, max) = (part.bottom_value.min(part.top_value), part.bottom_value.max(part.top_value));
            let size = |value: f64| -> Option<f64> {
                let b = font.component_outline(&at_value(c, name, value), &layer.master).ok()?.control_box();
                let size = if horizontal { b.width() } else { b.height() };
                size.is_finite().then_some(size)
            };
            Some(SmartAxis { property: name.to_string(), min, max, size_at_min: size(min)?, size_at_max: size(max)? })
        };
        out.push(SmartSize { shape, width: axis("Width", true), height: axis("Height", false) });
    }
    out
}

/// The paint layer of the smart handles.
pub fn paint(cx: &PaintCx<'_>) {
    cx.painter.extend(shapes(cx));
}

pub fn shapes(cx: &PaintCx<'_>) -> Vec<Paint> {
    let color = ToolTokens::standard().smart_handle;
    let radius = cx.tokens.anchor_size / 2.0;
    cx.tools.marks.handles.iter().filter_map(|p| screen(cx, [p.x, p.y])).map(|at| Paint::circle_filled(at, radius, color)).collect()
}
