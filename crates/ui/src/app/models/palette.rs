//! The view models of the Info box and the Palette sections (Transformations, Fit Curve, Anchor Context).
//!
//! They read the canvas's selection, which lives in the app's `Tools` and moves without the engine or the view state,
//! so [`project`] runs on every projection and skips the work while the stamp, the selection and the tool stay.

use craft::ui::contract::{PlainData, Tracked};
use fontcraft_engine::cmd::stroke::stroke_style;
use fontcraft_engine::features::marks::contextual::anchor_context;
use fontcraft_engine::model::{Dimension, Shape};
use fontcraft_tools::Selection;
use serde_json::{Map, Value, json};

use super::components::{self, CapBox, ComponentBox, CornerBox, SmartModel};
use super::{ProjectCx, Stamp};
use crate::canvas::tools::{Tools, scene, targets};
use crate::panels::widgets::{KeyState, KeyedNumber, format_number};
use crate::shell::palette::{OriginMode, fit_curve_range_of};

/// The target parameters of a command (`glyph`, `layer`, `side`, the selection's lists) as JSON object text: view
/// models hold plain data, and a view adds its own keys with [`Targets::with`].
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct Targets {
    pub json: String,
}

impl Targets {
    fn of(map: Map<String, Value>) -> Targets {
        Targets { json: Value::Object(map).to_string() }
    }

    /// The targets with the keys of `extra` added.
    pub fn with(&self, extra: Value) -> Value {
        let mut map = match serde_json::from_str::<Value>(&self.json) {
            Ok(Value::Object(map)) => map,
            _ => Map::new(),
        };
        if let Value::Object(extra) = extra {
            map.extend(extra);
        }
        Value::Object(map)
    }
}

/// The edited glyph's part of the Info box: name, Unicode value, metrics and kerning groups.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct GlyphBox {
    pub glyph: String,
    pub unicode: Option<u32>,
    /// The layer the setters edit.
    pub layer: String,
    /// The text runs top to bottom: the box shows T, B, O and H instead of LSB, RSB and width.
    pub vertical: bool,
    pub left: KeyedNumber,
    pub right: KeyedNumber,
    pub width: KeyedNumber,
    pub top: KeyedNumber,
    pub bottom: KeyedNumber,
    /// The vertical origin and the vertical width (height).
    pub vert_origin: KeyedNumber,
    pub vert_width: KeyedNumber,
    /// The name of the master whose metrics the layer's master takes; the fields are then read only.
    pub linked: Option<String>,
    /// The glyph's kerning groups: left, right, top, bottom.
    pub groups: [String; 4],
    /// With several glyphs selected in text mode: every glyph the setters change (`glyph` is the edited one). Empty
    /// for one glyph.
    pub glyphs: Vec<String>,
    /// The metric fields (`left`, `width`, `vertOrigin`, …) whose values differ between the selected glyphs.
    pub mixed: Vec<String>,
    /// The kerning group sides (left, right, top, bottom) whose groups differ between the selected glyphs.
    pub mixed_groups: [bool; 4],
}

impl GlyphBox {
    /// The glyphs a setter changes: the selected glyphs, else the edited one.
    pub fn targets(&self) -> Vec<String> {
        if self.glyphs.is_empty() { vec![self.glyph.clone()] } else { self.glyphs.clone() }
    }

    /// Whether the metric field `field` shows *Multiple Values*.
    pub fn is_mixed(&self, field: &str) -> bool {
        self.mixed.iter().any(|f| f == field)
    }
}

/// What kind of selection the Info box shows.
#[derive(PlainData, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SelectionKind {
    /// One node: X and Y.
    Node,
    /// One handle: X, Y, dX, dY and its length, measured from its node.
    Handle,
    /// Anything else: the box's X and Y at the reference point, width and height.
    #[default]
    Box,
}

/// The selection part of the Info box.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct SelectionBox {
    pub kind: SelectionKind,
    /// The reference point X and Y report and edit.
    pub reference: String,
    /// Editing the width or the height scales both.
    pub locked: bool,
    pub x: Option<f64>,
    pub y: Option<f64>,
    pub width: Option<f64>,
    pub height: Option<f64>,
    /// A handle's offset from its node and its length.
    pub dx: Option<f64>,
    pub dy: Option<f64>,
    pub length: Option<f64>,
    /// The distance between two selected nodes.
    pub distance: Option<f64>,
    /// Selected points and points in the layer.
    pub selected: usize,
    pub total: usize,
    /// The targets of `selection.setPosition` and of `transform.scale`.
    pub place: Targets,
    pub scale: Targets,
}

