//! Colour palette commands: list, add and remove colours and palettes, set a colour. Removing a
//! colour renumbers the palette layers that use later rows in the same undo step and refuses while
//! a layer uses the row.
//!
//! Package M11.2 (`docs/plans/M11.md` § 4 › M11.2) fills [`specs`]; rows: `colorPalette.list`,
//! `colorPalette.addColor`, `colorPalette.removeColor`, `colorPalette.setColor`,
//! `colorPalette.addPalette`, `colorPalette.removePalette`.
//!
//! Every command takes an optional `master`: without it the font's `Color Palettes` is read and
//! edited; with it, that master's own, which an edit creates from the font's palettes when the
//! master has none. A row's users are the palette layers and the palette colours of shapes on
//! the layers those palettes apply to: every layer of `master`, or, for the font's palettes, the
//! layers of masters without palettes of their own.

use std::collections::HashSet;
use std::sync::Arc;

use fontcraft_color::MAX_PALETTES;
use fontcraft_color::palette::{self, COLOR_PALETTES_PARAM, Change, FOREGROUND_INDEX, Palettes, Rgba};
use fontcraft_model::plist::Value as Plist;
use fontcraft_model::{Color, Font, Layer, ModelError, PaletteEntry, Shape, ShapeAttrs};
use serde_json::{Value, json};

use super::*;
use crate::EngineError;
use crate::params::Params;

/// Most glyph names a refusal lists before it says how many more there are.
const MAX_NAMED_GLYPHS: usize = 20;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "colorPalette.list",
            "List Color Palettes",
            [],
            None,
            "{master?: id} the palettes that apply: the font's `Color Palettes`, or with `master` that master's own when it has one, else the font's. Palettes and colours count from 0 → {master: id|null, source: \"font\"|\"master\"|null (null: no palettes), rows: count of colours in every palette, palettes: [[\"#RRGGBBAA\" per colour] per palette]}",
            has_doc,
            list
        ),
        cmd!(
            "colorPalette.addColor",
            "Add Color",
            [],
            None,
            "{color?: \"#RRGGBB\"|\"#RRGGBBAA\" (default: opaque black), colors?: [one colour per palette], master?: id} append a colour row to every palette; give `color` for the same colour in every palette or `colors` for one each, not both. With no palette yet, the first palette is made. With `master`, that master's own palettes are edited (made from the font's when it has none) → {index: the new row, rows, palettes: count}",
            has_doc,
            add_color
        ),
        cmd!(
            "colorPalette.removeColor",
            "Remove Color",
            [],
            None,
            "{index: row, master?: id} remove a colour row from every palette. Palette layers and shape colours that use a later row move down by one in the same step, so they keep their colour. Refused while a layer uses the row, naming the glyphs → {removed: index, rows, renumbered: [glyph names]}",
            has_doc,
            remove_color
        ),
        cmd!(
            "colorPalette.setColor",
            "Set Palette Color",
            [],
            None,
            "{palette?: number (default: 0), index: row, color: \"#RRGGBB\"|\"#RRGGBBAA\", master?: id} change one colour of one palette → {palette, index, color: \"#RRGGBBAA\"}",
            has_doc,
            set_color
        ),
        cmd!(
            "colorPalette.addPalette",
            "Add Palette",
            [],
            None,
            "{copyFrom?: palette (default: 0), master?: id} append a palette holding a copy of palette `copyFrom`'s colours; with no palette yet, an empty palette → {palette: the new palette's number, palettes: count, rows}",
            has_doc,
            add_palette
        ),
        cmd!(
            "colorPalette.removePalette",
            "Remove Palette",
            [],
            None,
            "{palette: number, master?: id} remove one palette; the palettes after it move down by one. The last palette cannot be removed → {removed: palette, palettes: count}",
            has_doc,
            remove_palette
        ),
    ]
}

// ── Reading ─────────────────────────────────────────────────────────────────────────────────────

/// The `master` parameter: a master the font has, or `None` for the font's palettes.
fn owner(p: &Params, font: &Font) -> Result<Option<String>> {
    match p.opt_str("master")? {
        None => Ok(None),
        Some(id) if font.masters.iter().any(|m| m.id == id) => Ok(Some(id.to_string())),
        Some(id) => Err(p.bad(format!("there is no master `{id}`"))),
    }
}

