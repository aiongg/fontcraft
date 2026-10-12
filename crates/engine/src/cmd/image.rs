//! Image commands: add images to glyphs (one file, or many matched to glyph names), move, lock,
//! unlock and crop them, and report what they are. Files are read through the session's asset
//! store.
//!
//! Rows: `glyph.addImage`, `image.setTransform`, `image.lock`, `image.unlock`, `image.setCrop`,
//! `image.setCropToLayerBounds`, `image.resetCrop`, `image.getInfo`; the behaviour rows
//! `image.defaultPlacement`, `image.lockedNotSelectable` (commands), `image.supportedFormats`,
//! `fontView.image.addImageSingle`, `fontView.image.batchByFileName` and
//! `fontView.image.caseSensitivity`.
//!
//! The image a command addresses is the layer's placed image (`side: "background"`: the
//! background's). Image shapes of the version 4 format are kept as read; these commands do not
//! address them. Crop rectangles are in image pixels from the image's bottom-left corner, the
//! space the image's transform maps into the glyph.

use std::collections::HashMap;

use fontcraft_color::AssetStore;
use fontcraft_geom::{Point, Rect};
use fontcraft_imagery::{self as imagery, ImageFormat};
use fontcraft_model::{Extra, Font, Image, Layer, LayerSide, MAX_COORDINATE, Transform, metric};
use serde_json::{Map, Value, json};

use super::common::in_range;
use super::*;
use crate::EngineError;
use crate::assets::MAX_ADD_IMAGE_FILES;
use crate::params::{Params, TargetKind, Targets};

