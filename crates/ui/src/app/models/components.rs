//! The view models of the component, corner, cap, segment component and brush parts of the Info box and of the Smart
//! Component controls, and the highlight bands that mark attachments on the canvas.
//!
//! | Model | Shows while |
//! |---|---|
//! | [`ComponentBox`] | components of the edited layer are selected |
//! | [`CornerBox`] | corner components are selected, else those on the selected nodes |
//! | [`CapBox`] | cap components are selected, else those on the selected nodes: each sits on two |
//! | [`SegmentBox`] | segment components (or brushes) are selected, else those whose segment ends at a selected node |
//! | [`SmartModel`] | smart components are selected (`palette.smartComponent`, `dialog.smartComponentSettings`) |
//!
//! Each box describes every selected object of its kind: a field holds the value they share, or `None` where they
//! differ.

use std::collections::HashMap;

use craft::ui::contract::PlainData;
use fontcraft_engine::cmd::smart::is_toggle;
use fontcraft_engine::components::align::{AlignMode, aligned_components};
use fontcraft_engine::components::propagate::{base_layer, effective_anchors_with};
use fontcraft_engine::model::attach::{Attachment, CapHint, SegmentHint, attachment};
use fontcraft_engine::model::corner::{CornerAlignment, CornerHint, CornerTarget, corner_hint, is_corner};
use fontcraft_engine::model::glyphdata::GlyphData;
use fontcraft_engine::model::smart::pole_layers;
use fontcraft_engine::model::{Component, Font, Glyph, Hint, Layer, LayerSide, MAX_PART_PROPERTIES, PartProperty, Path, Shape, Transform};
use fontcraft_geom::BezPath;
use fontcraft_tools::Selection;
use fontcraft_tools::attachment;

/// Most components before the selected one whose anchors the anchor menu reads.
const MAX_ANCHOR_SOURCES: usize = 64;
/// Most selected components whose shared anchors the anchor menu works out; with more it is not shown.
const MAX_ANCHOR_COMPONENTS: usize = 64;
/// Below this a matrix entry counts as zero.
const TINY: f64 = 1e-12;
/// Most components the Smart Component controls edit at once.
pub const MAX_SMART_COMPONENTS: usize = 1024;

/// The alignment modes of corner and cap components, as `cornerComponent.setAlignment` and
/// `capComponent.setAlignment` name them, in button order.
pub const MODES: [&str; 4] = ["left", "both", "right", "none"];

/// A component's transform as the Info box shows it: position, scale in percent, slant and rotation in degrees.
#[derive(PlainData, Clone, Copy, Debug, Default, PartialEq)]
pub struct ComponentValues {
    pub x: f64,
    pub y: f64,
    pub scale_x: f64,
    pub scale_y: f64,
    pub slant: f64,
    pub rotation: f64,
}

/// The component part of the Info box.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct ComponentBox {
    pub glyph: String,
    pub layer: String,
    /// The selected components' shape indices on the layer, ascending.
    pub components: Vec<usize>,
    pub base: Option<String>,
    pub x: Option<f64>,
    pub y: Option<f64>,
    /// Scales in percent, slant and rotation in degrees: also `None` for a matrix the box cannot split into its fields.
    pub scale_x: Option<f64>,
    pub scale_y: Option<f64>,
    pub slant: Option<f64>,
    pub rotation: Option<f64>,
    /// X and Y can be edited on at least one of them: not while a component is aligned automatically (only Y while it
    /// is aligned horizontally), and neither while it is locked.
    pub x_free: bool,
    pub y_free: bool,
    /// Scale, slant and rotation can be edited on at least one of them.
    pub editable: bool,
    /// The components whose base glyph is a smart glyph: the Smart badge shows.
    pub smart: Vec<usize>,
    /// The anchors every one of them can attach to; the menu shows when there are two or more.
    pub anchors: Vec<String>,
    /// The anchor they attach to as the menu shows it: the stored choice, else the first plain name offered.
    pub anchor: Option<String>,
}

