//! Autotrace: a layer's placed image traced into outlines ([`fontcraft_pathops::trace`]).
//! Row: `glyph.autotraceImage`.
//!
//! The image file is read through the session's asset store and decoded within the trace's
//! 64-megapixel limit. Its pixels map into the glyph through the image's transform, counted from
//! the image's bottom-left corner, so the outline lands where the image is drawn; a crop limits
//! the trace to the part shown. The contours are added after the layer's shapes, rounded to the
//! font's grid. Images are decoded and traced before the edit, which then applies every glyph's
//! result as one undo step.

use fontcraft_color::AssetStore;
use fontcraft_geom::{Affine, Point, Vec2, round_point};
use fontcraft_imagery as imagery;
use fontcraft_model::{Image, Layer, MAX_COORDINATE, MAX_OUTLINE_ELEMENTS, Node, NodeKind, Path, Shape};
use fontcraft_pathops::trace::{self, Bitmap, TraceOptions};
use fontcraft_pathops::{Contour, contours_from_bez};
use serde_json::{Value, json};

use super::common::grid;
use super::*;
use crate::params::{Params, TargetKind};

/// Most degrees between the directions into and out of an on-curve node for it to be smooth.
/// Looser than an exact join because handles are rounded to the grid.
const SMOOTH_DEGREES: f64 = 3.0;
/// Most skipped glyphs quoted when nothing could be traced.
const MAX_LISTED: usize = 5;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "glyph.autotraceImage",
            "Autotrace Image",
            ["Glyph"],
            None,
            "{glyphs?: [names] (default: every glyph whose layer has an image), layer?: id (default: the first master's layer), threshold?: number (above 0 and below 1, default 0.5), smoothness?: number (0 to 1, default 0.25), removeImage?: bool (default false)} trace the layer's placed image into outlines. A pixel darker than `threshold` is ink (0 is white, 1 black; transparent pixels count as white). Higher `smoothness` keeps fewer corners and fits curves more loosely. The contours are added to the layer's paths where the image is drawn, rounded to the grid; a crop limits the trace to the part shown, and specks under 2.5 square pixels are dropped. `removeImage` then removes each unlocked image. Images over 64 megapixels and traces of more than 100,000 contours are refused. Glyphs that cannot be traced are listed with the reason, and the call fails when none can → {traced: [{glyph, layer, contours, imageRemoved}], skipped: [{glyph, layer, reason}]}",
            has_doc,
            autotrace
        )
        .targets(&[TargetKind::Glyphs]),
    ]
}

/// One glyph's traced paths, waiting for the edit.
struct Traced {
    glyph: String,
    paths: Vec<Path>,
}

