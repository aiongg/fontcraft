//! Overlay Fonts (M13.66): stack the matching glyphs of several open documents (the parts of a
//! chromatic family) into one new colour font.
//!
//! Rows: `font.overlayFonts`, `font.overlayPreview` (`spec/inventory/90-secondary.toml`).
//!
//! `layers` lists the sources from the bottom up. Each source is one master of an open document
//! with a colour, an opacity, an offset and a scale. Glyphs match by name; the new font has every
//! glyph some source has, in the bottom source's order and then in the order of the sources that
//! add glyphs.
//!
//! The bottom source supplies the font: its font info, features, groups, glyph records, metrics
//! and kerning, cut down to one master with no axes, instances or special layers. A glyph the
//! bottom source lacks takes its record and width from the lowest source that has it, without
//! kerning groups and without code points another glyph already has.
//!
//! | `output` | What the new font holds |
//! |---|---|
//! | `colorLayers` | One master. Each glyph's master layer holds every source's outlines (the drawing shown where colour is not supported); above it, one Color Palette layer per source that draws the glyph, painting in palette row = the source's position. The font gets one palette: each source's colour, its alpha multiplied by the opacity. |
//! | `masters` | One master per source, holding that source's outlines and named after it. Every master has the bottom source's widths, vertical metrics and kerning. |
//!
//! Outlines are copied with components, smart components and corner components resolved to
//! paths, then scaled about the origin and moved by the offset. A source with a different units
//! per em is first scaled to the bottom source's. The bottom source's scale also applies to the
//! widths, vertical metrics and kerning. The new font opens as an untitled document; no source
//! changes.
//!
//! `font.overlayPreview` returns the same stacked outlines for a list of glyphs as SVG path data
//! with their colours, so a view can paint the overlay before the font is made.

use std::collections::HashSet;
use std::sync::Arc;

use fontcraft_color::layers::{self, ColorLayerKind};
use fontcraft_color::palette::{self, COLOR_PALETTES_PARAM, Change, Palettes, Rgba};
use fontcraft_fontinfo::{MAX_MASTERS, target};
use fontcraft_geom::Affine;
use fontcraft_model::{
    Anchor, Font, Glyph, GlyphTable, KernContext, KernGroups, Kerning, Layer, LayerAttrs, MAX_COORDINATE, MAX_GLYPHS, PaletteEntry, Shape,
};
use serde_json::{Value, json};

use super::*;
use crate::EngineError;
use crate::params::Params;

/// Most sources one call may stack (a font holds at most this many masters).
pub const MAX_SOURCES: usize = MAX_MASTERS;
/// Largest `scale` a source may have.
pub const MAX_SCALE: f64 = 100.0;
/// Most glyphs one `font.overlayPreview` call returns.
pub const MAX_PREVIEW_GLYPHS: usize = 1_000;
/// Most path elements one `font.overlayPreview` call returns, over all glyphs and sources.
pub const MAX_PREVIEW_ELEMENTS: usize = 2_000_000;
/// Most nodes the new font's outlines may hold, over all glyphs and layers.
pub const MAX_OUTPUT_NODES: usize = 20_000_000;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            "font.overlayFonts",
            "Overlay Fonts…",
            ["Font"],
            None,
            "{layers: [{font: index|title|path (an open document: its position, title or the path it was opened from), master?: id|name|index (default: the first master), color?: \"#RRGGBB\"|\"#RRGGBBAA\" (default: opaque black), opacity?: number 0 to 1 (default: 1), offset?: [x, y] (default: [0, 0]), scale?: number above 0, at most 100 (default: 1)}] (from the bottom up, at most 256), output: colorLayers|masters} stack the matching glyphs (by name) of the sources into a new untitled document. The bottom source supplies font info, features, glyph records, metrics and kerning; glyphs only other sources have are added. Outlines are copied with components resolved to paths, scaled about the origin (after scaling to the bottom source's units per em) and moved by the offset; the bottom source's scale also applies to widths, vertical metrics and kerning. colorLayers: one master whose master layers hold every source's outlines, plus one Color Palette layer per source that draws the glyph, painting in palette row = the source's position, and one palette of the sources' colours (alpha × opacity). masters: one master per source, named after it, each with the bottom source's widths, metrics and kerning. No source changes → {title, document: index, output, glyphs: count, masters: [{id, name, source}], palette?: [\"#RRGGBBAA\"] (colorLayers), warnings: [string]}",
            has_doc,
            overlay_fonts
        ),
        cmd!(
            query "font.overlayPreview",
            "Overlay Fonts Preview",
            [],
            None,
            "{layers: [the sources, as font.overlayFonts takes them], glyphs: [names] (1 to 1000)} the outlines font.overlayFonts would stack, for a view to paint: for each glyph, each source that has it from the bottom up, with its colour (alpha × opacity) and its outline as SVG path data in the bottom source's font units (y up), components resolved, scaled and moved. `width` is the glyph's advance as the new font would have it; null when no source has the glyph. A source whose outline cannot be drawn is left out with a warning → {unitsPerEm, glyphs: [{glyph, width, layers: [{source: index in layers, color: \"#RRGGBBAA\", path: string}]}], warnings: [string]}",
            has_doc,
            overlay_preview
        ),
    ]
}

