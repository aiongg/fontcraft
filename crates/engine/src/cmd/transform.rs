//! Transformations: the Palette's mirror, scale, rotate, slant and align buttons, Path › Align
//! Selection, and Path › Transformations with its export-filter string.
//!
//! Every transformation is one affine map about an origin, applied by [`apply`]. The Palette
//! buttons, the Transformations dialog, the bounding-box knobs and the Rotate and Scale tools all
//! call these commands; a tool passes its reference point as `origin` and runs the command inside
//! an interaction, so a drag is one undo step.
//!
//! The Palette's mirror, scale, rotate and slant commands also take `image: true`, which adds the
//! layer's placed image to what they change (without target lists, the image alone); a box
//! origin then includes the image's shown part.

use std::collections::{BTreeSet, HashSet};
use std::fmt::Write as _;
use std::sync::Arc;

use fontcraft_geom::{Affine, Point, Rect, Shape as _, Vec2, round_point};
use fontcraft_model::{Font, Guide, Layer, LayerParts, LayerSide, MAX_COORDINATE, MAX_GLYPHS, Master, Metrics, Node, Shape, Transform, metric};
use serde_json::{Map, Value, json};

use super::common::{grid, in_range, master_metrics};
use super::*;
use crate::EngineError;
use crate::params::{GuideScope, Params, TargetKind, Targets, kind};

/// A node as (shape index, node index).
type NodeRef = (usize, usize);

/// Largest scale, in percent (a factor of 10,000).
const MAX_PERCENT: f64 = 1_000_000.0;
/// Largest linear coefficient a component's or image's matrix may reach.
const MAX_MATRIX_FACTOR: f64 = 10_000.0;
/// Most layer ids `layers` may hold.
const MAX_LAYERS: usize = 1024;
/// Most layers one batch may change (glyphs × layers).
const MAX_LAYER_JOBS: usize = 1_000_000;
/// Longest export-filter string read, in bytes.
const MAX_FILTER_STRING: usize = 1 << 20;

const TRANSFORM_TARGETS: &[TargetKind] =
    &[TargetKind::Glyphs, TargetKind::Layer, TargetKind::Points, TargetKind::Segments, TargetKind::Shapes, TargetKind::Anchors, TargetKind::Guides];
const ALIGN_TARGETS: &[TargetKind] = &[TargetKind::Layer, TargetKind::Points, TargetKind::Segments, TargetKind::Shapes, TargetKind::Anchors];

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "transform.mirrorHorizontal",
            "Mirror Horizontally",
            [],
            None,
            "{glyph?: name, glyphs?: [names], layer?: id, side?: \"foreground\" | \"background\", points?: [[shape, node]], segments?: [[shape, node]], shapes?: [index], anchors?: [name], guides?: [{scope: \"local\" | \"global\", index}], annotations?: [index], image?: bool, origin?: origin} flips left to right across the vertical line through the origin. Target lists go with `glyph`; without them, or with `glyphs`, the whole layer (shapes and anchors) is changed. `image: true` (with `glyph`) changes the layer's placed image on that side too, and alone without target lists; a locked image, or none, refuses it. An on-curve node carries its handles. origin: {mode: \"box\" (default) | \"reference\" | \"metrics\" | \"layer\", boxPoint?: \"topLeft\" | \"top\" | \"topRight\" | \"left\" | \"center\" (default) | \"right\" | \"bottomLeft\" | \"bottom\" | \"bottomRight\", metric?: \"baseline\" | \"halfXHeight\" | \"xHeight\" | \"halfCapHeight\" | \"capHeight\", point?: [x, y]}, a metric name, or [x, y]. A metrics origin sits at half the advance width; the layer origin at half the width, midway between ascender and descender. Points and anchors are rounded to the grid. With `corners: [hint indices]` (and `glyph`, `layer?`; no other target or origin) it flips those corner components in place instead, as cornerComponent.setScale with `flip` does, and returns what that returns → {glyphs: [names], layers: count, origin?: [x, y] (with one layer)}",
            has_doc,
            mirror_horizontal
        )
        .targets(TRANSFORM_TARGETS),
        cmd!(
            "transform.mirrorVertical",
            "Mirror Vertically",
            [],
            None,
            "{glyph?: name, glyphs?: [names], layer?: id, side?, points?: [[shape, node]], segments?, shapes?: [index], anchors?: [name], guides?: [{scope, index}], annotations?, image?: bool, origin?: origin} flips top to bottom across the horizontal line through the origin; targets, `image`, origin and `corners` as for transform.mirrorHorizontal → {glyphs: [names], layers: count, origin?: [x, y]}",
            has_doc,
            mirror_vertical
        )
        .targets(TRANSFORM_TARGETS),
        cmd!(
            "transform.scale",
            "Scale",
            [],
            None,
            "{glyph?: name, glyphs?: [names], layer?: id, side?, points?, segments?, shapes?, anchors?, guides?, annotations?, image?: bool, x: percent, y?: percent (default x), origin?: origin} scales about the origin; 100 changes nothing, a negative value also mirrors; at most ±1,000,000. Targets, `image` and origin as for transform.mirrorHorizontal → {glyphs: [names], layers: count, origin?: [x, y]}",
            has_doc,
            scale
        )
        .targets(TRANSFORM_TARGETS),
        cmd!(
            "transform.reverseScale",
            "Reverse Scale",
            [],
            None,
            "{glyph?: name, glyphs?: [names], layer?: id, side?, points?, segments?, shapes?, anchors?, guides?, annotations?, image?: bool, x: percent, y?: percent (default x), origin?: origin} scales by 100 / percent, undoing transform.scale with the same values; 0 is refused → {glyphs: [names], layers: count, origin?: [x, y]}",
            has_doc,
            reverse_scale
        )
        .targets(TRANSFORM_TARGETS),
        cmd!(
            "transform.rotateClockwise",
            "Rotate Clockwise",
            [],
            None,
            "{glyph?: name, glyphs?: [names], layer?: id, side?, points?, segments?, shapes?, anchors?, guides?, annotations?, image?: bool, angle: degrees, origin?: origin} rotates clockwise about the origin; multiples of 90° are exact → {glyphs: [names], layers: count, origin?: [x, y]}",
            has_doc,
            rotate_clockwise
        )
        .targets(TRANSFORM_TARGETS),
        cmd!(
            "transform.rotateCounterclockwise",
            "Rotate Counterclockwise",
            [],
            None,
            "{glyph?: name, glyphs?: [names], layer?: id, side?, points?, segments?, shapes?, anchors?, guides?, annotations?, image?: bool, angle: degrees, origin?: origin} rotates counterclockwise about the origin; multiples of 90° are exact → {glyphs: [names], layers: count, origin?: [x, y]}",
            has_doc,
            rotate_counterclockwise
        )
        .targets(TRANSFORM_TARGETS),
        cmd!(
            "transform.slantLeft",
            "Slant Left",
            [],
            None,
            "{glyph?: name, glyphs?: [names], layer?: id, side?, points?, segments?, shapes?, anchors?, guides?, annotations?, image?: bool, angle: degrees (between -90 and 90), origin?: origin} shears horizontally by the tangent of the angle, leaning left; the origin's height stays fixed → {glyphs: [names], layers: count, origin?: [x, y]}",
            has_doc,
            slant_left
        )
        .targets(TRANSFORM_TARGETS),
        cmd!(
            "transform.slantRight",
            "Slant Right",
            [],
            None,
            "{glyph?: name, glyphs?: [names], layer?: id, side?, points?, segments?, shapes?, anchors?, guides?, annotations?, image?: bool, angle: degrees (between -90 and 90), origin?: origin} shears horizontally by the tangent of the angle, leaning right; the origin's height stays fixed → {glyphs: [names], layers: count, origin?: [x, y]}",
            has_doc,
            slant_right
        )
        .targets(TRANSFORM_TARGETS),
        cmd!(
            "transform.slantDown",
            "Slant Down",
            [],
            None,
            "{glyph?: name, glyphs?: [names], layer?: id, side?, points?, segments?, shapes?, anchors?, guides?, annotations?, image?: bool, angle: degrees (between -90 and 90), origin?: origin} shears vertically by the tangent of the angle: what lies right of the origin moves down, what lies left moves up → {glyphs: [names], layers: count, origin?: [x, y]}",
            has_doc,
            slant_down
        )
        .targets(TRANSFORM_TARGETS),
        cmd!(
            "transform.slantUp",
            "Slant Up",
            [],
            None,
            "{glyph?: name, glyphs?: [names], layer?: id, side?, points?, segments?, shapes?, anchors?, guides?, annotations?, image?: bool, angle: degrees (between -90 and 90), origin?: origin} shears vertically by the tangent of the angle: what lies right of the origin moves up, what lies left moves down → {glyphs: [names], layers: count, origin?: [x, y]}",
            has_doc,
            slant_up
        )
        .targets(TRANSFORM_TARGETS),
        cmd!(
            "transform.alignLeft",
            "Align Left",
            [],
            None,
            "{glyph: name, layer?: id, side?, points?: [[shape, node]], segments?: [[shape, node]], shapes?: [index], anchors?: [name]} moves each selected object so its left edge meets the left edge of the selection's box. Objects: each shape, each on-curve node with its handles, each lone handle, each anchor; without target lists, every shape of the layer. Ignores the transformation origin. Fewer than two objects change nothing → {glyph, moved: count, message?}",
            has_doc,
            align_left
        )
        .targets(ALIGN_TARGETS),
        cmd!(
            "transform.alignCenterHorizontal",
            "Align Horizontal Centers",
            [],
            None,
            "{glyph: name, layer?: id, side?, points?, segments?, shapes?, anchors?} moves each selected object so its horizontal centre is the centre of the selection's box: they share one vertical axis. Objects as for transform.alignLeft → {glyph, moved: count, message?}",
            has_doc,
            align_center_horizontal
        )
        .targets(ALIGN_TARGETS),
        cmd!(
            "transform.alignRight",
            "Align Right",
            [],
            None,
            "{glyph: name, layer?: id, side?, points?, segments?, shapes?, anchors?} moves each selected object so its right edge meets the right edge of the selection's box. Objects as for transform.alignLeft → {glyph, moved: count, message?}",
            has_doc,
            align_right
        )
        .targets(ALIGN_TARGETS),
        cmd!(
            "transform.alignTop",
            "Align Top",
            [],
            None,
            "{glyph: name, layer?: id, side?, points?, segments?, shapes?, anchors?} moves each selected object so its top edge meets the top edge of the selection's box. Objects as for transform.alignLeft → {glyph, moved: count, message?}",
            has_doc,
            align_top
        )
        .targets(ALIGN_TARGETS),
        cmd!(
            "transform.alignCenterVertical",
            "Align Vertical Centers",
            [],
            None,
            "{glyph: name, layer?: id, side?, points?, segments?, shapes?, anchors?} moves each selected object so its vertical centre is the centre of the selection's box: they share one horizontal axis. Objects as for transform.alignLeft → {glyph, moved: count, message?}",
            has_doc,
            align_center_vertical
        )
        .targets(ALIGN_TARGETS),
        cmd!(
            "transform.alignBottom",
            "Align Bottom",
            [],
            None,
            "{glyph: name, layer?: id, side?, points?, segments?, shapes?, anchors?} moves each selected object so its bottom edge meets the bottom edge of the selection's box. Objects as for transform.alignLeft → {glyph, moved: count, message?}",
            has_doc,
            align_bottom
        )
        .targets(ALIGN_TARGETS),
        cmd!(
            "path.alignSelection",
            "Align Selection",
            ["Path"],
            Some("Cmd+Shift+A"),
            "{glyph: name, layer?: id, side?, points?: [[shape, node]], segments?: [[shape, node]], shapes?: [index] (paths, or one component), anchors?: [name], axis?: \"auto\" (default) | \"vertical\" | \"horizontal\", origin?: origin (default the box centre)} lines the selected nodes, handles and anchors up on one coordinate. vertical: they share one x; horizontal: one y; auto collapses the smaller of the selection's width and height (a tie shares one x). The coordinate is the origin's (see transform.mirrorHorizontal): the box's left, centre or right for a box origin. A smooth node's handles move with it. One node alone moves onto the nearest node of the background. One anchor with one point moves horizontally above or below the point, with two points between them, along the master's italic angle; the points stay. One component (in `shapes`) with one node moves so its origin point lands on the node: the base glyph's `origin` anchor, else where its baseline meets its left sidebearing line; a locked or automatically aligned component refuses it. Fewer than two points change nothing → {glyph, moved: count, axis?: \"x\" | \"y\", coordinate?, anchor?, x?, component?, origin?: [x, y], message?}",
            has_doc,
            align_selection
        )
        .targets(ALIGN_TARGETS),
        cmd!(
            "path.transformations",
            "Transformations",
            ["Path"],
            None,
            "{glyph?: name, glyphs?: [names], layer?: id, layers?: [ids] (with glyphs; default the first master's layer), side?, points?: [[shape, node]], segments?, shapes?: [index] (target lists with glyph; without them, whole layers), translate?: {x, y}, scale?: {x: percent, y?: percent (default x)}, slant?: degrees, cursivy?: bool, origin?: \"capHeight\" | \"halfCapHeight\" | \"xHeight\" | \"halfXHeight\" | \"baseline\" (default) or an origin object as for transform.mirrorHorizontal, string?: export-filter string} scales, then slants, about the origin, then translates. `string` takes the place of the other settings, e.g. \"Transformations;ScaleX:90;ScaleY:90;Slant:10;SlantCorrection:0;OffsetX:20;Origin:4;\" (Origin 0 cap height … 4 baseline; include: or exclude: name the glyphs); with `string` and no glyph or glyphs, every glyph is changed. cursivy needs the master's stems and is not supported yet. Points and anchors are rounded to the grid → {glyphs: [names], layers: count, origin?: [x, y]}",
            has_doc,
            transformations
        )
        .targets(&[TargetKind::Glyphs, TargetKind::Layer, TargetKind::Points, TargetKind::Segments, TargetKind::Shapes]),
        cmd!(
            query "path.transformations.copyFilterParameter",
            "Copy Filter Parameter",
            [],
            None,
            "{translate?: {x, y}, scale?: {x: percent, y?: percent}, slant?: degrees, cursivy?: bool, origin?: \"capHeight\" | \"halfCapHeight\" | \"xHeight\" | \"halfXHeight\" | \"baseline\" (default)} the Transformations settings as the value of a Filter custom parameter: no spaces, a semicolon after every item, settings at their neutral value left out, the origin always written → {text, parameter: \"Filter\"}",
            always,
            copy_filter_parameter
        ),
        cmd!(
            query "path.transformations.copyPreFilterParameter",
            "Copy PreFilter Parameter",
            [],
            None,
            "{translate?: {x, y}, scale?: {x: percent, y?: percent}, slant?: degrees, cursivy?: bool, origin?: \"capHeight\" | \"halfCapHeight\" | \"xHeight\" | \"halfXHeight\" | \"baseline\" (default)} the same text as path.transformations.copyFilterParameter, for a PreFilter custom parameter → {text, parameter: \"PreFilter\"}",
            always,
            copy_prefilter_parameter
        ),
    ]
}

