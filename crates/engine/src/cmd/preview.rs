//! The engine half of the preview of instances (M9.20): `preview.render` draws the preview line
//! to an image, and `preview.toggleInstanceVisibility` shows or hides an instance in the
//! preview of all instances. The Preview Area and the Preview Panel themselves are the shell's
//! (`fontcraft_ui::shell::preview`); they share the strip's edges ([`preview_range`]) and
//! the blur ([`blur_pixels`]) with this module.
//!
//! Rows: `preview.render`, `preview.toggleInstanceVisibility`, `preview.respectsInstanceData`.
//!
//! An instance is drawn from the document's instance view (`crate::interp_cache`), so it shows
//! what the static instance would hold. The line is laid out left to right with each glyph's
//! advance and the kerning of the master or instance; feature code is not applied, as in the
//! Preview Area.

use std::sync::Arc;

use base64::Engine as _;
use fontcraft_fontinfo::target;
use fontcraft_geom::{Affine, BezPath};
use fontcraft_model::plist::Value as Plist;
use fontcraft_model::{Font, Master, Params as ModelParams, metric};
use fontcraft_render::{Image, MAX_PIXELS, MAX_SIDE, Style, render_path};
use fontcraft_spacing::kerning::{Direction, KernIndex};
use serde_json::{Value, json};

use super::*;
use crate::interp_cache::view_master_id;
use crate::params::{Params, kind};
use crate::prelude::*;

/// The master parameters that set the strip's upper and lower edge.
pub const PREVIEW_ASCENDER: &str = "Preview Ascender";
pub const PREVIEW_DESCENDER: &str = "Preview Descender";
/// The upper edge without `Preview Ascender`, in font units.
pub const DEFAULT_PREVIEW_ASCENDER: f64 = 1000.0;
/// Largest absolute value read from `Preview Ascender`, `Preview Descender` and `winDescent`.
const MAX_EDGE: f64 = 65_535.0;
/// The strongest blur, in screen points (pixels for `preview.render`).
pub const MAX_BLUR: f64 = 10.0;
/// Box blur passes: three approximate a Gaussian.
pub const BLUR_PASSES: usize = 3;
/// Most glyphs `preview.render` draws.
pub const MAX_RENDER_GLYPHS: usize = 1_000;
/// The image height `preview.render` draws without `size`, and the range it allows, in pixels.
pub const DEFAULT_RENDER_HEIGHT: f64 = 100.0;
pub const MIN_RENDER_HEIGHT: f64 = 8.0;
pub const MAX_RENDER_HEIGHT: f64 = 2_048.0;
/// Empty pixels left and right of the line.
const RENDER_PADDING: f64 = 8.0;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "preview.render",
            "Render Preview",
            [],
            None,
            "{text?: string (its characters, after `glyphs`), glyphs?: [names] (one of the two is required; at most 1000 glyphs together), instance?: name|index|'master' (a static instance, interpolated as the static instance would be; default 'master'), master?: id or name (with 'master': the master drawn; default the first), features?: [tags] (accepted; feature code is not applied, as in the Preview Area), size?: pixels (the image height, 8 to 2048; default 100), invert?: bool (light on dark), flip?: bool (upside down), blur?: pixels (0 to 10), format?: 'png'|'svg' (default 'png')} draws the preview line as the Preview Area would: the glyphs on one line, left to right, with their advances and the kerning of the master or instance, scaled so the strip from `Preview Descender` to `Preview Ascender` fills the height. A glyph that cannot be interpolated at the instance is left out with a warning → {format, width, height, instance, master, glyphs: [drawn names], png? (base64), svg?, warnings}",
            has_doc,
            render
        ),
        cmd!(
            "preview.toggleInstanceVisibility",
            "Show Instance in Preview",
            [],
            None,
            "{instance: id|name|index, on?: bool (omitted: toggle)} include the instance in the preview of all instances (`preview.showAllInstances`) or leave it out; stored with the instance as its visible flag. One undo step; nothing changes when the flag already has that value → {instance, name, on}",
            has_doc,
            toggle_visibility
        ),
    ]
}

/// A master parameter as a finite number within ±[`MAX_EDGE`].
fn number_param(params: &ModelParams, name: &str) -> Option<f64> {
    let v = match params.get(name)? {
        Plist::Number(n) => n.value().as_f64(),
        Plist::String(s) => s.as_str().trim().parse::<f64>().ok()?,
        _ => return None,
    };
    (v.is_finite() && v.abs() <= MAX_EDGE).then_some(v)
}

