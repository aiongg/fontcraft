//! The mark cloud: while anchors are selected, the marks that attach at them, laid over every glyph of the text that
//! has an anchor of that name (`appearance.markCloudOnAnchorSelection`, `appearance.displayMarkCloud`).
//!
//! Which marks attach comes from [`fontcraft_engine::components::marks`]; a glyph's anchors are its effective ones, so a
//! composite shows the cloud at the anchors it gets from its components. The marks of one list and anchor are one shape
//! placed at each glyph's anchor: a fill of their union, worked out once, and on the edited glyph their outlines.
//! [`refresh`] keeps the shapes while a drag runs, and only places them again.

use std::collections::HashMap;
use std::sync::Arc;

use craft::behavior::kurbo::Affine;
use egui::{Color32, Mesh, Pos2, Rect, Shape, Stroke};
use fontcraft_engine::components::marks::{attaching_marks, listed_marks};
use fontcraft_engine::components::propagate::effective_anchors_with;
use fontcraft_engine::model::glyphdata::GlyphData;
use fontcraft_engine::model::{Font, LayerSide};
use fontcraft_geom::BezPath;

use super::layers::PaintCx;
use super::paint::{fill, flatten, outline_shapes, screen};
use super::text::{shows_text, text_mode};
use super::tools::{Target, target};
use crate::app::FontCraftApp;

/// Most text items that show a cloud.
const MAX_ITEMS: usize = 500;
/// Most selected anchors that show one.
const MAX_ANCHORS: usize = 16;
/// Most path elements one cloud holds; marks that would exceed it are left out.
const MAX_ELEMENTS: usize = 50_000;
/// Font units per em the fill is worked out at, whatever the font's own.
const FILL_UNITS: f64 = 1_000.0;

/// The marks of one list that attach at one anchor, around the anchor at (0, 0).
#[derive(Debug)]
pub struct MarkShape {
    pub outline: BezPath,
    /// The fill of their union, in font units.
    fill: Option<Mesh>,
    bounds: Rect,
}

/// The cloud of one anchor on one glyph.
#[derive(Clone, Debug)]
pub struct CloudPart {
    /// The text item; `None` for the edited glyph.
    pub item: Option<usize>,
    /// The anchor, in the glyph's font units.
    pub at: [f64; 2],
    pub shape: Arc<MarkShape>,
}

impl PartialEq for CloudPart {
    fn eq(&self, other: &Self) -> bool {
        self.item == other.item && self.at == other.at && Arc::ptr_eq(&self.shape, &other.shape)
    }
}

/// What the canvas draws.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Cloud {
    pub parts: Vec<CloudPart>,
}

/// What the parts were placed from.
#[derive(Clone, Debug, PartialEq)]
struct Key {
    doc: (u64, u64),
    data: u64,
    target: Target,
    anchors: Vec<String>,
    text: Option<u64>,
}

/// The shapes by the database's list and the anchor's name; `None` when no mark attaches.
type Shapes = HashMap<(Option<Vec<String>>, String), Option<Arc<MarkShape>>>;

/// The shapes and what they were made from.
#[derive(Debug, Default)]
pub struct CloudCache {
    key: Option<Key>,
    /// The document and its revision, the glyph data's revision and the master the shapes are of.
    made: Option<((u64, u64), u64, String)>,
    shapes: Shapes,
}

/// Brings `ToolView::cloud` up to date.
pub fn refresh(app: &mut FontCraftApp) {
    let Some(key) = key(app) else {
        if app.tools.cloud.key.is_some() {
            app.tools.cloud = CloudCache::default();
            app.tools.view.cloud = Cloud::default();
        }
        return;
    };
    if app.tools.cloud.key.as_ref() == Some(&key) {
        return;
    }
    let mut cache = std::mem::take(&mut app.tools.cloud);
    let parts = parts(app, &key, &mut cache).unwrap_or_default();
    cache.key = Some(key);
    app.tools.cloud = cache;
    app.tools.view.cloud = Cloud { parts };
}

/// What to show a cloud for; `None` for no cloud.
fn key(app: &FontCraftApp) -> Option<Key> {
    let anchors: Vec<String> = app.tools.selection.anchors.iter().filter(|a| !a.starts_with('_')).take(MAX_ANCHORS).cloned().collect();
    if anchors.is_empty() || text_mode(&app.view) || !app.session.settings().get_bool("appearance.displayMarkCloud").unwrap_or(true) {
        return None;
    }
    let target = target(&app.session, &app.view).filter(|t| t.side == LayerSide::Foreground)?;
    let doc = app.session.active()?;
    let text = shows_text(&doc.font, &app.view).then_some(app.view.text.revision);
    Some(Key { doc: (doc.id, doc.revision), data: app.session.glyph_data_revision(), target, anchors, text })
}