// ── Origins ─────────────────────────────────────────────────────────────────────────────────────

/// One of the nine points of a selection's box.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BoxPoint {
    TopLeft,
    Top,
    TopRight,
    Left,
    Center,
    Right,
    BottomLeft,
    Bottom,
    BottomRight,
}

impl BoxPoint {
    const NAMES: [(&'static str, BoxPoint); 9] = [
        ("topLeft", BoxPoint::TopLeft),
        ("top", BoxPoint::Top),
        ("topRight", BoxPoint::TopRight),
        ("left", BoxPoint::Left),
        ("center", BoxPoint::Center),
        ("right", BoxPoint::Right),
        ("bottomLeft", BoxPoint::BottomLeft),
        ("bottom", BoxPoint::Bottom),
        ("bottomRight", BoxPoint::BottomRight),
    ];

    pub fn parse(s: &str) -> Option<Self> {
        Self::NAMES.iter().find(|(n, _)| *n == s).map(|(_, b)| *b)
    }

    /// The point in `r`. Centres are computed as the mean of the two edges, so a box mapped onto
    /// itself (a mirror, a quarter turn) keeps the same centre to the last bit.
    pub fn point_in(self, r: Rect) -> Point {
        use BoxPoint::*;
        let x = match self {
            TopLeft | Left | BottomLeft => r.x0,
            Top | Center | Bottom => (r.x0 + r.x1) / 2.0,
            TopRight | Right | BottomRight => r.x1,
        };
        let y = match self {
            BottomLeft | Bottom | BottomRight => r.y0,
            Left | Center | Right => (r.y0 + r.y1) / 2.0,
            TopLeft | Top | TopRight => r.y1,
        };
        Point::new(x, y)
    }
}

/// A height taken from the master's metrics. The order is the export string's `Origin` code.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MetricPoint {
    CapHeight,
    HalfCapHeight,
    XHeight,
    HalfXHeight,
    Baseline,
}

impl MetricPoint {
    const ALL: [(&'static str, MetricPoint); 5] = [
        ("capHeight", MetricPoint::CapHeight),
        ("halfCapHeight", MetricPoint::HalfCapHeight),
        ("xHeight", MetricPoint::XHeight),
        ("halfXHeight", MetricPoint::HalfXHeight),
        ("baseline", MetricPoint::Baseline),
    ];

    pub fn parse(s: &str) -> Option<Self> {
        Self::ALL.iter().find(|(n, _)| *n == s).map(|(_, m)| *m)
    }

    pub fn name(self) -> &'static str {
        Self::ALL.iter().find(|(_, m)| *m == self).map_or("baseline", |(n, _)| n)
    }

