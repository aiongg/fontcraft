//! The Space Center's view model: the window's text laid out for the master shown, the outlines it draws and the
//! matrix values.
//!
//! The text is laid out with `textlayout` and kerned. Multi-line text is laid out with its own line breaks only;
//! the view wraps each line at the window's width, which the projection does not know. Single-line and waterfall
//! text is one line. Nothing is projected while the window is hidden.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use craft::ui::contract::{PlainData, Tracked};
use fontcraft_engine::model::{Font, Layer};
use fontcraft_geom::BezPath;
use fontcraft_textlayout::{Direction, KerningMode, LENGTH_RANGE, Layout, Options, Text};

use super::ProjectCx;
use crate::shell::dock::PanelId;
use crate::shell::space_center::text::{MAX_GLYPHS_SHOWN, Resolved, Row, value, x_height};
use crate::shell::space_center::{self as sc, LineMode, MAX_OVERLAY_LAYERS, SpaceCenterState};

/// Most distinct glyph outlines one projection keeps; the glyphs past it are drawn as their advance only.
const MAX_OUTLINES: usize = 4_000;
/// Most alignment zones drawn.
const MAX_ZONES: usize = 64;

/// One glyph of a line as laid out, in font units along the line.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct PlacedGlyph {
    /// The glyph drawn; `None` for a missing character or a glyph the font lacks.
    pub glyph: Option<String>,
    /// The origin along the line: left to right from the line's start, also in right-to-left text.
    pub offset: f64,
    pub advance: f64,
    /// The matrix column it stands for; control glyphs have none.
    pub column: Option<usize>,
    /// A line may wrap after it: a space.
    pub breaks_after: bool,
    /// The margins measured on the beam, when the beam shows and meets the outline.
    pub beam: Option<(f64, f64)>,
}

/// One line of the text: its glyphs in visual order, left to right.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct SpaceLine {
    pub glyphs: Vec<PlacedGlyph>,
    pub length: f64,
}

/// One matrix column: a glyph of the text and its values in the layer drawn.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct MatrixColumn {
    pub glyph: String,
    pub layer: String,
    pub width: Option<f64>,
    pub left: Option<f64>,
    pub right: Option<f64>,
    /// The glyph's master layers: a drag with Alt held edits every one.
    pub master_layers: Vec<String>,
}

impl MatrixColumn {
    pub fn value(&self, row: Row) -> Option<f64> {
        match row {
            Row::Width => self.width,
            Row::Left => self.left,
            Row::Right => self.right,
        }
    }
}

/// A glyph's outlines as drawn: its own, and those of the overlay layers.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct GlyphArt {
    #[plain_data(assume = "a kurbo path: owned points")]
    pub outline: BezPath,
    #[plain_data(assume = "kurbo paths: owned points")]
    pub overlays: Vec<BezPath>,
}

/// What the Space Center shows.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct SpaceCenterModel {
    pub has_font: bool,
    /// The fields and options, as the view state holds them.
    pub state: SpaceCenterState,
    pub master: Option<String>,
    pub upm: f64,
    /// The layout's ascender and line height: a line reaches from the ascender down by one line height.
    pub ascender: f64,
    pub line_height: f64,
    pub lines: Vec<SpaceLine>,
    /// The outlines by glyph name.
    pub art: BTreeMap<String, Arc<GlyphArt>>,
    pub columns: Vec<MatrixColumn>,
    /// The master's alignment zones as (bottom, top), when zones show.
    pub zones: Vec<(f64, f64)>,
    /// The cut-off band's (bottom, top), when it is on.
    pub band: Option<(f64, f64)>,
    /// The beam's height, when it shows.
    pub beam_y: Option<f64>,
    /// The first problem met reading or laying out the text.
    pub message: Option<String>,
}

pub fn project(model: &mut Tracked<SpaceCenterModel>, cx: &mut ProjectCx<'_>) {
    if cx.view.workspace.layout.panel_state(&PanelId::SpaceCenter) == craft::behavior::dock::PanelState::Hidden {
        return;
    }
    let state = cx.view.space_center.clone();
    let Ok(font) = cx.session.font() else {
        super::set(model, SpaceCenterModel { state, ..Default::default() });
        return;
    };
    let Some((master, resolved)) = sc::resolved(font, cx.view, &state) else {
        super::set(model, SpaceCenterModel { has_font: true, state, ..Default::default() });
        return;
    };
    super::set(model, build(font, &master, &resolved, state));
}