// ---------- sources ----------

/// One parsed `layers` item.
struct Source {
    title: String,
    font: Arc<Font>,
    /// The id of the master the source draws from.
    master: String,
    /// The colour with the opacity applied to its alpha.
    color: Rgba,
    /// The scale applied to the outlines: `scale` times the bottom source's units per em over
    /// this source's.
    k: f64,
    offset: (f64, f64),
}

impl Source {
    fn affine(&self) -> Affine {
        Affine::new([self.k, 0.0, 0.0, self.k, self.offset.0, self.offset.1])
    }

    fn map(&self, x: f64, y: f64) -> (f64, f64) {
        (x * self.k + self.offset.0, y * self.k + self.offset.1)
    }

    /// The scaled advance width of `name` on the source's master; `None` when the source lacks
    /// the glyph or its master layer.
    fn width(&self, name: &str) -> Option<f64> {
        self.font.glyph(name)?.layer(&self.master).map(|l| l.width * self.k)
    }
}

/// The `layers` parameter.
fn sources(s: &Session, p: &Params) -> Result<Vec<Source>> {
    let list = p.array("layers")?;
    if list.is_empty() {
        return Err(p.bad("`layers` is empty"));
    }
    if list.len() > MAX_SOURCES {
        return Err(p.bad(format!("`layers` holds {} sources; at most {MAX_SOURCES} are allowed", list.len())));
    }
    let mut out: Vec<Source> = Vec::with_capacity(list.len());
    for (n, item) in list.iter().enumerate() {
        let at = format!("layers[{n}]");
        let obj = item.as_object().ok_or_else(|| p.bad(format!("`{at}` must be an object {{font, master?, color?, opacity?, offset?, scale?}}")))?;
        let field = |name: &str| obj.get(name).filter(|v| !v.is_null());
        let doc_ref = field("font").ok_or_else(|| p.bad(format!("`{at}.font` is required")))?;
        let doc = super::compare::document_index(s, p, &format!("{at}.font"), doc_ref)?;
        let d = s.documents().get(doc).ok_or(EngineError::NoDocument)?;
        let font = Arc::clone(&d.font);
        let master = match field("master") {
            None => font.default_master()?.id.clone(),
            Some(v) => {
                let r = target::parse(v).map_err(|e| p.bad(format!("`{at}.master`: {e}")))?;
                let i = target::master(&font, &r).map_err(|e| p.bad(format!("`{at}.master`: {e}")))?;
                font.masters.get(i).map(|m| m.id.clone()).ok_or_else(|| p.bad(format!("`{at}.master`: there is no master #{i}")))?
            }
        };
        let color = match field("color") {
            None => Rgba::BLACK,
            Some(Value::String(text)) => {
                Rgba::parse_hex(text).ok_or_else(|| p.bad(format!("`{at}.color` must be a colour written #RRGGBB or #RRGGBBAA, got \"{text}\"")))?
            }
            Some(_) => return Err(p.bad(format!("`{at}.color` must be a colour written #RRGGBB or #RRGGBBAA"))),
        };
        let opacity = match field("opacity") {
            None => 1.0,
            Some(v) => v
                .as_f64()
                .filter(|o| o.is_finite() && (0.0..=1.0).contains(o))
                .ok_or_else(|| p.bad(format!("`{at}.opacity` must be a number from 0 to 1")))?,
        };
        let offset = match field("offset") {
            None => (0.0, 0.0),
            Some(Value::Array(xy)) if xy.len() == 2 => {
                let coord = |i: usize| {
                    xy.get(i)
                        .and_then(Value::as_f64)
                        .filter(|c| c.is_finite() && c.abs() <= MAX_COORDINATE)
                        .ok_or_else(|| p.bad(format!("`{at}.offset` must be two numbers within ±{MAX_COORDINATE}")))
                };
                (coord(0)?, coord(1)?)
            }
            Some(_) => return Err(p.bad(format!("`{at}.offset` must be [x, y]"))),
        };
        let scale = match field("scale") {
            None => 1.0,
            Some(v) => v
                .as_f64()
                .filter(|k| k.is_finite() && *k > 0.0 && *k <= MAX_SCALE)
                .ok_or_else(|| p.bad(format!("`{at}.scale` must be a number above 0 and at most {MAX_SCALE}")))?,
        };
        let bottom_upm = out.first().map_or(font.units_per_em, |b| b.font.units_per_em);
        let k = scale * f64::from(bottom_upm) / f64::from(font.units_per_em.max(1));
        let alpha = (f64::from(color.a) * opacity).round().clamp(0.0, 255.0) as u8;
        out.push(Source { title: d.title.clone(), font, master, color: Rgba { a: alpha, ..color }, k, offset });
    }
    Ok(out)
}