    /// The export string's `Origin` value: 0 cap height … 4 baseline.
    pub fn code(self) -> u8 {
        match self {
            MetricPoint::CapHeight => 0,
            MetricPoint::HalfCapHeight => 1,
            MetricPoint::XHeight => 2,
            MetricPoint::HalfXHeight => 3,
            MetricPoint::Baseline => 4,
        }
    }

    pub fn from_code(code: u8) -> Option<Self> {
        Self::ALL.get(usize::from(code)).map(|(_, m)| *m)
    }

    fn y(self, m: &Metrics, baseline: f64) -> f64 {
        match self {
            MetricPoint::CapHeight => m.cap_height,
            MetricPoint::HalfCapHeight => m.cap_height / 2.0,
            MetricPoint::XHeight => m.x_height,
            MetricPoint::HalfXHeight => m.x_height / 2.0,
            MetricPoint::Baseline => baseline,
        }
    }
}

/// The fixed point of a transformation (`transform.setOrigin` keeps the shell's choice).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Origin {
    /// A point of the box around what is transformed.
    Box(BoxPoint),
    /// A point the user placed (the Rotate and Scale tools' click).
    Reference(Point),
    /// A metric height of the layer's master, at half the advance width.
    Metric(MetricPoint),
    /// The centre of the layer: half the width, midway between ascender and descender.
    Layer,
}

/// Reads `origin`; `default` when it is absent.
fn parse_origin(p: &Params, default: Origin) -> Result<Origin> {
    let Some(v) = p.get("origin") else { return Ok(default) };
    match v {
        Value::String(s) => MetricPoint::parse(s)
            .map(Origin::Metric)
            .ok_or_else(|| p.bad(format!("`origin`: \"{s}\" is not capHeight, halfCapHeight, xHeight, halfXHeight or baseline"))),
        Value::Array(_) => point(p, "origin", v).map(Origin::Reference),
        Value::Object(o) => {
            let field = |key: &str| o.get(key).filter(|v| !v.is_null());
            let text = |key: &str| match field(key) {
                None => Ok(None),
                Some(Value::String(s)) => Ok(Some(s.as_str())),
                Some(other) => Err(p.bad(format!("`origin.{key}` must be a string, got {}", kind(other)))),
            };
            match text("mode")?.unwrap_or("box") {
                "box" => match text("boxPoint")? {
                    None => Ok(Origin::Box(BoxPoint::Center)),
                    Some(s) => BoxPoint::parse(s).map(Origin::Box).ok_or_else(|| {
                        p.bad(format!(
                            "`origin.boxPoint`: \"{s}\" is not topLeft, top, topRight, left, center, right, bottomLeft, bottom or bottomRight"
                        ))
                    }),
                },
                "reference" => {
                    let v = field("point").ok_or_else(|| p.bad("`origin`: mode \"reference\" needs `point: [x, y]`"))?;
                    point(p, "origin.point", v).map(Origin::Reference)
                }
                "metrics" => match text("metric")? {
                    None => Ok(Origin::Metric(MetricPoint::Baseline)),
                    Some(s) => MetricPoint::parse(s)
                        .map(Origin::Metric)
                        .ok_or_else(|| p.bad(format!("`origin.metric`: \"{s}\" is not baseline, halfXHeight, xHeight, halfCapHeight or capHeight"))),
                },
                "layer" => Ok(Origin::Layer),
                other => Err(p.bad(format!("`origin.mode` must be box, reference, metrics or layer, got \"{other}\""))),
            }
        }
        other => Err(p.bad(format!("`origin` must be an object {{mode, boxPoint?, metric?, point?}}, a metric name or [x, y], got {}", kind(other)))),
    }
}

/// An `[x, y]` point within the coordinate limits.
fn point(p: &Params, key: &str, v: &Value) -> Result<Point> {
    if let Value::Array(a) = v
        && let [x, y] = a.as_slice()
        && let (Some(x), Some(y)) = (x.as_f64(), y.as_f64())
        && in_range(x)
        && in_range(y)
    {
        return Ok(Point::new(x, y));
    }
    Err(p.bad(format!("`{key}` must be [x, y], two numbers within ±{MAX_COORDINATE}")))
}

/// Where `origin` is for one layer. `bounds` is the box of what is transformed (used by a box
/// origin; with nothing to transform the origin does not matter).
pub fn origin_point(origin: Origin, bounds: Option<Rect>, font: &Font, master: &Master, width: f64) -> Point {
    match origin {
        Origin::Box(b) => bounds.map_or(Point::ZERO, |r| b.point_in(r)),
        Origin::Reference(p) => p,
        Origin::Metric(m) => Point::new(width / 2.0, m.y(&master_metrics(font, master), font.metric_pos(master, metric::BASELINE, 0.0))),
        Origin::Layer => {
            let m = master_metrics(font, master);
            Point::new(width / 2.0, (m.ascender + m.descender) / 2.0)
        }
    }
}

// ── What is transformed ────────────────────────────────────────────────────────────────────────

/// What a transformation moves in one side of a layer. Every index was checked by
/// [`Targets::resolve`].
#[derive(Clone, Debug, Default)]
pub struct Selection {
    /// The addressed nodes and handles (the selection's box is measured from these).
    pub named_points: BTreeSet<(usize, usize)>,
    /// Every point moved: the addressed ones, the handles of addressed on-curve nodes and the
    /// handles of addressed segments.
    pub points: BTreeSet<(usize, usize)>,
    /// Whole shapes: every node of a path, the placement of a component or image.
    pub shapes: BTreeSet<usize>,
    pub anchors: BTreeSet<usize>,
    pub local_guides: BTreeSet<usize>,
    pub global_guides: BTreeSet<usize>,
    pub annotations: BTreeSet<usize>,
    /// No target lists: the whole layer (shapes and anchors).
    pub whole_layer: bool,
}

/// What `t` moves in `parts`: its lists, or the whole layer when it has none.
pub fn selection(t: &Targets, parts: LayerParts) -> Selection {
    let mut sel = Selection::default();
    if !t.has_lists() {
        sel.shapes = (0..parts.shapes.len()).collect();
        sel.anchors = (0..parts.anchors.len()).collect();
        sel.whole_layer = true;
        return sel;
    }
    sel.shapes = t.shapes.iter().copied().collect();
    sel.named_points.extend(t.points.iter().copied());
    for &(s, n) in &t.segments {
        if let Some(seg) = parts.path(s).and_then(|p| p.segment_ending_at(n)) {
            sel.named_points.insert((s, seg.start));
            sel.named_points.insert((s, seg.end));
            sel.points.extend(seg.handles.iter().map(|h| (s, *h)));
        }
    }
    // The points of a whole shape move with it.
    sel.named_points.retain(|(s, _)| !sel.shapes.contains(s));
    sel.points.retain(|(s, _)| !sel.shapes.contains(s));
    for &(s, n) in &sel.named_points {
        sel.points.insert((s, n));
        sel.points.extend(handles_of(parts, s, n));
    }
    sel.anchors = t.anchors.iter().filter_map(|name| parts.anchor(name).map(|(i, _)| i)).collect();
    for g in &t.guides {
        match g.scope {
            GuideScope::Local => sel.local_guides.insert(g.index),
            GuideScope::Global => sel.global_guides.insert(g.index),
        };
    }
    sel.annotations = t.annotations.iter().copied().collect();
    sel
}

/// The off-curve neighbours of on-curve node `n`: the handles that belong to it. A lone
/// off-curve node has none.
fn handles_of(parts: LayerParts, s: usize, n: usize) -> Vec<(usize, usize)> {
    let Some(path) = parts.path(s) else { return Vec::new() };
    if !path.nodes.get(n).is_some_and(Node::on_curve) {
        return Vec::new();
    }
    let len = path.nodes.len();
    let next = if n + 1 < len {
        Some(n + 1)
    } else if path.closed {
        Some(0)
    } else {
        None
    };
    let prev = if n > 0 {
        Some(n - 1)
    } else if path.closed {
        len.checked_sub(1)
    } else {
        None
    };
    [prev, next].into_iter().flatten().filter(|&i| i != n && path.nodes.get(i).is_some_and(|x| !x.on_curve())).map(|i| (s, i)).collect()
}

