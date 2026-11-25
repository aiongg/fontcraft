//! The line `preview.render` draws: its glyphs shaped with the features asked for, laid out as Edit View lays text out,
//! at a master or an instance, in one colour or with each glyph's colour drawing.

use std::collections::HashMap;
use std::sync::Arc;

use base64::Engine as _;
use fontcraft_fontinfo::target;
use fontcraft_geom::{Affine, BezPath};
use fontcraft_model::{Font, Layer};
use fontcraft_render::color::artwork::{self, Item as Art, Leaf};
use fontcraft_render::{Image, MAX_PIXELS, RenderError, Style, render_path};
use fontcraft_textlayout::shaping::Tag;
use fontcraft_textlayout::{Direction, Item, Layout, Options, PreviewShaper, Shaping, Text, bidi, tag};
use serde_json::{Value, json};

use super::preview::{MAX_RENDER_GLYPHS, preview_range};
use super::stroke::{OpenCorners, layer_outline};
use super::*;
use crate::color_drawing::{ColorArt, ColorFiles, Content, Mode, PlacedColor, Sources, cell_color, cell_mode, raster};
use crate::interp_cache::view_master_id;
use crate::params::Params;
use crate::prelude::*;
use crate::preview_shapers::{PreviewPlace, shaper};

/// Most features one line is shaped with.
pub const MAX_FEATURES: usize = 256;
/// Image pixels per pixel of an SVG drawing, for the colour glyphs an SVG path cannot hold.
const SVG_IMAGE_SCALE: f64 = 2.0;
/// Most bytes of PNG an SVG drawing embeds; glyphs past it are left out with a warning.
const MAX_SVG_IMAGE_BYTES: usize = 8 << 20;

/// What the line is drawn in: a master, or the static instance at an index.
#[derive(Clone, Debug, PartialEq)]
pub(super) enum Place {
    Master(String),
    Instance(usize),
}

/// A line ready to draw, in font units: the baseline at y = 0, from x = 0 to `length`.
pub(super) struct Line {
    /// The glyphs drawn, in text order, as shaped.
    pub glyphs: Vec<String>,
    /// The glyphs drawn in the text colour.
    pub ink: BezPath,
    pub colors: Vec<PlacedColor>,
    pub length: f64,
    /// The strip's lower and upper edge.
    pub range: (f64, f64),
    pub direction: Direction,
    /// The master drawn (an instance's is its view's), and the reply's `instance`.
    pub master: String,
    pub label: Value,
}

/// How the line is shaped.
struct Features {
    active: Vec<Tag>,
    localized: Option<(Tag, Tag)>,
}

fn tag_of(p: &Params, key: &str, text: &str) -> Result<Tag> {
    tag(text).ok_or_else(|| {
        p.bad(format!("`{key}`: a tag is one to four printable ASCII characters, got \"{}\"", text.chars().take(16).collect::<String>()))
    })
}

/// `features`, `script` and `language`; `None` without any: the line is not shaped.
fn features(p: &Params) -> Result<Option<Features>> {
    let asked = p.opt_strings("features")?.unwrap_or_default();
    if asked.len() > MAX_FEATURES {
        return Err(p.bad(format!("`features` holds at most {MAX_FEATURES} tags")));
    }
    let active = asked.into_iter().map(|t| tag_of(p, "features", t)).collect::<Result<Vec<_>>>()?;
    let script = p.opt_str("script")?.map(|t| tag_of(p, "script", t)).transpose()?;
    let language = p.opt_str("language")?.map(|t| tag_of(p, "language", t)).transpose()?;
    let localized = (script.is_some() || language.is_some()).then(|| (script.unwrap_or(Tag::new(b"DFLT")), language.unwrap_or(Tag::new(b"dflt"))));
    Ok((!active.is_empty() || localized.is_some()).then_some(Features { active, localized }))
}