/// The Info box.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct InfoBoxModel {
    pub glyph: Option<GlyphBox>,
    /// `None` while nothing is selected.
    pub selection: Option<SelectionBox>,
    /// The first selected PostScript hint, in place of the selection part (`panels::info_box_ps_hint`).
    pub ps_hint: Option<super::hint_canvas::PsHintBox>,
    /// The TrueType Instructor's box, in place of the whole box while it is the chosen tool (`panels::info_box_tt`).
    pub tt: Option<super::hint_canvas::TtBox>,
    /// The selected component, the corner and the cap on the selected nodes (`super::components`).
    pub component: Option<ComponentBox>,
    pub corner: Option<CornerBox>,
    pub cap: Option<CapBox>,
    /// The one selected node's interpolation curve and its timing graph (`panels::info_box_hoi`).
    pub hoi: Option<super::hoi::HoiBox>,
    /// The one selected guide (`panels::info_box_guide`).
    pub guide: Option<super::guide_box::GuideBox>,
    /// The edited glyph's kerning towards its neighbours in the edit view's text (`panels::info_box_kerning`).
    pub kerning: Option<super::info_box_kerning::KernBox>,
    /// The selected placed image, in place of the selection part (`panels::info_box_image`).
    pub image: Option<super::image_box::ImageBox>,
}

/// The Transformations section: its settings and what its buttons act on.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct TransformModel {
    /// `box`, `reference`, `metrics` or `layer`.
    pub mode: String,
    pub box_point: String,
    pub metric: String,
    /// The reference point placed with the Rotate or Scale tool.
    pub point: Option<[f64; 2]>,
    pub scale: [f64; 2],
    pub scale_locked: bool,
    pub rotate: f64,
    pub slant: f64,
    /// The `origin` parameter every transform command gets, as JSON text.
    pub origin: String,
    /// The targets of the transform buttons: the selection, or the whole layer when nothing is selected. `None`
    /// without an edited glyph.
    pub targets: Option<Targets>,
    /// The targets of the boolean buttons: the selected paths (a path counts when any of its nodes is selected), else
    /// every path.
    pub boolean: Option<Targets>,
    /// The selected corner components as `corners`, when there are any: the mirror buttons flip them in place
    /// (`corner.mirrorViaPalette`) and take no origin.
    pub corners: Option<Targets>,
    /// The edited glyph is of a CJK script: the layer-centre control stands in for the metric heights, and the engine
    /// puts a metrics origin at the layer's centre (`transform.originCjkLayerCenter`).
    pub cjk: bool,
}

/// The Fit Curve section.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct FitCurveModel {
    pub min: f64,
    pub max: f64,
    /// The selected curve segments; `None` when no point is selected.
    pub targets: Option<Targets>,
    /// The mean handle length of the selected segments, in percent.
    pub percent: Option<f64>,
}

/// The Anchor Context section: the one selected anchor whose name starts with `*`.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct AnchorContextModel {
    pub anchor: Option<String>,
    pub context: String,
    pub targets: Option<Targets>,
}

/// The Stroke section: what the selected paths' strokes have in common. A value is `None` when the paths differ or
/// one has no stroke.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct StrokeModel {
    /// The targets of `path.setStroke` with the selected paths as `shapes`; `None` while no path is selected.
    pub targets: Option<Targets>,
    /// Some selected path has a stroke.
    pub stroked: bool,
    pub any_open: bool,
    pub any_closed: bool,
    pub width: Option<f64>,
    /// As stored: `None` follows the width.
    pub height: Option<f64>,
    /// Percent, −100 to 100.
    pub position: Option<f64>,
    pub join: Option<String>,
    pub cap_start: Option<String>,
    pub cap_end: Option<String>,
    pub cap_axis: Option<String>,
    pub fill: Option<bool>,
}