/// The box of one shape: a path's curve, a component's outline, an image's placement point.
fn shape_box(font: &Font, master: &str, shape: &Shape) -> Option<Rect> {
    match shape {
        Shape::Path(p) => match p.to_bez() {
            Ok(b) if !b.elements().is_empty() => Some(b.bounding_box()),
            _ => fontcraft_geom::bounds(p.nodes.iter().map(Node::point)),
        },
        Shape::Component(c) => match font.component_outline(c, master) {
            Ok(b) if !b.elements().is_empty() => Some(b.bounding_box()),
            _ => Some(dot(c.affine() * Point::ZERO)),
        },
        Shape::Image(img) => Some(dot(img.transform.affine() * Point::ZERO)),
        Shape::Raw(_) => None,
    }
}

fn dot(p: Point) -> Rect {
    Rect::from_points(p, p)
}

fn union(a: Option<Rect>, b: Option<Rect>) -> Option<Rect> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.union(b)),
        (a, b) => a.or(b),
    }
}

/// The box of a selection, the one a box origin and the bounding-box knobs use: addressed
/// points and whole shapes, then anchors, guides and annotations. For a whole layer, the shapes
/// alone, or the anchors when it has no shapes.
pub fn selection_box(font: &Font, master: &Master, parts: LayerParts, sel: &Selection) -> Option<Rect> {
    let points = fontcraft_geom::bounds(sel.named_points.iter().filter_map(|(s, n)| parts.node(*s, *n)).map(Node::point));
    let shapes = sel.shapes.iter().filter_map(|i| parts.shapes.get(*i)).filter_map(|s| shape_box(font, &master.id, s)).reduce(|a, b| a.union(b));
    let anchors = fontcraft_geom::bounds(sel.anchors.iter().filter_map(|i| parts.anchors.get(*i)).map(|a| Point::new(a.x, a.y)));
    let main = union(points, shapes);
    if sel.whole_layer {
        return main.or(anchors);
    }
    let guides = fontcraft_geom::bounds(
        sel.local_guides
            .iter()
            .filter_map(|i| parts.guides.get(*i))
            .chain(sel.global_guides.iter().filter_map(|i| master.guides.get(*i)))
            .map(|g| Point::new(g.x, g.y)),
    );
    let notes = fontcraft_geom::bounds(sel.annotations.iter().filter_map(|i| parts.annotations.get(*i)).map(|a| Point::new(a.x, a.y)));
    union(union(main, anchors), union(guides, notes))
}

// ── Applying a map ─────────────────────────────────────────────────────────────────────────────

/// `lin` about `o`: the map that keeps `o` where it is.
fn about(o: Point, lin: Affine) -> Affine {
    Affine::translate(o.to_vec2()) * lin * Affine::translate(-o.to_vec2())
}

/// A counterclockwise rotation. Quarter turns use exact sines and cosines, so four of them, or
/// one each way, give back the start coordinates.
fn rotation(degrees: f64) -> Affine {
    let a = degrees.rem_euclid(360.0);
    let (c, s) = if a == 0.0 {
        (1.0, 0.0)
    } else if a == 90.0 {
        (0.0, 1.0)
    } else if a == 180.0 {
        (-1.0, 0.0)
    } else if a == 270.0 {
        (0.0, -1.0)
    } else {
        (a.to_radians().cos(), a.to_radians().sin())
    };
    Affine::new([c, s, -s, c, 0.0, 0.0])
}

/// `p` mapped by `m`, rounded to the grid, within the coordinate limits.
fn map_point(p: &Params, m: Affine, at: Point, step: f64) -> Result<Point> {
    let q = round_point(m * at, step);
    if in_range(q.x) && in_range(q.y) {
        Ok(q)
    } else {
        Err(p.bad(format!("the result puts a point at ({}, {}), beyond ±{MAX_COORDINATE}", q.x, q.y)))
    }
}

/// A placement followed by `m`. A placement without rotation or slant keeps its decomposed form
/// under a map without rotation or slant; anything else becomes a matrix.
fn compose(p: &Params, t: &Transform, m: Affine) -> Result<Transform> {
    let [a, b, c, d, e, f] = m.as_coeffs();
    let out = match *t {
        Transform::Decomposed { pos, scale, angle, slant } if b == 0.0 && c == 0.0 && angle == 0.0 && slant == (0.0, 0.0) => {
            Transform::Decomposed { pos: (a * pos.0 + e, d * pos.1 + f), scale: (scale.0 * a, scale.1 * d), angle, slant }
        }
        _ => Transform::Matrix((m * t.affine()).as_coeffs()),
    };
    let [a, b, c, d, e, f] = out.affine().as_coeffs();
    if [a, b, c, d].iter().all(|v| v.is_finite() && v.abs() <= MAX_MATRIX_FACTOR) && in_range(e) && in_range(f) {
        Ok(out)
    } else {
        Err(p.bad(format!("the result scales a component or image beyond {MAX_MATRIX_FACTOR}× or moves it beyond ±{MAX_COORDINATE}")))
    }
}

/// A guide through the mapped point, along the mapped direction.
fn map_guide(p: &Params, g: &mut Guide, m: Affine) -> Result<()> {
    let q = m * Point::new(g.x, g.y);
    if !(in_range(q.x) && in_range(q.y)) {
        return Err(p.bad(format!("the result puts a guide at ({}, {}), beyond ±{MAX_COORDINATE}", q.x, q.y)));
    }
    let [a, b, c, d, _, _] = m.as_coeffs();
    let r = g.angle.to_radians();
    let (dx, dy) = (r.cos(), r.sin());
    let (ex, ey) = (a * dx + c * dy, b * dx + d * dy);
    if ex.hypot(ey) > 1e-12 {
        // Rounded to 1e-9 degree so 90° stays 90° after cos/sin.
        let angle = (ey.atan2(ex).to_degrees().rem_euclid(360.0) * 1e9).round() / 1e9;
        g.angle = if angle >= 360.0 { 0.0 } else { angle };
    }
    g.x = q.x;
    g.y = q.y;
    Ok(())
}

/// Moves `sel` in one side of `layer` by `m`. Points and anchors are rounded to the grid.
pub(crate) fn apply(p: &Params, layer: &mut Layer, side: LayerSide, sel: &Selection, m: Affine, step: f64) -> Result<()> {
    let mut parts = layer.parts_mut(side);
    for &i in &sel.shapes {
        match parts.shapes.get_mut(i) {
            Some(Shape::Path(path)) => {
                for n in &mut path.nodes {
                    let q = map_point(p, m, n.point(), step)?;
                    (n.x, n.y) = (q.x, q.y);
                }
            }
            Some(Shape::Component(c)) => c.transform = compose(p, &c.transform, m)?,
            Some(Shape::Image(img)) => img.transform = compose(p, &img.transform, m)?,
            Some(Shape::Raw(_)) => return Err(p.bad(format!("shape {i} is of a kind that cannot be transformed yet"))),
            None => return Err(p.bad(format!("shape {i} does not resolve"))),
        }
    }
    for &(s, n) in sel.points.iter().filter(|(s, _)| !sel.shapes.contains(s)) {
        let node = parts.node_mut(s, n).ok_or_else(|| p.bad(format!("point [{s}, {n}] does not resolve")))?;
        let q = map_point(p, m, node.point(), step)?;
        (node.x, node.y) = (q.x, q.y);
    }
    for &i in &sel.anchors {
        let a = parts.anchors.get_mut(i).ok_or_else(|| p.bad(format!("anchor {i} does not resolve")))?;
        let q = map_point(p, m, Point::new(a.x, a.y), step)?;
        (a.x, a.y) = (q.x, q.y);
    }
    for &i in &sel.local_guides {
        let g = parts.guides.get_mut(i).ok_or_else(|| p.bad(format!("local guide {i} does not resolve")))?;
        map_guide(p, g, m)?;
    }
    for &i in &sel.annotations {
        let a = parts.annotations.get_mut(i).ok_or_else(|| p.bad(format!("annotation {i} does not resolve")))?;
        let q = map_point(p, m, Point::new(a.x, a.y), 0.0)?;
        (a.x, a.y) = (q.x, q.y);
    }
    Ok(())
}

// ── Running a transformation ───────────────────────────────────────────────────────────────────