/// The resolution of an image file that states none: one pixel per point.
pub const DEFAULT_PPI: f64 = 72.0;
/// Points per inch: Add Image maps one point of the image to one font unit.
const POINTS_PER_INCH: f64 = 72.0;
/// Most file names quoted in one message.
const MAX_LISTED: usize = 10;
/// Bytes of a file Add Image reads for its format, pixel size and resolution.
const HEADER_BYTES: usize = 1 << 20;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "glyph.addImage",
            "Add Image…",
            ["Glyph"],
            None,
            "{files: [path], glyphs?: [names], layer?: id (default: the first master's layer), side?: \"foreground\" | \"background\"} place image files on glyph layers. With `glyphs`, give one file: it goes on each named glyph. Without `glyphs`, each file goes on the glyph named like the file without its extension (`Dje-cy.jpeg` on Dje-cy), case included; files with no such glyph change nothing and are listed. A relative path is relative to the document's folder (to the working directory for a document never saved); the document stores the path relative to its folder, absolute until its first save. The image sits at the layer origin, one point of the image (its stated resolution, else 72 pixels per inch) to one unit, and replaces the layer's image unless that one is locked. PNG, JPEG and TIFF are drawn; PDF, PSD and SVG are kept as references. At most 10,000 files → {placed: [{glyph, layer, file, path, format, pixelWidth, pixelHeight, scale: [x, y], replaced}], unmatched: [file]}",
            has_doc,
            add_image
        ).files(const { &[FileParam::read("files").from_document()] }),
        cmd!(
            "image.setTransform",
            "Set Image Transform",
            [],
            None,
            "{glyph: name, layer?: id, side?: \"foreground\" | \"background\", x?: number, y?: number, width?: number, height?: number, rotation?: degrees} set the image's position (its bottom-left corner before rotation), displayed size in font units (of the whole image; a crop does not change it) and rotation about its position, as the image Info box does; give at least one value. Width and height need the image file's pixel size. A locked image refuses it → {glyph, layer, image}",
            has_doc,
            set_transform
        )
        .targets(&[TargetKind::Layer]),
        cmd!(
            "image.lock",
            "Lock Image",
            [],
            None,
            "{glyph: name, layer?: id, side?: \"foreground\" | \"background\"} lock the layer's image: commands that move, scale, rotate or crop it refuse until it is unlocked → {glyph, layer, locked: true}",
            has_doc,
            lock
        )
        .targets(&[TargetKind::Layer]),
        cmd!(
            "image.unlock",
            "Unlock Image",
            [],
            None,
            "{glyph: name, layer?: id, side?: \"foreground\" | \"background\"} unlock the layer's image → {glyph, layer, locked: false}",
            has_doc,
            unlock
        )
        .targets(&[TargetKind::Layer]),
        cmd!(
            "image.setCrop",
            "Crop Image",
            [],
            None,
            "{glyph: name, layer?: id, side?: \"foreground\" | \"background\", rect: [x, y, width, height]} show only this part of the image, in image pixels from its bottom-left corner; the rectangle must lie within the image when its file can be read. A locked image refuses it → {glyph, layer, image}",
            has_doc,
            set_crop
        )
        .targets(&[TargetKind::Layer]),
        cmd!(
            "image.setCropToLayerBounds",
            "Set Crop to Layer Bounds",
            [],
            None,
            "{glyph: name, layer?: id, side?: \"foreground\" | \"background\"} crop the image to the layer bounds: from 0 to the layer's width, from the master's descender to its ascender. Needs the image file's pixel size; refused when the image lies outside the bounds or is locked → {glyph, layer, image}",
            has_doc,
            set_crop_to_layer_bounds
        )
        .targets(&[TargetKind::Layer]),
        cmd!(
            "image.resetCrop",
            "Reset Crop",
            [],
            None,
            "{glyph: name, layer?: id, side?: \"foreground\" | \"background\"} show the whole image again. A locked image refuses it → {glyph, layer, image}",
            has_doc,
            reset_crop
        )
        .targets(&[TargetKind::Layer]),
        cmd!(
            query "image.getInfo",
            "Get Image Info",
            [],
            None,
            "{glyph: name, layer?: id, side?: \"foreground\" | \"background\"} the layer's image: its stored path, whether the file can be read and the file it names, its format and pixel size, position, displayed size, rotation, scale, slant, crop and lock state; `image` is null when the layer has none. A transform with shear beyond slant gives `matrix` instead of the decomposed values → {glyph, layer, side, image: {path, resolved, file?, format, pixelWidth?, pixelHeight?, x, y, width?, height?, rotation?, scale?, slant?, matrix?, crop, locked, alpha?, problem?} | null}",
            has_doc,
            get_info
        )
        .targets(&[TargetKind::Layer]),
    ]
}

// ── addressing ──────────────────────────────────────────────────────────────────────────────────

/// The image a command addresses on `layer`.
fn image_of(layer: &Layer, side: LayerSide) -> Option<&Image> {
    match side {
        LayerSide::Foreground => layer.info.image.as_ref(),
        LayerSide::Background => layer.info.background.as_ref()?.image.as_ref(),
    }
}

/// Where the addressed image is stored, for placing or editing it.
fn image_slot(layer: &mut Layer, side: LayerSide) -> &mut Option<Image> {
    match side {
        LayerSide::Foreground => &mut layer.info.image,
        LayerSide::Background => &mut layer.info.background.get_or_insert_with(Default::default).image,
    }
}

/// The one layer a command addresses; target lists are refused.
fn layer_targets(s: &Session, p: &Params) -> Result<Targets> {
    let mut t = Targets::parse(p)?;
    if t.has_lists() {
        return Err(p.bad("takes no target lists: it acts on the layer's image"));
    }
    t.layer = Some(t.layer_id(s.font()?)?);
    Ok(t)
}

fn no_image(p: &Params, t: &Targets) -> EngineError {
    let side = if t.side == LayerSide::Background { " background" } else { "" };
    p.bad(format!("layer `{}` of glyph `{}` has no{side} image", t.layer.as_deref().unwrap_or_default(), t.glyph))
}

fn locked(p: &Params, t: &Targets) -> EngineError {
    p.bad(format!("the image on glyph `{}` is locked: unlock it first", t.glyph))
}