/// One path's stroke.
#[derive(Clone, PartialEq)]
struct OneStroke {
    width: f64,
    height: Option<f64>,
    position: f64,
    join: &'static str,
    cap_start: &'static str,
    cap_end: &'static str,
    cap_axis: &'static str,
    fill: bool,
}

fn one_stroke(path: &fontcraft_engine::model::Path) -> Option<OneStroke> {
    let attrs = &path.info.attrs;
    // A size stored as an expression cannot be shown or edited here.
    let style = stroke_style(attrs).ok()??;
    let height = match &attrs.stroke_height {
        Some(Dimension::Number(n)) => Some(*n),
        _ => None,
    };
    Some(OneStroke {
        width: style.width,
        height,
        position: style.position * 100.0,
        join: style.join.name(),
        cap_start: style.cap_start.name(),
        cap_end: style.cap_end.name(),
        cap_axis: style.cap_axis.name(),
        fill: style.fill,
    })
}

/// The Stroke model of `paths` in `scene`.
fn stroke_model(scene: &fontcraft_tools::Scene<'_>, paths: &[usize], targets: Option<Map<String, Value>>) -> StrokeModel {
    let Some(mut targets) = targets.filter(|_| !paths.is_empty()) else { return StrokeModel::default() };
    targets.remove("points");
    targets.insert("shapes".into(), json!(paths));
    let mut m = StrokeModel { targets: Some(Targets::of(targets)), ..Default::default() };
    let mut strokes = Vec::new();
    for &i in paths {
        let Some(path) = scene.path(i) else { continue };
        m.any_open |= !path.closed;
        m.any_closed |= path.closed;
        strokes.push(one_stroke(path));
    }
    m.stroked = strokes.iter().any(Option::is_some);
    fn common<T: PartialEq + Clone>(strokes: &[Option<OneStroke>], get: impl Fn(&OneStroke) -> T) -> Option<T> {
        let mut values = strokes.iter().map(|s| s.as_ref().map(&get));
        let first = values.next()??;
        values.all(|v| v.as_ref() == Some(&first)).then_some(first)
    }
    m.width = common(&strokes, |s| s.width);
    m.height = common(&strokes, |s| s.height).flatten();
    m.position = common(&strokes, |s| s.position);
    m.join = common(&strokes, |s| s.join).map(str::to_string);
    m.cap_start = common(&strokes, |s| s.cap_start).map(str::to_string);
    m.cap_end = common(&strokes, |s| s.cap_end).map(str::to_string);
    m.cap_axis = common(&strokes, |s| s.cap_axis).map(str::to_string);
    m.fill = common(&strokes, |s| s.fill);
    m
}

/// The models of the Info box and the Palette sections.
#[derive(Default)]
pub struct PaletteModels {
    /// What the models were last projected from.
    seen: Option<(Stamp, Selection, Option<Vec<usize>>, bool)>,
    pub info_box: Tracked<InfoBoxModel>,
    pub transform: Tracked<TransformModel>,
    pub fit_curve: Tracked<FitCurveModel>,
    pub anchor_context: Tracked<AnchorContextModel>,
    pub stroke: Tracked<StrokeModel>,
    pub smart: Tracked<SmartModel>,
}

/// Projects the Info box and Palette models when the stamp, the selection or the focus moved.
pub fn project(models: &mut PaletteModels, cx: &mut ProjectCx<'_>, tools: &Tools, stamp: Stamp) {
    let key = (stamp, tools.selection.clone(), tools.focus.clone(), tools.image);
    if models.seen.as_ref() == Some(&key) {
        return;
    }
    models.seen = Some(key);
    let edit = Edit::read(cx, tools);
    let mut info = info_box(cx, edit.as_ref());
    info.ps_hint = super::hint_canvas::ps_hint_box(cx, &tools.selection);
    info.tt = super::hint_canvas::tt_box(cx);
    info.kerning = info.glyph.as_ref().and_then(|_| super::info_box_kerning::kern_box(cx));
    info.guide = super::guide_box::guide_box(cx, tools);
    info.image = edit.as_ref().filter(|_| tools.image).and_then(|e| {
        let mut targets = serde_json::Map::new();
        targets.insert("glyph".into(), Value::from(e.glyph.clone()));
        targets.insert("layer".into(), Value::from(e.layer.clone()));
        if cx.view.edit_background {
            targets.insert("side".into(), Value::from("background"));
        }
        super::image_box::image_box(cx.session, &targets)
    });
    super::set(&mut models.info_box, info);
    super::set(&mut models.transform, transform(cx, edit.as_ref()));
    super::set(&mut models.fit_curve, fit_curve(cx, edit.as_ref()));
    super::set(&mut models.anchor_context, anchor(edit.as_ref()));
    super::set(&mut models.stroke, edit.as_ref().map(|e| e.stroke.clone()).unwrap_or_default());
    super::set(&mut models.smart, smart(cx, tools).unwrap_or_default());
}