/// The stored value that applies and the palettes it holds.
fn current<'f>(p: &Params, font: &'f Font, master: Option<&str>) -> Result<(Option<&'f Plist>, Palettes)> {
    let stored = palette::stored(font, master).map_err(|e| p.bad(e.to_string()))?;
    let palettes = stored.map_or_else(|| Ok(Palettes::default()), palette::parse).map_err(|e| p.bad(e.to_string()))?;
    Ok((stored, palettes))
}

fn has_own(font: &Font, master: &str) -> bool {
    font.masters.iter().any(|m| m.id == master && m.params.get(COLOR_PALETTES_PARAM).is_some())
}

fn list(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("colorPalette.list", p)?;
    let font = s.font()?;
    let master = owner(&p, font)?;
    let (stored, palettes) = current(&p, font, master.as_deref())?;
    let source = match (&master, stored) {
        (_, None) => None,
        (Some(id), Some(_)) if has_own(font, id) => Some("master"),
        (_, Some(_)) => Some("font"),
    };
    let colours: Vec<Vec<String>> = palettes.palettes().iter().map(|pal| pal.iter().map(Rgba::hex).collect()).collect();
    Ok(json!({"master": master, "source": source, "rows": palettes.rows(), "palettes": colours}))
}

// ── Editing ─────────────────────────────────────────────────────────────────────────────────────

/// A colour parameter written `#RRGGBB` or `#RRGGBBAA`.
fn colour(p: &Params, key: &str, text: &str) -> Result<Rgba> {
    Rgba::parse_hex(text).ok_or_else(|| p.bad(format!("`{key}` must be a colour written #RRGGBB or #RRGGBBAA, got \"{text}\"")))
}

/// A palette number or row index parameter (whole number from 0).
fn index(p: &Params, key: &str) -> Result<Option<usize>> {
    Ok(p.opt_int(key, 0, i64::from(u16::MAX))?.and_then(|i| usize::try_from(i).ok()))
}

fn required(p: &Params, key: &str) -> Result<usize> {
    index(p, key)?.ok_or_else(|| p.bad(format!("`{key}` is required")))
}

/// Stores `next` as the palettes of `master` (the font's when `None`), in the form the value
/// had before, as one undoable step that also runs `then` on the font.
fn save<T>(
    s: &mut Session,
    label: &str,
    master: Option<&str>,
    next: &Palettes,
    change: Change,
    then: impl FnOnce(&mut Font) -> Result<T>,
) -> Result<T> {
    let font = s.font()?;
    let value = palette::store(palette::stored(font, master).ok().flatten(), next, change, palette::form_for(font));
    s.edit(label, |font| {
        match master {
            None => Arc::make_mut(&mut font.info).params.set(COLOR_PALETTES_PARAM, value),
            Some(id) => {
                let m = Arc::make_mut(&mut font.masters)
                    .iter_mut()
                    .find(|m| m.id == id)
                    .ok_or_else(|| EngineError::Model(ModelError::NoSuchMaster(id.to_string())))?;
                m.params.set(COLOR_PALETTES_PARAM, value);
            }
        }
        then(font)
    })
}

fn add_color(s: &mut Session, raw: &Value) -> Result<Value> {
    let p = Params::new("colorPalette.addColor", raw)?;
    let font = s.font()?;
    let master = owner(&p, font)?;
    let (_, palettes) = current(&p, font, master.as_deref())?;
    let wanted = palettes.palettes().len().max(1);
    let colours = match (p.opt_str("color")?, p.get("colors")) {
        (Some(_), Some(_)) => return Err(p.bad("give `color` or `colors`, not both")),
        (Some(text), None) => vec![colour(&p, "color", text)?; wanted],
        (None, None) => vec![Rgba::BLACK; wanted],
        (None, Some(_)) => {
            if p.opt_array("colors")?.is_some_and(|a| a.len() > MAX_PALETTES) {
                return Err(p.bad(format!("`colors` may hold at most {MAX_PALETTES} colours")));
            }
            let texts = p.strings("colors")?;
            if texts.len() != wanted {
                return Err(p.bad(format!("`colors` holds {} colours; give one for each of the {wanted} palettes", texts.len())));
            }
            texts.iter().map(|t| colour(&p, "colors", t)).collect::<Result<Vec<_>>>()?
        }
    };
    let next = palettes.add_row_with(&colours).map_err(|e| p.bad(e.to_string()))?;
    save(s, "Add Color", master.as_deref(), &next, Change::Colors, |_| Ok(()))?;
    Ok(json!({"index": next.rows().saturating_sub(1), "rows": next.rows(), "palettes": next.palettes().len()}))
}

