//! Colour layer commands: convert a layer to a palette, full-colour, iColor or svg layer, set its
//! palette index, size or file, and list a glyph's colour layers. A master layer is never
//! converted.
//!
//! Package M11.3 (`docs/plans/M11.md` § 4 › M11.3) fills [`specs`]; rows:
//! `layer.convertToColorPalette`, `layer.setColorPaletteIndex`, `layer.convertToIColor`,
//! `layer.setIColorSize`, `layer.setIColorImage`, `layer.convertToSvg`, `layer.setSvgImage`,
//! `layer.convertToColor`, `layer.listColorLayers`.
//!
//! - A colour layer carries its master layer's width: converting writes it, and a width change on
//!   the master layer follows through [`super::widths::color_layers_follow_master`].
//! - Image files are referenced, never embedded (`color.images.referencedNotEmbedded`): the layer
//!   stores the path, relative to the document's folder when the document has one.

use std::path::Path as FilePath;

use fontcraft_color::MAX_PALETTE_ROWS;
use fontcraft_color::layers::{self, ColorLayerKind, DEFAULT_ICOLOR_SIZE, ICOLOR_SIZES};
use fontcraft_geom::{Point, Rect, Shape as _};
use fontcraft_model::{Font, Image, Layer, MAX_COORDINATE, NodeKind, PaletteEntry, Shape, Transform};
use serde_json::{Value, json};

use super::*;
use crate::params::{Params, TargetKind};

/// Most bytes in a referenced file path.
const MAX_PATH_BYTES: usize = 4096;
/// The file kinds an iColor layer references.
const BITMAP_EXTENSIONS: [&str; 5] = ["png", "jpg", "jpeg", "tif", "tiff"];
const SVG_EXTENSIONS: [&str; 1] = ["svg"];

const LAYER_TARGET: &[TargetKind] = &[TargetKind::Layer];
const ROW_CONTEXT: &[&str] = &["layers.row"];

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "layer.convertToColorPalette",
            "Color Palette",
            [],
            None,
            "{glyph: name, layer: id (not a master layer), index?: int 0 to 65533 | \"*\" (the text colour; default: the layer's index if it is a palette layer, else 0)} make the layer a Color Palette layer that paints its outlines in one palette colour; any other colour kind it had is replaced and it takes its master layer's width → {glyph, layer, master, kind: \"palette\", label, index, width}",
            has_doc,
            convert_to_palette
        )
        .targets(LAYER_TARGET)
        .context(ROW_CONTEXT),
        cmd!(
            "layer.setColorPaletteIndex",
            "Set Palette Color of Layer",
            [],
            None,
            "{glyph: name, layer: id (a Color Palette layer), index: int 0 to 65533 | \"*\" (the text colour)} set which palette colour the layer paints in → {glyph, layer, master, kind: \"palette\", label, index, width}",
            has_doc,
            set_palette_index
        )
        .targets(LAYER_TARGET),
        cmd!(
            "layer.convertToIColor",
            "iColor",
            [],
            None,
            "{glyph: name, layer: id (not a master layer), size?: int 1 to 4096 pixels per em (default: the layer's size if it is an iColor layer, else 128)} make the layer an iColor layer, one bitmap at that size for the sbix table; any other colour kind it had is replaced and it takes its master layer's width → {glyph, layer, master, kind: \"iColor\", label, size, width}",
            has_doc,
            convert_to_icolor
        )
        .targets(LAYER_TARGET)
        .context(ROW_CONTEXT),
        cmd!(
            "layer.setIColorSize",
            "Set iColor Size",
            [],
            None,
            "{glyph: name, layer: id (an iColor layer), size: int 1 to 4096 pixels per em} set the layer's pixel size; its image is scaled about its bottom-left corner so it stays at unitsPerEm ÷ size units per pixel, refused when that would scale it to 0 or beyond the coordinate limit → {glyph, layer, master, kind: \"iColor\", label, size, width}",
            has_doc,
            set_icolor_size
        )
        .targets(LAYER_TARGET),
        cmd!(
            "layer.setIColorImage",
            "Set iColor Image",
            [],
            None,
            "{glyph: name, layer: id (an iColor layer), path: string (a .png, .jpg, .jpeg, .tif or .tiff file)} reference the bitmap file as the layer's image, replacing any image it had; it is placed with its bottom-left corner at the bottom left of the master layer's path bounds (the origin when the master layer has no paths) at unitsPerEm ÷ size units per pixel. The file is not read. A relative path is relative to the document's folder (to the working directory for a document never saved); the layer stores it relative to that folder, absolute until the first save → {glyph, layer, master, kind: \"iColor\", label, size, width, image: stored path}",
            has_doc,
            set_icolor_image
        )
        .targets(LAYER_TARGET),
        cmd!(
            "layer.convertToSvg",
            "svg",
            [],
            None,
            "{glyph: name, layer: id (not a master layer)} make the layer an svg layer, which holds one referenced SVG file for the SVG table; any other colour kind it had is replaced and it takes its master layer's width → {glyph, layer, master, kind: \"svg\", label, width}",
            has_doc,
            convert_to_svg
        )
        .targets(LAYER_TARGET)
        .context(ROW_CONTEXT),
        cmd!(
            "layer.setSvgImage",
            "Set SVG Image",
            [],
            None,
            "{glyph: name, layer: id (an svg layer), path: string (an .svg file)} reference the SVG file as the layer's artwork, replacing any image it had; its viewBox is placed with the bottom-left corner at the layer origin, one SVG unit per font unit. The file is not read. A relative path is relative to the document's folder (to the working directory for a document never saved); the layer stores it relative to that folder, absolute until the first save → {glyph, layer, master, kind: \"svg\", label, width, image: stored path}",
            has_doc,
            set_svg_image
        )
        .targets(LAYER_TARGET),
        cmd!(
            "layer.convertToColor",
            "Color",
            [],
            None,
            "{glyph: name, layer: id (not a master layer)} make the layer a full-colour layer, on which each path carries its own fill, stroke or gradient; any other colour kind it had is replaced and it takes its master layer's width → {glyph, layer, master, kind: \"fullColor\", label, width}",
            has_doc,
            convert_to_color
        )
        .targets(LAYER_TARGET)
        .context(ROW_CONTEXT),
        cmd!(
            query "layer.listColorLayers",
            "List Color Layers",
            [],
            None,
            "{glyphs?: [names] (default: every glyph with a colour layer), master?: id (default: every master)} each glyph's colour layers by master, in paint order (the first is painted at the bottom) → {glyphs: [{glyph, layers: [{layer, master, kind: \"palette\" | \"fullColor\" | \"iColor\" | \"svg\", label, index?: int | \"*\", size?: int, width, image?: stored path (iColor and svg layers), exists?: bool (whether the image file can be found)}]}]}",
            has_doc,
            list_color_layers
        ),
    ]
}