/// The items `glyphs` and then `text` name. A character no glyph has is skipped with a warning.
fn items(p: &Params, font: &Font, warnings: &mut Vec<String>) -> Result<Vec<Item>> {
    let too_many = || p.bad(format!("at most {MAX_RENDER_GLYPHS} glyphs can be drawn"));
    let names = p.opt_strings("glyphs")?.unwrap_or_default();
    let text = p.opt_str("text")?.unwrap_or_default();
    if names.len() > MAX_RENDER_GLYPHS || text.chars().count().saturating_add(names.len()) > MAX_RENDER_GLYPHS {
        return Err(too_many());
    }
    let mut out = Vec::new();
    for name in names {
        if font.glyph(name).is_none() {
            return Err(p.bad(format!("there is no glyph `{name}`")));
        }
        out.push(Item::glyph(name));
    }
    if !text.is_empty() {
        let mut map: HashMap<u32, &str> = HashMap::new();
        for g in font.glyphs.iter() {
            for u in &g.unicodes {
                map.entry(*u).or_insert(g.name.as_str());
            }
        }
        for ch in text.chars().filter(|c| !c.is_control()) {
            match map.get(&u32::from(ch)) {
                Some(name) => out.push(Item::Glyph { name: Arc::from(*name), typed: Some(ch) }),
                None => warnings.push(format!("no glyph has the character U+{:04X}", u32::from(ch))),
            }
        }
    }
    if out.is_empty() {
        return Err(p.bad("give `glyphs` or `text` with something to draw"));
    }
    Ok(out)
}

/// `direction`, else the direction of the first letter of the line.
fn direction(p: &Params, font: &Font, items: &[Item]) -> Result<Direction> {
    match p.opt_str("direction")? {
        Some("ltr") => return Ok(Direction::Ltr),
        Some("rtl") => return Ok(Direction::Rtl),
        Some(other) => return Err(p.bad(format!("`direction` must be 'ltr' or 'rtl', got '{other}'"))),
        None => {}
    }
    let letter = items
        .iter()
        .filter_map(|i| match i {
            Item::Glyph { name, typed } => typed.or_else(|| font.glyph(name)?.unicodes.iter().find_map(|u| char::from_u32(*u))),
            _ => None,
        })
        .find(|c| bidi::is_strong_rtl(*c) || c.is_alphabetic());
    Ok(if letter.is_some_and(bidi::is_strong_rtl) { Direction::Rtl } else { Direction::Ltr })
}

fn place(p: &Params, font: &Font) -> Result<Place> {
    match p.get("instance") {
        None => {}
        Some(Value::String(x)) if x == "master" => {}
        Some(raw) => {
            if p.get("master").is_some() {
                return Err(p.bad("give `instance` or `master`, not both"));
            }
            let r = target::parse(raw).map_err(|e| p.bad(e.to_string()))?;
            return Ok(Place::Instance(target::instance(font, &r).map_err(|e| p.bad(e.to_string()))?));
        }
    }
    let master = match p.opt_str("master")? {
        Some(m) => font
            .masters
            .iter()
            .find(|x| x.id == m)
            .or_else(|| font.masters.iter().find(|x| x.name == m))
            .ok_or_else(|| p.bad(format!("no master has the id or name `{m}`")))?,
        None => font.masters.first().ok_or_else(|| p.bad("the font has no master"))?,
    };
    Ok(Place::Master(master.id.clone()))
}

/// Text holding `items`.
fn text_of(p: &Params, items: Vec<Item>) -> Result<Text> {
    let mut text = Text::new();
    text.replace(0..0, items).map_err(|e| p.bad(e.to_string()))?;
    Ok(text)
}

/// The glyphs `layout` draws, in text order.
fn drawn(layout: &Layout) -> impl Iterator<Item = (&Arc<str>, f64, f64, usize)> {
    layout.placed.iter().enumerate().flat_map(|(i, p)| {
        let first = p.glyph.as_ref().map(|g| (g, p.x, p.y, i));
        first.into_iter().chain(p.extra.iter().map(move |(g, x, y)| (g, *x, *y, i)))
    })
}