/// Warnings about the sources themselves: units per em that were scaled.
fn source_warnings(sources: &[Source]) -> Vec<String> {
    let Some(bottom) = sources.first() else { return Vec::new() };
    sources
        .iter()
        .skip(1)
        .filter(|s| s.font.units_per_em != bottom.font.units_per_em)
        .map(|s| format!("`{}` has {} units per em and was scaled to the bottom source's {}", s.title, s.font.units_per_em, bottom.font.units_per_em))
        .collect()
}

// ---------- outlines ----------

/// Takes `n` nodes from `budget`, refusing when it runs out.
fn charge(p: &Params, budget: &mut usize, n: usize) -> Result<()> {
    *budget = budget.checked_sub(n).ok_or_else(|| p.bad(format!("the overlaid font would hold more than {MAX_OUTPUT_NODES} nodes")))?;
    Ok(())
}

fn node_count(shapes: &[Shape]) -> usize {
    shapes.iter().map(|s| if let Shape::Path(path) = s { path.nodes.len() } else { 0 }).sum()
}

/// The paths and anchors of glyph `name` on the source's master, with components and corner
/// components resolved to paths, scaled and moved; `None` when the source lacks the glyph or its
/// master layer. Images and other shapes that are not paths are left out.
fn placed(p: &Params, src: &Source, name: &str, budget: &mut usize) -> Result<Option<(Vec<Shape>, Vec<Anchor>)>> {
    let Some(glyph) = src.font.glyph(name) else { return Ok(None) };
    let Some(layer) = glyph.layer(&src.master) else { return Ok(None) };
    // A layer whose corners cannot be fitted is copied without them, as the outline draws it.
    let cornered = fontcraft_model::corner::insert_corners(&src.font, layer).ok().flatten();
    let shapes = cornered.as_deref().unwrap_or(&layer.shapes);
    let flat = super::decompose::decompose_where(&src.font, layer, shapes, |_: &str| true)
        .map_err(|e| p.bad(format!("glyph `{name}` of `{}`: {e}", src.title)))?;
    let far = || p.bad(format!("glyph `{name}` of `{}` would have points beyond ±{MAX_COORDINATE}", src.title));
    let mut out = Vec::with_capacity(flat.len());
    for shape in flat {
        let Shape::Path(mut path) = shape else { continue };
        charge(p, budget, path.nodes.len())?;
        for node in &mut path.nodes {
            (node.x, node.y) = src.map(node.x, node.y);
            if !(in_range(node.x) && in_range(node.y)) {
                return Err(far());
            }
        }
        out.push(Shape::Path(path));
    }
    let mut anchors = Vec::with_capacity(layer.anchors.len());
    for anchor in &layer.anchors {
        let mut a = anchor.clone();
        (a.x, a.y) = src.map(a.x, a.y);
        if !(in_range(a.x) && in_range(a.y)) {
            return Err(far());
        }
        anchors.push(a);
    }
    Ok(Some((out, anchors)))
}

fn in_range(v: f64) -> bool {
    v.is_finite() && v.abs() <= MAX_COORDINATE
}