/// The addressed image as it is now, refused when the layer has none.
fn current<'f>(s: &'f Session, p: &Params, t: &Targets) -> Result<&'f Image> {
    let r = t.resolve(s.font()?)?;
    image_of(r.layer, t.side).ok_or_else(|| no_image(p, t))
}

/// Runs `f` on the addressed image as one undo step. A missing or locked image refuses it,
/// unless `even_locked`.
fn edit_image<T>(s: &mut Session, p: &Params, t: &Targets, label: &str, even_locked: bool, f: impl FnOnce(&mut Image) -> Result<T>) -> Result<T> {
    s.edit(label, |font| {
        t.edit_layer(font, |l| {
            let image = image_slot(l, t.side).as_mut().ok_or_else(|| no_image(p, t))?;
            if image.locked && !even_locked {
                return Err(locked(p, t));
            }
            f(image)
        })
    })
}

// ── the file behind an image ────────────────────────────────────────────────────────────────────

/// What the image file says about itself.
struct Probe {
    resolved: bool,
    format: ImageFormat,
    /// The pixel size, when the header states one.
    pixels: Option<(u32, u32)>,
    problem: Option<String>,
}

fn probe(store: &dyn AssetStore, path: Option<&str>) -> Probe {
    let Some(path) = path else {
        return Probe { resolved: false, format: ImageFormat::Other, pixels: None, problem: Some("the image names no file".into()) };
    };
    let guess = ImageFormat::from_extension(path);
    match store.read(path) {
        Err(e) => Probe { resolved: false, format: guess, pixels: None, problem: Some(e.to_string()) },
        Ok(bytes) => match imagery::sniff(&bytes) {
            Err(e) => Probe { resolved: true, format: guess, pixels: None, problem: Some(e.to_string()) },
            Ok(info) => {
                let pixels = (info.width > 0 && info.height > 0).then_some((info.width, info.height));
                let problem = (!info.format.decodable())
                    .then(|| format!("{} images are kept as references and not drawn", format_name(info.format).to_uppercase()));
                Probe { resolved: true, format: info.format, pixels, problem }
            }
        },
    }
}

fn format_name(f: ImageFormat) -> &'static str {
    match f {
        ImageFormat::Png => "png",
        ImageFormat::Jpeg => "jpeg",
        ImageFormat::Tiff => "tiff",
        ImageFormat::Pdf => "pdf",
        ImageFormat::Psd => "psd",
        ImageFormat::Svg => "svg",
        ImageFormat::Other => "other",
    }
}

/// The pixel size of the addressed image's file, refused when it cannot be learnt.
fn pixel_size(s: &Session, p: &Params, image: &Image, why: &str) -> Result<(f64, f64)> {
    let found = probe(s.assets().as_ref(), image.path.as_deref());
    let (w, h) = found.pixels.ok_or_else(|| {
        let reason = found.problem.unwrap_or_else(|| "its file states no pixel size".into());
        p.bad(format!("{why} needs the image's pixel size: {reason}"))
    })?;
    Ok((f64::from(w), f64::from(h)))
}

// ── transforms ──────────────────────────────────────────────────────────────────────────────────

/// A transform as position, scale, rotation (degrees) and slant (degrees), the way format
/// versions 3 and 4 store an image's.
#[derive(Clone, Copy)]
struct Parts {
    pos: (f64, f64),
    scale: (f64, f64),
    angle: f64,
    slant: (f64, f64),
}