/// The corner component part of the Info box.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct CornerBox {
    pub glyph: String,
    pub layer: String,
    /// The corners' indices in the layer's hint list.
    pub corners: Vec<usize>,
    /// The corner glyph.
    pub name: Option<String>,
    /// Horizontal and vertical scale in percent.
    pub scale: [Option<f64>; 2],
    /// The mode of [`MODES`] the corners have; `None` also for the stored value that fits a corner unturned.
    pub mode: Option<String>,
    /// Every corner's glyph exists.
    pub exists: bool,
}

/// The cap component part of the Info box.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct CapBox {
    pub glyph: String,
    pub layer: String,
    /// The caps' indices in the layer's hint list.
    pub caps: Vec<usize>,
    /// The cap glyph.
    pub name: Option<String>,
    /// Horizontal and vertical scale in percent; with Fit on every cap the horizontal one is computed and greyed out.
    pub scale: [Option<f64>; 2],
    pub mode: Option<String>,
    /// Sized to the stroke's width.
    pub fit: Option<bool>,
    pub exists: bool,
}

/// The segment component or brush part of the Info box: the glyph's name and the arrow button that edits it.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct SegmentBox {
    pub glyph: String,
    pub layer: String,
    /// The attachments' indices in the layer's hint list.
    pub hints: Vec<usize>,
    /// The segment or brush glyph.
    pub name: Option<String>,
    pub exists: bool,
    /// Brushes rather than segment components.
    pub brush: bool,
}

/// One property of the selected smart components, as the Smart Component controls show it.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct SmartRow {
    pub name: String,
    pub min: f64,
    pub max: f64,
    /// The range is 0 to 1: a checkbox instead of a slider.
    pub toggle: bool,
    /// The components' common value; `None` when they differ.
    pub value: Option<f64>,
    /// The components whose glyph has the property.
    pub components: Vec<usize>,
}

/// The Smart Component controls: the components they act on and one row per property.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct SmartModel {
    pub glyph: String,
    pub layer: String,
    /// The components are on the layer's background.
    pub background: bool,
    /// Shape indices on the layer, ascending.
    pub components: Vec<usize>,
    pub rows: Vec<SmartRow>,
}

/// The Info box fields of a transform; `None` for a matrix that is not a rotation and scale (a slanted matrix), or
/// that is singular.
pub fn values(t: &Transform) -> Option<ComponentValues> {
    match *t {
        Transform::Decomposed { pos, scale, angle, slant } => {
            Some(ComponentValues { x: pos.0, y: pos.1, scale_x: scale.0 * 100.0, scale_y: scale.1 * 100.0, slant: slant.0, rotation: angle })
        }
        Transform::Matrix([a, b, c, d, dx, dy]) => {
            if ![a, b, c, d, dx, dy].iter().all(|v| v.is_finite()) {
                return None;
            }
            // Orthogonal columns: a rotation times a scale, with no slant.
            if (a * c + b * d).abs() > TINY * (1.0 + a.abs() + b.abs() + c.abs() + d.abs()) {
                return None;
            }
            let sx = a.hypot(b);
            if sx < TINY {
                return None;
            }
            let mut angle = b.atan2(a).to_degrees();
            let mut sy = (a * d - b * c) / sx;
            let mut sx = sx;
            // A mirror reads as a negative horizontal scale rather than a turn by 180°.
            if (angle.abs() - 180.0).abs() < 1e-9 && sy < 0.0 {
                (angle, sx, sy) = (0.0, -sx, -sy);
            }
            if sy.abs() < TINY {
                return None;
            }
            Some(ComponentValues { x: dx, y: dy, scale_x: sx * 100.0, scale_y: sy * 100.0, slant: 0.0, rotation: angle })
        }
    }
}