fn parts(app: &FontCraftApp, key: &Key, cache: &mut CloudCache) -> Option<Vec<CloudPart>> {
    let font = app.session.font().ok()?;
    let data = app.session.glyph_data().ok()?;
    let master = key.target.layer_in(font)?.master.clone();
    let made = (key.doc, key.data, master.clone());
    // While a drag runs, the marks stay as they were when it began.
    let same_drag = app.session.interaction().is_some()
        && cache.made.as_ref().is_some_and(|((doc, _), data, m)| *doc == key.doc.0 && *data == key.data && *m == master);
    if cache.made.as_ref() != Some(&made) && !same_drag {
        cache.shapes.clear();
        cache.made = Some(made);
    }
    let mut items: Vec<(Option<usize>, &str, &str)> = vec![(None, key.target.glyph.as_str(), key.target.layer.as_str())];
    if key.text.is_some() {
        let text = &app.view.text;
        let edited = text.edited_index();
        let others = (0..text.text.len()).filter(|i| Some(*i) != edited).filter_map(|i| {
            let name = text.item_glyph(i)?;
            Some((Some(i), name, text.text.layer_of(i, name).unwrap_or(master.as_str())))
        });
        items.extend(others.take(MAX_ITEMS));
    }
    // The places of the selected anchors on each glyph, by name and layer, with the shape each shows.
    let mut placed: HashMap<(&str, &str), Vec<([f64; 2], Arc<MarkShape>)>> = HashMap::new();
    let mut out = Vec::new();
    for (item, glyph, layer) in items {
        let on = placed.entry((glyph, layer)).or_insert_with(|| clouds_on(font, &data, glyph, layer, key, &master, &mut cache.shapes));
        out.extend(on.iter().map(|(at, shape)| CloudPart { item, at: *at, shape: Arc::clone(shape) }));
    }
    Some(out)
}

/// The selected anchors that layer `layer` of `glyph` has, with the marks that attach at each.
fn clouds_on(
    font: &Font,
    data: &GlyphData,
    glyph: &str,
    layer: &str,
    key: &Key,
    master: &str,
    shapes: &mut Shapes,
) -> Vec<([f64; 2], Arc<MarkShape>)> {
    let Ok(anchors) = effective_anchors_with(font, Some(data), glyph, layer) else { return Vec::new() };
    let mut listed: Option<Option<Vec<String>>> = None;
    let mut out = Vec::new();
    for name in &key.anchors {
        let Some(a) = anchors.iter().find(|a| a.name == *name) else { continue };
        let list = listed.get_or_insert_with(|| listed_marks(data, glyph)).clone();
        let shape = shapes.entry((list, name.clone())).or_insert_with_key(|(list, name)| shape(font, data, list.as_deref(), name, master));
        if let Some(shape) = shape
            && a.x.is_finite()
            && a.y.is_finite()
        {
            out.push(([a.x, a.y], Arc::clone(shape)));
        }
    }
    out
}

/// The marks of `listed` that attach at `anchor`, as one shape; `None` when there are none with an outline.
fn shape(font: &Font, data: &GlyphData, listed: Option<&[String]>, anchor: &str, master: &str) -> Option<Arc<MarkShape>> {
    let mut outline = BezPath::new();
    for mark in attaching_marks(font, data, listed, anchor, master) {
        let Ok(mut path) = font.outline(&mark.glyph, master) else { continue };
        if outline.elements().len() + path.elements().len() > MAX_ELEMENTS {
            break;
        }
        path.apply_affine(Affine::translate((-mark.x, -mark.y)));
        outline.extend(path.elements().iter().copied());
    }
    if outline.elements().is_empty() {
        return None;
    }
    let scale = FILL_UNITS / f64::from(font.units_per_em.max(1));
    let contours: Vec<Vec<Pos2>> = flatten(&outline, Affine::scale(scale)).into_iter().filter(|c| c.first() == c.last()).collect();
    let mut fill = fill::nonzero(&contours, Rect::EVERYTHING, Color32::WHITE);
    let back = (1.0 / scale) as f32;
    let mut bounds = Rect::NOTHING;
    if let Some(mesh) = &mut fill {
        for v in &mut mesh.vertices {
            v.pos = Pos2::new(v.pos.x * back, v.pos.y * back);
            bounds.extend_with(v.pos);
        }
    }
    Some(Arc::new(MarkShape { outline, fill, bounds }))
}

pub fn paint(cx: &PaintCx<'_>) {
    cx.painter.extend(shapes(cx));
}

/// The cloud's fills, and the marks' outlines on the edited glyph.
pub fn shapes(cx: &PaintCx<'_>) -> Vec<Shape> {
    if !cx.display.anchors {
        return Vec::new();
    }
    let t = cx.tokens;
    let colour = t.outline.gamma_multiply(t.mark_cloud_opacity);
    let mut out = Vec::new();
    for part in &cx.tools.cloud.parts {
        let origin = match part.item {
            None => [0.0, 0.0],
            Some(item) => match cx.text.glyphs.iter().find(|g| g.item == item) {
                Some(g) => g.at,
                None => continue,
            },
        };
        let [x, y] = [origin[0] + part.at[0], origin[1] + part.at[1]];
        let place = |p: Pos2| screen(cx, [x + f64::from(p.x), y + f64::from(p.y)]);
        let b = part.shape.bounds;
        let corners = [b.left_top(), b.right_top(), b.left_bottom(), b.right_bottom()].into_iter().filter_map(place);
        let shown = corners.fold(Rect::NOTHING, |r, p| r.union(Rect::from_min_max(p, p)));
        if !shown.intersects(cx.rect) {
            continue;
        }
        if let Some(fill) = &part.shape.fill {
            let mut mesh = fill.clone();
            for v in &mut mesh.vertices {
                v.pos = place(v.pos).unwrap_or(cx.rect.min);
                v.color = colour;
            }
            out.push(Shape::mesh(mesh));
        }
        if part.item.is_none() {
            let stroke = Stroke::new(t.outline_width, t.outline.gamma_multiply(t.mark_cloud_outline_opacity));
            out.extend(outline_shapes(cx, &(Affine::translate((x, y)) * &part.shape.outline), stroke));
        }
    }
    out
}