impl Parts {
    /// `t` decomposed; `None` for a matrix with shear, which has no rotation-and-scale form.
    fn of(t: &Transform) -> Option<Parts> {
        match *t {
            Transform::Decomposed { pos, scale, angle, slant } => Some(Parts { pos, scale, angle, slant }),
            Transform::Matrix([a, b, c, d, dx, dy]) => {
                let sx = a.hypot(b);
                let size = a.abs().max(b.abs()).max(c.abs()).max(d.abs());
                // The columns must be at right angles: rotation times scale.
                if sx.is_nan() || sx <= 0.0 || sx.is_infinite() || (a * c + b * d).abs() > 1e-9 * size * size {
                    return None;
                }
                Some(Parts { pos: (dx, dy), scale: (sx, (a * d - b * c) / sx), angle: b.atan2(a).to_degrees(), slant: (0.0, 0.0) })
            }
        }
    }

    fn transform(self) -> Transform {
        Transform::Decomposed { pos: self.pos, scale: self.scale, angle: self.angle, slant: self.slant }
    }

    fn valid(self) -> bool {
        let finite = [self.angle, self.slant.0, self.slant.1].iter().all(|v| v.is_finite());
        let scale = |v: f64| in_range(v) && v != 0.0;
        finite && in_range(self.pos.0) && in_range(self.pos.1) && scale(self.scale.0) && scale(self.scale.1)
    }
}

/// `magnitude` with the sign of `like`, so a mirrored image stays mirrored.
fn signed(magnitude: f64, like: f64) -> f64 {
    if like < 0.0 { -magnitude } else { magnitude }
}

// ── description ─────────────────────────────────────────────────────────────────────────────────

fn describe(s: &Session, image: &Image) -> Value {
    let found = probe(s.assets().as_ref(), image.path.as_deref());
    let mut m = Map::new();
    m.insert("path".into(), json!(image.path));
    m.insert("resolved".into(), json!(found.resolved));
    if let Some(file) = image.path.as_deref().and_then(|p| s.asset_file(p)) {
        m.insert("file".into(), json!(file.to_string_lossy()));
    }
    m.insert("format".into(), json!(format_name(found.format)));
    if let Some((w, h)) = found.pixels {
        m.insert("pixelWidth".into(), json!(w));
        m.insert("pixelHeight".into(), json!(h));
    }
    match Parts::of(&image.transform) {
        Some(parts) => {
            m.insert("x".into(), json!(parts.pos.0));
            m.insert("y".into(), json!(parts.pos.1));
            if let Some((w, h)) = found.pixels {
                m.insert("width".into(), json!(f64::from(w) * parts.scale.0.abs()));
                m.insert("height".into(), json!(f64::from(h) * parts.scale.1.abs()));
            }
            m.insert("rotation".into(), json!(parts.angle));
            m.insert("scale".into(), json!([parts.scale.0, parts.scale.1]));
            m.insert("slant".into(), json!([parts.slant.0, parts.slant.1]));
        }
        None => {
            let a = image.transform.affine().as_coeffs();
            let [_, _, _, _, x, y] = a;
            m.insert("x".into(), json!(x));
            m.insert("y".into(), json!(y));
            m.insert("matrix".into(), json!(a));
        }
    }
    m.insert("crop".into(), json!(image.crop));
    m.insert("locked".into(), json!(image.locked));
    if let Some(alpha) = image.alpha {
        m.insert("alpha".into(), json!(alpha));
    }
    if let Some(problem) = found.problem {
        m.insert("problem".into(), json!(problem));
    }
    Value::Object(m)
}

/// The result of an editing command: the layer and the image after the edit.
fn edited(s: &Session, p: &Params, t: &Targets) -> Result<Value> {
    let image = current(s, p, t)?;
    Ok(json!({ "glyph": t.glyph, "layer": t.layer, "image": describe(s, image) }))
}

// ── commands ────────────────────────────────────────────────────────────────────────────────────

/// One file placed on one glyph.
struct Placement {
    glyph: String,
    file: String,
    image: Image,
    format: ImageFormat,
    pixels: Option<(u32, u32)>,
}