/// The preview strip's lower and upper edge in font units for a master: `Preview Descender` (a
/// distance below the baseline, of either sign), else the master's or the font's `winDescent`
/// (the handbook names `winAscent`, read as a misprint), else the master's descender; and
/// `Preview Ascender`, else 1000. Falls back to the master's descender and ascender when the
/// edges would not enclose the baseline.
pub fn preview_range(font: &Font, master: &Master) -> (f64, f64) {
    let upm = f64::from(font.units_per_em);
    let ascender = number_param(&master.params, PREVIEW_ASCENDER).unwrap_or(DEFAULT_PREVIEW_ASCENDER);
    let descender = number_param(&master.params, PREVIEW_DESCENDER)
        .or_else(|| number_param(&master.params, "winDescent"))
        .or_else(|| number_param(&font.info.params, "winDescent"))
        .map(|d| -d.abs())
        .unwrap_or_else(|| font.metric_pos(master, metric::DESCENDER, -upm * 0.2).min(0.0));
    if ascender > 0.0 && ascender > descender {
        return (descender, ascender);
    }
    let ascender = font.metric_pos(master, metric::ASCENDER, upm * 0.8);
    let descender = font.metric_pos(master, metric::DESCENDER, -upm * 0.2);
    if ascender > descender { (descender, ascender) } else { (-upm * 0.2, upm * 0.8) }
}

/// Blurs RGBA pixels in place with [`BLUR_PASSES`] box blurs of `radius` pixels each way; the
/// edges repeat.
pub fn blur_pixels(pixels: &mut [u8], width: usize, height: usize, radius: usize) {
    if width == 0 || height == 0 || radius == 0 || pixels.len() < width.saturating_mul(height).saturating_mul(4) {
        return;
    }
    let mut line = Vec::new();
    for _ in 0..BLUR_PASSES {
        for y in 0..height {
            blur_run(pixels, y * width * 4, 4, width, radius, &mut line);
        }
        for x in 0..width {
            blur_run(pixels, x * 4, width * 4, height, radius, &mut line);
        }
    }
}

/// One box blur along `n` pixels starting at byte `start`, `stride` bytes apart.
fn blur_run(pixels: &mut [u8], start: usize, stride: usize, n: usize, radius: usize, line: &mut Vec<[u32; 4]>) {
    line.clear();
    for i in 0..n {
        let o = start + i * stride;
        let px = pixels.get(o..o + 4).unwrap_or(&[0; 4]);
        line.push([0, 1, 2, 3].map(|c| px.get(c).copied().map_or(0, u32::from)));
    }
    let last = n.saturating_sub(1);
    let at = |i: isize| line.get(i.clamp(0, last as isize) as usize).copied().unwrap_or([0; 4]);
    let r = radius as isize;
    let window = (2 * radius + 1) as u32;
    let mut sum = [0u32; 4];
    for k in -r..=r {
        let v = at(k);
        for c in 0..4 {
            if let (Some(s), Some(v)) = (sum.get_mut(c), v.get(c)) {
                *s += v;
            }
        }
    }
    for i in 0..n {
        let o = start + i * stride;
        if let Some(px) = pixels.get_mut(o..o + 4) {
            for (dst, s) in px.iter_mut().zip(sum) {
                *dst = (s / window).min(255) as u8;
            }
        }
        let (add, sub) = (at(i as isize + r + 1), at(i as isize - r));
        for c in 0..4 {
            if let (Some(s), Some(a), Some(b)) = (sum.get_mut(c), add.get(c), sub.get(c)) {
                *s = (*s + a).saturating_sub(*b);
            }
        }
    }
}

/// Largest blur [`blur_radius`] takes, in pixels: [`MAX_BLUR`] points on a very dense screen.
const MAX_BLUR_PIXELS: f64 = 16.0 * MAX_BLUR;

/// The blur radius of each box pass for a blur of `amount` pixels.
pub fn blur_radius(amount: f64) -> usize {
    let amount = if amount.is_finite() { amount.clamp(0.0, MAX_BLUR_PIXELS) } else { 0.0 };
    (amount / BLUR_PASSES as f64).ceil() as usize
}

/// Image pixels turned half a circle: the order of the pixels reversed.
pub fn turn_pixels(pixels: &[u8]) -> Vec<u8> {
    pixels.as_chunks::<4>().0.iter().rev().flatten().copied().collect()
}

enum Format {
    Png,
    Svg,
}