/// The layers a transformation changes: one target object (`glyph`, with or without target
/// lists), or the same layer of several glyphs (`glyphs`; with `many_layers`, `layers` names
/// several). With `all_glyphs`, neither given means every glyph.
fn jobs(p: &Params, font: &Font, many_layers: bool, all_glyphs: bool) -> Result<Vec<Targets>> {
    if p.get("glyph").is_some() {
        if p.get("glyphs").is_some() {
            return Err(p.bad("give `glyph` (with target lists) or `glyphs`, not both"));
        }
        if p.get("layers").is_some() {
            return Err(p.bad("`layers` goes with `glyphs`; with `glyph`, give `layer`"));
        }
        return Ok(vec![Targets::parse(p)?]);
    }
    for key in ["points", "segments", "segment", "shapes", "anchors", "guides", "annotations"] {
        if p.get(key).is_some() {
            return Err(p.bad(format!("`{key}` names things in one glyph: give `glyph`, not `glyphs`")));
        }
    }
    let glyphs = match p.opt_glyphs("glyphs", font)? {
        Some(g) => g,
        None if all_glyphs => font.glyphs.as_arcs().iter().map(|g| g.name.clone()).collect(),
        None => return Err(p.bad("`glyph` or `glyphs` is required")),
    };
    let side = match p.opt_str("side")? {
        None => LayerSide::Foreground,
        Some(s) => LayerSide::parse(s).ok_or_else(|| p.bad(format!("`side` must be \"foreground\" or \"background\", got \"{s}\"")))?,
    };
    let mut layers: Vec<Option<String>> = vec![p.opt_str("layer")?.map(str::to_string)];
    if many_layers && let Some(a) = p.opt_array("layers")? {
        if p.get("layer").is_some() {
            return Err(p.bad("give `layer` or `layers`, not both"));
        }
        if a.len() > MAX_LAYERS {
            return Err(p.bad(format!("`layers` holds {} ids; at most {MAX_LAYERS} are allowed", a.len())));
        }
        let mut seen = HashSet::new();
        layers = p.strings("layers")?.into_iter().filter(|id| seen.insert(*id)).map(|id| Some(id.to_string())).collect();
    }
    if glyphs.len().saturating_mul(layers.len()) > MAX_LAYER_JOBS {
        return Err(p.bad(format!("this would change more than {MAX_LAYER_JOBS} layers")));
    }
    let mut out = Vec::with_capacity(glyphs.len().saturating_mul(layers.len()));
    for name in &glyphs {
        for layer in &layers {
            let mut t = Targets::new(p.command(), name.clone(), layer.clone());
            t.side = side;
            out.push(t);
        }
    }
    Ok(out)
}

/// Runs `lin` about `origin`, then `shift`, on every job, as one undo step. Returns the origin
/// used in each layer.
///
/// `image` is the box of the layer's placed image when the image is transformed too (one job):
/// see [`image_bounds`].
#[allow(clippy::too_many_arguments)]
fn run_jobs(
    s: &mut Session,
    p: &Params,
    label: &str,
    jobs: &[Targets],
    origin: Origin,
    lin: Affine,
    shift: Vec2,
    image: Option<Rect>,
) -> Result<Vec<Point>> {
    let step = grid(s.font()?);
    s.edit(label, |font| {
        let mut origins = Vec::with_capacity(jobs.len());
        for t in jobs {
            let (sel, o, master) = {
                let r = t.resolve(font)?;
                crate::check_unlocked(r.glyph)?;
                // The image alone, when it is given without target lists.
                let sel = if image.is_some() && !t.has_lists() { Selection::default() } else { selection(t, r.parts) };
                let bounds = if matches!(origin, Origin::Box(_)) { union(selection_box(font, r.master, r.parts, &sel), image) } else { None };
                (sel, origin_point(origin, bounds, font, r.master, r.layer.width), r.master.id.clone())
            };
            let m = Affine::translate(shift) * about(o, lin);
            t.edit_layer(font, |l| {
                apply(p, l, t.side, &sel, m, step)?;
                if image.is_some() {
                    let placed = super::image::target_mut(p, t, l)?;
                    placed.transform = compose(p, &placed.transform, m)?;
                }
                Ok(())
            })?;
            if !sel.global_guides.is_empty() {
                let masters = Arc::make_mut(&mut font.masters);
                let master = masters.iter_mut().find(|x| x.id == master).ok_or_else(|| p.bad(format!("there is no master `{master}`")))?;
                for &i in &sel.global_guides {
                    let g = master.guides.get_mut(i).ok_or_else(|| p.bad(format!("global guide {i} does not resolve")))?;
                    map_guide(p, g, m)?;
                }
            }
            origins.push(o);
        }
        Ok(origins)
    })
}

fn result(jobs: &[Targets], origins: &[Point]) -> Value {
    let mut names: Vec<&str> = Vec::new();
    for t in jobs {
        if names.last() != Some(&t.glyph.as_str()) {
            names.push(&t.glyph);
        }
    }
    let mut out = json!({"glyphs": names, "layers": jobs.len()});
    if let ([o], Some(map)) = (origins, out.as_object_mut()) {
        map.insert("origin".into(), json!([o.x, o.y]));
    }
    out
}

/// With `image: true`, the box of the layer's placed image: the image of one glyph's layer
/// only, refused when it is missing or locked. `None` without the flag.
fn image_bounds(s: &Session, p: &Params, jobs: &[Targets]) -> Result<Option<Rect>> {
    if !super::image::targeted(p)? {
        return Ok(None);
    }
    let ([t], true) = (jobs, p.get("glyph").is_some()) else {
        return Err(p.bad("`image` names the placed image of one layer: give `glyph`, not `glyphs`"));
    };
    super::image::shown_box(s, p, t).map(Some)
}

/// One Palette transformation.
fn palette(s: &mut Session, v: &Value, id: &'static str, label: &str, lin: impl FnOnce(&Params) -> Result<Affine>) -> Result<Value> {
    let p = Params::new(id, v)?;
    let lin = lin(&p)?;
    let origin = parse_origin(&p, Origin::Box(BoxPoint::Center))?;
    let jobs = jobs(&p, s.font()?, false, false)?;
    let image = image_bounds(s, &p, &jobs)?;
    let origins = run_jobs(s, &p, label, &jobs, origin, lin, Vec2::ZERO, image)?;
    Ok(result(&jobs, &origins))
}

/// `x` and `y` in percent (`y` defaults to `x`).
fn percents(p: &Params) -> Result<(f64, f64)> {
    let x = p.f64("x")?;
    let y = p.opt_f64("y")?.unwrap_or(x);
    if x.abs() > MAX_PERCENT || y.abs() > MAX_PERCENT {
        return Err(p.bad(format!("scale percentages must be within ±{MAX_PERCENT}")));
    }
    Ok((x, y))
}

/// The shear factor for a slant angle (strictly between -90° and 90°).
fn slant_factor(p: &Params, key: &str, degrees: f64) -> Result<f64> {
    if degrees.abs() >= 90.0 {
        return Err(p.bad(format!("`{key}` must be between -90 and 90 degrees, got {degrees}")));
    }
    Ok(degrees.to_radians().tan())
}

fn mirror_horizontal(s: &mut Session, v: &Value) -> Result<Value> {
    const ID: &str = "transform.mirrorHorizontal";
    const LABEL: &str = "Mirror Horizontally";
    if let Some(out) = mirror_corners(s, v, "horizontal", ID, LABEL)? {
        return Ok(out);
    }
    palette(s, v, ID, LABEL, |_| Ok(Affine::new([-1.0, 0.0, 0.0, 1.0, 0.0, 0.0])))
}

fn mirror_vertical(s: &mut Session, v: &Value) -> Result<Value> {
    const ID: &str = "transform.mirrorVertical";
    const LABEL: &str = "Mirror Vertically";
    if let Some(out) = mirror_corners(s, v, "vertical", ID, LABEL)? {
        return Ok(out);
    }
    palette(s, v, ID, LABEL, |_| Ok(Affine::new([1.0, 0.0, 0.0, -1.0, 0.0, 0.0])))
}

/// The keys mirroring with `corners` does not take: the corners flip in place, on the foreground
/// of one glyph's layer.
const NOT_WITH_CORNERS: [&str; 10] = ["glyphs", "side", "points", "segments", "shapes", "anchors", "guides", "annotations", "image", "origin"];

/// Mirroring with `corners` flips those corner components in place, as their scale does
/// (cornerComponent.setScale with `flip`), as one undo step named after the mirror; `None`
/// without `corners`.
fn mirror_corners(s: &mut Session, v: &Value, flip: &str, id: &'static str, label: &str) -> Result<Option<Value>> {
    let Some(corners) = v.get("corners") else { return Ok(None) };
    if let Some(key) = NOT_WITH_CORNERS.iter().find(|k| v.get(**k).is_some()) {
        return Err(EngineError::BadParams {
            command: id.into(),
            message: format!("`corners` flips corner components in place: `{key}` is not taken with it"),
        });
    }
    let mut p = json!({"corners": corners, "flip": flip});
    for key in ["glyph", "layer"] {
        if let (Some(value), Some(o)) = (v.get(key), p.as_object_mut()) {
            o.insert(key.into(), value.clone());
        }
    }
    super::corner::scale_corners(s, &p, id, label).map(Some)
}

fn scale(s: &mut Session, v: &Value) -> Result<Value> {
    palette(s, v, "transform.scale", "Scale", |p| {
        let (x, y) = percents(p)?;
        Ok(Affine::scale_non_uniform(x / 100.0, y / 100.0))
    })
}

