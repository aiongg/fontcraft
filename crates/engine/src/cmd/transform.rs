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
use fontcraft_model::glyphdata::GlyphData;
use fontcraft_model::guide_follow::{can_place, edit_placed, placed, placed_x};
use fontcraft_model::{
    Font, Guide, Layer, LayerParts, LayerSide, MAX_COORDINATE, MAX_GLYPHS, Master, Metrics, Node, NodeKind, Shape, Transform, metric,
};
use fontcraft_pathops as po;
use fontcraft_spacing::{SIDEBEARING_RANGE, WIDTH_RANGE};
use serde_json::{Map, Value, json};

use super::common::{grid, in_range, master_metrics};
use super::node::{Plan, undo_label, with_matched};
use super::transform_metrics::{self as tm, Shift};
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
const ALIGN_TARGETS: &[TargetKind] = &[
    TargetKind::Layer,
    TargetKind::Points,
    TargetKind::Segments,
    TargetKind::Shapes,
    TargetKind::Anchors,
    TargetKind::Guides,
    TargetKind::Annotations,
];

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "transform.mirrorHorizontal",
            "Mirror Horizontally",
            [],
            None,
            "{glyph?: name, glyphs?: [names], layer?: id, side?: \"foreground\" | \"background\", points?: [[shape, node]], segments?: [[shape, node]], shapes?: [index], anchors?: [name], guides?: [{scope: \"local\" | \"global\", index}], annotations?: [index], image?: bool, origin?: origin} flips left to right across the vertical line through the origin. Target lists go with `glyph`; without them, or with `glyphs`, the whole layer (shapes and anchors) is changed. `image: true` (with `glyph`) changes the layer's placed image on that side too, and alone without target lists; a locked image, or none, refuses it. An on-curve node carries its handles. origin: {mode: \"box\" (default) | \"reference\" | \"metrics\" | \"layer\", boxPoint?: \"topLeft\" | \"top\" | \"topRight\" | \"left\" | \"center\" (default) | \"right\" | \"bottomLeft\" | \"bottom\" | \"bottomRight\", metric?: \"baseline\" | \"halfXHeight\" | \"xHeight\" | \"halfCapHeight\" | \"capHeight\", point?: [x, y]}, a metric name, or [x, y]. A metrics origin sits at half the advance width; the layer origin at half the width, midway between ascender and descender. For a glyph of the han, hangul, kana or bopomofo script a metrics origin is the layer origin. Points and anchors are rounded to the grid; a mirror, scale, rotation or slant that undoes the last of these on the same targets and origin, with nothing changed in between, gives back the coordinates from before it exactly. With `corners: [hint indices]` (and `glyph`, `layer?`; no other target or origin) it flips those corner components in place instead, as cornerComponent.setScale with `flip` does, and returns what that returns → {glyphs: [names], layers: count, origin?: [x, y] (with one layer)}",
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
            "{glyph: name, layer?: id, side?, points?: [[shape, node]], segments?: [[shape, node]], shapes?: [index], anchors?: [name], guides?: [{scope: \"local\" | \"global\", index}], annotations?: [index]} moves each selected object so its left edge meets the left edge of the selection's box. Objects: each shape, each on-curve node with its handles, each lone handle, each anchor, each guide (by the point it passes through; its angle stays) and each annotation; without target lists, every shape of the layer. Ignores the transformation origin. Fewer than two objects change nothing → {glyph, moved: count, message?}",
            has_doc,
            align_left
        )
        .targets(ALIGN_TARGETS),
        cmd!(
            "transform.alignCenterHorizontal",
            "Align Horizontal Centers",
            [],
            None,
            "{glyph: name, layer?: id, side?, points?, segments?, shapes?, anchors?, guides?, annotations?} moves each selected object so its horizontal centre is the centre of the selection's box: they share one vertical axis. Objects as for transform.alignLeft → {glyph, moved: count, message?}",
            has_doc,
            align_center_horizontal
        )
        .targets(ALIGN_TARGETS),
        cmd!(
            "transform.alignRight",
            "Align Right",
            [],
            None,
            "{glyph: name, layer?: id, side?, points?, segments?, shapes?, anchors?, guides?, annotations?} moves each selected object so its right edge meets the right edge of the selection's box. Objects as for transform.alignLeft → {glyph, moved: count, message?}",
            has_doc,
            align_right
        )
        .targets(ALIGN_TARGETS),
        cmd!(
            "transform.alignTop",
            "Align Top",
            [],
            None,
            "{glyph: name, layer?: id, side?, points?, segments?, shapes?, anchors?, guides?, annotations?} moves each selected object so its top edge meets the top edge of the selection's box. Objects as for transform.alignLeft → {glyph, moved: count, message?}",
            has_doc,
            align_top
        )
        .targets(ALIGN_TARGETS),
        cmd!(
            "transform.alignCenterVertical",
            "Align Vertical Centers",
            [],
            None,
            "{glyph: name, layer?: id, side?, points?, segments?, shapes?, anchors?, guides?, annotations?} moves each selected object so its vertical centre is the centre of the selection's box: they share one horizontal axis. Objects as for transform.alignLeft → {glyph, moved: count, message?}",
            has_doc,
            align_center_vertical
        )
        .targets(ALIGN_TARGETS),
        cmd!(
            "transform.alignBottom",
            "Align Bottom",
            [],
            None,
            "{glyph: name, layer?: id, side?, points?, segments?, shapes?, anchors?, guides?, annotations?} moves each selected object so its bottom edge meets the bottom edge of the selection's box. Objects as for transform.alignLeft → {glyph, moved: count, message?}",
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
            "{glyph?: name, glyphs?: [names], layer?: id, layers?: [ids] (with glyphs; default the first master's layer), side?, points?: [[shape, node]], segments?, shapes?: [index] (target lists with glyph; without them, whole layers), translate?: {x, y}, scale?: {x: percent, y?: percent (default x)}, slant?: degrees, cursivy?: bool, origin?: \"capHeight\" | \"halfCapHeight\" | \"xHeight\" | \"halfXHeight\" | \"baseline\" (default) or an origin object as for transform.mirrorHorizontal, string?: export-filter string} scales, then slants, about the origin, then translates. `string` takes the place of the other settings, e.g. \"Transformations;ScaleX:90;ScaleY:90;Slant:10;SlantCorrection:0;OffsetX:20;Origin:4;\" (Origin 0 cap height … 4 baseline; LSB, RSB or Width change the metrics after the transformation, added with a sign and set without one; include: or exclude: name the glyphs); with `string` and no glyph or glyphs, every glyph is changed. cursivy needs a horizontal and a vertical stem in the layer's master: the slant is corrected so that round strokes keep their thickness; closed paths' curves move, straight segments stay on the plain slant. Points and anchors are rounded to the grid → {glyphs: [names], layers: count, origin?: [x, y]}",
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

/// Whether the metrics origin of `glyph` in the active document is the centre of the layer: the
/// glyph's script is han, hangul, kana or bopomofo, as stored or as the glyph data gives it. The
/// Transformations section shows the layer-centre control in place of the metric heights then.
pub fn layer_center_origin(s: &Session, glyph: &str) -> bool {
    let Ok(font) = s.font() else { return false };
    let data = s.glyph_data().ok();
    super::widths::is_cjk(font, data.as_deref(), glyph)
}

/// `origin` for a layer of `glyph`: a metrics origin is the layer centre for a CJK glyph.
fn origin_for(origin: Origin, font: &Font, data: Option<&GlyphData>, glyph: &str) -> Origin {
    match origin {
        Origin::Metric(_) if super::widths::is_cjk(font, data, glyph) => Origin::Layer,
        other => other,
    }
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
            .map(|g| Point::new(placed_x(g, parts.width), g.y)),
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

/// A guide through the mapped point, along the mapped direction, in a layer `width` wide.
fn map_guide(p: &Params, g: &mut Guide, m: Affine, width: f64) -> Result<()> {
    let mut to = placed(g, width);
    map_placed(p, &mut to, m)?;
    if !can_place(g, to.x, width) {
        return Err(p.bad(format!("the result puts a guide further than {MAX_COORDINATE} from the right sidebearing, which it is referenced to")));
    }
    edit_placed(g, width, |g| {
        *g = to;
        Ok(())
    })
}

fn map_placed(p: &Params, g: &mut Guide, m: Affine) -> Result<()> {
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
        map_guide(p, g, m, parts.width)?;
    }
    for &i in &sel.annotations {
        let a = parts.annotations.get_mut(i).ok_or_else(|| p.bad(format!("annotation {i} does not resolve")))?;
        let q = map_point(p, m, Point::new(a.x, a.y), 0.0)?;
        (a.x, a.y) = (q.x, q.y);
    }
    Ok(())
}

/// Runs `edit`, which moves the points `touched`, then turns the handles of the smooth nodes next
/// to a line in those paths back onto their lines ([`super::node::keep_tangents`]), except the
/// handles in `placed`.
fn keeping_tangents<T>(
    layer: &mut Layer,
    side: LayerSide,
    touched: &BTreeSet<NodeRef>,
    placed: &BTreeSet<NodeRef>,
    step: f64,
    edit: impl FnOnce(&mut Layer) -> Result<T>,
) -> Result<T> {
    let paths: BTreeSet<usize> = touched.iter().map(|p| p.0).collect();
    let before: Vec<(usize, Vec<Point>)> = {
        let parts = layer.parts(side);
        paths.into_iter().filter_map(|i| Some((i, parts.path(i)?.nodes.iter().map(Node::point).collect()))).collect()
    };
    let out = edit(layer)?;
    let mut parts = layer.parts_mut(side);
    for (i, old) in before {
        if let Some(path) = parts.path_mut(i) {
            super::node::keep_tangents(path, &old, step, |j| placed.contains(&(i, j)));
        }
    }
    Ok(out)
}

/// Where Cursivy puts the nodes of `sel` that it moves off the place the map `m` gives them,
/// unrounded.
fn cursivy_places(p: &Params, parts: LayerParts, sel: &Selection, m: Affine, options: po::CursivyOptions) -> Result<Vec<(usize, usize, Point)>> {
    let moves = |s: usize, n: usize| sel.shapes.contains(&s) || sel.points.contains(&(s, n));
    let (mut contours, mut shapes) = (Vec::new(), Vec::new());
    for (i, shape) in parts.shapes.iter().enumerate() {
        let Shape::Path(path) = shape else { continue };
        let nodes = path
            .nodes
            .iter()
            .enumerate()
            .map(|(k, n)| po::Node {
                point: if moves(i, k) { m * n.point() } else { n.point() },
                kind: match n.kind {
                    NodeKind::Line => po::NodeKind::Line,
                    NodeKind::Curve => po::NodeKind::Curve,
                    NodeKind::QCurve => po::NodeKind::QCurve,
                    NodeKind::OffCurve => po::NodeKind::OffCurve,
                },
                smooth: n.smooth,
                tag: None,
            })
            .collect();
        let contour = po::Contour::new(path.closed, nodes);
        // A path that makes no outline is slanted only.
        if contour.to_bez().is_ok() {
            contours.push(contour);
            shapes.push(i);
        }
    }
    let shape = |c: usize| shapes.get(c).copied();
    let places = po::cursivy(&contours, |c, n| shape(c).is_some_and(|s| moves(s, n)), options).map_err(|e| p.bad(e.to_string()))?;
    Ok(places.into_iter().filter_map(|(c, n, at)| Some((shape(c)?, n, at))).collect())
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
/// see [`image_bounds`]. `settings` holds the rest of a transformation: the optical correction of
/// its slant, for which every layer's master needs its stems ([`primary_stems`]), and the metrics
/// changed once every layer is transformed.
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
    settings: Option<&Transformations>,
) -> Result<Vec<Point>> {
    let step = grid(s.font()?);
    let data = if matches!(origin, Origin::Metric(_)) { s.glyph_data().ok() } else { None };
    s.edit(label, |font| {
        let mut origins = Vec::with_capacity(jobs.len());
        // Jobs come glyph by glyph: the script is looked up once per glyph.
        let mut glyph_origin: Option<(&str, Origin)> = None;
        for t in jobs {
            let origin = match glyph_origin {
                Some((name, o)) if name == t.glyph => o,
                _ => {
                    let o = origin_for(origin, font, data.as_deref(), &t.glyph);
                    glyph_origin = Some((t.glyph.as_str(), o));
                    o
                }
            };
            let (sel, o, master, width, stems) = {
                let r = t.resolve(font)?;
                crate::check_unlocked(r.glyph)?;
                // The image alone, when it is given without target lists.
                let sel = if image.is_some() && !t.has_lists() { Selection::default() } else { selection(t, r.parts) };
                let bounds = if matches!(origin, Origin::Box(_)) { union(selection_box(font, r.master, r.parts, &sel), image) } else { None };
                let stems = match settings.filter(|s| s.cursivy) {
                    Some(c) => Some(c.cursivy_options(font, r.master).ok_or_else(|| no_stems(p, r.master))?),
                    None => None,
                };
                (sel, origin_point(origin, bounds, font, r.master, r.layer.width), r.master.id.clone(), r.layer.width, stems)
            };
            let m = Affine::translate(shift) * about(o, lin);
            t.edit_layer(font, |l| {
                let corrected = match stems {
                    Some(options) => cursivy_places(p, l.parts(t.side), &sel, m, options)?,
                    None => Vec::new(),
                };
                apply(p, l, t.side, &sel, m, step)?;
                let mut parts = l.parts_mut(t.side);
                for (s, n, at) in corrected {
                    let node = parts.node_mut(s, n).ok_or_else(|| p.bad(format!("point [{s}, {n}] does not resolve")))?;
                    let q = map_point(p, Affine::IDENTITY, at, step)?;
                    (node.x, node.y) = (q.x, q.y);
                }
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
                    map_guide(p, g, m, width)?;
                }
            }
            origins.push(o);
        }
        if let Some(mode) = settings.and_then(Transformations::metrics_mode) {
            for t in jobs {
                tm::change_target(p, font, t, mode, step)?;
            }
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
///
/// A transformation that undoes the last one exactly (`transform.oppositeButtonsExact`) gives back
/// the layers from before that one instead of mapping the rounded coordinates again: see
/// [`LastPalette`].
fn palette(s: &mut Session, v: &Value, id: &'static str, label: &str, lin: impl FnOnce(&Params) -> Result<Affine>) -> Result<Value> {
    let p = Params::new(id, v)?;
    let lin = lin(&p)?;
    let origin = parse_origin(&p, Origin::Box(BoxPoint::Center))?;
    let jobs = jobs(&p, s.font()?, false, false)?;
    let image = image_bounds(s, &p, &jobs)?;
    // A drag with the Rotate or Scale tool previews from the interaction's start: no memory.
    if s.interaction().is_some() {
        s.slot_mut::<LastPalette>().0 = None;
        return run_jobs(s, &p, label, &jobs, origin, lin, Vec2::ZERO, image, None).map(|origins| result(&jobs, &origins));
    }
    let (doc, before) = {
        let d = s.doc()?;
        (d.id, d.font.clone())
    };
    let step = PaletteStep { doc, revision: 0, before, jobs, image: image.is_some(), origin, lin, origins: Vec::new() };
    let origins = match undone_layers(s, &step)? {
        Some((layers, origins)) => {
            restore(s, &p, label, &step.jobs, layers)?;
            origins
        }
        None => run_jobs(s, &p, label, &step.jobs, origin, lin, Vec2::ZERO, image, None)?,
    };
    let out = result(&step.jobs, &origins);
    let revision = s.doc()?.revision;
    s.slot_mut::<LastPalette>().0 = Some(PaletteStep { revision, origins, ..step });
    Ok(out)
}

/// The last Palette transformation, kept for the one after it (`transform.oppositeButtonsExact`).
///
/// A session slot is not rolled back with a failing command, so it is written only after the
/// transformation succeeded, and it names the document's revision right after it: any other
/// change, an undo included, gives the document a revision never used before.
#[derive(Default)]
struct LastPalette(Option<PaletteStep>);

struct PaletteStep {
    doc: u64,
    revision: u64,
    /// The font before the transformation.
    before: Arc<Font>,
    jobs: Vec<Targets>,
    image: bool,
    origin: Origin,
    lin: Affine,
    /// The origin used in each layer.
    origins: Vec<Point>,
}

/// When `next` undoes the remembered transformation: the layers it changed as they were before
/// it, by job, and the origins it used. `None` when anything differs: the document, its
/// revision, the targets, the image flag, the origin, or a linear map that is not the inverse.
fn undone_layers(s: &Session, next: &PaletteStep) -> Result<Option<(Vec<Layer>, Vec<Point>)>> {
    let Some(last) = s.slot::<LastPalette>().and_then(|l| l.0.as_ref()) else { return Ok(None) };
    let same = last.doc == next.doc
        && last.revision == s.doc()?.revision
        && last.image == next.image
        && last.origin == next.origin
        && last.jobs.len() == next.jobs.len()
        && last.jobs.iter().zip(&next.jobs).all(|(a, b)| same_targets(a, b))
        && undoes(last.lin, next.lin);
    if !same {
        return Ok(None);
    }
    let mut layers = Vec::with_capacity(last.jobs.len());
    for t in &last.jobs {
        // A layer the transformation added has nothing to go back to: map it as usual.
        let Ok(r) = t.resolve(&last.before) else { return Ok(None) };
        layers.push(r.layer.clone());
    }
    Ok(Some((layers, last.origins.clone())))
}

/// Whether two target objects name the same things (the command that parsed them aside).
fn same_targets(a: &Targets, b: &Targets) -> bool {
    a.glyph == b.glyph
        && a.layer == b.layer
        && a.side == b.side
        && a.points == b.points
        && a.segments == b.segments
        && a.shapes == b.shapes
        && a.anchors == b.anchors
        && a.guides == b.guides
        && a.annotations == b.annotations
}

/// Whether `next` after `last` is the identity, up to floating-point error: mirror twice,
/// scale by s and by 1/s, rotate or slant by +a and −a.
fn undoes(last: Affine, next: Affine) -> bool {
    const TOLERANCE: f64 = 1e-9;
    let [a, b, c, d, e, f] = (next * last).as_coeffs();
    [a - 1.0, b, c, d - 1.0, e, f].iter().all(|v| v.abs() <= TOLERANCE)
}

/// Puts back each job's layer from before the transformation being undone, and the master's
/// global guides where a job moved some, as one undo step.
fn restore(s: &mut Session, p: &Params, label: &str, jobs: &[Targets], layers: Vec<Layer>) -> Result<()> {
    let before = s.slot::<LastPalette>().and_then(|l| l.0.as_ref()).map(|l| l.before.clone()).ok_or_else(|| p.bad("nothing to undo exactly"))?;
    s.edit(label, |font| {
        for (t, old) in jobs.iter().zip(layers) {
            t.edit_layer(font, |l| {
                *l = old;
                Ok(())
            })?;
            if t.guides.iter().any(|g| g.scope == GuideScope::Global) {
                let (id, guides) = {
                    let r = t.resolve(&before)?;
                    (r.master.id.clone(), r.master.guides.clone())
                };
                let masters = Arc::make_mut(&mut font.masters);
                let m = masters.iter_mut().find(|x| x.id == id).ok_or_else(|| p.bad(format!("there is no master `{id}`")))?;
                m.guides = guides;
            }
        }
        Ok(())
    })
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
/// each anchor, each guide by the point it passes through, each annotation. Without target
/// lists, every shape of the layer.
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
    let at = |g: &Guide| Point::new(placed_x(g, parts.width), g.y);
    for &i in &sel.local_guides {
        let Some(g) = parts.guides.get(i) else { continue };
        units.push(Unit { bounds: dot(at(g)), sel: Selection { local_guides: BTreeSet::from([i]), ..Selection::default() } });
    }
    for &i in &sel.global_guides {
        let Some(g) = master.guides.get(i) else { continue };
        units.push(Unit { bounds: dot(at(g)), sel: Selection { global_guides: BTreeSet::from([i]), ..Selection::default() } });
    }
    for &i in &sel.annotations {
        let Some(a) = parts.annotations.get(i) else { continue };
        units.push(Unit { bounds: dot(Point::new(a.x, a.y)), sel: Selection { annotations: BTreeSet::from([i]), ..Selection::default() } });
    }
    units
}

fn align(s: &mut Session, v: &Value, id: &'static str, label: &str, edge: Edge) -> Result<Value> {
    let p = Params::new(id, v)?;
    let t = Targets::parse(&p)?;
    let step = grid(s.font()?);
    let (units, master, width) = {
        let font = s.font()?;
        let r = t.resolve(font)?;
        (align_units(font, r.master, r.parts, &t), r.master.id.clone(), r.layer.width)
    };
    let Some(all) = units.iter().map(|u| u.bounds).reduce(|a, b| a.union(b)) else {
        return Ok(json!({"glyph": t.glyph, "moved": 0, "message": "Nothing is selected to align."}));
    };
    if units.len() < 2 {
        return Ok(json!({"glyph": t.glyph, "moved": 0, "message": "Select at least two objects to align them with each other."}));
    }
    let centre = |lo: f64, hi: f64| (lo + hi) / 2.0;
    let shift = |b: Rect| match edge {
        Edge::Left => Vec2::new(all.x0 - b.x0, 0.0),
        Edge::CenterHorizontal => Vec2::new(centre(all.x0, all.x1) - centre(b.x0, b.x1), 0.0),
        Edge::Right => Vec2::new(all.x1 - b.x1, 0.0),
        Edge::Top => Vec2::new(0.0, all.y1 - b.y1),
        Edge::CenterVertical => Vec2::new(0.0, centre(all.y0, all.y1) - centre(b.y0, b.y1)),
        Edge::Bottom => Vec2::new(0.0, all.y0 - b.y0),
    };
    // Handles that are objects of their own stay where the alignment puts them.
    let touched: BTreeSet<NodeRef> = units.iter().flat_map(|u| u.sel.points.iter().copied()).collect();
    let lone: BTreeSet<NodeRef> = units.iter().filter(|u| u.sel.points.len() == 1).flat_map(|u| u.sel.points.iter().copied()).collect();
    let moved = s.edit(label, |font| {
        let mut moved = t.edit_layer(font, |l| {
            keeping_tangents(l, t.side, &touched, &lone, step, |l| {
                let mut moved = 0usize;
                for u in units.iter().filter(|u| u.sel.global_guides.is_empty()) {
                    let d = shift(u.bounds);
                    if d != Vec2::ZERO {
                        apply(&p, l, t.side, &u.sel, Affine::translate(d), step)?;
                        moved += 1;
                    }
                }
                Ok(moved)
            })
        })?;
        // Global guides belong to the master, not the layer.
        let global: Vec<(usize, Vec2)> =
            units.iter().flat_map(|u| u.sel.global_guides.iter().map(move |&i| (i, shift(u.bounds)))).filter(|(_, d)| *d != Vec2::ZERO).collect();
        if !global.is_empty() {
            let masters = Arc::make_mut(&mut font.masters);
            let m = masters.iter_mut().find(|x| x.id == master).ok_or_else(|| p.bad(format!("there is no master `{master}`")))?;
            for (i, d) in global {
                let g = m.guides.get_mut(i).ok_or_else(|| p.bad(format!("global guide {i} does not resolve")))?;
                map_guide(&p, g, Affine::translate(d), width)?;
                moved += 1;
            }
        }
        Ok(moved)
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

/// What Align Selection lines up along.
#[derive(Clone, Copy, PartialEq)]
enum AlignAxis {
    /// The smaller of the selection's width and height.
    Auto,
    /// One x.
    X,
    /// One y.
    Y,
}

/// Path › Align Selection: the selected points onto one coordinate.
fn align_selection(s: &mut Session, v: &Value) -> Result<Value> {
    align_selection_as(s, v, "path.alignSelection")
}

pub(super) fn align_selection_as(s: &mut Session, v: &Value, command: &'static str) -> Result<Value> {
    let p = Params::new(command, v)?;
    let t = Targets::parse(&p)?;
    if !t.guides.is_empty() || !t.annotations.is_empty() {
        return Err(p.bad("aligns nodes, handles and anchors; guides and annotations are not aligned"));
    }
    let axis = match p.opt_str("axis")? {
        None | Some("auto") => AlignAxis::Auto,
        Some("vertical") => AlignAxis::X,
        Some("horizontal") => AlignAxis::Y,
        Some(other) => return Err(p.bad(format!("`axis` must be auto, vertical or horizontal, got \"{other}\""))),
    };
    let origin = parse_origin(&p, Origin::Box(BoxPoint::Center))?;
    let plan = Plan::of(s.font()?, &p, &t)?;
    let reply = s.edit(&undo_label(&p, "Align Selection"), |font| {
        let reply = align_in(font, &p, &t, axis, origin)?;
        if let Some(plan) = &plan {
            let axis = match reply.get("axis").and_then(Value::as_str) {
                Some("x") => AlignAxis::X,
                Some(_) => AlignAxis::Y,
                None => axis,
            };
            plan.apply_targets(font, &p, &t, |font, at| align_in(font, &p, at, axis, origin).map(|_| ()))?;
        }
        Ok(reply)
    })?;
    Ok(with_matched(reply, plan))
}

/// Align Selection on the layer `t` names. Returns the reply.
fn align_in(font: &mut Font, p: &Params, t: &Targets, axis: AlignAxis, origin: Origin) -> Result<Value> {
    if let Some(out) = super::anchor::align_special(font, p, t)? {
        return Ok(out);
    }
    let step = grid(font);
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
    let sel = selection(t, r.parts);
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
        t.edit_layer(font, |l| {
            keeping_tangents(l, t.side, &sel.points, &BTreeSet::new(), step, |l| apply(p, l, t.side, &sel, Affine::translate(to - from), step))
        })?;
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
        AlignAxis::X => true,
        AlignAxis::Y => false,
        AlignAxis::Auto => bounds.width() <= bounds.height(),
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
    let moved = t.edit_layer(font, |l| {
        let mut moved = keeping_tangents(l, t.side, &points, &points, step, |l| {
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
                apply(p, l, t.side, &sel, Affine::translate(d), step)?;
                moved += 1;
            }
            Ok(moved)
        })?;
        for &i in &anchors {
            let Some(at) = l.parts(t.side).anchors.get(i).map(|a| Point::new(a.x, a.y)) else { continue };
            let d = shift(at);
            if d != Vec2::ZERO {
                apply(p, l, t.side, &Selection { anchors: BTreeSet::from([i]), ..Selection::default() }, Affine::translate(d), step)?;
                moved += 1;
            }
        }
        Ok(moved)
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
    /// Changes of the sidebearings or the width, which only the export string carries.
    pub lsb: Option<Shift>,
    pub rsb: Option<Shift>,
    pub width: Option<Shift>,
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
            lsb: None,
            rsb: None,
            width: None,
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

    /// The change of metrics, as Transform Metrics makes it.
    fn metrics_mode(&self) -> Option<tm::Mode> {
        match (self.width, self.lsb, self.rsb) {
            (Some(width), _, _) => Some(tm::Mode::Width { width, both: false }),
            (None, None, None) => None,
            (None, lsb, rsb) => Some(tm::Mode::Sides { lsb, rsb }),
        }
    }

    /// The correction of this slant in a layer of `master`; `None` without stems. The outline's
    /// stems are scaled with it.
    fn cursivy_options(&self, font: &Font, master: &Master) -> Option<po::CursivyOptions> {
        let (vertical, horizontal) = primary_stems(font, master)?;
        let (sx, sy) = (self.scale.0 / 100.0, self.scale.1 / 100.0);
        Some(po::CursivyOptions { shear: self.slant.to_radians().tan(), vertical_stem: vertical * sx.abs(), horizontal_stem: horizontal * sy.abs() })
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
        if self.width.is_some() && (self.lsb.is_some() || self.rsb.is_some()) {
            return Err("give `Width`, or `LSB` and `RSB`, not both".into());
        }
        for (key, side) in [("LSB", self.lsb), ("RSB", self.rsb)] {
            if side.is_some_and(|s| !SIDEBEARING_RANGE.contains(&s.value)) {
                return Err(format!("`{key}` must be between -32768 and 32767"));
            }
        }
        if self.width.is_some_and(|w| !WIDTH_RANGE.contains(&w.value.abs())) {
            return Err("`Width` must be between 0 and 65535, or change it by at most that".into());
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
/// allowed around items, keys and values. A `LSB`, `RSB` or `Width` value with a sign is added to
/// the metric; one without replaces it.
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
    let shift = |key: &str, value: &str| -> std::result::Result<Shift, String> {
        Ok(Shift { value: number(key, value)?, relative: value.starts_with(['+', '-']) })
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
            "LSB" => out.lsb = Some(shift(key, value)?),
            "RSB" => out.rsb = Some(shift(key, value)?),
            "Width" => out.width = Some(shift(key, value)?),
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
    for (key, shift) in [("LSB", t.lsb), ("RSB", t.rsb), ("Width", t.width)] {
        match shift {
            Some(Shift { value, relative: true }) => item(key, &format_args!("{value:+}")),
            Some(Shift { value, relative: false }) => item(key, &value),
            None => {}
        }
    }
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

/// The first vertical and the first horizontal stem value of `master` that are above zero, which
/// Cursivy needs.
fn primary_stems(font: &Font, master: &Master) -> Option<(f64, f64)> {
    let first = |horizontal: bool| font.stem_widths(master, horizontal).into_iter().find(|v| v.is_finite() && *v > 0.0);
    Some((first(false)?, first(true)?))
}

fn no_stems(p: &Params, master: &Master) -> EngineError {
    p.bad(format!("Cursivy needs a horizontal and a vertical stem in master `{}` (Font Info › Masters › Stems)", master.name))
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
    if args.metrics_mode().is_some() && jobs.iter().any(|t| t.has_lists() || t.side != LayerSide::Foreground) {
        return Err(p.bad("`LSB`, `RSB` and `Width` change whole layers: leave out the target lists and the background"));
    }
    let shift = Vec2::new(args.translate.0, args.translate.1);
    let origins = run_jobs(s, &p, "Transformations", &jobs, args.origin, args.linear(), shift, None, Some(&args))?;
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
        // A sign adds to the metric, no sign replaces it.
        let t = parse_transformations("Transformations; LSB: +20; RSB: 35; Origin: 4").unwrap();
        assert_eq!((t.lsb, t.rsb), (Some(Shift { value: 20.0, relative: true }), Some(Shift { value: 35.0, relative: false })));
        assert_eq!(write_transformations(&t).unwrap(), "Transformations;LSB:+20;RSB:35;Origin:4;");
        let t = parse_transformations("Transformations;Width:-12.5;Origin:4;").unwrap();
        assert_eq!(t.width, Some(Shift { value: -12.5, relative: true }));
        assert_eq!(write_transformations(&t).unwrap(), "Transformations;Width:-12.5;Origin:4;");
        // Neutral settings are left out; the origin is always written.
        assert_eq!(write_transformations(&Transformations::default()).unwrap(), "Transformations;Origin:4;");

        for bad in [
            "",
            "RemoveOverlap",
            "Transformations;Slant",
            "Transformations;Slant:90",
            "Transformations;ScaleX:abc",
            "Transformations;Origin:5",
            "Transformations;LSB:10;Width:500",
            "Transformations;RSB:40000",
            "Transformations;Width:wide",
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