fn joined(parts: Vec<BezPath>) -> BezPath {
    parts.into_iter().fold(BezPath::new(), |mut all, part| {
        all.extend(part.elements().iter().copied());
        all
    })
}

/// `layer` of `name` as it is filled: strokes expanded, masks applied, open corners closed.
fn filled(font: &Font, corners: &OpenCorners, master: &str, name: &str, layer: &Layer) -> BezPath {
    let outline = layer_outline(font, layer).map(joined).unwrap_or_else(|_| font.outline(name, master).unwrap_or_default());
    corners.filled(master, name, &outline).unwrap_or(outline)
}

/// The line the parameters ask for. `invert` draws Master Colors for a dark ground.
pub(super) fn line(s: &mut Session, p: &Params, invert: bool, warnings: &mut Vec<String>) -> Result<Line> {
    let font = Arc::clone(&s.doc()?.font);
    let mut items = items(p, &font, warnings)?;
    let place = place(p, &font)?;
    let color = p.bool_or("color", false)?;
    let direction = direction(p, &font, &items)?;
    let asked = features(p)?;
    let first = font.masters.first().map(|m| m.id.clone()).ok_or_else(|| p.bad("the font has no master"))?;

    let shaped_at = match &place {
        Place::Master(m) => PreviewPlace::Master(m.clone()),
        Place::Instance(i) => PreviewPlace::Instance(*i),
    };
    let mut shaper_of: Option<Arc<PreviewShaper>> = None;
    if let Some(f) = &asked {
        match shaper(s, &shaped_at)? {
            Ok(made) => {
                let missing: Vec<String> =
                    f.active.iter().filter(|t| !made.features().contains(t)).map(|t| t.to_string().trim_end().to_string()).collect();
                if !missing.is_empty() {
                    warnings.push(format!(
                        "the font has no substitution feature {}",
                        missing.iter().map(|t| format!("`{t}`")).collect::<Vec<_>>().join(", ")
                    ));
                }
                shaper_of = Some(made);
            }
            Err(why) => warnings.push(format!("the features are not applied: {why}")),
        }
    }
    let shaping = match (&shaper_of, &asked) {
        (Some(shaper), Some(f)) => Some(Shaping { shaper, active: &f.active, localized: f.localized }),
        _ => None,
    };

    // The glyphs the instance is interpolated for: the text's own and the ones they shape to.
    let wanted = |items: &[Item]| -> Result<Vec<String>> {
        let text = text_of(p, items.to_vec())?;
        let plain =
            Options { direction, kerning: fontcraft_textlayout::KerningMode::None, single_line: true, shaping, cursive: true, ..Options::default() };
        let layout = fontcraft_textlayout::layout(&font, &first, &text, &plain).map_err(|e| p.bad(e.to_string()))?;
        let mut seen = std::collections::HashSet::new();
        let typed = items.iter().filter_map(|i| if let Item::Glyph { name, .. } = i { Some(name) } else { None });
        Ok(typed.chain(drawn(&layout).map(|(name, ..)| name)).filter(|n| seen.insert((*n).clone())).map(|n| n.to_string()).collect())
    };
    let (view, master, label) = match &place {
        Place::Master(m) => (Arc::clone(&font), m.clone(), json!("master")),
        Place::Instance(i) => {
            let names = wanted(&items)?;
            let view = if color { s.instance_color_view(*i, &names) } else { s.instance_view(*i, &names) }.map_err(|e| p.bad(e))?;
            (view, view_master_id(*i), json!(font.instances.get(*i).map(target::instance_name).unwrap_or_default()))
        }
    };
    items.retain(|item| {
        let Item::Glyph { name, .. } = item else { return true };
        let has = view.glyph(name).and_then(|g| g.layer(&master)).is_some();
        if !has {
            warnings.push(match place {
                Place::Instance(_) => format!("`{name}` cannot be interpolated at the instance and is left out"),
                Place::Master(_) => format!("`{name}` has no layer in the master and is left out"),
            });
        }
        has
    });
    let index = match place {
        Place::Master(_) => Some(s.kern_index()?),
        Place::Instance(_) => None,
    };
    let text = text_of(p, items)?;
    let options = Options { direction, single_line: true, index: index.as_deref(), shaping, cursive: true, ..Options::default() };
    let layout = fontcraft_textlayout::layout(&view, &master, &text, &options).map_err(|e| p.bad(e.to_string()))?;
    if let Some(error) = &layout.shaping_error {
        warnings.push(error.clone());
    }
    let length = layout.lines.first().map_or(0.0, |l| l.length);
    let length = if length.is_finite() { length.max(0.0) } else { 0.0 };
    let shift = if direction == Direction::Rtl { length } else { 0.0 };

    // A layered font draws its own masters whatever the instance; colour layers are the view's.
    let mode = color.then(|| {
        match cell_mode(
            &font,
            match &place {
                Place::Master(m) => m,
                Place::Instance(_) => &first,
            },
            invert,
        ) {
            layered @ Mode::Layered { .. } => (Arc::clone(&font), layered),
            Mode::Layers { kind, .. } => (Arc::clone(&view), Mode::Layers { master: master.clone(), kind }),
        }
    });
    let assets = s.assets();
    let mut files = ColorFiles::default();
    let text_color = if invert { [1.0; 4] } else { crate::color_drawing::TEXT };
    let corners = OpenCorners::of(&view);
    let mut found: HashMap<&str, (Option<Arc<ColorArt>>, BezPath)> = HashMap::new();
    let mut out = Line {
        glyphs: Vec::new(),
        ink: BezPath::new(),
        colors: Vec::new(),
        length,
        range: preview_range(&view, view.master(&master).map_err(|e| p.bad(e.to_string()))?),
        direction,
        master: master.clone(),
        label,
    };
    for (name, x, y, _) in drawn(&layout) {
        let Some(glyph) = view.glyph(name) else { continue };
        let Some(layer) = glyph.layer(&master) else { continue };
        if out.glyphs.len() >= MAX_RENDER_GLYPHS {
            warnings.push(format!("only the first {MAX_RENDER_GLYPHS} glyphs are drawn"));
            break;
        }
        out.glyphs.push(name.to_string());
        let (art, outline) = found.entry(&**name).or_insert_with(|| {
            let art = mode.as_ref().and_then(|(source, mode)| {
                let mut src = Sources { assets: assets.as_ref(), files: &mut files, text: text_color };
                cell_color(source, source.glyph(name)?, mode, &mut src)
            });
            let outline = if art.is_some() { BezPath::new() } else { filled(&view, &corners, &master, name, layer) };
            (art, outline)
        });
        match art {
            Some(art) => out.colors.push(PlacedColor { at: [x + shift, y], art: Arc::clone(art) }),
            None => {
                let mut moved = outline.clone();
                moved.apply_affine(Affine::translate((x + shift, y)));
                out.ink.extend(moved.elements().iter().copied());
            }
        }
    }
    Ok(out)
}