// ---------- the commands ----------

fn convert_to_palette(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("layer.convertToColorPalette", v)?;
    let index = palette_entry(&p)?;
    edit_color_layer(s, &p, "Color Palette", |layer, master| {
        let entry = match (index, layers::kind(layer)) {
            (Some(entry), _) => entry,
            (None, Some(ColorLayerKind::Palette(entry))) => entry,
            (None, _) => PaletteEntry::Index(0),
        };
        convert(layer, master, ColorLayerKind::Palette(entry));
        Ok(())
    })
}

fn set_palette_index(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("layer.setColorPaletteIndex", v)?;
    let entry = palette_entry(&p)?.ok_or_else(|| p.bad("`index` is required"))?;
    edit_color_layer(s, &p, "Set Palette Color of Layer", |layer, _| {
        expect_kind(&p, layer, "a Color Palette layer", |k| matches!(k, ColorLayerKind::Palette(_)))?;
        layer.info.attrs = layers::with_kind(&layer.info.attrs, Some(ColorLayerKind::Palette(entry)));
        Ok(())
    })
}

fn convert_to_icolor(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("layer.convertToIColor", v)?;
    let size = icolor_size(&p)?;
    edit_color_layer(s, &p, "iColor", |layer, master| {
        let size = match (size, layers::kind(layer)) {
            (Some(size), _) => size,
            (None, Some(ColorLayerKind::IColor(size))) if ICOLOR_SIZES.contains(&size) => size,
            (None, _) => DEFAULT_ICOLOR_SIZE,
        };
        convert(layer, master, ColorLayerKind::IColor(size));
        Ok(())
    })
}