/// The glyphs `glyphs` and then `text` name, in order. A character no glyph has is skipped with
/// a warning.
fn glyph_names(p: &Params, font: &Font, warnings: &mut Vec<String>) -> Result<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    for name in p.opt_strings("glyphs")?.unwrap_or_default() {
        if font.glyph(name).is_none() {
            return Err(p.bad(format!("there is no glyph `{name}`")));
        }
        out.push(name.to_string());
        if out.len() > MAX_RENDER_GLYPHS {
            return Err(p.bad(format!("at most {MAX_RENDER_GLYPHS} glyphs can be drawn")));
        }
    }
    if let Some(text) = p.opt_str("text")? {
        let mut map: std::collections::HashMap<u32, &str> = std::collections::HashMap::new();
        for g in font.glyphs.iter() {
            for u in &g.unicodes {
                map.entry(*u).or_insert(g.name.as_str());
            }
        }
        for ch in text.chars().filter(|c| !c.is_control()) {
            match map.get(&u32::from(ch)) {
                Some(name) => out.push((*name).to_string()),
                None => warnings.push(format!("no glyph has the character U+{:04X}", u32::from(ch))),
            }
            if out.len() > MAX_RENDER_GLYPHS {
                return Err(p.bad(format!("at most {MAX_RENDER_GLYPHS} glyphs can be drawn")));
            }
        }
    }
    Ok(out)
}

/// The finite number `key` holds, else `default`.
fn number(p: &Params, key: &str, default: f64) -> Result<f64> {
    match p.opt_f64(key)? {
        None => Ok(default),
        Some(v) if v.is_finite() => Ok(v),
        Some(_) => Err(p.bad(format!("`{key}` must be a finite number"))),
    }
}

fn render(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("preview.render", v)?;
    let format = match p.opt_str("format")? {
        None | Some("png") => Format::Png,
        Some("svg") => Format::Svg,
        Some(other) => return Err(p.bad(format!("`format` must be 'png' or 'svg', got '{other}'"))),
    };
    let height = number(&p, "size", DEFAULT_RENDER_HEIGHT)?;
    if !(MIN_RENDER_HEIGHT..=MAX_RENDER_HEIGHT).contains(&height) {
        return Err(p.bad(format!("`size` must be from {MIN_RENDER_HEIGHT} to {MAX_RENDER_HEIGHT} pixels")));
    }
    let blur = number(&p, "blur", 0.0)?.clamp(0.0, MAX_BLUR);
    let invert = p.bool_or("invert", false)?;
    let flip = p.bool_or("flip", false)?;
    let mut warnings = Vec::new();
    if p.opt_strings("features")?.is_some_and(|f| !f.is_empty()) {
        warnings.push("feature code is not applied: the preview line draws the glyphs as given".to_string());
    }
    let doc_font: Arc<Font> = Arc::clone(&s.doc()?.font);
    let names = glyph_names(&p, &doc_font, &mut warnings)?;
    if names.is_empty() {
        return Err(p.bad("give `glyphs` or `text` with something to draw"));
    }
    let instance = match p.get("instance") {
        None => None,
        Some(Value::String(x)) if x == "master" => None,
        Some(raw) => {
            let r = target::parse(raw).map_err(|e| p.bad(e.to_string()))?;
            Some(target::instance(&doc_font, &r).map_err(|e| p.bad(e.to_string()))?)
        }
    };
    let (font, master_id, label) = match instance {
        Some(i) => {
            if p.get("master").is_some() {
                return Err(p.bad("give `instance` or `master`, not both"));
            }
            let view = s.instance_view(i, &names).map_err(|e| p.bad(e))?;
            let name = doc_font.instances.get(i).map(target::instance_name).unwrap_or_default().to_string();
            (view, view_master_id(i), json!(name))
        }
        None => {
            let master = match p.opt_str("master")? {
                Some(m) => doc_font
                    .masters
                    .iter()
                    .find(|x| x.id == m)
                    .or_else(|| doc_font.masters.iter().find(|x| x.name == m))
                    .ok_or_else(|| p.bad(format!("no master has the id or name `{m}`")))?,
                None => doc_font.masters.first().ok_or_else(|| p.bad("the font has no master"))?,
            };
            let id = master.id.clone();
            (Arc::clone(&doc_font), id, json!("master"))
        }
    };
    let index: Arc<KernIndex> = match instance {
        Some(_) => Arc::new(KernIndex::new(&font)),
        None => s.kern_index()?,
    };
    let master = font.master(&master_id).map_err(|e| p.bad(e.to_string()))?;
    let (low, high) = preview_range(&font, master);
    let scale = height / (high - low);
    if !(scale.is_finite() && scale > 0.0) {
        return Err(p.bad("the master's preview edges enclose nothing"));
    }

    let mut x = 0.0;
    let mut filled = BezPath::new();
    let mut drawn: Vec<String> = Vec::new();
    let mut previous: Option<&str> = None;
    for name in &names {
        let Some(layer) = font.glyph(name).and_then(|g| g.layer(&master_id)) else {
            warnings.push(match instance {
                Some(_) => format!("`{name}` cannot be interpolated at the instance and is left out"),
                None => format!("`{name}` has no layer in the master and is left out"),
            });
            continue;
        };
        if let Some(prev) = previous
            && let Ok(Some(k)) = index.lookup(Direction::Ltr, &master_id, prev, name)
            && k.value.is_finite()
        {
            x += k.value;
        }
        let mut outline = match stroke::layer_outline(&font, layer) {
            Ok(parts) => parts.into_iter().fold(BezPath::new(), |mut all, part| {
                all.extend(part.elements().iter().copied());
                all
            }),
            Err(_) => font.outline(name, &master_id).unwrap_or_default(),
        };
        outline.apply_affine(Affine::translate((x, 0.0)));
        filled.extend(outline.elements().iter().copied());
        x += if layer.width.is_finite() { layer.width.max(0.0) } else { 0.0 };
        previous = Some(name.as_str());
        drawn.push(name.clone());
    }

    let (w, h) = ((x.max(0.0) * scale + 2.0 * RENDER_PADDING).ceil(), height.ceil());
    if !(w.is_finite() && w <= f64::from(MAX_SIDE) && w * h <= MAX_PIXELS as f64) {
        return Err(p.bad("the line is too long to draw at this size; draw fewer glyphs or a smaller size"));
    }
    let (width, height_px) = ((w as u32).max(1), (h as u32).max(1));
    let mut to_pixels = Affine::translate((RENDER_PADDING, high * scale)) * Affine::scale_non_uniform(scale, -scale);
    if flip {
        to_pixels = Affine::translate((f64::from(width), f64::from(height_px))) * Affine::scale(-1.0) * to_pixels;
    }
    let (ink, paper) = if invert { ([255, 255, 255, 255], [0, 0, 0, 255]) } else { ([0, 0, 0, 255], [255, 255, 255, 255]) };
    let mut out = json!({
        "format": match format { Format::Png => "png", Format::Svg => "svg" },
        "width": width,
        "height": height_px,
        "instance": label,
        "master": master_id,
        "glyphs": drawn,
    });
    match format {
        Format::Png => {
            let image = render_path(&filled, to_pixels, width, height_px, &Style { ink, background: paper, anti_alias: true })
                .map_err(|e| p.bad(e.to_string()))?;
            let mut pixels = image.pixels;
            blur_pixels(&mut pixels, width as usize, height_px as usize, blur_radius(blur));
            let image = Image::from_rgba_premultiplied(width, height_px, pixels).map_err(|e| p.bad(e.to_string()))?;
            let png = image.png().map_err(|e| p.bad(e.to_string()))?;
            set(&mut out, "png", json!(base64::engine::general_purpose::STANDARD.encode(png)));
        }
        Format::Svg => {
            let mut path = filled;
            path.apply_affine(to_pixels);
            let hex = |c: [u8; 4]| format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2]);
            let (filter, use_filter) = if blur > 0.0 {
                (
                    format!("<defs><filter id=\"blur\"><feGaussianBlur stdDeviation=\"{}\"/></filter></defs>", blur / BLUR_PASSES as f64),
                    " filter=\"url(#blur)\"",
                )
            } else {
                (String::new(), "")
            };
            set(
                &mut out,
                "svg",
                json!(format!(
                    "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" height=\"{height_px}\" viewBox=\"0 0 {width} {height_px}\">{filter}<rect width=\"{width}\" height=\"{height_px}\" fill=\"{}\"/><path d=\"{}\" fill=\"{}\"{use_filter}/></svg>",
                    hex(paper),
                    path.to_svg(),
                    hex(ink)
                )),
            );
        }
    }
    set(&mut out, "warnings", json!(warnings));
    Ok(out)
}