fn set_color(s: &mut Session, raw: &Value) -> Result<Value> {
    let p = Params::new("colorPalette.setColor", raw)?;
    let font = s.font()?;
    let master = owner(&p, font)?;
    let (_, palettes) = current(&p, font, master.as_deref())?;
    let which = index(&p, "palette")?.unwrap_or(0);
    let row = required(&p, "index")?;
    let wanted = colour(&p, "color", p.str("color")?)?;
    let next = palettes.set(which, row, wanted).map_err(|e| p.bad(e.to_string()))?;
    save(s, "Set Palette Color", master.as_deref(), &next, Change::Colors, |_| Ok(()))?;
    Ok(json!({"palette": which, "index": row, "color": wanted.hex()}))
}

fn add_palette(s: &mut Session, raw: &Value) -> Result<Value> {
    let p = Params::new("colorPalette.addPalette", raw)?;
    let font = s.font()?;
    let master = owner(&p, font)?;
    let (_, palettes) = current(&p, font, master.as_deref())?;
    let next = match index(&p, "copyFrom")? {
        Some(source) => palettes.add_palette_from(source),
        None => palettes.add_palette(),
    }
    .map_err(|e| p.bad(e.to_string()))?;
    save(s, "Add Palette", master.as_deref(), &next, Change::Colors, |_| Ok(()))?;
    Ok(json!({"palette": next.palettes().len().saturating_sub(1), "palettes": next.palettes().len(), "rows": next.rows()}))
}

fn remove_palette(s: &mut Session, raw: &Value) -> Result<Value> {
    let p = Params::new("colorPalette.removePalette", raw)?;
    let font = s.font()?;
    let master = owner(&p, font)?;
    let (_, palettes) = current(&p, font, master.as_deref())?;
    let which = required(&p, "palette")?;
    let next = palettes.remove_palette(which).map_err(|e| p.bad(e.to_string()))?;
    save(s, "Remove Palette", master.as_deref(), &next, Change::RemovedPalette(which), |_| Ok(()))?;
    Ok(json!({"removed": which, "palettes": next.palettes().len()}))
}

fn remove_color(s: &mut Session, raw: &Value) -> Result<Value> {
    let p = Params::new("colorPalette.removeColor", raw)?;
    let font = s.font()?;
    let master = owner(&p, font)?;
    let (_, palettes) = current(&p, font, master.as_deref())?;
    let row = required(&p, "index")?;
    let next = palettes.remove_row(row).map_err(|e| p.bad(e.to_string()))?;
    // Rows stop below 65,535, so a row that exists fits a palette index.
    let row16 = u16::try_from(row).map_err(|_| p.bad(format!("there is no colour {row} in the palettes")))?;
    let scope = Scope::new(font, master.as_deref());
    let users: Vec<&str> =
        font.glyphs.iter().filter(|g| g.layers.iter().any(|l| scope.covers(l) && indices(l).any(|i| i == row16))).map(|g| g.name.as_str()).collect();
    if !users.is_empty() {
        let named = users.iter().take(MAX_NAMED_GLYPHS).copied().collect::<Vec<_>>().join(", ");
        let more = users.len().saturating_sub(MAX_NAMED_GLYPHS);
        let more = if more > 0 { format!(" and {more} more glyphs") } else { String::new() };
        return Err(p.bad(format!("colour {row} is used by {named}{more}: give those layers another colour first")));
    }
    let later: Vec<String> =
        font.glyphs.iter().filter(|g| g.layers.iter().any(|l| scope.covers(l) && indices(l).any(|i| i > row16))).map(|g| g.name.clone()).collect();
    save(s, "Remove Color", master.as_deref(), &next, Change::RemovedRow(row), |font| {
        for name in &later {
            let mut g = font.glyph_mut(name)?;
            for layer in g.layers.iter_mut().filter(|l| scope.covers(l)) {
                renumber(layer, row16);
            }
        }
        Ok(())
    })?;
    Ok(json!({"removed": row, "rows": next.rows(), "renumbered": later}))
}