fn reverse_scale(s: &mut Session, v: &Value) -> Result<Value> {
    palette(s, v, "transform.reverseScale", "Reverse Scale", |p| {
        let (x, y) = percents(p)?;
        if x == 0.0 || y == 0.0 {
            return Err(p.bad("a scale of 0 percent cannot be reversed"));
        }
        Ok(Affine::scale_non_uniform(100.0 / x, 100.0 / y))
    })
}

fn rotate_clockwise(s: &mut Session, v: &Value) -> Result<Value> {
    palette(s, v, "transform.rotateClockwise", "Rotate Clockwise", |p| Ok(rotation(-p.f64("angle")?)))
}

fn rotate_counterclockwise(s: &mut Session, v: &Value) -> Result<Value> {
    palette(s, v, "transform.rotateCounterclockwise", "Rotate Counterclockwise", |p| Ok(rotation(p.f64("angle")?)))
}

fn slant_left(s: &mut Session, v: &Value) -> Result<Value> {
    palette(s, v, "transform.slantLeft", "Slant Left", |p| Ok(Affine::new([1.0, 0.0, -slant_factor(p, "angle", p.f64("angle")?)?, 1.0, 0.0, 0.0])))
}

fn slant_right(s: &mut Session, v: &Value) -> Result<Value> {
    palette(s, v, "transform.slantRight", "Slant Right", |p| Ok(Affine::new([1.0, 0.0, slant_factor(p, "angle", p.f64("angle")?)?, 1.0, 0.0, 0.0])))
}

fn slant_down(s: &mut Session, v: &Value) -> Result<Value> {
    palette(s, v, "transform.slantDown", "Slant Down", |p| Ok(Affine::new([1.0, -slant_factor(p, "angle", p.f64("angle")?)?, 0.0, 1.0, 0.0, 0.0])))
}

fn slant_up(s: &mut Session, v: &Value) -> Result<Value> {
    palette(s, v, "transform.slantUp", "Slant Up", |p| Ok(Affine::new([1.0, slant_factor(p, "angle", p.f64("angle")?)?, 0.0, 1.0, 0.0, 0.0])))
}

// ── Aligning ───────────────────────────────────────────────────────────────────────────────────

#[derive(Clone, Copy)]
enum Edge {
    Left,
    CenterHorizontal,
    Right,
    Top,
    CenterVertical,
    Bottom,
}

/// One object the align buttons move as a whole, with its box.
struct Unit {
    bounds: Rect,
    sel: Selection,
}

/// The objects of a selection: each shape, each on-curve node with its handles, each lone handle,
/// each anchor. Without target lists, every shape of the layer.
fn align_units(font: &Font, master: &Master, parts: LayerParts, t: &Targets) -> Vec<Unit> {
    let sel = selection(t, parts);
    let mut units = Vec::new();
    for &i in &sel.shapes {
        if let Some(bounds) = parts.shapes.get(i).and_then(|s| shape_box(font, &master.id, s)) {
            units.push(Unit { bounds, sel: Selection { shapes: BTreeSet::from([i]), ..Selection::default() } });
        }
    }
    if sel.whole_layer {
        return units;
    }
    let mut covered = BTreeSet::new();
    let on_curve = |&(s, n): &(usize, usize)| parts.node(s, n).is_some_and(Node::on_curve);
    for &(s, n) in sel.named_points.iter().filter(|a| on_curve(a)) {
        let Some(node) = parts.node(s, n) else { continue };
        let points: BTreeSet<_> = std::iter::once((s, n)).chain(handles_of(parts, s, n)).collect();
        covered.extend(points.iter().copied());
        units.push(Unit { bounds: dot(node.point()), sel: Selection { points, ..Selection::default() } });
    }
    for &(s, n) in sel.named_points.iter().filter(|a| !on_curve(a) && !covered.contains(*a)) {
        let Some(node) = parts.node(s, n) else { continue };
        units.push(Unit { bounds: dot(node.point()), sel: Selection { points: BTreeSet::from([(s, n)]), ..Selection::default() } });
    }
    for &i in &sel.anchors {
        let Some(a) = parts.anchors.get(i) else { continue };
        units.push(Unit { bounds: dot(Point::new(a.x, a.y)), sel: Selection { anchors: BTreeSet::from([i]), ..Selection::default() } });
    }
    units
}

fn align(s: &mut Session, v: &Value, id: &'static str, label: &str, edge: Edge) -> Result<Value> {
    let p = Params::new(id, v)?;
    let t = Targets::parse(&p)?;
    if !t.guides.is_empty() || !t.annotations.is_empty() {
        return Err(p.bad("aligns points, segments, shapes and anchors; guides and annotations are not aligned"));
    }
    let step = grid(s.font()?);
    let units = {
        let font = s.font()?;
        let r = t.resolve(font)?;
        align_units(font, r.master, r.parts, &t)
    };
    let Some(all) = units.iter().map(|u| u.bounds).reduce(|a, b| a.union(b)) else {
        return Ok(json!({"glyph": t.glyph, "moved": 0, "message": "Nothing is selected to align."}));
    };
    if units.len() < 2 {
        return Ok(json!({"glyph": t.glyph, "moved": 0, "message": "Select at least two objects to align them with each other."}));
    }
    let centre = |lo: f64, hi: f64| (lo + hi) / 2.0;
    let moved = s.edit(label, |font| {
        t.edit_layer(font, |l| {
            let mut moved = 0usize;
            for u in &units {
                let b = u.bounds;
                let d = match edge {
                    Edge::Left => Vec2::new(all.x0 - b.x0, 0.0),
                    Edge::CenterHorizontal => Vec2::new(centre(all.x0, all.x1) - centre(b.x0, b.x1), 0.0),
                    Edge::Right => Vec2::new(all.x1 - b.x1, 0.0),
                    Edge::Top => Vec2::new(0.0, all.y1 - b.y1),
                    Edge::CenterVertical => Vec2::new(0.0, centre(all.y0, all.y1) - centre(b.y0, b.y1)),
                    Edge::Bottom => Vec2::new(0.0, all.y0 - b.y0),
                };
                if d != Vec2::ZERO {
                    apply(&p, l, t.side, &u.sel, Affine::translate(d), step)?;
                    moved += 1;
                }
            }
            Ok(moved)
        })
    })?;
    Ok(json!({"glyph": t.glyph, "moved": moved}))
}

fn align_left(s: &mut Session, v: &Value) -> Result<Value> {
    align(s, v, "transform.alignLeft", "Align Left", Edge::Left)
}

fn align_center_horizontal(s: &mut Session, v: &Value) -> Result<Value> {
    align(s, v, "transform.alignCenterHorizontal", "Align Horizontal Centers", Edge::CenterHorizontal)
}

fn align_right(s: &mut Session, v: &Value) -> Result<Value> {
    align(s, v, "transform.alignRight", "Align Right", Edge::Right)
}

fn align_top(s: &mut Session, v: &Value) -> Result<Value> {
    align(s, v, "transform.alignTop", "Align Top", Edge::Top)
}

fn align_center_vertical(s: &mut Session, v: &Value) -> Result<Value> {
    align(s, v, "transform.alignCenterVertical", "Align Vertical Centers", Edge::CenterVertical)
}

fn align_bottom(s: &mut Session, v: &Value) -> Result<Value> {
    align(s, v, "transform.alignBottom", "Align Bottom", Edge::Bottom)
}