/// A line as pixels: `ink` filled on the ground and each colour drawing laid over it. `to_pixels` maps the line's font
/// units to pixels. A colour drawing past its work budget is filled in the ink.
pub fn line_picture(
    ink: &BezPath,
    colors: &[PlacedColor],
    to_pixels: Affine,
    width: u32,
    height: u32,
    style: &Style,
) -> std::result::Result<Image, RenderError> {
    let mut image = render_path(ink, to_pixels, width, height, style)?;
    raster::draw_over(&mut image, colors, to_pixels, style.ink.map(|c| f32::from(c) / 255.0));
    Ok(image)
}

fn hex(c: [f32; 4]) -> String {
    let [r, g, b, _] = c.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8);
    format!("#{r:02x}{g:02x}{b:02x}")
}

/// The colour glyphs of a line as SVG elements: flat fills as paths, every other drawing as an image.
pub(super) fn svg_colors(colors: &[PlacedColor], to_pixels: Affine, width: u32, height: u32, ink: [f32; 4], warnings: &mut Vec<String>) -> String {
    let mut out = String::new();
    let mut bytes = 0usize;
    let mut left_out = 0usize;
    for placed in colors {
        let place = to_pixels * Affine::translate((placed.at[0], placed.at[1]));
        if let Content::Items(items) = &placed.art.content
            && let Some(paths) = flat(items, place)
        {
            out.push_str(&paths);
            continue;
        }
        let Some(image) = svg_image(placed, to_pixels, width, height, ink) else { continue };
        if bytes.saturating_add(image.len()) > MAX_SVG_IMAGE_BYTES {
            left_out += 1;
            continue;
        }
        bytes += image.len();
        out.push_str(&image);
    }
    if left_out > 0 {
        warnings.push(format!(
            "{left_out} colour glyphs are left out of the SVG: their images are too large together; draw fewer glyphs, a smaller size or a PNG"
        ));
    }
    out
}