/// The anchors the component at `index` of `layer` can attach to: the anchors of the components before it named `X`
/// or `X_suffix`, where `_X` is one of its own attaching anchors, in the order the components offer them.
pub fn anchor_choices(font: &Font, data: Option<&GlyphData>, layer: &Layer, index: usize) -> Vec<String> {
    let Some(Shape::Component(c)) = layer.shapes.get(index) else { return Vec::new() };
    let Some(base) = font.glyph(&c.glyph) else { return Vec::new() };
    let Some(from) = base_layer(base, layer, c) else { return Vec::new() };
    let Ok(own) = effective_anchors_with(font, data, &c.glyph, &from.id) else { return Vec::new() };
    let plain: Vec<&str> = own.iter().filter_map(|a| a.name.strip_prefix('_')).filter(|n| !n.is_empty()).collect();
    if plain.is_empty() {
        return Vec::new();
    }
    let fits = |name: &str| plain.iter().any(|p| name == *p || name.strip_prefix(p).is_some_and(|rest| rest.len() > 1 && rest.starts_with('_')));
    let mut offered: Vec<String> = Vec::new();
    let before = layer.shapes.iter().take(index).filter_map(|s| if let Shape::Component(p) = s { Some(p) } else { None });
    for prev in before.take(MAX_ANCHOR_SOURCES) {
        let Some(g) = font.glyph(&prev.glyph) else { continue };
        let Some(l) = base_layer(g, layer, prev) else { continue };
        let Ok(anchors) = effective_anchors_with(font, data, &prev.glyph, &l.id) else { continue };
        for a in anchors {
            if !a.name.starts_with('_') && fits(&a.name) && !offered.contains(&a.name) {
                offered.push(a.name);
            }
        }
    }
    offered
}

/// The value every item has; `None` where they differ, and for no item.
fn shared<T: PartialEq>(mut values: impl Iterator<Item = T>) -> Option<T> {
    let first = values.next()?;
    values.all(|v| v == first).then_some(first)
}

/// The selected components of `layer`, by shape index.
pub fn selected_components(layer: &Layer, selection: &Selection) -> Vec<usize> {
    selection.shapes.iter().copied().filter(|&i| matches!(layer.shapes.get(i), Some(Shape::Component(_)))).collect()
}

/// The anchors every component of `picked` can attach to, and the one they share now.
fn shared_anchors(font: &Font, data: Option<&GlyphData>, layer: &Layer, picked: &[(usize, &Component)]) -> (Vec<String>, Option<String>) {
    if picked.len() > MAX_ANCHOR_COMPONENTS {
        return (Vec::new(), None);
    }
    let mut common: Option<Vec<String>> = None;
    let mut chosen: Vec<Option<String>> = Vec::new();
    for &(i, c) in picked {
        let offered = anchor_choices(font, data, layer, i);
        chosen.push(c.info.anchor.clone().or_else(|| {
            let plain: Vec<String> = font
                .glyph(&c.glyph)
                .and_then(|g| base_layer(g, layer, c))
                .map(|l| l.anchors.iter().filter_map(|a| a.name.strip_prefix('_').map(str::to_string)).collect())
                .unwrap_or_default();
            offered.iter().find(|a| plain.contains(a)).cloned()
        }));
        common = Some(match common {
            None => offered,
            Some(list) => list.into_iter().filter(|a| offered.contains(a)).collect(),
        });
        if common.as_ref().is_some_and(Vec::is_empty) {
            return (Vec::new(), shared(chosen.into_iter()).flatten().filter(|_| picked.len() == 1));
        }
    }
    (common.unwrap_or_default(), shared(chosen.into_iter()).flatten())
}

/// The component part for the selected components of `layer`; `None` when no component is selected.
pub fn component_box(font: &Font, data: Option<&GlyphData>, glyph: &str, layer: &Layer, selection: &Selection) -> Option<ComponentBox> {
    let components = selected_components(layer, selection);
    let picked: Vec<(usize, &Component)> = components
        .iter()
        .filter_map(|&i| match layer.shapes.get(i) {
            Some(Shape::Component(c)) => Some((i, c)),
            _ => None,
        })
        .collect();
    if picked.is_empty() {
        return None;
    }
    let aligned: HashMap<usize, AlignMode> = aligned_components(font, layer).into_iter().collect();
    let free = |i: usize, c: &Component| match (c.info.locked, aligned.get(&i)) {
        (true, _) | (false, Some(AlignMode::Full)) => (false, false),
        (false, Some(AlignMode::Horizontal)) => (false, true),
        (false, None) => (true, true),
    };
    let fields: Vec<Option<ComponentValues>> = picked.iter().map(|(_, c)| values(&c.transform)).collect();
    let field = |of: fn(&ComponentValues) -> f64| shared(fields.iter().map(|v| v.as_ref().map(of))).flatten();
    let place = |at: usize| shared(picked.iter().filter_map(|(_, c)| c.transform.affine().as_coeffs().get(at).copied()));
    let (anchors, anchor) = shared_anchors(font, data, layer, &picked);
    Some(ComponentBox {
        glyph: glyph.to_string(),
        layer: layer.id.clone(),
        base: shared(picked.iter().map(|(_, c)| c.glyph.clone())),
        x: place(4),
        y: place(5),
        scale_x: field(|v| v.scale_x),
        scale_y: field(|v| v.scale_y),
        slant: field(|v| v.slant),
        rotation: field(|v| v.rotation),
        x_free: picked.iter().any(|&(i, c)| free(i, c).0),
        y_free: picked.iter().any(|&(i, c)| free(i, c).1),
        editable: picked.iter().zip(&fields).any(|((_, c), v)| !c.info.locked && v.is_some()),
        smart: picked.iter().filter(|(_, c)| font.glyph(&c.glyph).is_some_and(|g| !g.info.parts.is_empty())).map(|&(i, _)| i).collect(),
        anchors,
        anchor,
        components,
    })
}