/// The file name without its folder and last extension (`Dje-cy.jpeg` → `Dje-cy`).
fn glyph_name_of(file: &str) -> Option<&str> {
    let name = file.rsplit(['/', '\\']).next()?;
    let stem = match name.rsplit_once('.') {
        Some((stem, _)) if !stem.is_empty() => stem,
        _ => name,
    };
    (!stem.is_empty()).then_some(stem)
}

fn listed(files: &[&str]) -> String {
    let mut out = files.iter().take(MAX_LISTED).map(|f| format!("`{f}`")).collect::<Vec<_>>().join(", ");
    if files.len() > MAX_LISTED {
        out.push_str(&format!(" and {} more", files.len() - MAX_LISTED));
    }
    out
}

fn add_image(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("glyph.addImage", p)?;
    let files = p.strings("files")?;
    if files.is_empty() {
        return Err(p.bad("`files` must name at least one file"));
    }
    if files.len() > MAX_ADD_IMAGE_FILES {
        return Err(p.bad(format!("`files` names {} files; at most {MAX_ADD_IMAGE_FILES} are placed at once", files.len())));
    }
    let side = match p.opt_str("side")? {
        None => LayerSide::Foreground,
        Some(v) => LayerSide::parse(v).ok_or_else(|| p.bad(format!("`side` must be \"foreground\" or \"background\", got \"{v}\"")))?,
    };
    let font = s.font()?;
    let layer = match p.opt_str("layer")? {
        Some(id) => id.to_string(),
        None => font.default_master()?.id.clone(),
    };

    // Which file goes on which glyph.
    let mut pairs: Vec<(String, &str)> = Vec::new();
    let mut unmatched: Vec<&str> = Vec::new();
    match p.opt_glyphs("glyphs", font)? {
        Some(glyphs) => {
            let [file] = files.as_slice() else {
                return Err(p.bad("with `glyphs`, give one file; without `glyphs`, several files are matched to glyphs by name"));
            };
            pairs.extend(glyphs.into_iter().map(|g| (g, *file)));
        }
        None => {
            let mut by_glyph: HashMap<&str, &str> = HashMap::with_capacity(files.len());
            for &file in &files {
                match glyph_name_of(file).filter(|name| font.glyph(name).is_some()) {
                    Some(name) => {
                        if let Some(first) = by_glyph.insert(name, file) {
                            return Err(p.bad(format!("`{first}` and `{file}` both name glyph `{name}`: give one file per glyph")));
                        }
                        pairs.push((name.to_string(), file));
                    }
                    None => unmatched.push(file),
                }
            }
            if pairs.is_empty() {
                return Err(p.bad(format!("no file is named after a glyph of the font: {}", listed(&unmatched))));
            }
        }
    }

    // Read every file before anything changes, so a bad file refuses the whole call. Each file
    // is read once, however many glyphs it goes on.
    let store = s.assets();
    let mut read: HashMap<&str, ReadImage> = HashMap::new();
    let mut placements = Vec::with_capacity(pairs.len());
    for (glyph, file) in pairs {
        let (image, format, pixels) = match read.get(file) {
            Some(known) => known.clone(),
            None => {
                let known = read_image(s, store.as_ref(), &p, file)?;
                read.insert(file, known.clone());
                known
            }
        };
        placements.push(Placement { glyph, file: file.to_string(), image, format, pixels });
    }

    let replaced = s.edit("Add Image", |font| {
        let mut replaced = Vec::with_capacity(placements.len());
        for place in &placements {
            let mut t = Targets::new("glyph.addImage", place.glyph.clone(), Some(layer.clone()));
            t.side = side;
            let had = t.edit_layer(font, |l| {
                let slot = image_slot(l, side);
                if slot.as_ref().is_some_and(|i| i.locked) {
                    return Err(locked(&p, &t));
                }
                Ok(slot.replace(place.image.clone()).is_some())
            })?;
            replaced.push(had);
        }
        Ok(replaced)
    })?;

    let placed: Vec<Value> = placements
        .iter()
        .zip(replaced)
        .map(|(place, replaced)| {
            let scale = Parts::of(&place.image.transform).map_or([1.0, 1.0], |q| [q.scale.0, q.scale.1]);
            json!({
                "glyph": place.glyph, "layer": layer, "file": place.file, "path": place.image.path,
                "format": format_name(place.format), "pixelWidth": place.pixels.map(|p| p.0), "pixelHeight": place.pixels.map(|p| p.1),
                "scale": scale, "replaced": replaced,
            })
        })
        .collect();
    Ok(json!({ "placed": placed, "unmatched": unmatched }))
}