fn set_icolor_size(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("layer.setIColorSize", v)?;
    let size = icolor_size(&p)?.ok_or_else(|| p.bad("`size` is required"))?;
    edit_color_layer(s, &p, "Set iColor Size", |layer, _| {
        let old = expect_kind(&p, layer, "an iColor layer", |k| matches!(k, ColorLayerKind::IColor(_)))?;
        if let ColorLayerKind::IColor(old) = old
            && ICOLOR_SIZES.contains(&old)
            && old != size
            && let Some(image) = layer.info.image.as_ref()
        {
            let transform = rescale(image.transform, f64::from(old) / f64::from(size));
            if !scale_in_range(&transform) {
                return Err(p.bad(format!("at {size} pixels per em the image would be scaled to 0 or beyond ±{MAX_COORDINATE}")));
            }
            if let Some(image) = layer.info.image.as_mut() {
                image.transform = transform;
            }
        }
        layer.info.attrs = layers::with_kind(&layer.info.attrs, Some(ColorLayerKind::IColor(size)));
        Ok(())
    })
}

fn set_icolor_image(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("layer.setIColorImage", v)?;
    let path = file_path(s, &p, &BITMAP_EXTENSIONS, "an iColor layer takes a PNG, JPEG or TIFF file")?;
    edit_color_layer(s, &p, "Set iColor Image", |layer, master| {
        let kind = expect_kind(&p, layer, "an iColor layer", |k| matches!(k, ColorLayerKind::IColor(_)))?;
        let size = match kind {
            ColorLayerKind::IColor(size) if ICOLOR_SIZES.contains(&size) => size,
            _ => {
                return Err(p.bad(format!(
                    "layer `{}` has an iColor size outside {} to {}; set its size first",
                    layer.id,
                    ICOLOR_SIZES.start(),
                    ICOLOR_SIZES.end()
                )));
            }
        };
        let origin = master.bounds.map_or(Point::ZERO, |r| Point::new(r.x0, r.y0));
        let k = master.units_per_em / f64::from(size);
        let transform = Transform::Decomposed { pos: (origin.x, origin.y), scale: (k, k), angle: 0.0, slant: (0.0, 0.0) };
        layer.info.image = Some(Image { path: Some(path), transform, ..Image::default() });
        Ok(())
    })
}

fn convert_to_svg(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("layer.convertToSvg", v)?;
    edit_color_layer(s, &p, "svg", |layer, master| {
        convert(layer, master, ColorLayerKind::Svg);
        Ok(())
    })
}

fn set_svg_image(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("layer.setSvgImage", v)?;
    let path = file_path(s, &p, &SVG_EXTENSIONS, "an svg layer takes an SVG file")?;
    edit_color_layer(s, &p, "Set SVG Image", |layer, _| {
        expect_kind(&p, layer, "an svg layer", |k| matches!(k, ColorLayerKind::Svg))?;
        layer.info.image = Some(Image { path: Some(path), ..Image::default() });
        Ok(())
    })
}

fn convert_to_color(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("layer.convertToColor", v)?;
    edit_color_layer(s, &p, "Color", |layer, master| {
        convert(layer, master, ColorLayerKind::FullColor);
        Ok(())
    })
}

fn list_color_layers(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("layer.listColorLayers", v)?;
    let assets = s.assets();
    let font = s.font()?;
    let master = match p.opt_str("master")? {
        Some(id) => Some(font.master(id)?.id.clone()),
        None => None,
    };
    let in_master = |l: &Layer| master.as_deref().is_none_or(|m| l.master == m);
    let names: Vec<String> = match p.opt_glyphs("glyphs", font)? {
        Some(names) => names,
        None => font.glyphs.iter().filter(|g| g.layers.iter().any(|l| in_master(l) && layers::kind(l).is_some())).map(|g| g.name.clone()).collect(),
    };
    let mut out = Vec::with_capacity(names.len());
    for name in &names {
        let glyph = font.glyph(name).ok_or_else(|| p.bad(format!("there is no glyph `{name}`")))?;
        let masters: Vec<&str> = font.masters.iter().map(|m| m.id.as_str()).filter(|m| master.as_deref().is_none_or(|w| w == *m)).collect();
        let mut found = Vec::new();
        for m in masters {
            for (layer, kind) in layers::color_layers(glyph, m) {
                let mut entry = layer_json(layer, kind);
                if matches!(kind, ColorLayerKind::IColor(_) | ColorLayerKind::Svg)
                    && let Some(path) = layer.info.image.as_ref().and_then(|i| i.path.as_deref())
                    && let Some(o) = entry.as_object_mut()
                {
                    o.insert("image".into(), json!(path));
                    o.insert("exists".into(), json!(assets.exists(path)));
                }
                found.push(entry);
            }
        }
        out.push(json!({"glyph": name, "layers": found}));
    }
    Ok(json!({"glyphs": out}))
}

// ---------- shared steps ----------