// ---------- the new font ----------

/// Every glyph name some source has: the bottom source's in its order, then each later source's
/// new names in its order.
fn glyph_names(p: &Params, sources: &[Source]) -> Result<Vec<String>> {
    let mut seen: HashSet<&str> = HashSet::new();
    let mut out = Vec::new();
    for src in sources {
        for g in src.font.glyphs.iter() {
            if seen.insert(g.name.as_str()) {
                if out.len() >= MAX_GLYPHS {
                    return Err(p.bad(format!("the sources hold more than {MAX_GLYPHS} different glyphs")));
                }
                out.push(g.name.clone());
            }
        }
    }
    Ok(out)
}

/// The glyph record `name` gets, without layers, and the position of the source it comes from:
/// the bottom source's, else the lowest source's that has the glyph, without kerning groups and
/// without code points an earlier glyph has (a warning names them). `None` when no source has it.
fn glyph_record(sources: &[Source], name: &str, codes: &mut HashSet<u32>, warnings: &mut Vec<String>) -> Option<(Glyph, usize)> {
    let (owner, src) = sources.iter().enumerate().find(|(_, s)| s.font.glyph(name).is_some())?;
    let mut glyph = Glyph::clone(src.font.glyph(name)?);
    glyph.layers = Vec::new();
    if owner > 0 {
        glyph.kern_groups = KernGroups::default();
        let dropped: Vec<String> = glyph.unicodes.iter().filter(|cp| codes.contains(*cp)).map(|cp| format!("U+{cp:04X}")).collect();
        if !dropped.is_empty() {
            glyph.unicodes.retain(|cp| !codes.contains(cp));
            warnings.push(format!("glyph `{name}` (from `{}`) lost {}, which another glyph has", src.title, dropped.join(", ")));
        }
    }
    codes.extend(glyph.unicodes.iter().copied());
    Some((glyph, owner))
}

/// Copies the kerning `from` has under `from_master` into `to` under `to_master`, every value
/// multiplied by `k`.
fn copy_kerning(from: &Kerning, from_master: &str, to: &mut Kerning, to_master: &str, k: f64) {
    for (src, dst) in [(&from.ltr, &mut to.ltr), (&from.rtl, &mut to.rtl), (&from.vertical, &mut to.vertical)] {
        if let Some(entry) = src.0.iter().find(|m| m.master == from_master) {
            let mut e = entry.clone();
            e.master = to_master.to_string();
            for first in &mut e.firsts {
                for (_, v) in &mut first.seconds {
                    *v *= k;
                }
            }
            dst.0.retain(|m| m.master != to_master);
            dst.0.push(e);
        }
    }
    for c in &from.context {
        let Some((_, v)) = c.values.iter().find(|(m, _)| m == from_master) else { continue };
        let value = (to_master.to_string(), v * k);
        match to.context.iter_mut().find(|o| o.context == c.context) {
            Some(o) => {
                o.values.retain(|(m, _)| m != to_master);
                o.values.push(value);
            }
            None => to.context.push(KernContext { context: c.context.clone(), values: vec![value] }),
        }
    }
}

/// The bottom source's font cut down to its one master, with no glyphs yet: no axes, instances
/// or other masters, the master's own palettes removed, kerning of that master only, and its
/// vertical metrics and kerning scaled by the source's scale.
fn base_font(src: &Source) -> Result<Font> {
    let mut font = Font::clone(&src.font);
    let mut master = src.font.master(&src.master)?.clone();
    master.location.clear();
    master.params.remove(COLOR_PALETTES_PARAM);
    for v in &mut master.metric_values {
        v.pos *= src.k;
        v.over *= src.k;
    }
    font.masters = Arc::new(vec![master]);
    font.axes = Arc::default();
    font.instances = Arc::default();
    let mut kerning = Kerning::default();
    copy_kerning(&src.font.kerning, &src.master, &mut kerning, &src.master, src.k);
    font.kerning = Arc::new(kerning);
    font.glyphs = GlyphTable::default();
    Ok(font)
}

/// The finished font, its masters as the result lists them, and the palette (colorLayers).
struct Overlay {
    font: Font,
    masters: Vec<Value>,
    palette: Option<Vec<String>>,
}