/// Path › Align Selection: the selected points onto one coordinate.
fn align_selection(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("path.alignSelection", v)?;
    let t = Targets::parse(&p)?;
    if !t.guides.is_empty() || !t.annotations.is_empty() {
        return Err(p.bad("aligns nodes, handles and anchors; guides and annotations are not aligned"));
    }
    #[derive(Clone, Copy, PartialEq)]
    enum Axis {
        Auto,
        X,
        Y,
    }
    let axis = match p.opt_str("axis")? {
        None | Some("auto") => Axis::Auto,
        Some("vertical") => Axis::X,
        Some("horizontal") => Axis::Y,
        Some(other) => return Err(p.bad(format!("`axis` must be auto, vertical or horizontal, got \"{other}\""))),
    };
    let origin = parse_origin(&p, Origin::Box(BoxPoint::Center))?;
    if let Some(out) = super::anchor::align_special(s, &p, &t)? {
        return Ok(out);
    }
    let step = grid(s.font()?);
    let font = s.font()?;
    let r = t.resolve(font)?;
    // Whole paths take part with all their points; a component only in the case above.
    let mut points: BTreeSet<(usize, usize)> = BTreeSet::new();
    for &i in &t.shapes {
        match r.parts.shapes.get(i) {
            Some(Shape::Path(path)) => points.extend((0..path.nodes.len()).map(|n| (i, n))),
            _ => {
                return Err(p.bad(format!(
                    "shape {i} is not a path: a component is aligned only when it is selected with exactly one node; the Palette's align buttons line components up with each other"
                )));
            }
        }
    }
    let sel = selection(&t, r.parts);
    points.extend(sel.named_points.iter().copied());
    let anchors = sel.anchors.clone();
    let none = || json!({"glyph": t.glyph, "moved": 0, "message": "Select at least two points to align them."});
    if !t.has_lists() {
        return Ok(none());
    }

    // One node alone goes onto the nearest node of the background.
    let lone = if anchors.is_empty() && points.len() == 1 { points.first().copied() } else { None };
    if let Some((s0, n0)) = lone
        && let Some(node) = r.parts.node(s0, n0).filter(|n| n.on_curve())
    {
        if t.side == LayerSide::Background {
            return Ok(json!({"glyph": t.glyph, "moved": 0, "message": "A background node has no background to align with."}));
        }
        let from = node.point();
        let bg = r.layer.parts(LayerSide::Background);
        let nearest = bg
            .shapes
            .iter()
            .filter_map(|shape| match shape {
                Shape::Path(path) => Some(path),
                _ => None,
            })
            .flat_map(|path| path.nodes.iter().filter(|n| n.on_curve()).map(Node::point))
            .fold(None::<Point>, |best, q| match best {
                Some(b) if b.distance(from) <= q.distance(from) => Some(b),
                _ => Some(q),
            });
        let Some(to) = nearest else {
            return Ok(json!({"glyph": t.glyph, "moved": 0, "message": "The background has no node to align the node with."}));
        };
        let sel = Selection { points: std::iter::once((s0, n0)).chain(handles_of(r.parts, s0, n0)).collect(), ..Selection::default() };
        s.edit("Align Selection", |font| t.edit_layer(font, |l| apply(&p, l, t.side, &sel, Affine::translate(to - from), step)))?;
        return Ok(json!({"glyph": t.glyph, "moved": 1, "to": [to.x, to.y]}));
    }

    let positions: Vec<Point> = points
        .iter()
        .filter_map(|(s, n)| r.parts.node(*s, *n).map(Node::point))
        .chain(anchors.iter().filter_map(|i| r.parts.anchors.get(*i)).map(|a| Point::new(a.x, a.y)))
        .collect();
    if positions.len() < 2 {
        return Ok(none());
    }
    let Some(bounds) = fontcraft_geom::bounds(positions.iter().copied()) else { return Ok(none()) };
    let on_x = match axis {
        Axis::X => true,
        Axis::Y => false,
        Axis::Auto => bounds.width() <= bounds.height(),
    };
    let o = origin_point(origin, Some(bounds), font, r.master, r.layer.width);
    let target = if on_x { o.x } else { o.y };
    // The handles of a smooth node that is not itself among the points move with it.
    let mut followers: Vec<(NodeRef, Vec<NodeRef>)> = Vec::new();
    for &(s0, n0) in &points {
        if r.parts.node(s0, n0).is_some_and(|n| n.on_curve() && n.smooth) {
            let handles: Vec<_> = handles_of(r.parts, s0, n0).into_iter().filter(|h| !points.contains(h)).collect();
            if !handles.is_empty() {
                followers.push(((s0, n0), handles));
            }
        }
    }
    let shift = |at: Point| if on_x { Vec2::new(target - at.x, 0.0) } else { Vec2::new(0.0, target - at.y) };
    let moved = s.edit("Align Selection", |font| {
        t.edit_layer(font, |l| {
            let mut moved = 0usize;
            for &(s0, n0) in &points {
                let Some(at) = l.parts(t.side).node(s0, n0).map(Node::point) else { continue };
                let d = shift(at);
                if d == Vec2::ZERO {
                    continue;
                }
                let mut sel = Selection { points: BTreeSet::from([(s0, n0)]), ..Selection::default() };
                if let Some((_, handles)) = followers.iter().find(|(k, _)| *k == (s0, n0)) {
                    sel.points.extend(handles.iter().copied());
                }
                apply(&p, l, t.side, &sel, Affine::translate(d), step)?;
                moved += 1;
            }
            for &i in &anchors {
                let Some(at) = l.parts(t.side).anchors.get(i).map(|a| Point::new(a.x, a.y)) else { continue };
                let d = shift(at);
                if d != Vec2::ZERO {
                    apply(&p, l, t.side, &Selection { anchors: BTreeSet::from([i]), ..Selection::default() }, Affine::translate(d), step)?;
                    moved += 1;
                }
            }
            Ok(moved)
        })
    })?;
    Ok(json!({"glyph": t.glyph, "moved": moved, "axis": if on_x { "x" } else { "y" }, "coordinate": target}))
}

// ── Path › Transformations and its export string ───────────────────────────────────────────────

/// The settings of Path › Transformations, which the `Transformations` export filter also
/// carries.
#[derive(Clone, Debug, PartialEq)]
pub struct Transformations {
    /// Font units.
    pub translate: (f64, f64),
    /// Percent per axis.
    pub scale: (f64, f64),
    /// Degrees; a plain shear by the tangent.
    pub slant: f64,
    /// The slant with optical correction (`SlantCorrection` in the string).
    pub cursivy: bool,
    pub origin: Origin,
    /// Glyph scoping of the export string: only these glyphs, or every glyph but these.
    pub include: Vec<String>,
    pub exclude: Vec<String>,
}

impl Default for Transformations {
    fn default() -> Self {
        Self {
            translate: (0.0, 0.0),
            scale: (100.0, 100.0),
            slant: 0.0,
            cursivy: false,
            origin: Origin::Metric(MetricPoint::Baseline),
            include: Vec::new(),
            exclude: Vec::new(),
        }
    }
}

impl Transformations {
    /// The linear part, about the origin: scale, then slant.
    fn linear(&self) -> Affine {
        let (sx, sy) = (self.scale.0 / 100.0, self.scale.1 / 100.0);
        let shear = self.slant.to_radians().tan();
        Affine::new([1.0, 0.0, shear, 1.0, 0.0, 0.0]) * Affine::scale_non_uniform(sx, sy)
    }

    /// Whether the export string's glyph scoping lets `glyph` through.
    pub fn includes(&self, glyph: &str) -> bool {
        (self.include.is_empty() || self.include.iter().any(|g| g == glyph)) && !self.exclude.iter().any(|g| g == glyph)
    }

    fn check(&self) -> std::result::Result<(), String> {
        let (sx, sy) = self.scale;
        if !(sx.is_finite() && sy.is_finite() && sx.abs() <= MAX_PERCENT && sy.abs() <= MAX_PERCENT) {
            return Err(format!("scale percentages must be within ±{MAX_PERCENT}"));
        }
        if !(self.slant.is_finite() && self.slant.abs() < 90.0) {
            return Err(format!("the slant must be between -90 and 90 degrees, got {}", self.slant));
        }
        if !(in_range(self.translate.0) && in_range(self.translate.1)) {
            return Err(format!("the offset must be within ±{MAX_COORDINATE}"));
        }
        Ok(())
    }
}

/// Reads the settings of `path.transformations` and its copy commands from parameters.
fn transformations_from_params(p: &Params) -> Result<Transformations> {
    let num = |o: &Map<String, Value>, key: &str, name: &str| -> Result<Option<f64>> {
        match o.get(key).filter(|v| !v.is_null()) {
            None => Ok(None),
            Some(v) => v
                .as_f64()
                .filter(|n| n.is_finite())
                .map(Some)
                .ok_or_else(|| p.bad(format!("`{name}.{key}` must be a finite number, got {}", kind(v)))),
        }
    };
    let mut out = Transformations::default();
    if let Some(o) = p.opt_object("translate")? {
        out.translate = (num(o, "x", "translate")?.unwrap_or(0.0), num(o, "y", "translate")?.unwrap_or(0.0));
    }
    if let Some(o) = p.opt_object("scale")? {
        let x = num(o, "x", "scale")?.ok_or_else(|| p.bad("`scale.x` is required"))?;
        out.scale = (x, num(o, "y", "scale")?.unwrap_or(x));
    }
    out.slant = p.opt_f64("slant")?.unwrap_or(0.0);
    out.cursivy = p.bool_or("cursivy", false)?;
    out.origin = parse_origin(p, Origin::Metric(MetricPoint::Baseline))?;
    out.check().map_err(|e| p.bad(e))?;
    Ok(out)
}