/// The node of a path that a corner or cap hint's node index names: the hint counts a closed path's nodes with the
/// start node last.
fn model_node(path: &Path, node: usize) -> usize {
    let len = path.nodes.len();
    if path.closed && len > 0 { (node + 1) % len } else { node }
}

/// The corner components (hint indices) of `layer` that sit on the nodes `points` (model order).
pub fn corners_on_nodes(layer: &Layer, points: &[(usize, usize)]) -> Vec<usize> {
    layer
        .info
        .hints
        .iter()
        .enumerate()
        .filter(|(_, h)| is_corner(h))
        .filter_map(|(i, h)| {
            let corner = corner_hint(h).ok().flatten()?;
            let CornerTarget::Node { shape, node } = corner.target else { return None };
            let Some(Shape::Path(path)) = layer.shapes.get(shape) else { return None };
            points.contains(&(shape, model_node(path, node))).then_some(i)
        })
        .collect()
}

/// The cap components (hint index and hint) of `layer` with one of their two nodes among `points` (model order).
pub fn caps_on_nodes(layer: &Layer, points: &[(usize, usize)]) -> Vec<(usize, CapHint)> {
    layer
        .info
        .hints
        .iter()
        .enumerate()
        .filter_map(|(i, h)| match attachment(h) {
            Ok(Some(Attachment::Cap(cap))) => Some((i, cap)),
            _ => None,
        })
        .filter(|(_, cap)| {
            let Some(Shape::Path(path)) = layer.shapes.get(cap.shape) else { return false };
            let first = model_node(path, cap.node);
            let len = path.nodes.len().max(1);
            points.contains(&(cap.shape, first)) || points.contains(&(cap.shape, (first + 1) % len))
        })
        .collect()
}

fn mode_of(a: CornerAlignment) -> Option<String> {
    let mode = match a {
        CornerAlignment::Left => "left",
        CornerAlignment::Right => "right",
        CornerAlignment::Both => "both",
        CornerAlignment::Unaligned => "none",
        CornerAlignment::Unused => return None,
    };
    Some(mode.to_string())
}

fn selected_points(selection: &Selection) -> Vec<(usize, usize)> {
    selection.points.iter().copied().collect()
}

/// The selected hints of `layer` that `read` accepts, with what it read, in hint order.
fn selected_hints<T>(layer: &Layer, selection: &Selection, read: impl Fn(&Hint) -> Option<T>) -> Vec<(usize, T)> {
    selection.hints.iter().copied().filter_map(|i| layer.info.hints.get(i).and_then(&read).map(|v| (i, v))).collect()
}