/// What a colour layer command needs from the layer's master layer and the font.
struct MasterInfo {
    width: f64,
    /// The master layer's path bounds; `None` when it has no paths.
    bounds: Option<Rect>,
    units_per_em: f64,
}

/// Runs `f` on layer `layer` of glyph `glyph` as one undo step, after the checks every colour
/// layer command makes: the glyph exists and is not locked, the layer exists, is not a master
/// layer, and its master layer exists. Returns the layer as [`layer_json`] describes it.
fn edit_color_layer(s: &mut Session, p: &Params, label: &str, f: impl FnOnce(&mut Layer, &MasterInfo) -> Result<()>) -> Result<Value> {
    let glyph = p.str("glyph")?.to_string();
    let id = p.str("layer")?.to_string();
    s.edit(label, |font: &mut Font| {
        let g = font.glyph(&glyph).ok_or_else(|| p.bad(format!("there is no glyph `{glyph}`")))?;
        crate::check_unlocked(g)?;
        let layer = g.layers.iter().find(|l| l.id == id).ok_or_else(|| p.bad(format!("glyph `{glyph}` has no layer `{id}`")))?;
        if layer.is_master_layer() {
            return Err(p.bad(format!("`{id}` is the master layer of `{glyph}`: only a layer that is not a master layer can be a colour layer")));
        }
        let master = g.layer(&layer.master).ok_or_else(|| p.bad(format!("glyph `{glyph}` has no master layer for the master of `{id}`")))?;
        let info = MasterInfo { width: master.width, bounds: path_bounds(master), units_per_em: f64::from(font.units_per_em) };
        let mut gm = font.glyph_mut(&glyph)?;
        let layer = gm.layers.iter_mut().find(|l| l.id == id).ok_or_else(|| p.bad(format!("glyph `{glyph}` has no layer `{id}`")))?;
        f(layer, &info)?;
        let kind = layers::kind(layer).ok_or_else(|| p.bad(format!("layer `{id}` is not a colour layer")))?;
        let mut out = layer_json(layer, kind);
        if let Some(o) = out.as_object_mut() {
            o.insert("glyph".into(), json!(glyph));
            if let Some(path) = layer.info.image.as_ref().and_then(|i| i.path.as_deref())
                && matches!(kind, ColorLayerKind::IColor(_) | ColorLayerKind::Svg)
            {
                o.insert("image".into(), json!(path));
            }
        }
        Ok(out)
    })
}

/// Makes `layer` a colour layer of `kind` with its master layer's width.
fn convert(layer: &mut Layer, master: &MasterInfo, kind: ColorLayerKind) {
    layer.info.attrs = layers::with_kind(&layer.info.attrs, Some(kind));
    layer.width = master.width;
}

/// The layer's colour kind when `ok` accepts it; otherwise an error saying what it must be.
fn expect_kind(p: &Params, layer: &Layer, what: &str, ok: impl Fn(&ColorLayerKind) -> bool) -> Result<ColorLayerKind> {
    match layers::kind(layer) {
        Some(k) if ok(&k) => Ok(k),
        Some(k) => Err(p.bad(format!("layer `{}` is a {} layer, not {what}", layer.id, layers::label(&k)))),
        None => Err(p.bad(format!("layer `{}` is not a colour layer, not {what}", layer.id))),
    }
}

/// `index`: a palette row, or `"*"` for the text colour.
fn palette_entry(p: &Params) -> Result<Option<PaletteEntry>> {
    let last = i64::try_from(MAX_PALETTE_ROWS).unwrap_or(i64::MAX).saturating_sub(1);
    match p.get("index") {
        None => Ok(None),
        Some(Value::String(s)) if s == "*" => Ok(Some(PaletteEntry::Foreground)),
        Some(Value::String(s)) => Err(p.bad(format!("`index` must be a whole number from 0 to {last} or \"*\", got \"{s}\""))),
        Some(_) => {
            let n = p.opt_int("index", 0, last)?.ok_or_else(|| p.bad("`index` is required"))?;
            let n = u16::try_from(n).map_err(|_| p.bad(format!("`index` must be a whole number from 0 to {last} or \"*\"")))?;
            Ok(Some(PaletteEntry::Index(n)))
        }
    }
}

/// `size`: pixels per em, within [`ICOLOR_SIZES`].
fn icolor_size(p: &Params) -> Result<Option<u16>> {
    let Some(n) = p.opt_int("size", i64::from(*ICOLOR_SIZES.start()), i64::from(*ICOLOR_SIZES.end()))? else { return Ok(None) };
    u16::try_from(n).map(Some).map_err(|_| p.bad("`size` is out of range"))
}