/// Items that are all one outline in one colour, as SVG paths.
fn flat(items: &[Art], place: Affine) -> Option<String> {
    let mut out = String::new();
    for item in items {
        let Art::Fill(d) = item else { return None };
        let ([clip], Leaf::Solid(c)) = (d.clips.as_slice(), &d.leaf) else { return None };
        let [.., alpha] = *c;
        out.push_str(&format!("<path d=\"{}\" fill=\"{}\" fill-opacity=\"{}\"/>", (place * clip.clone()).to_svg(), hex(*c), alpha.clamp(0.0, 1.0)));
    }
    Some(out)
}

/// One colour drawing as an embedded PNG where it falls on the picture; `None` outside it.
fn svg_image(placed: &PlacedColor, to_pixels: Affine, width: u32, height: u32, ink: [f32; 4]) -> Option<String> {
    let place = to_pixels * Affine::translate((placed.at[0], placed.at[1]));
    let b = match &placed.art.content {
        Content::Items(items) => place.transform_rect_bbox(artwork::bounds(items)?),
        Content::Bitmap { bitmap, place: own } => {
            (place * *own).transform_rect_bbox(fontcraft_geom::Rect::new(0.0, 0.0, f64::from(bitmap.image.width), f64::from(bitmap.image.height)))
        }
    };
    let (x0, y0) = (b.x0.max(0.0).floor(), b.y0.max(0.0).floor());
    let (x1, y1) = (b.x1.min(f64::from(width)).ceil(), b.y1.min(f64::from(height)).ceil());
    let (w, h) = ((x1 - x0) * SVG_IMAGE_SCALE, (y1 - y0) * SVG_IMAGE_SCALE);
    if !(w >= 1.0 && h >= 1.0 && w * h <= MAX_PIXELS as f64) {
        return None;
    }
    let (pw, ph) = (w as u32, h as u32);
    let mut image = Image::from_rgba_premultiplied(pw, ph, vec![0; (pw as usize).checked_mul(ph as usize)?.checked_mul(4)?]).ok()?;
    let to_image = Affine::scale(SVG_IMAGE_SCALE) * Affine::translate((-x0, -y0)) * to_pixels;
    raster::draw_over(&mut image, std::slice::from_ref(placed), to_image, ink);
    let png = base64::engine::general_purpose::STANDARD.encode(image.png().ok()?);
    Some(format!("<image x=\"{x0}\" y=\"{y0}\" width=\"{}\" height=\"{}\" href=\"data:image/png;base64,{png}\"/>", x1 - x0, y1 - y0))
}