fn build(font: &Font, master: &str, resolved: &Resolved, state: SpaceCenterState) -> SpaceCenterModel {
    let display = &state.display;
    let columns = resolved
        .columns
        .iter()
        .map(|c| MatrixColumn {
            glyph: c.glyph.clone(),
            layer: c.layer.clone(),
            width: value(font, &c.glyph, &c.layer, Row::Width),
            left: value(font, &c.glyph, &c.layer, Row::Left),
            right: value(font, &c.glyph, &c.layer, Row::Right),
            master_layers: font
                .glyph(&c.glyph)
                .map(|g| font.masters.iter().filter_map(|m| g.layer(&m.id).map(|l| l.id.clone())).collect())
                .unwrap_or_default(),
        })
        .collect();
    let shown_master = font.masters.iter().find(|m| m.id == master);
    let zones = match shown_master {
        Some(m) if display.zones => {
            font.alignment_zones(m).into_iter().take(MAX_ZONES).map(|z| (z.pos.min(z.pos + z.size), z.pos.max(z.pos + z.size))).collect()
        }
        _ => Vec::new(),
    };
    let band = state.cut_off.on.then(|| sc::band(font, master, &state.cut_off));
    let beam_y = display.beam.then(|| display.beam_y.unwrap_or_else(|| x_height(font, master) / 2.0)).filter(|y| y.is_finite());
    let mut model = SpaceCenterModel {
        has_font: true,
        master: Some(master.to_string()),
        upm: f64::from(font.units_per_em.max(1)),
        columns,
        zones,
        band,
        beam_y,
        message: resolved.message.clone(),
        ..Default::default()
    };
    match lay_out(font, master, resolved, &state) {
        Ok(layout) => {
            model.ascender = layout.ascender;
            model.line_height = layout.line_height;
            model.lines = lines(font, master, resolved, &layout, beam_y);
            model.art = art(font, master, resolved, &state, &model.lines);
        }
        Err(e) => model.message = Some(e),
    }
    model.state = state;
    model
}

/// Lays the text out for `master` in the direction, line spacing and tracking of the display options.
fn lay_out(font: &Font, master: &str, resolved: &Resolved, state: &SpaceCenterState) -> Result<Layout, String> {
    let mut text = Text::with_limit(MAX_GLYPHS_SHOWN).map_err(|e| e.to_string())?;
    text.insert(0, resolved.items.clone()).map_err(|e| e.to_string())?;
    let display = &state.display;
    let spaced = if sc::LINE_SPACE_RANGE.contains(&display.line_space) { display.line_space } else { 1.0 };
    let line_height = if spaced == 1.0 {
        None
    } else {
        font.masters
            .iter()
            .find(|m| m.id == master)
            .map(|m| fontcraft_textlayout::line_height(font, m) * spaced)
            .filter(|h| h.is_finite())
            .map(|h| h.clamp(*LENGTH_RANGE.start(), *LENGTH_RANGE.end()))
    };
    let options = Options {
        direction: if display.right_to_left { Direction::Rtl } else { Direction::Ltr },
        kerning: KerningMode::Kerning,
        // The view wraps the lines; the layout breaks only where the text does.
        line_width: *LENGTH_RANGE.end(),
        line_height,
        single_line: state.line_mode != LineMode::Multi,
        layers: Some(&resolved.layers),
        ..Options::default()
    };
    let mut layout = fontcraft_textlayout::layout(font, master, &text, &options).map_err(|e| e.to_string())?;
    let tracking = if sc::TRACKING_RANGE.contains(&display.tracking) { display.tracking } else { 0.0 };
    track(&mut layout, tracking);
    Ok(layout)
}

/// Adds `tracking` font units between neighbouring items of each line.
fn track(layout: &mut Layout, tracking: f64) {
    if tracking == 0.0 || !tracking.is_finite() {
        return;
    }
    let sign = if layout.direction == Direction::Rtl { -1.0 } else { 1.0 };
    let Layout { placed, lines, .. } = layout;
    for line in lines.iter_mut() {
        let mut before = 0.0_f64;
        for i in line.items.clone() {
            let Some(p) = placed.get_mut(i) else { continue };
            if p.glyph.is_none() && p.advance <= 0.0 {
                continue;
            }
            let shift = before * tracking;
            p.x += sign * shift;
            p.offset += shift;
            before += 1.0;
        }
        line.length = (line.length + (before - 1.0).max(0.0) * tracking).max(0.0);
    }
}