/// Reads a `Transformations` export-filter string: `Transformations;Key:value;…`, spaces
/// allowed around items, keys and values. `LSB`, `RSB` and `Width` belong to Transform Metrics
/// and are refused.
pub fn parse_transformations(text: &str) -> std::result::Result<Transformations, String> {
    if text.len() > MAX_FILTER_STRING {
        return Err(format!("the string is longer than {MAX_FILTER_STRING} bytes"));
    }
    let mut items = text.split(';').map(str::trim).filter(|s| !s.is_empty());
    match items.next() {
        Some("Transformations") => {}
        Some(other) => return Err(format!("the string names the filter \"{other}\", not Transformations")),
        None => return Err("the string is empty".into()),
    }
    let number = |key: &str, value: &str| -> std::result::Result<f64, String> {
        value.parse::<f64>().ok().filter(|v| v.is_finite()).ok_or_else(|| format!("`{key}` must be a number, got \"{value}\""))
    };
    let names = |key: &str, value: &str| -> std::result::Result<Vec<String>, String> {
        let list: Vec<String> = value.split([' ', ',']).filter(|n| !n.is_empty()).map(str::to_string).collect();
        if list.len() > MAX_GLYPHS {
            return Err(format!("`{key}` names more than {MAX_GLYPHS} glyphs"));
        }
        Ok(list)
    };
    let mut out = Transformations::default();
    let mut seen = HashSet::new();
    for item in items {
        let (key, value) = item.split_once(':').ok_or_else(|| format!("\"{item}\" is not a Key:value item"))?;
        let (key, value) = (key.trim(), value.trim());
        if !seen.insert(key) {
            return Err(format!("`{key}` is given twice"));
        }
        match key {
            "ScaleX" => out.scale.0 = number(key, value)?,
            "ScaleY" => out.scale.1 = number(key, value)?,
            "Slant" => out.slant = number(key, value)?,
            "SlantCorrection" => {
                out.cursivy = match value {
                    "1" | "true" => true,
                    "0" | "false" => false,
                    _ => return Err(format!("`SlantCorrection` must be 1 or 0, got \"{value}\"")),
                }
            }
            "OffsetX" => out.translate.0 = number(key, value)?,
            "OffsetY" => out.translate.1 = number(key, value)?,
            "Origin" => {
                out.origin = value.parse::<u8>().ok().and_then(MetricPoint::from_code).map(Origin::Metric).ok_or_else(|| {
                    format!("`Origin` must be 0 (cap height), 1 (half cap height), 2 (x-height), 3 (half x-height) or 4 (baseline), got \"{value}\"")
                })?
            }
            "LSB" | "RSB" | "Width" => return Err(format!("`{key}` changes metrics, which Transform Metrics does; it is not supported yet")),
            "include" => out.include = names(key, value)?,
            "exclude" => out.exclude = names(key, value)?,
            other => return Err(format!("unknown key `{other}`")),
        }
    }
    if !out.include.is_empty() && !out.exclude.is_empty() {
        return Err("give `include` or `exclude`, not both".into());
    }
    out.check()?;
    Ok(out)
}

/// Writes settings as a `Transformations` export-filter string: no spaces, a semicolon after
/// every item, settings at their neutral value left out, the origin always written. Only a
/// metric origin can be written.
pub fn write_transformations(t: &Transformations) -> std::result::Result<String, String> {
    let Origin::Metric(origin) = t.origin else {
        return Err("the export string holds only a metric origin: capHeight, halfCapHeight, xHeight, halfXHeight or baseline".into());
    };
    let mut out = String::from("Transformations;");
    let mut item = |key: &str, value: &dyn std::fmt::Display| {
        let _ = write!(out, "{key}:{value};");
    };
    if t.scale.0 != 100.0 {
        item("ScaleX", &t.scale.0);
    }
    if t.scale.1 != 100.0 {
        item("ScaleY", &t.scale.1);
    }
    if t.slant != 0.0 || t.cursivy {
        item("Slant", &t.slant);
        item("SlantCorrection", &u8::from(t.cursivy));
    }
    if t.translate.0 != 0.0 {
        item("OffsetX", &t.translate.0);
    }
    if t.translate.1 != 0.0 {
        item("OffsetY", &t.translate.1);
    }
    item("Origin", &origin.code());
    if !t.include.is_empty() {
        item("include", &t.include.join(","));
    }
    if !t.exclude.is_empty() {
        item("exclude", &t.exclude.join(","));
    }
    Ok(out)
}

/// Whether `master` has a horizontal and a vertical stem value, which Cursivy needs.
fn has_stems(font: &Font, master: &Master) -> bool {
    let has = |horizontal: bool| {
        font.stems.iter().enumerate().any(|(i, s)| s.horizontal == horizontal && master.stem_values.get(i).is_some_and(|v| *v > 0.0))
    };
    has(true) && has(false)
}

fn transformations(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("path.transformations", v)?;
    let (args, mut jobs) = match p.opt_str("string")? {
        Some(text) => {
            if let Some(key) = ["translate", "scale", "slant", "cursivy", "origin"].into_iter().find(|k| p.get(k).is_some()) {
                return Err(p.bad(format!("`string` holds every setting: give it or `{key}`, not both")));
            }
            let args = parse_transformations(text).map_err(|e| p.bad(format!("`string`: {e}")))?;
            (args, jobs(&p, s.font()?, true, true)?)
        }
        None => (transformations_from_params(&p)?, jobs(&p, s.font()?, true, false)?),
    };
    jobs.retain(|t| args.includes(&t.glyph));
    if args.cursivy {
        let font = s.font()?;
        for t in &jobs {
            let r = t.resolve(font)?;
            if !has_stems(font, r.master) {
                return Err(
                    p.bad(format!("Cursivy needs a horizontal and a vertical stem in master `{}` (Font Info › Masters › Stems)", r.master.name))
                );
            }
        }
        return Err(EngineError::Unsupported("Cursivy (a slant with optical correction of the curves)".into()));
    }
    let origins = run_jobs(s, &p, "Transformations", &jobs, args.origin, args.linear(), Vec2::new(args.translate.0, args.translate.1), None)?;
    Ok(result(&jobs, &origins))
}

fn copy_parameter(v: &Value, id: &'static str, parameter: &str) -> Result<Value> {
    let p = Params::new(id, v)?;
    let args = transformations_from_params(&p)?;
    let text = write_transformations(&args).map_err(|e| p.bad(e))?;
    Ok(json!({"text": text, "parameter": parameter}))
}

fn copy_filter_parameter(_: &mut Session, v: &Value) -> Result<Value> {
    copy_parameter(v, "path.transformations.copyFilterParameter", "Filter")
}

fn copy_prefilter_parameter(_: &mut Session, v: &Value) -> Result<Value> {
    copy_parameter(v, "path.transformations.copyPreFilterParameter", "PreFilter")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_export_string_reads_and_writes() {
        let t = parse_transformations("Transformations;Slant:10;SlantCorrection:0;Origin:4;").unwrap();
        assert_eq!((t.slant, t.cursivy, t.origin), (10.0, false, Origin::Metric(MetricPoint::Baseline)));
        assert_eq!(write_transformations(&t).unwrap(), "Transformations;Slant:10;SlantCorrection:0;Origin:4;");

        let t = parse_transformations(" Transformations; ScaleX: 80; ScaleY: 90.5; OffsetX: +20; OffsetY: -5; Origin: 2; include: a b,c ").unwrap();
        assert_eq!((t.scale, t.translate, t.origin), ((80.0, 90.5), (20.0, -5.0), Origin::Metric(MetricPoint::XHeight)));
        assert_eq!(t.include, ["a", "b", "c"]);
        assert!(t.includes("b") && !t.includes("d"));
        assert_eq!(parse_transformations(&write_transformations(&t).unwrap()).unwrap(), t);
        // Neutral settings are left out; the origin is always written.
        assert_eq!(write_transformations(&Transformations::default()).unwrap(), "Transformations;Origin:4;");

        for bad in [
            "",
            "RemoveOverlap",
            "Transformations;Slant",
            "Transformations;Slant:90",
            "Transformations;ScaleX:abc",
            "Transformations;Origin:5",
            "Transformations;LSB:10",
            "Transformations;Mystery:1",
            "Transformations;Slant:1;Slant:2",
            "Transformations;include:a;exclude:b",
            "Transformations;OffsetX:1e300",
        ] {
            assert!(parse_transformations(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn quarter_turns_are_exact() {
        let p = Point::new(37.0, -12.0);
        let mut q = p;
        for _ in 0..4 {
            q = rotation(90.0) * q;
        }
        assert_eq!(q, p);
        assert_eq!(rotation(-90.0) * (rotation(90.0) * p), p);
        assert_eq!(rotation(450.0) * p, rotation(90.0) * p);
    }

    #[test]
    fn box_points_pick_edges_and_centres() {
        let r = Rect::new(0.0, 0.0, 101.0, 50.0);
        assert_eq!(BoxPoint::TopLeft.point_in(r), Point::new(0.0, 50.0));
        assert_eq!(BoxPoint::Center.point_in(r), Point::new(50.5, 25.0));
        assert_eq!(BoxPoint::BottomRight.point_in(r), Point::new(101.0, 0.0));
        assert_eq!(MetricPoint::from_code(4), Some(MetricPoint::Baseline));
        assert_eq!(MetricPoint::from_code(5), None);
    }
}
