//! The view models of the component, corner, cap, segment component and brush parts of the Info box and of the Smart
//! Component controls, and the highlight bands that mark attachments on the canvas.
//!
//! | Model | Shows while |
//! |---|---|
//! | [`ComponentBox`] | a component of the edited layer is selected: the first in shape order |
//! | [`CornerBox`] | a corner component is selected, or a selected node holds one: the first selected corner, else the first on a selected node in the layer's hint order |
//! | [`CapBox`] | a cap component is selected, or a selected node is one of the two nodes of one |
//! | [`SegmentBox`] | a segment component (or brush) is selected, or a selected node ends the segment it sits on |
//! | [`SmartModel`] | smart components are selected (`palette.smartComponent`, `dialog.smartComponentSettings`) |

use craft::ui::contract::PlainData;
use fontcraft_engine::cmd::smart::is_toggle;
use fontcraft_engine::components::align::{AlignMode, component_alignment};
use fontcraft_engine::components::propagate::{base_layer, effective_anchors_with};
use fontcraft_engine::model::attach::{Attachment, CapHint, SegmentHint, attachment};
use fontcraft_engine::model::corner::{CornerAlignment, CornerTarget, corner_hint, is_corner};
use fontcraft_engine::model::glyphdata::GlyphData;
use fontcraft_engine::model::smart::pole_layers;
use fontcraft_engine::model::{Component, Font, Glyph, Layer, LayerSide, MAX_PART_PROPERTIES, PartProperty, Path, Shape, Transform};
use fontcraft_geom::BezPath;
use fontcraft_tools::Selection;
use fontcraft_tools::attachment;

/// Most components before the selected one whose anchors the anchor menu reads.
const MAX_ANCHOR_SOURCES: usize = 64;
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
    /// The component's shape index on the layer.
    pub component: usize,
    pub base: String,
    /// `None` for a matrix the box cannot split into its fields; X and Y still show.
    pub values: Option<ComponentValues>,
    pub x: f64,
    pub y: f64,
    /// X and Y can be edited: not while the component is aligned automatically (only Y while it is aligned
    /// horizontally), and neither while it is locked.
    pub x_free: bool,
    pub y_free: bool,
    pub locked: bool,
    /// The base glyph is a smart glyph: the Smart badge shows.
    pub smart: bool,
    /// The anchors the component can attach to; the menu shows when there are two or more.
    pub anchors: Vec<String>,
    /// The anchor it attaches to as the menu shows it: the stored choice, else the first plain name offered.
    pub anchor: Option<String>,
    /// How many components are selected.
    pub selected: usize,
}

/// The corner component part of the Info box.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct CornerBox {
    pub glyph: String,
    pub layer: String,
    /// The corner's index in the layer's hint list.
    pub corner: usize,
    /// The corner glyph.
    pub name: String,
    /// Horizontal and vertical scale in percent.
    pub scale: [f64; 2],
    /// The mode of [`MODES`] the corner has; `None` for the stored value that fits it unturned.
    pub mode: Option<String>,
    /// The corner glyph exists.
    pub exists: bool,
}

/// The cap component part of the Info box.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct CapBox {
    pub glyph: String,
    pub layer: String,
    /// The cap's index in the layer's hint list.
    pub cap: usize,
    /// The cap glyph.
    pub name: String,
    /// Horizontal and vertical scale in percent; with Fit on the horizontal one is computed and greyed out.
    pub scale: [f64; 2],
    pub mode: Option<String>,
    /// Sized to the stroke's width.
    pub fit: bool,
    pub exists: bool,
}

/// The segment component or brush part of the Info box: the glyph's name and the arrow button that edits it.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct SegmentBox {
    pub glyph: String,
    pub layer: String,
    /// The attachment's index in the layer's hint list.
    pub hint: usize,
    /// The segment or brush glyph.
    pub name: String,
    pub exists: bool,
    /// A brush rather than a segment component.
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