/// The layer `name` is drawn from: the one named after `@`, else its layer of `master`.
fn drawn_layer<'f>(font: &'f Font, master: &str, resolved: &Resolved, name: &str) -> Option<&'f Layer> {
    let glyph = font.glyph(name)?;
    let id = resolved.layers.get(name).map_or(master, String::as_str);
    glyph.layers.iter().find(|l| l.id == id).or_else(|| glyph.layer(master))
}

/// The layout's lines in visual order, each glyph with its column and, with the beam, its margins on it.
fn lines(font: &Font, master: &str, resolved: &Resolved, layout: &Layout, beam_y: Option<f64>) -> Vec<SpaceLine> {
    let column_of: HashMap<usize, usize> = resolved.columns.iter().enumerate().map(|(c, col)| (col.item, c)).collect();
    let mut measured: HashMap<Arc<str>, Option<(f64, f64)>> = HashMap::new();
    let rtl = layout.direction == Direction::Rtl;
    layout
        .lines
        .iter()
        .map(|line| {
            let mut glyphs: Vec<PlacedGlyph> = line
                .items
                .clone()
                .filter_map(|i| {
                    let p = layout.placed.get(i)?;
                    if p.glyph.is_none() && p.advance <= 0.0 {
                        return None;
                    }
                    let beam = match (&p.glyph, beam_y) {
                        (Some(name), Some(y)) => *measured.entry(name.clone()).or_insert_with(|| {
                            let layer = drawn_layer(font, master, resolved, name)?;
                            fontcraft_spacing::sidebearings_at_height(font, layer, y).ok().flatten()
                        }),
                        _ => None,
                    };
                    let breaks_after = matches!(resolved.items.get(i), Some(fontcraft_textlayout::Item::Glyph { typed: Some(' '), .. }))
                        || p.glyph.as_deref() == Some("space");
                    // A right-to-left line runs leftwards from 0; its visual left edge is at -length.
                    let offset = if rtl { p.x + line.length } else { p.x };
                    Some(PlacedGlyph {
                        glyph: p.glyph.as_ref().map(|g| g.to_string()),
                        offset,
                        advance: p.advance,
                        column: column_of.get(&i).copied(),
                        breaks_after,
                        beam,
                    })
                })
                .collect();
            glyphs.sort_by(|a, b| a.offset.total_cmp(&b.offset));
            SpaceLine { glyphs, length: line.length }
        })
        .collect()
}

/// The outlines of every glyph the lines draw, at most [`MAX_OUTLINES`] of them.
fn art(font: &Font, master: &str, resolved: &Resolved, state: &SpaceCenterState, lines: &[SpaceLine]) -> BTreeMap<String, Arc<GlyphArt>> {
    let overlay: Vec<&String> = state.display.layers.iter().take(MAX_OVERLAY_LAYERS).collect();
    let mut out = BTreeMap::new();
    for name in lines.iter().flat_map(|l| l.glyphs.iter()).filter_map(|g| g.glyph.as_deref()) {
        if out.contains_key(name) {
            continue;
        }
        if out.len() >= MAX_OUTLINES {
            break;
        }
        let Some(glyph) = font.glyph(name) else { continue };
        let outline = drawn_layer(font, master, resolved, name).map(|layer| layer_outline(font, glyph, layer, master)).unwrap_or_default();
        let overlays = overlay
            .iter()
            .filter_map(|typed| sc::text::layer_named(font, name, master, typed))
            .filter_map(|id| glyph.layers.iter().find(|l| l.id == id))
            .map(|layer| layer_outline(font, glyph, layer, master))
            .collect();
        out.insert(name.to_string(), Arc::new(GlyphArt { outline, overlays }));
    }
    out
}

/// The outline of `layer` of `glyph` with its components resolved in `master`: the layer is put in the master's
/// place, so a layer other than the master's draws through the same resolution.
fn layer_outline(font: &Font, glyph: &fontcraft_engine::model::Glyph, layer: &Layer, master: &str) -> BezPath {
    if layer.id == master {
        return font.outline_of(glyph, master).unwrap_or_default();
    }
    let mut stand_in = glyph.clone();
    let mut moved = layer.clone();
    moved.id = master.to_string();
    moved.master = master.to_string();
    stand_in.layers = vec![moved];
    font.outline_of(&stand_in, master).unwrap_or_default()
}