/// The corner part for the selected corners of `layer`, else for the corners on the selected nodes.
pub fn corner_box(font: &Font, glyph: &str, layer: &Layer, selection: &Selection) -> Option<CornerBox> {
    let read = |h: &Hint| corner_hint(h).ok().flatten();
    let mut picked: Vec<(usize, CornerHint)> = selected_hints(layer, selection, read);
    if picked.is_empty() {
        let on = corners_on_nodes(layer, &selected_points(selection));
        picked = on.into_iter().filter_map(|i| layer.info.hints.get(i).and_then(read).map(|c| (i, c))).collect();
    }
    if picked.is_empty() {
        return None;
    }
    Some(CornerBox {
        glyph: glyph.to_string(),
        layer: layer.id.clone(),
        exists: picked.iter().all(|(_, c)| font.glyph(&c.name).is_some()),
        scale: [shared(picked.iter().map(|(_, c)| c.scale.x * 100.0)), shared(picked.iter().map(|(_, c)| c.scale.y * 100.0))],
        mode: shared(picked.iter().map(|(_, c)| mode_of(c.alignment))).flatten(),
        name: shared(picked.iter().map(|(_, c)| c.name.clone())),
        corners: picked.into_iter().map(|(i, _)| i).collect(),
    })
}

/// The cap part for the selected caps of `layer`, else for the caps on the selected nodes.
pub fn cap_box(font: &Font, glyph: &str, layer: &Layer, selection: &Selection) -> Option<CapBox> {
    let mut picked: Vec<(usize, CapHint)> = selected_hints(layer, selection, |h| match attachment(h) {
        Ok(Some(Attachment::Cap(cap))) => Some(cap),
        _ => None,
    });
    if picked.is_empty() {
        picked = caps_on_nodes(layer, &selected_points(selection));
    }
    if picked.is_empty() {
        return None;
    }
    Some(CapBox {
        glyph: glyph.to_string(),
        layer: layer.id.clone(),
        exists: picked.iter().all(|(_, c)| font.glyph(&c.name).is_some()),
        scale: [shared(picked.iter().map(|(_, c)| c.scale.x * 100.0)), shared(picked.iter().map(|(_, c)| c.scale.y * 100.0))],
        mode: shared(picked.iter().map(|(_, c)| mode_of(c.alignment))).flatten(),
        fit: shared(picked.iter().map(|(_, c)| c.fit)),
        name: shared(picked.iter().map(|(_, c)| c.name.clone())),
        caps: picked.into_iter().map(|(i, _)| i).collect(),
    })
}

/// The segment components (`brush` false) or brushes of `layer`, by hint index, whose host segment ends at one of
/// `points` (model order).
pub fn segments_on_nodes(layer: &Layer, points: &[(usize, usize)], brush: bool) -> Vec<(usize, SegmentHint)> {
    layer
        .info
        .hints
        .iter()
        .enumerate()
        .filter_map(|(i, h)| match (attachment(h), brush) {
            (Ok(Some(Attachment::Segment(s))), false) | (Ok(Some(Attachment::Brush(s))), true) => Some((i, s)),
            _ => None,
        })
        .filter(|(_, s)| {
            let Some(Shape::Path(path)) = layer.shapes.get(s.shape) else { return false };
            points.contains(&(s.shape, model_node(path, s.node)))
        })
        .collect()
}

/// The segment component part (`brush` false) or the brush part for the selected ones of `layer`, else for those
/// whose host segment ends at a selected node.
pub fn segment_box(font: &Font, glyph: &str, layer: &Layer, selection: &Selection, brush: bool) -> Option<SegmentBox> {
    let mut picked: Vec<(usize, SegmentHint)> = selected_hints(layer, selection, |h| match (attachment(h), brush) {
        (Ok(Some(Attachment::Segment(s))), false) | (Ok(Some(Attachment::Brush(s))), true) => Some(s),
        _ => None,
    });
    if picked.is_empty() {
        picked = segments_on_nodes(layer, &selected_points(selection), brush);
    }
    if picked.is_empty() {
        return None;
    }
    Some(SegmentBox {
        glyph: glyph.to_string(),
        layer: layer.id.clone(),
        exists: picked.iter().all(|(_, s)| font.glyph(&s.name).is_some()),
        name: shared(picked.iter().map(|(_, s)| s.name.clone())),
        hints: picked.into_iter().map(|(i, _)| i).collect(),
        brush,
    })
}

/// The component at shape `index` of `shapes` and its glyph, when that glyph has properties.
fn smart_component<'f>(font: &'f Font, shapes: &'f [Shape], index: usize) -> Option<(&'f Component, &'f Glyph)> {
    let Some(Shape::Component(c)) = shapes.get(index) else { return None };
    let base = font.glyph(&c.glyph)?;
    (!base.info.parts.is_empty()).then_some((c, base.as_ref()))
}