fn color_layers_font(p: &Params, sources: &[Source], warnings: &mut Vec<String>) -> Result<Overlay> {
    let bottom = sources.first().ok_or_else(|| p.bad("`layers` is empty"))?;
    let mut font = base_font(bottom)?;
    let master = bottom.master.clone();
    let colors: Vec<Rgba> = sources.iter().map(|s| s.color).collect();
    let palettes = Palettes::new(vec![colors.clone()]).map_err(|e| p.bad(e.to_string()))?;
    let value = palette::store(None, &palettes, Change::Colors, palette::form_for(&font));
    Arc::make_mut(&mut font.info).params.set(COLOR_PALETTES_PARAM, value);

    // One id per source for its colour layers; ids only need to be unique within a glyph.
    let mut ids: Vec<String> = Vec::with_capacity(sources.len());
    for i in 0..sources.len() {
        let id = super::layers::new_layer_id(&format!("overlay|{master}|{i}"), |id| id == master || ids.iter().any(|t| t.as_str() == id))
            .ok_or_else(|| p.bad("could not find a free layer id"))?;
        ids.push(id);
    }

    let mut budget = MAX_OUTPUT_NODES;
    let mut codes = HashSet::new();
    let mut glyphs = Vec::new();
    for name in glyph_names(p, sources)? {
        let Some((mut glyph, owner)) = glyph_record(sources, &name, &mut codes, warnings) else { continue };
        let width = sources.get(owner).and_then(|s| s.width(&name)).unwrap_or(0.0);
        let mut main = Layer::new(master.as_str(), width);
        let mut colored = Vec::new();
        for (i, (src, id)) in sources.iter().zip(&ids).enumerate() {
            let Some((shapes, anchors)) = placed(p, src, &name, &mut budget)? else { continue };
            if i == owner {
                main.anchors = anchors;
            }
            if shapes.is_empty() {
                continue;
            }
            charge(p, &mut budget, node_count(&shapes))?;
            main.shapes.extend(shapes.iter().cloned());
            let row = u16::try_from(i).map_err(|_| p.bad("too many sources for one palette"))?;
            let mut layer = Layer::new(master.as_str(), width);
            layer.id = id.clone();
            layer.shapes = shapes;
            layer.info.name = Some(src.title.clone());
            layer.info.attrs = layers::with_kind(&LayerAttrs::default(), Some(ColorLayerKind::Palette(PaletteEntry::Index(row))));
            colored.push(layer);
        }
        glyph.layers.push(main);
        glyph.layers.extend(colored);
        glyphs.push(Arc::new(glyph));
    }
    font.glyphs = GlyphTable::from_glyphs(glyphs);
    let name = font.masters.first().map(|m| m.name.clone()).unwrap_or_default();
    Ok(Overlay { font, masters: vec![json!({"id": master, "name": name, "source": 0})], palette: Some(colors.iter().map(Rgba::hex).collect()) })
}

/// The name of the master made from `src`: its family name and master name.
fn master_name(src: &Source) -> String {
    let theirs = src.font.master(&src.master).map(|m| m.name.trim().to_string()).unwrap_or_default();
    let name = format!("{} {theirs}", src.font.family_name.trim());
    let name = name.trim();
    if name.is_empty() { src.title.clone() } else { name.to_string() }
}