/// An image as Add Image places it, its format and its pixel size when known.
type ReadImage = (Image, ImageFormat, Option<(u32, u32)>);

/// The image Add Image places for `file`, its format and pixel size. Only the file's start is
/// read ([`HEADER_BYTES`]) unless the header lies beyond it, as a TIFF's may.
fn read_image(s: &Session, store: &dyn AssetStore, p: &Params, file: &str) -> Result<ReadImage> {
    let stored = s.stored_image_path(file).ok_or_else(|| p.bad(format!("`{file}` is not a usable file path")))?;
    let mut bytes = store.read_head(&stored, HEADER_BYTES).map_err(|e| p.bad(e.to_string()))?;
    let cut = bytes.len() >= HEADER_BYTES;
    let mut info = imagery::sniff(&bytes);
    let tiff_without_resolution = matches!(&info, Ok(i) if i.format == ImageFormat::Tiff) && imagery::resolution(&bytes).is_none();
    if cut && (info.is_err() || tiff_without_resolution) {
        bytes = store.read(&stored).map_err(|e| p.bad(e.to_string()))?;
        info = imagery::sniff(&bytes);
    }
    let info = info.map_err(|e| p.bad(format!("`{file}`: {e}")))?;
    if info.format == ImageFormat::Other {
        return Err(p.bad(format!("`{file}` is not a PNG, JPEG, TIFF, PDF, PSD or SVG file")));
    }
    let (ppi_x, ppi_y) = imagery::resolution(&bytes).unwrap_or((DEFAULT_PPI, DEFAULT_PPI));
    let parts = Parts { pos: (0.0, 0.0), scale: (POINTS_PER_INCH / ppi_x, POINTS_PER_INCH / ppi_y), angle: 0.0, slant: (0.0, 0.0) };
    let image = Image { path: Some(stored), transform: parts.transform(), crop: None, alpha: None, locked: false, extra: Extra::new() };
    let pixels = (info.width > 0 && info.height > 0).then_some((info.width, info.height));
    Ok((image, info.format, pixels))
}