/// `path`: a file of one of `extensions`, as the layer stores it (see
/// `Session::stored_image_path`): relative to the document's folder in a saved document,
/// absolute in an unsaved one.
fn file_path(s: &Session, p: &Params, extensions: &[&str], what: &str) -> Result<String> {
    let path = p.str("path")?;
    if path.trim().is_empty() {
        return Err(p.bad("`path` is empty"));
    }
    check_path_text(p, "path", path)?;
    let ext = FilePath::new(path).extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
    if !extensions.contains(&ext.as_str()) {
        return Err(p.bad(format!("`path` must name {}: {what}", extensions.iter().map(|e| format!(".{e}")).collect::<Vec<_>>().join(", "))));
    }
    s.stored_image_path(path).ok_or_else(|| p.bad(format!("`path` `{path}` is not a usable file path")))
}

/// Refuses a file path parameter `key` that is too long or holds a control character.
pub(crate) fn check_path_text(p: &Params, key: &str, path: &str) -> Result<()> {
    if path.len() > MAX_PATH_BYTES {
        return Err(p.bad(format!("`{key}` is longer than {MAX_PATH_BYTES} bytes")));
    }
    if path.chars().any(char::is_control) {
        return Err(p.bad(format!("`{key}` holds a control character")));
    }
    Ok(())
}

/// The tight bounds of `layer`'s paths; `None` when it has none. A path of one node counts as
/// that point.
fn path_bounds(layer: &Layer) -> Option<Rect> {
    let mut out: Option<Rect> = None;
    for shape in &layer.shapes {
        let Shape::Path(path) = shape else { continue };
        let on_curve = path.nodes.iter().filter(|n| n.kind != NodeKind::OffCurve).map(|n| Point::new(n.x, n.y));
        let mut boxes: Vec<Rect> = fontcraft_geom::bounds(on_curve).into_iter().collect();
        if let Ok(bez) = path.to_bez() {
            boxes.extend(bez.segments().map(|seg| seg.bounding_box()));
        }
        for b in boxes {
            out = Some(out.map_or(b, |o| o.union(b)));
        }
    }
    out
}

/// `t` with its linear part scaled by `factor` and its translation kept, so the image grows or
/// shrinks about its own origin (its bottom-left corner).
fn rescale(t: Transform, factor: f64) -> Transform {
    match t {
        Transform::Decomposed { pos, scale, angle, slant } => {
            Transform::Decomposed { pos, scale: (scale.0 * factor, scale.1 * factor), angle, slant }
        }
        Transform::Matrix([a, b, c, d, dx, dy]) => Transform::Matrix([a * factor, b * factor, c * factor, d * factor, dx, dy]),
    }
}

/// Whether the linear part of `t` is finite, within ±[`MAX_COORDINATE`] and does not flatten the
/// image to nothing.
fn scale_in_range(t: &Transform) -> bool {
    let fits = |v: f64| v.is_finite() && v.abs() <= MAX_COORDINATE;
    match *t {
        Transform::Decomposed { scale: (x, y), .. } => fits(x) && fits(y) && x != 0.0 && y != 0.0,
        Transform::Matrix([a, b, c, d, _, _]) => [a, b, c, d].into_iter().all(fits) && a * d - b * c != 0.0,
    }
}

/// One colour layer as the commands report it.
fn layer_json(layer: &Layer, kind: ColorLayerKind) -> Value {
    let mut v = json!({
        "layer": layer.id,
        "master": layer.master,
        "kind": kind_name(&kind),
        "label": layers::label(&kind),
        "width": layer.width,
    });
    if let Some(o) = v.as_object_mut() {
        match kind {
            ColorLayerKind::Palette(PaletteEntry::Index(i)) => {
                o.insert("index".into(), json!(i));
            }
            ColorLayerKind::Palette(PaletteEntry::Foreground) => {
                o.insert("index".into(), json!("*"));
            }
            ColorLayerKind::IColor(size) => {
                o.insert("size".into(), json!(size));
            }
            ColorLayerKind::FullColor | ColorLayerKind::Svg => {}
        }
    }
    v
}

fn kind_name(kind: &ColorLayerKind) -> &'static str {
    match kind {
        ColorLayerKind::Palette(_) => "palette",
        ColorLayerKind::FullColor => "fullColor",
        ColorLayerKind::IColor(_) => "iColor",
        ColorLayerKind::Svg => "svg",
    }
}