fn masters_font(p: &Params, sources: &[Source], warnings: &mut Vec<String>) -> Result<Overlay> {
    let bottom = sources.first().ok_or_else(|| p.bad("`layers` is empty"))?;
    let mut font = base_font(bottom)?;
    let base_master = font.masters.first().cloned().ok_or_else(|| p.bad("the bottom source has no master"))?;
    let base_kerning = Arc::clone(&font.kerning);

    let mut ids: Vec<String> = vec![bottom.master.clone()];
    let mut n = 0u32;
    while ids.len() < sources.len() {
        n += 1;
        if n > 100_000 {
            return Err(p.bad("could not find a free master id"));
        }
        let id = format!("m{n:02}");
        if !ids.contains(&id) {
            ids.push(id);
        }
    }

    let mut masters = Vec::with_capacity(sources.len());
    let mut listed = Vec::with_capacity(sources.len());
    let mut names: HashSet<String> = HashSet::new();
    let mut kerning = Kerning::default();
    for (i, (src, id)) in sources.iter().zip(&ids).enumerate() {
        let wanted = master_name(src);
        let mut name = wanted.clone();
        let mut copy = 1;
        while names.contains(&name) {
            copy += 1;
            name = format!("{wanted} {copy}");
        }
        names.insert(name.clone());
        let mut m = base_master.clone();
        m.id = id.clone();
        m.name = name.clone();
        crate::linked::unmark(&mut m.user_data);
        copy_kerning(&base_kerning, &bottom.master, &mut kerning, id, 1.0);
        listed.push(json!({"id": id, "name": name, "source": i}));
        masters.push(m);
    }
    font.masters = Arc::new(masters);
    font.kerning = Arc::new(kerning);

    let mut budget = MAX_OUTPUT_NODES;
    let mut codes = HashSet::new();
    let mut glyphs = Vec::new();
    for name in glyph_names(p, sources)? {
        let Some((mut glyph, owner)) = glyph_record(sources, &name, &mut codes, warnings) else { continue };
        let width = sources.get(owner).and_then(|s| s.width(&name)).unwrap_or(0.0);
        for (src, id) in sources.iter().zip(&ids) {
            let mut layer = Layer::new(id.as_str(), width);
            if let Some((shapes, anchors)) = placed(p, src, &name, &mut budget)? {
                layer.shapes = shapes;
                layer.anchors = anchors;
            }
            glyph.layers.push(layer);
        }
        glyphs.push(Arc::new(glyph));
    }
    font.glyphs = GlyphTable::from_glyphs(glyphs);
    Ok(Overlay { font, masters: listed, palette: None })
}

// ---------- the commands ----------

fn overlay_fonts(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("font.overlayFonts", v)?;
    let output = p.str("output")?;
    let sources = sources(s, &p)?;
    let mut warnings = source_warnings(&sources);
    let overlay = match output {
        "colorLayers" => color_layers_font(&p, &sources, &mut warnings)?,
        "masters" => masters_font(&p, &sources, &mut warnings)?,
        other => return Err(p.bad(format!("`output` must be \"colorLayers\" or \"masters\", got \"{other}\""))),
    };
    let count = overlay.font.glyphs.len();
    s.add_document(overlay.font, None)?;
    let document = s.documents().len().saturating_sub(1);
    let title = s.doc()?.title.clone();
    let mut out = json!({
        "title": title,
        "document": document,
        "output": output,
        "glyphs": count,
        "masters": overlay.masters,
        "warnings": warnings,
    });
    if let (Some(palette), Some(o)) = (overlay.palette, out.as_object_mut()) {
        o.insert("palette".into(), json!(palette));
    }
    Ok(out)
}

fn overlay_preview(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("font.overlayPreview", v)?;
    if p.opt_array("glyphs")?.is_some_and(|a| a.len() > MAX_PREVIEW_GLYPHS) {
        return Err(p.bad(format!("`glyphs` may name at most {MAX_PREVIEW_GLYPHS} glyphs")));
    }
    let names = p.strings("glyphs")?;
    if names.is_empty() {
        return Err(p.bad("`glyphs` is empty"));
    }
    let sources = sources(s, &p)?;
    let units_per_em = sources.first().map_or(0, |b| b.font.units_per_em);
    let mut warnings = source_warnings(&sources);
    let mut left = MAX_PREVIEW_ELEMENTS;
    let mut glyphs = Vec::with_capacity(names.len());
    for name in names {
        let width = sources.iter().find(|s| s.font.glyph(name).is_some()).and_then(|s| s.width(name));
        let mut drawn = Vec::new();
        for (i, src) in sources.iter().enumerate() {
            let Some(glyph) = src.font.glyph(name) else { continue };
            if glyph.layer(&src.master).is_none() {
                continue;
            }
            match src.font.outline_of(glyph, &src.master) {
                Ok(mut path) => {
                    left = left.checked_sub(path.elements().len()).ok_or_else(|| {
                        p.bad(format!("the preview would hold more than {MAX_PREVIEW_ELEMENTS} path elements; ask for fewer glyphs"))
                    })?;
                    path.apply_affine(src.affine());
                    drawn.push(json!({"source": i, "color": src.color.hex(), "path": path.to_svg()}));
                }
                Err(e) => warnings.push(format!("glyph `{name}` of `{}` was left out: {e}", src.title)),
            }
        }
        glyphs.push(json!({"glyph": name, "width": width, "layers": drawn}));
    }
    Ok(json!({"unitsPerEm": units_per_em, "glyphs": glyphs, "warnings": warnings}))
}