/// The value of `part` that component `c` of the smart glyph `base` draws in `master`: its stored value, else the end
/// of the range its glyph's default pole layer (the master layer when it is a pole layer, else the first) draws.
pub fn current_value(base: &Glyph, c: &Component, master: &str, part: &PartProperty) -> f64 {
    if let Some(v) = c.info.smart_values.iter().find(|v| v.property == part.name) {
        return v.value;
    }
    let mut poles = pole_layers(base, master);
    let first = poles.next();
    let default = if first.is_some_and(|l| l.is_master_layer()) { first } else { poles.find(|l| l.is_master_layer()).or(first) };
    match default.and_then(|l| l.info.attrs.smart_poles.iter().find(|p| p.property == part.name)) {
        Some(p) if p.top => part.top_value,
        _ => part.bottom_value,
    }
}

/// The smart components among `components` of `layer`'s `side` and one row per property, by name in order of first
/// appearance; the range comes from the first glyph that has the name. `None` when none of them is a smart component.
pub fn smart_model(font: &Font, glyph: &str, layer: &Layer, side: LayerSide, components: &[usize]) -> Option<SmartModel> {
    let shapes = layer.parts(side).shapes;
    let mut picked: Vec<usize> =
        components.iter().copied().filter(|&i| smart_component(font, shapes, i).is_some()).take(MAX_SMART_COMPONENTS).collect();
    picked.sort_unstable();
    picked.dedup();
    if picked.is_empty() {
        return None;
    }
    let mut rows: Vec<SmartRow> = Vec::new();
    for &i in &picked {
        let Some((c, base)) = smart_component(font, shapes, i) else { continue };
        for part in base.info.parts.iter().take(MAX_PART_PROPERTIES) {
            let value = current_value(base, c, &layer.master, part);
            if let Some(r) = rows.iter_mut().find(|r| r.name == part.name) {
                if r.value.is_some_and(|v| v != value) {
                    r.value = None;
                }
                r.components.push(i);
            } else if rows.len() < MAX_PART_PROPERTIES {
                rows.push(SmartRow {
                    name: part.name.clone(),
                    min: part.bottom_value,
                    max: part.top_value,
                    toggle: is_toggle(part),
                    value: Some(value),
                    components: vec![i],
                });
            }
        }
    }
    Some(SmartModel { glyph: glyph.to_string(), layer: layer.id.clone(), background: side == LayerSide::Background, components: picked, rows })
}

/// One attachment's highlight band: the part of its host path that fitting it put in, in font units.
#[derive(PlainData, Clone, Debug, PartialEq)]
pub struct AttachmentBand {
    /// The attachment's position in the layer's hint list: its selection address.
    pub hint: usize,
    #[plain_data(assume = "a kurbo path: owned points")]
    pub band: BezPath,
}

/// The highlight bands of `layer`'s attachments (corner, cap and segment components and brushes), at most [`attachment::MAX_ATTACHMENTS`]. An attachment that does
/// not fit has no band.
pub fn attachment_bands(font: &Font, layer: &Layer) -> Vec<AttachmentBand> {
    attachment::marks(font, layer, |_| true).into_iter().map(|m| AttachmentBand { hint: m.hint, band: m.band }).collect()
}

#[cfg(test)]
mod tests {
    use fontcraft_engine::model::Transform;

    use super::values;

    /// A mirrored matrix reads as a negative horizontal scale; a slanted one has no fields.
    #[test]
    fn matrices_split_into_scale_and_rotation() {
        let mirrored = values(&Transform::Matrix([-1.0, 0.0, 0.0, 1.0, 10.0, 20.0])).map(|v| (v.scale_x, v.scale_y, v.rotation, v.x));
        assert_eq!(mirrored, Some((-100.0, 100.0, 0.0, 10.0)));
        let turned = values(&Transform::Matrix([0.0, 2.0, -2.0, 0.0, 0.0, 0.0])).map(|v| (v.scale_x.round(), v.rotation.round()));
        assert_eq!(turned, Some((200.0, 90.0)));
        assert_eq!(values(&Transform::Matrix([1.0, 0.0, 0.5, 1.0, 0.0, 0.0])), None);
    }
}