fn set_transform(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("image.setTransform", p)?;
    let t = layer_targets(s, &p)?;
    let coordinate = |key: &str| -> Result<Option<f64>> {
        match p.opt_f64(key)? {
            Some(v) if !in_range(v) => Err(p.bad(format!("`{key}` must be within ±{MAX_COORDINATE}, got {v}"))),
            other => Ok(other),
        }
    };
    let size = |key: &str| -> Result<Option<f64>> {
        match p.opt_f64(key)? {
            Some(v) if v <= 0.0 || v > MAX_COORDINATE => Err(p.bad(format!("`{key}` must be more than 0 and at most {MAX_COORDINATE}, got {v}"))),
            other => Ok(other),
        }
    };
    let (x, y) = (coordinate("x")?, coordinate("y")?);
    let (width, height) = (size("width")?, size("height")?);
    // `+ 0.0` turns -0 into 0.
    let rotation = p.opt_f64("rotation")?.map(|r| r.rem_euclid(360.0) + 0.0);
    if x.is_none() && y.is_none() && width.is_none() && height.is_none() && rotation.is_none() {
        return Err(p.bad("give at least one of `x`, `y`, `width`, `height` or `rotation`"));
    }
    let image = current(s, &p, &t)?;
    if image.locked {
        return Err(locked(&p, &t));
    }
    let pixels = if width.is_some() || height.is_some() { Some(pixel_size(s, &p, image, "setting the width or height")?) } else { None };
    let only_moves = width.is_none() && height.is_none() && rotation.is_none();
    edit_image(s, &p, &t, "Set Image Transform", false, |image| {
        // A matrix with shear can still be moved.
        if let (true, Transform::Matrix(m)) = (only_moves, &mut image.transform) {
            if let Some(x) = x
                && let Some(dx) = m.get_mut(4)
            {
                *dx = x;
            }
            if let Some(y) = y
                && let Some(dy) = m.get_mut(5)
            {
                *dy = y;
            }
            return Ok(());
        }
        let mut parts = Parts::of(&image.transform).ok_or_else(|| p.bad("the image is sheared: only `x` and `y` can be set"))?;
        if let Some(x) = x {
            parts.pos.0 = x;
        }
        if let Some(y) = y {
            parts.pos.1 = y;
        }
        if let (Some(w), Some((pw, _))) = (width, pixels) {
            parts.scale.0 = signed(w / pw, parts.scale.0);
        }
        if let (Some(h), Some((_, ph))) = (height, pixels) {
            parts.scale.1 = signed(h / ph, parts.scale.1);
        }
        if let Some(r) = rotation {
            parts.angle = r;
        }
        if !parts.valid() {
            return Err(p.bad(format!("the image would be scaled to 0 or beyond ±{MAX_COORDINATE}")));
        }
        image.transform = parts.transform();
        Ok(())
    })?;
    edited(s, &p, &t)
}

fn set_locked(s: &mut Session, p: &Value, id: &'static str, label: &str, lock: bool) -> Result<Value> {
    let p = Params::new(id, p)?;
    let t = layer_targets(s, &p)?;
    edit_image(s, &p, &t, label, true, |image| {
        image.locked = lock;
        Ok(())
    })?;
    Ok(json!({ "glyph": t.glyph, "layer": t.layer, "locked": lock }))
}

fn lock(s: &mut Session, p: &Value) -> Result<Value> {
    set_locked(s, p, "image.lock", "Lock Image", true)
}

fn unlock(s: &mut Session, p: &Value) -> Result<Value> {
    set_locked(s, p, "image.unlock", "Unlock Image", false)
}

fn set_crop(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("image.setCrop", p)?;
    let t = layer_targets(s, &p)?;
    let rect = p.array("rect")?;
    let numbers: Vec<f64> = rect.iter().filter_map(Value::as_f64).filter(|v| v.is_finite()).collect();
    let ([x, y, w, h], 4) = (numbers.as_slice(), rect.len()) else {
        return Err(p.bad("`rect` must be [x, y, width, height], four finite numbers"));
    };
    let (x, y, w, h) = (*x, *y, *w, *h);
    if x < 0.0 || y < 0.0 || w <= 0.0 || h <= 0.0 || x + w > MAX_COORDINATE || y + h > MAX_COORDINATE {
        return Err(p.bad(format!("`rect` must start at 0 or more and have a width and height above 0, within {MAX_COORDINATE} pixels")));
    }
    let image = current(s, &p, &t)?;
    if image.locked {
        return Err(locked(&p, &t));
    }
    // A file that cannot be read keeps the rectangle as given.
    if let Some((pw, ph)) = probe(s.assets().as_ref(), image.path.as_deref()).pixels {
        let (pw, ph) = (f64::from(pw), f64::from(ph));
        if x + w > pw || y + h > ph {
            return Err(p.bad(format!("`rect` reaches beyond the image, which is {pw} × {ph} pixels")));
        }
    }
    edit_image(s, &p, &t, "Crop Image", false, |image| {
        image.crop = Some([x, y, w, h]);
        Ok(())
    })?;
    edited(s, &p, &t)
}