/// The component part for the first selected component of `layer`; `None` when no component is selected.
pub fn component_box(font: &Font, data: Option<&GlyphData>, glyph: &str, layer: &Layer, selection: &Selection) -> Option<ComponentBox> {
    let components: Vec<usize> = selection.shapes.iter().copied().filter(|&i| matches!(layer.shapes.get(i), Some(Shape::Component(_)))).collect();
    let index = *components.first()?;
    let Some(Shape::Component(c)) = layer.shapes.get(index) else { return None };
    let locked = c.info.locked;
    let (x_free, y_free) = match (locked, component_alignment(font, layer, index)) {
        (true, _) | (false, Some(AlignMode::Full)) => (false, false),
        (false, Some(AlignMode::Horizontal)) => (false, true),
        (false, None) => (true, true),
    };
    let anchors = anchor_choices(font, data, layer, index);
    let anchor = c.info.anchor.clone().or_else(|| {
        let plain: Vec<String> = font
            .glyph(&c.glyph)
            .and_then(|g| base_layer(g, layer, c))
            .map(|l| l.anchors.iter().filter_map(|a| a.name.strip_prefix('_').map(str::to_string)).collect())
            .unwrap_or_default();
        anchors.iter().find(|a| plain.contains(a)).cloned()
    });
    let [_, _, _, _, x, y] = c.transform.affine().as_coeffs();
    Some(ComponentBox {
        glyph: glyph.to_string(),
        layer: layer.id.clone(),
        component: index,
        base: c.glyph.clone(),
        values: values(&c.transform),
        x,
        y,
        x_free,
        y_free,
        locked,
        smart: font.glyph(&c.glyph).is_some_and(|g| !g.info.parts.is_empty()),
        anchors,
        anchor,
        selected: components.len(),
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

/// The corner part for the first selected corner of `layer`, else the corner on the first selected node that holds
/// one.
pub fn corner_box(font: &Font, glyph: &str, layer: &Layer, selection: &Selection) -> Option<CornerBox> {
    let picked = selection.hints.iter().copied().find(|&i| layer.info.hints.get(i).is_some_and(is_corner));
    let corner = match picked {
        Some(i) => i,
        None => *corners_on_nodes(layer, &selected_points(selection)).first()?,
    };
    let hint = corner_hint(layer.info.hints.get(corner)?).ok().flatten()?;
    Some(CornerBox {
        glyph: glyph.to_string(),
        layer: layer.id.clone(),
        corner,
        exists: font.glyph(&hint.name).is_some(),
        name: hint.name,
        scale: [hint.scale.x * 100.0, hint.scale.y * 100.0],
        mode: mode_of(hint.alignment),
    })
}

/// The cap part for the first selected cap of `layer`, else the first cap that sits on a selected node.
pub fn cap_box(font: &Font, glyph: &str, layer: &Layer, selection: &Selection) -> Option<CapBox> {
    let picked = selection.hints.iter().copied().find_map(|i| match layer.info.hints.get(i).map(attachment) {
        Some(Ok(Some(Attachment::Cap(cap)))) => Some((i, cap)),
        _ => None,
    });
    let (cap, hint) = match picked {
        Some(found) => found,
        None => caps_on_nodes(layer, &selected_points(selection)).into_iter().next()?,
    };
    Some(CapBox {
        glyph: glyph.to_string(),
        layer: layer.id.clone(),
        cap,
        exists: font.glyph(&hint.name).is_some(),
        scale: [hint.scale.x * 100.0, hint.scale.y * 100.0],
        mode: mode_of(hint.alignment),
        fit: hint.fit,
        name: hint.name,
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

/// The segment component part (`brush` false) or the brush part for the first selected one of `layer`, else the
/// first whose host segment ends at a selected node.
pub fn segment_box(font: &Font, glyph: &str, layer: &Layer, selection: &Selection, brush: bool) -> Option<SegmentBox> {
    let picked = selection.hints.iter().copied().find_map(|i| match (layer.info.hints.get(i).map(attachment), brush) {
        (Some(Ok(Some(Attachment::Segment(s)))), false) | (Some(Ok(Some(Attachment::Brush(s)))), true) => Some((i, s)),
        _ => None,
    });
    let (hint, s) = match picked {
        Some(found) => found,
        None => segments_on_nodes(layer, &selected_points(selection), brush).into_iter().next()?,
    };
    Some(SegmentBox { glyph: glyph.to_string(), layer: layer.id.clone(), hint, exists: font.glyph(&s.name).is_some(), name: s.name, brush })
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