// ── Users of a row ──────────────────────────────────────────────────────────────────────────────

/// The layers the edited palettes apply to.
struct Scope {
    master: Option<String>,
    /// Masters with palettes of their own; the font's palettes do not apply to their layers.
    own: HashSet<String>,
}

impl Scope {
    fn new(font: &Font, master: Option<&str>) -> Self {
        let own = font.masters.iter().filter(|m| m.params.get(COLOR_PALETTES_PARAM).is_some()).map(|m| m.id.clone()).collect();
        Scope { master: master.map(str::to_string), own }
    }

    fn covers(&self, layer: &Layer) -> bool {
        match &self.master {
            Some(id) => layer.master == *id,
            None => !self.own.contains(&layer.master),
        }
    }
}

fn shape_attrs(shape: &Shape) -> Option<&ShapeAttrs> {
    match shape {
        Shape::Path(p) => Some(&p.info.attrs),
        Shape::Component(c) => Some(&c.info.attrs),
        _ => None,
    }
}

fn shape_attrs_mut(shape: &mut Shape) -> Option<&mut ShapeAttrs> {
    match shape {
        Shape::Path(p) => Some(&mut p.info.attrs),
        Shape::Component(c) => Some(&mut c.info.attrs),
        _ => None,
    }
}

fn colours(a: &ShapeAttrs) -> impl Iterator<Item = &Color> {
    a.fill_color
        .iter()
        .chain(a.stroke_color.iter())
        .chain(a.gradient.iter().flat_map(|g| g.stops.iter().map(|s| &s.color)))
        .chain(a.shadow.iter().map(|s| &s.color))
        .chain(a.shadow_in.iter().map(|s| &s.color))
}

fn colours_mut(a: &mut ShapeAttrs) -> impl Iterator<Item = &mut Color> {
    a.fill_color
        .iter_mut()
        .chain(a.stroke_color.iter_mut())
        .chain(a.gradient.iter_mut().flat_map(|g| g.stops.iter_mut().map(|s| &mut s.color)))
        .chain(a.shadow.iter_mut().map(|s| &mut s.color))
        .chain(a.shadow_in.iter_mut().map(|s| &mut s.color))
}

fn palette_row(c: &Color) -> Option<u16> {
    match c {
        Color::Palette { index, .. } if *index != FOREGROUND_INDEX => Some(*index),
        _ => None,
    }
}

/// Every palette row `layer` uses: its palette entry and its shapes' palette colours. The text
/// colour is not a row.
fn indices(layer: &Layer) -> impl Iterator<Item = u16> + '_ {
    let entry = match layer.info.attrs.color_palette {
        Some(PaletteEntry::Index(i)) if i != FOREGROUND_INDEX => Some(i),
        _ => None,
    };
    entry.into_iter().chain(layer.shapes.iter().filter_map(shape_attrs).flat_map(colours).filter_map(palette_row))
}

/// Moves every row after `removed` that `layer` uses down by one.
fn renumber(layer: &mut Layer, removed: u16) {
    if let Some(PaletteEntry::Index(i)) = layer.info.attrs.color_palette
        && i > removed
        && i != FOREGROUND_INDEX
    {
        layer.info.attrs.color_palette = Some(PaletteEntry::Index(i - 1));
    }
    for shape in &mut layer.shapes {
        // Only shapes that change are touched, so the others keep their shared storage.
        if !shape_attrs(shape).is_some_and(|a| colours(a).filter_map(palette_row).any(|i| i > removed)) {
            continue;
        }
        let Some(attrs) = shape_attrs_mut(shape) else { continue };
        for c in colours_mut(attrs) {
            if let Color::Palette { index, .. } = c
                && *index > removed
                && *index != FOREGROUND_INDEX
            {
                *index -= 1;
            }
        }
    }
}