fn autotrace(s: &mut Session, value: &Value) -> Result<Value> {
    const ID: &str = "glyph.autotraceImage";
    let p = Params::new(ID, value)?;
    let options = TraceOptions {
        threshold: p.opt_f64("threshold")?.unwrap_or(trace::DEFAULT_THRESHOLD),
        smoothness: p.opt_f64("smoothness")?.unwrap_or(trace::DEFAULT_SMOOTHNESS),
    };
    options.validate().map_err(|e| p.bad(e.to_string()))?;
    let remove = p.bool_or("removeImage", false)?;
    let font = s.font()?;
    let layer_id = match p.opt_str("layer")? {
        Some(id) => id.to_string(),
        None => font.default_master()?.id.clone(),
    };
    let names: Vec<String> = match p.opt_glyphs("glyphs", font)? {
        Some(g) if g.is_empty() => return Err(p.bad("`glyphs` is empty")),
        Some(g) => g,
        None => font.glyphs.iter().filter(|g| g.layers.iter().any(|l| l.id == layer_id && l.info.image.is_some())).map(|g| g.name.clone()).collect(),
    };
    if names.is_empty() {
        return Err(p.bad(format!("no glyph has an image on layer `{layer_id}`")));
    }
    let step = grid(font);
    let store = s.assets();
    let mut done: Vec<Traced> = Vec::new();
    let mut skipped: Vec<Value> = Vec::new();
    for name in &names {
        let Some(glyph) = font.glyph(name) else { continue };
        let reason = if glyph.info.locked {
            Err("the glyph is locked".to_string())
        } else if let Some(layer) = glyph.layers.iter().find(|l| l.id == layer_id) {
            trace_layer(store.as_ref(), layer, &options, step)
        } else {
            Err("the glyph has no such layer".to_string())
        };
        match reason {
            Ok(paths) => done.push(Traced { glyph: name.clone(), paths }),
            Err(reason) => skipped.push(json!({"glyph": name, "layer": layer_id, "reason": reason})),
        }
    }
    if done.is_empty() {
        let listed: Vec<String> = skipped
            .iter()
            .take(MAX_LISTED)
            .map(|v| {
                let field = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
                format!("`{}`: {}", field("glyph"), field("reason"))
            })
            .collect();
        return Err(p.bad(format!("nothing could be traced ({})", listed.join("; "))));
    }
    let traced = s.edit("Autotrace Image", |font| {
        let mut out = Vec::with_capacity(done.len());
        for t in done {
            let mut g = font.glyph_mut(&t.glyph)?;
            let Some(layer) = g.layers.iter_mut().find(|l| l.id == layer_id) else { continue };
            let contours = t.paths.len();
            layer.shapes.extend(t.paths.into_iter().map(Shape::Path));
            let removed = remove && layer.info.image.as_ref().is_some_and(|i| !i.locked);
            if removed {
                layer.info.image = None;
            }
            out.push(json!({"glyph": t.glyph, "layer": layer_id, "contours": contours, "imageRemoved": removed}));
        }
        Ok(out)
    })?;
    Ok(json!({"traced": traced, "skipped": skipped}))
}

/// The traced paths of `layer`'s image, or why there are none.
fn trace_layer(store: &dyn AssetStore, layer: &Layer, options: &TraceOptions, step: f64) -> std::result::Result<Vec<Path>, String> {
    let image = layer.info.image.as_ref().ok_or("the layer has no image")?;
    let paths = trace_image(store, image, options, step)?;
    if paths.is_empty() {
        return Err("the image has nothing darker than the threshold".into());
    }
    let existing: usize = layer
        .shapes
        .iter()
        .map(|sh| match sh {
            Shape::Path(path) => path.nodes.len(),
            _ => 0,
        })
        .sum();
    let added: usize = paths.iter().map(|path| path.nodes.len()).sum();
    if existing.saturating_add(added) > MAX_OUTLINE_ELEMENTS {
        return Err(format!("the traced outline would give the layer more than {MAX_OUTLINE_ELEMENTS} nodes"));
    }
    Ok(paths)
}

/// `image` traced and placed in the glyph, as closed paths rounded to `step`.
fn trace_image(store: &dyn AssetStore, image: &Image, options: &TraceOptions, step: f64) -> std::result::Result<Vec<Path>, String> {
    let file = image.path.as_deref().ok_or("the image names no file")?;
    let bytes = store.read(file).map_err(|e| e.to_string())?;
    let rgba = imagery::decode_rgba(&bytes, trace::MAX_PIXELS).map_err(|e| e.to_string())?;
    drop(bytes);
    let mut bitmap = Bitmap::from_rgba(rgba.width, rgba.height, &rgba.pixels).map_err(|e| e.to_string())?;
    drop(rgba);
    let mut place = image.transform.affine();
    if let Some((x, y, w, h)) = image.crop.and_then(|c| crop_box(c, bitmap.width(), bitmap.height())) {
        bitmap = bitmap.cropped(x, y, w, h).map_err(|e| e.to_string())?;
        place = place * Affine::translate((f64::from(x), f64::from(y)));
    }
    let traced = trace::trace(&bitmap, options).map_err(|e| e.to_string())?;
    // A mirroring transform turns every contour round; turn them back.
    let mirrored = place.determinant() < 0.0;
    let mut out = Vec::new();
    for bez in traced {
        for contour in contours_from_bez(&(place * bez)) {
            let contour = if mirrored { contour.reversed().map_err(|e| e.to_string())? } else { contour };
            if let Some(path) = to_path(&contour, step)? {
                out.push(path);
            }
        }
    }
    Ok(out)
}