fn set(out: &mut Value, key: &str, value: Value) {
    if let Some(m) = out.as_object_mut() {
        m.insert(key.into(), value);
    }
}

fn toggle_visibility(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("preview.toggleInstanceVisibility", v)?;
    let font = s.font()?;
    let raw = p.get("instance").ok_or_else(|| p.bad("`instance` (an id, name or index) is required"))?;
    let r = target::parse(raw).map_err(|e| p.bad(e.to_string()))?;
    let i = target::instance(font, &r).map_err(|e| p.bad(e.to_string()))?;
    let inst = font.instances.get(i).ok_or_else(|| p.bad(format!("there is no instance #{i}")))?;
    let (current, name) = (inst.visible, target::instance_name(inst).to_string());
    let on = match p.get("on") {
        None => !current,
        Some(Value::Bool(b)) => *b,
        Some(other) => return Err(p.bad(format!("`on` must be true or false, got {}", kind(other)))),
    };
    if on != current {
        s.edit("Show Instance in Preview", |font| {
            let inst = Arc::make_mut(&mut font.instances).get_mut(i).ok_or_else(|| p.bad(format!("there is no instance #{i}")))?;
            inst.visible = on;
            Ok(())
        })?;
    }
    Ok(json!({"instance": i, "name": name, "on": on}))
}