/// The layer bounds, from 0 to the advance width and from the master's descender to its
/// ascender, in the image's pixel space and within the image.
fn layer_bounds_crop(font: &Font, layer: &Layer, image: &Image, (pw, ph): (f64, f64)) -> Option<[f64; 4]> {
    let master = font.master(&layer.master).ok()?;
    let upm = f64::from(font.units_per_em);
    let ascender = font.metric_pos(master, metric::ASCENDER, upm * 0.8);
    let descender = font.metric_pos(master, metric::DESCENDER, -upm * 0.2);
    let affine = image.transform.affine();
    let det = affine.determinant();
    if det.is_nan() || det.abs() <= f64::EPSILON {
        return None;
    }
    let to_pixels = affine.inverse();
    let [a, b, c, d] =
        [(0.0, descender), (layer.width, descender), (layer.width, ascender), (0.0, ascender)].map(|(x, y)| to_pixels * Point::new(x, y));
    let bounds = Rect::from_points(a, c).union_pt(b).union_pt(d);
    let crop = bounds.intersect(Rect::new(0.0, 0.0, pw, ph));
    let values = [crop.x0, crop.y0, crop.width(), crop.height()];
    (crop.width() > 0.0 && crop.height() > 0.0 && values.iter().all(|v| v.is_finite())).then_some(values)
}

fn set_crop_to_layer_bounds(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("image.setCropToLayerBounds", p)?;
    let t = layer_targets(s, &p)?;
    let image = current(s, &p, &t)?;
    if image.locked {
        return Err(locked(&p, &t));
    }
    let pixels = pixel_size(s, &p, image, "cropping to the layer bounds")?;
    let font = s.font()?;
    let layer = t.resolve(font)?.layer;
    let crop = layer_bounds_crop(font, layer, image, pixels).ok_or_else(|| p.bad("the image lies outside the layer bounds"))?;
    edit_image(s, &p, &t, "Set Crop to Layer Bounds", false, |image| {
        image.crop = Some(crop);
        Ok(())
    })?;
    edited(s, &p, &t)
}

fn reset_crop(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("image.resetCrop", p)?;
    let t = layer_targets(s, &p)?;
    edit_image(s, &p, &t, "Reset Crop", false, |image| {
        image.crop = None;
        Ok(())
    })?;
    edited(s, &p, &t)
}

fn get_info(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("image.getInfo", p)?;
    let t = layer_targets(s, &p)?;
    let r = t.resolve(s.font()?)?;
    let image = image_of(r.layer, t.side).map(|i| describe(s, i));
    Ok(json!({ "glyph": t.glyph, "layer": t.layer, "side": t.side.as_str(), "image": image }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_give_glyph_names() {
        assert_eq!(glyph_name_of("/scans/Dje-cy.jpeg"), Some("Dje-cy"));
        assert_eq!(glyph_name_of("A.alt.png"), Some("A.alt"));
        assert_eq!(glyph_name_of("C:\\scans\\a.PNG"), Some("a"));
        assert_eq!(glyph_name_of("noext"), Some("noext"));
        assert_eq!(glyph_name_of(".png"), Some(".png"));
        assert_eq!(glyph_name_of("folder/"), None);
    }

    #[test]
    fn matrices_decompose_into_rotation_and_scale() {
        let parts = Parts { pos: (10.0, -5.0), scale: (0.5, -2.0), angle: 30.0, slant: (0.0, 0.0) };
        let m = parts.transform().affine().as_coeffs();
        let back = Parts::of(&Transform::Matrix(m)).unwrap();
        assert!((back.angle - 30.0).abs() < 1e-9 && (back.scale.0 - 0.5).abs() < 1e-9 && (back.scale.1 + 2.0).abs() < 1e-9);
        assert_eq!(back.pos, (10.0, -5.0));
        // Shear has no such form.
        assert!(Parts::of(&Transform::Matrix([1.0, 0.0, 0.5, 1.0, 0.0, 0.0])).is_none());
    }
}