/// A crop rectangle `[x, y, width, height]` (image pixels from the bottom-left) as whole pixels
/// within a `width × height` image: the pixels it touches. `None` when it is not usable.
fn crop_box([x, y, w, h]: [f64; 4], width: u32, height: u32) -> Option<(u32, u32, u32, u32)> {
    if ![x, y, w, h].iter().all(|v| v.is_finite()) || w <= 0.0 || h <= 0.0 {
        return None;
    }
    let clamp = |v: f64, max: u32| v.clamp(0.0, f64::from(max)) as u32;
    let (x0, y0) = (clamp(x.floor(), width), clamp(y.floor(), height));
    let (x1, y1) = (clamp((x + w).ceil(), width), clamp((y + h).ceil(), height));
    (x1 > x0 && y1 > y0).then_some((x0, y0, x1 - x0, y1 - y0))
}

fn node_kind(k: fontcraft_pathops::NodeKind) -> NodeKind {
    match k {
        fontcraft_pathops::NodeKind::Line => NodeKind::Line,
        fontcraft_pathops::NodeKind::Curve => NodeKind::Curve,
        fontcraft_pathops::NodeKind::QCurve => NodeKind::QCurve,
        fontcraft_pathops::NodeKind::OffCurve => NodeKind::OffCurve,
    }
}

/// One traced contour as a closed model path on the grid; `None` when fewer than three nodes
/// are left once nodes that repeat the one before are dropped.
fn to_path(contour: &Contour, step: f64) -> std::result::Result<Option<Path>, String> {
    let mut nodes: Vec<Node> = Vec::with_capacity(contour.nodes.len());
    for n in &contour.nodes {
        let at = round_point(n.point, step);
        if !(at.x.is_finite() && at.y.is_finite() && at.x.abs() <= MAX_COORDINATE && at.y.abs() <= MAX_COORDINATE) {
            return Err(format!("the traced outline lies beyond ±{MAX_COORDINATE} units"));
        }
        let node = Node::new(at.x, at.y, node_kind(n.kind));
        let repeats = node.kind == NodeKind::Line && nodes.last().is_some_and(|prev| prev.on_curve() && prev.point() == at);
        if !repeats {
            nodes.push(node);
        }
    }
    if nodes.len() > 1
        && nodes.last().is_some_and(|l| l.kind == NodeKind::Line)
        && nodes.first().is_some_and(|f| f.on_curve() && Some(f.point()) == nodes.last().map(Node::point))
    {
        nodes.pop();
    }
    if nodes.len() < 3 {
        return Ok(None);
    }
    mark_smooth(&mut nodes);
    Ok(Some(Path::new(true, nodes)))
}

/// Marks on-curve nodes of a closed path smooth where a handle leaves them in the direction the
/// path arrives in (within [`SMOOTH_DEGREES`]).
fn mark_smooth(nodes: &mut [Node]) {
    let count = nodes.len();
    let points: Vec<(Point, bool)> = nodes.iter().map(|n| (n.point(), n.on_curve())).collect();
    for (i, node) in nodes.iter_mut().enumerate() {
        if !node.on_curve() {
            continue;
        }
        let (Some(&(prev, prev_on)), Some(&(next, next_on))) = (points.get((i + count - 1) % count), points.get((i + 1) % count)) else {
            continue;
        };
        if prev_on && next_on {
            continue;
        }
        let here = node.point();
        let (a, b): (Vec2, Vec2) = (here - prev, next - here);
        if a.hypot() <= 0.0 || b.hypot() <= 0.0 {
            continue;
        }
        node.smooth = a.cross(b).atan2(a.dot(b)).abs().to_degrees() <= SMOOTH_DEGREES;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crops_cover_the_pixels_they_touch_within_the_image() {
        assert_eq!(crop_box([10.5, 0.0, 20.0, 5.2], 100, 50), Some((10, 0, 21, 6)));
        assert_eq!(crop_box([-5.0, 40.0, 500.0, 500.0], 100, 50), Some((0, 40, 100, 10)));
        assert_eq!(crop_box([200.0, 0.0, 5.0, 5.0], 100, 50), None);
        assert_eq!(crop_box([0.0, 0.0, f64::NAN, 5.0], 100, 50), None);
        assert_eq!(crop_box([0.0, 0.0, -5.0, 5.0], 100, 50), None);
    }
}