/// What the selection gives, read once per projection.
struct Edit {
    glyph: String,
    layer: String,
    selection: Selection,
    /// Target parameters by command id.
    targets: Vec<(&'static str, Map<String, Value>)>,
    /// The selected paths: those with a selected node, or selected whole.
    paths: Vec<usize>,
    /// Points in the layer.
    total: usize,
    /// The selected anchor whose name starts with `*`, alone, and its context.
    contextual: Option<(String, String)>,
    stroke: StrokeModel,
}

/// The commands whose targets the models need.
const TARGETED: &[&str] = &[
    "selection.getInfo",
    "selection.setPosition",
    "transform.scale",
    "transform.mirrorHorizontal",
    "fitCurve.applyStep",
    "anchor.setContext",
    "path.setStroke",
];

impl Edit {
    fn read(cx: &ProjectCx<'_>, tools: &Tools) -> Option<Edit> {
        let font = cx.session.font().ok()?;
        let scene = scene(font, cx.view, tools.focus.as_ref())?;
        let selection = tools.selection.clone();
        let targets: Vec<(&'static str, Map<String, Value>)> = TARGETED
            .iter()
            .filter_map(|id| fontcraft_engine::find_command(id).map(|spec| (*id, targets::selection_targets(&scene, &selection, spec))))
            .collect();
        let mut paths: Vec<usize> = selection.points.iter().map(|&(s, _)| s).chain(selection.shapes.iter().copied()).collect();
        paths.sort_unstable();
        paths.dedup();
        paths.retain(|&s| scene.path(s).is_some());
        let total = scene.layer.shapes.iter().map(|s| if let Shape::Path(p) = s { p.nodes.len() } else { 0 }).sum();
        let mut anchors = selection.anchors.iter();
        let contextual = match (anchors.next(), anchors.next()) {
            (Some(name), None) if name.starts_with('*') => {
                scene.layer.anchors.iter().find(|a| a.name == *name).map(|a| (name.clone(), anchor_context(a).unwrap_or_default().to_string()))
            }
            _ => None,
        };
        let stroke_targets = targets.iter().find(|(id, _)| *id == "path.setStroke").map(|(_, t): &(&str, Map<String, Value>)| t.clone());
        let stroke = stroke_model(&scene, &paths, stroke_targets);
        Some(Edit { glyph: scene.glyph.clone(), layer: scene.layer_id.clone(), selection, targets, paths, total, contextual, stroke })
    }

    fn targets(&self, id: &str) -> Option<Map<String, Value>> {
        self.targets.iter().find(|(i, _)| *i == id).map(|(_, t)| t.clone())
    }
}

/// Whether `targets` names part of a selection rather than the whole layer.
fn has_lists(targets: &Map<String, Value>) -> bool {
    ["points", "segments", "shapes", "anchors", "guides", "annotations"]
        .iter()
        .any(|k| targets.get(*k).and_then(Value::as_array).is_some_and(|l| !l.is_empty()))
}

fn info_box(cx: &mut ProjectCx<'_>, edit: Option<&Edit>) -> InfoBoxModel {
    let Some(edit) = edit else { return InfoBoxModel::default() };
    let selected = text_selection_glyphs(cx);
    let glyph = if selected.len() > 1 { shared_glyph_box(cx, edit, selected) } else { glyph_box(cx, edit) };
    let mut b = InfoBoxModel { glyph, selection: selection_box(cx, edit), ..Default::default() };
    // Component setters act on the foreground only.
    if let Ok(font) = cx.session.font()
        && let Some(layer) = font.glyph(&edit.glyph).and_then(|g| g.layers.iter().find(|l| l.id == edit.layer))
        && !cx.view.edit_background
    {
        let data = cx.session.glyph_data().ok();
        b.component = components::component_box(font, data.as_deref(), &edit.glyph, layer, &edit.selection);
        b.corner = components::corner_box(font, &edit.glyph, layer, &edit.selection);
        b.cap = components::cap_box(font, &edit.glyph, layer, &edit.selection);
    }
    if !cx.view.edit_background {
        b.hoi = cx.session.font().ok().and_then(|font| super::hoi::hoi_of(font, &edit.glyph, &edit.layer, &edit.selection.points));
    }
    b
}

/// The Smart Component controls for the selected smart components of the edited side.
fn smart(cx: &ProjectCx<'_>, tools: &Tools) -> Option<SmartModel> {
    let font = cx.session.font().ok()?;
    let scene = scene(font, cx.view, tools.focus.as_ref())?;
    let glyph = font.glyph(&scene.glyph)?;
    let layer = glyph.layers.iter().find(|l| l.id == scene.layer_id)?;
    let shapes: Vec<usize> = tools.selection.shapes.iter().copied().collect();
    components::smart_model(font, &scene.glyph, layer, scene.side, &shapes)
}

/// The fields of `spacing.getMetrics`'s reply for `field`.
fn keyed(reply: &Value, field: &str) -> KeyedNumber {
    let value = reply.get(field).and_then(Value::as_f64).filter(|v| v.is_finite());
    let Some(entry) = reply.get("keys").and_then(|k| k.get(field)).filter(|e| e.is_object()) else {
        return KeyedNumber { value, ..Default::default() };
    };
    let key = entry.get("key").and_then(Value::as_str).map(str::to_string);
    let target = entry.get("value").and_then(Value::as_f64);
    let (state, message) = match (entry.get("inSync").and_then(Value::as_bool), entry.get("error").and_then(Value::as_str)) {
        (_, Some(e)) => (KeyState::Error, Some(e.to_string())),
        (Some(false), None) => {
            let target = target.map_or_else(String::new, format_number);
            let actual = value.map_or_else(String::new, format_number);
            (KeyState::OutOfSync, Some(format!("Out of sync: the key gives {target}, the metric is {actual}")))
        }
        _ => (KeyState::InSync, target.map(|t| format!("= {}", format_number(t)))),
    };
    KeyedNumber { value, key, state, message }
}

fn glyph_box(cx: &mut ProjectCx<'_>, edit: &Edit) -> Option<GlyphBox> {
    let font = cx.session.font().ok()?;
    let glyph = font.glyph(&edit.glyph)?;
    let groups =
        [&glyph.kern_groups.left, &glyph.kern_groups.right, &glyph.kern_groups.top, &glyph.kern_groups.bottom].map(|g| g.clone().unwrap_or_default());
    let unicode = glyph.unicodes.first().copied();
    let reply = cx.session.query("spacing.getMetrics", &json!({"glyph": edit.glyph, "layer": edit.layer})).ok()?;
    let font = cx.session.font().ok()?;
    // A master that takes its metrics from another shows that master's layer, read only.
    let linked_master = reply.get("linkedTo").and_then(Value::as_str).and_then(|id| font.masters.iter().find(|m| m.id == id));
    let (reply, linked) = match linked_master {
        Some(master) => {
            let name = master.name.clone();
            let shown = font.glyph(&edit.glyph).and_then(|g| g.layer(&master.id)).map(|l| l.id.clone())?;
            (cx.session.query("spacing.getMetrics", &json!({"glyph": edit.glyph, "layer": shown})).ok()?, Some(name))
        }
        None => (reply, None),
    };
    let mut b = GlyphBox {
        glyph: edit.glyph.clone(),
        unicode,
        layer: edit.layer.clone(),
        // The canvas lays out one glyph left to right; a vertical text layout sets this.
        vertical: false,
        left: keyed(&reply, "left"),
        right: keyed(&reply, "right"),
        width: keyed(&reply, "width"),
        top: keyed(&reply, "top"),
        bottom: keyed(&reply, "bottom"),
        vert_origin: keyed(&reply, "vertOrigin"),
        vert_width: keyed(&reply, "vertWidth"),
        linked,
        groups,
        ..Default::default()
    };
    mark_derived(cx, &mut b);
    Some(b)
}

/// Most text items the Info box reads for the glyphs of a text selection.
const MAX_SELECTED_ITEMS: usize = 10_000;

/// The distinct glyphs of the text selection in text mode, in text order. Empty outside text mode.
fn text_selection_glyphs(cx: &ProjectCx<'_>) -> Vec<String> {
    if !crate::canvas::text::text_mode(cx.view) {
        return Vec::new();
    }
    let text = &cx.view.text;
    let mut names: Vec<String> = Vec::new();
    for index in text.text.selection().take(MAX_SELECTED_ITEMS) {
        if let Some(name) = text.item_glyph(index)
            && !names.iter().any(|n| n == name)
        {
            names.push(name.to_string());
        }
    }
    names
}

/// The glyph part for several glyphs, from `spacing.getSharedMetrics` on the edited glyph's master: a field shows
/// what every glyph shares, and the fields that differ are listed as mixed.
fn shared_glyph_box(cx: &mut ProjectCx<'_>, edit: &Edit, glyphs: Vec<String>) -> Option<GlyphBox> {
    let font = cx.session.font().ok()?;
    let master = font.glyph(&edit.glyph)?.layers.iter().find(|l| l.id == edit.layer).map(|l| l.master.clone())?;
    let reply = cx.session.query("spacing.getSharedMetrics", &json!({"glyphs": glyphs, "layer": master})).ok()?;
    let read: Vec<String> =
        reply.get("glyphs").and_then(Value::as_array).map(|l| l.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default();
    let fields = reply.get("fields");
    let mut mixed = Vec::new();
    let mut shared = |name: &str| -> KeyedNumber {
        let entry = fields.and_then(|f| f.get(name));
        if entry.and_then(|e| e.get("mixed")).and_then(Value::as_bool) == Some(true) {
            mixed.push(name.to_string());
            return KeyedNumber::default();
        }
        let key = entry.and_then(|e| e.get("key")).and_then(Value::as_str).map(str::to_string);
        let value = entry.and_then(|e| e.get("value")).and_then(Value::as_f64).filter(|v| v.is_finite());
        let state = if key.is_some() { KeyState::InSync } else { KeyState::Number };
        KeyedNumber { value, key, state, message: None }
    };
    let (left, right, width) = (shared("left"), shared("right"), shared("width"));
    let (top, bottom) = (shared("top"), shared("bottom"));
    let (vert_origin, vert_width) = (shared("vertOrigin"), shared("vertWidth"));
    let groups_reply = reply.get("groups");
    let group = |side: &str| -> (String, bool) {
        let entry = groups_reply.and_then(|g| g.get(side));
        let value = entry.and_then(|e| e.get("value")).and_then(Value::as_str).unwrap_or_default().to_string();
        (value, entry.and_then(|e| e.get("mixed")).and_then(Value::as_bool) == Some(true))
    };
    let [(l, lm), (r, rm), (t, tm), (b, bm)] = ["left", "right", "top", "bottom"].map(group);
    Some(GlyphBox {
        glyph: edit.glyph.clone(),
        unicode: None,
        layer: master,
        vertical: false,
        left,
        right,
        width,
        top,
        bottom,
        vert_origin,
        vert_width,
        linked: None,
        groups: [l, r, t, b],
        glyphs: if read.is_empty() { glyphs } else { read },
        mixed,
        mixed_groups: [lm, rm, tm, bm],
    })
}

/// The left, right and width fields of a layer aligned automatically show the derived values and the offset keys.
fn mark_derived(cx: &mut ProjectCx<'_>, b: &mut GlyphBox) {
    let Ok(reply) = cx.session.query("spacing.getDerivedMetrics", &json!({"glyph": b.glyph, "layer": b.layer})) else { return };
    if reply.get("aligned").and_then(Value::as_bool) != Some(true) {
        return;
    }
    for (name, field) in [("left", &mut b.left), ("right", &mut b.right), ("width", &mut b.width)] {
        field.value = reply.get(name).and_then(Value::as_f64).filter(|v| v.is_finite());
        field.key = reply.get("offsets").and_then(|o| o.get(name)).and_then(Value::as_str).map(str::to_string);
        field.state = KeyState::Auto;
        field.message = Some(if name == "width" {
            "Automatic alignment: the width comes from the components".to_string()
        } else {
            "Automatic alignment: the value comes from the components; type =+N or =-N to add to it or take from it".to_string()
        });
    }
}

fn selection_box(cx: &mut ProjectCx<'_>, edit: &Edit) -> Option<SelectionBox> {
    let reference = cx.view.palette.reference_point.clone();
    let mut query = edit.targets("selection.getInfo").filter(has_lists)?;
    query.insert("referencePoint".into(), json!(reference));
    let info = cx.session.query("selection.getInfo", &Value::Object(query)).ok()?;
    let num = |v: &Value, key: &str| v.get(key).and_then(Value::as_f64).filter(|n| n.is_finite());
    let mut b = SelectionBox {
        reference,
        locked: cx.view.palette.size_locked,
        selected: edit.selection.points.len(),
        total: edit.total,
        place: Targets::of(edit.targets("selection.setPosition").unwrap_or_default()),
        scale: Targets::of(edit.targets("transform.scale").unwrap_or_default()),
        ..Default::default()
    };
    if let Some(node) = info.get("node") {
        b.kind = SelectionKind::Node;
        (b.x, b.y) = (num(node, "x"), num(node, "y"));
    } else if let Some(handle) = info.get("handle") {
        b.kind = SelectionKind::Handle;
        (b.x, b.y) = (num(handle, "x"), num(handle, "y"));
        (b.dx, b.dy, b.length) = (num(handle, "dx"), num(handle, "dy"), num(handle, "length"));
    } else {
        (b.x, b.y, b.width, b.height) = (num(&info, "x"), num(&info, "y"), num(&info, "width"), num(&info, "height"));
        b.distance = info.get("distance").and_then(|d| num(d, "length"));
    }
    Some(b)
}

fn transform(cx: &ProjectCx<'_>, edit: Option<&Edit>) -> TransformModel {
    let s = &cx.view.palette.transform;
    let mode = match s.mode {
        OriginMode::Box => "box",
        OriginMode::Reference => "reference",
        OriginMode::Metrics => "metrics",
        OriginMode::Layer => "layer",
    };
    let targets = edit.and_then(|e| e.targets("transform.scale"));
    let boolean = edit.zip(targets.clone()).map(|(e, mut t)| {
        t.remove("points");
        t.remove("shapes");
        if !e.paths.is_empty() {
            t.insert("shapes".into(), json!(e.paths));
        }
        Targets::of(t)
    });
    TransformModel {
        mode: mode.into(),
        box_point: s.box_point.clone(),
        metric: s.metric.clone(),
        point: s.point,
        scale: s.scale,
        scale_locked: s.scale_locked,
        rotate: s.rotate_angle,
        slant: s.slant_angle,
        origin: s.origin_param().to_string(),
        targets: targets.map(Targets::of),
        boolean,
        corners: edit.and_then(|e| e.targets("transform.mirrorHorizontal")).filter(|t| t.contains_key("corners")).map(Targets::of),
        cjk: edit.is_some_and(|e| fontcraft_engine::cmd::transform::layer_center_origin(&*cx.session, &e.glyph)),
    }
}

fn fit_curve(cx: &mut ProjectCx<'_>, edit: Option<&Edit>) -> FitCurveModel {
    let [min, max] = fit_curve_range_of(cx.session, cx.view);
    let targets = edit.and_then(|e| e.targets("fitCurve.applyStep")).filter(|t| t.contains_key("points"));
    let percent = targets
        .clone()
        .and_then(|t| cx.session.query("fitCurve.getHandleLength", &Value::Object(t)).ok())
        .and_then(|r| r.get("percent").and_then(Value::as_f64))
        .filter(|p| p.is_finite());
    FitCurveModel { min, max, targets: targets.map(Targets::of), percent }
}

fn anchor(edit: Option<&Edit>) -> AnchorContextModel {
    let Some(edit) = edit else { return AnchorContextModel::default() };
    let Some((name, context)) = edit.contextual.clone() else { return AnchorContextModel::default() };
    let targets = edit.targets("anchor.setContext").map(|mut t| {
        t.remove("anchors");
        Targets::of(t)
    });
    AnchorContextModel { anchor: Some(name), context, targets }
}
