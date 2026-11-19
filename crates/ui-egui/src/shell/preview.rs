//! The Preview Area under an Edit View tab's canvas, and the commands of both previews (the area
//! and the Preview Panel, [`super::preview_panel`]).
//!
//! A preview draws the tab's whole text filled, on one line without wrapping
//! (`preview.singleLine`), in the tab's direction and kerning mode. The strip reaches from the
//! tab master's `Preview Descender` up to its `Preview Ascender`. The edited glyph sits in the
//! middle while Always Center Active Glyph is on; otherwise the text keeps flush with the edge
//! where it fits. Invert swaps text and ground, Flip turns the strip upside down, Blur softens
//! it, and the offset (dragging the text) shifts it sideways. A double-click on a glyph makes it
//! the edited glyph (`editView.setCursor`).
//!
//! What the line shows is each preview's own choice in its instance pop-up
//! (`preview.setInstance`):
//!
//! | Choice | The strip shows |
//! |---|---|
//! | the dash (`master`) | the text in the tab's master and picked layers, following every master switch (`preview.dashFollowsMaster`) |
//! | an instance | the text interpolated at the instance, with the instance's widths and kerning, from the document's instance view (`fontcraft_engine::interp_cache`) |
//! | Show All Instances (`all`) | the edited glyph once for each static instance whose eye is on (`preview.showAllInstances`, `preview.toggleInstanceVisibility`) |
//!
//! Variable font settings are not instances here: the pop-up lists static instances only.
//!
//! The strip is one texture rasterised by the `render` crate, rebuilt only when what it shows
//! changes; the one-line layouts are cached beside it in egui's memory, so a frame that changes
//! nothing asks the engine for nothing.

use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use egui::{Color32, ColorImage, Pos2, Rect, Sense, TextureHandle, TextureOptions, pos2, vec2};
use fontcraft_engine::Session;
use fontcraft_engine::cmd::preview::{blur_pixels, blur_radius, preview_range, turn_pixels};
use fontcraft_engine::interp_cache::view_master_id;
use fontcraft_engine::prelude::*;
use fontcraft_geom::{Affine, BezPath};
use fontcraft_model::{Font, Master};
use fontcraft_render::{Style, render_path};
use fontcraft_textlayout::{Direction, Layout, Options};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{Args, UiCommand};
use crate::app::FontcraftApp;
use crate::canvas::composite::CompositeCache;
use crate::canvas::tt_pixels::instance_index;
use crate::canvas::view;
use crate::icons::Icon;
use crate::panels::{Pending, widgets};
use crate::state::{Tab, WritingDirection};
use crate::theme::Tokens;

pub use fontcraft_engine::cmd::preview::{DEFAULT_PREVIEW_ASCENDER, MAX_BLUR, PREVIEW_ASCENDER, PREVIEW_DESCENDER};
/// The tallest Preview Area, in screen points.
pub const MAX_AREA_HEIGHT: f64 = 4000.0;
/// The largest offset, in screen points either way.
pub const MAX_OFFSET: f64 = 1.0e6;

/// One preview's options.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PreviewOptions {
    /// Light text on a dark ground (`preview.invert`).
    pub invert: bool,
    /// Upside down (`preview.flip`).
    pub flip: bool,
    /// Blur in screen points, 0 to [`MAX_BLUR`] (`preview.setBlur`).
    pub blur: f64,
    /// Keep the edited glyph in the middle (`preview.alwaysCenterActiveGlyph`).
    pub always_center: bool,
    /// The sideways shift in screen points (`preview.setOffset`).
    pub offset: f64,
}

impl Default for PreviewOptions {
    fn default() -> Self {
        Self { invert: false, flip: false, blur: 0.0, always_center: true, offset: 0.0 }
    }
}

impl PreviewOptions {
    fn blur(&self) -> f64 {
        if self.blur.is_finite() { self.blur.clamp(0.0, MAX_BLUR) } else { 0.0 }
    }

    fn offset(&self) -> f64 {
        if self.offset.is_finite() { self.offset.clamp(-MAX_OFFSET, MAX_OFFSET) } else { 0.0 }
    }
}

/// Both previews' settings.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PreviewState {
    /// The tabs that show the Preview Area, by id.
    pub area_tabs: Vec<u64>,
    /// The area's height in screen points; `None` for the theme's.
    pub area_height: Option<f64>,
    pub area: PreviewOptions,
    /// The Preview Panel is open (`window.previewPanel`).
    pub panel_open: bool,
    pub panel: PreviewOptions,
    /// What the area's and the panel's lines show (`preview.setInstance`).
    pub area_shows: Shows,
    pub panel_shows: Shows,
}

/// What a preview's line shows: the choice in its instance pop-up.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Shows {
    /// The master Edit View draws (the dash).
    #[default]
    Master,
    /// One instance, named as `hint.tt.showPixelPreview` names it: its name, or `#n` for the
    /// instance at position n when the name does not find it.
    Instance(String),
    /// The edited glyph once for each visible static instance.
    All,
}

impl Shows {
    /// The `instance` value of `preview.setInstance` that chooses it.
    pub fn name(&self) -> &str {
        match self {
            Shows::Master => "master",
            Shows::Instance(key) => key,
            Shows::All => "all",
        }
    }
}

/// What a preview draws this frame: [`Shows`] resolved against the font.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Drawn {
    Master,
    Instance(usize),
    /// The static instances whose eye is on, in font order.
    All(Vec<usize>),
}

/// Whether the instance at `index` is a static instance, not a variable font setting.
pub fn is_static(font: &Font, index: usize) -> bool {
    font.instances.get(index).is_some_and(|i| i.kind.as_deref() != Some("variable"))
}

/// How [`Shows::Instance`] and `hint.tt.showPixelPreview` name the instance at `index`: its
/// name when that finds it, else `#index`.
pub fn instance_key(font: &Font, index: usize) -> String {
    match font.instances.get(index) {
        Some(i) if !i.name.is_empty() && instance_index(font, &i.name) == Some(index) => i.name.clone(),
        _ => format!("#{index}"),
    }
}

/// What `shows` draws in `font`. An instance that is gone draws the master.
pub fn drawn(font: &Font, shows: &Shows) -> Drawn {
    match shows {
        Shows::Master => Drawn::Master,
        Shows::Instance(key) => instance_index(font, key).filter(|i| is_static(font, *i)).map_or(Drawn::Master, Drawn::Instance),
        Shows::All => {
            Drawn::All((0..font.instances.len()).filter(|i| is_static(font, *i) && font.instances.get(*i).is_some_and(|x| x.visible)).collect())
        }
    }
}

impl PreviewState {
    pub fn shows(&self, target: Target) -> &Shows {
        match target {
            Target::Area => &self.area_shows,
            Target::Panel => &self.panel_shows,
        }
    }

    fn shows_mut(&mut self, target: Target) -> &mut Shows {
        match target {
            Target::Area => &mut self.area_shows,
            Target::Panel => &mut self.panel_shows,
        }
    }

    pub fn options(&self, target: Target) -> &PreviewOptions {
        match target {
            Target::Area => &self.area,
            Target::Panel => &self.panel,
        }
    }

    fn options_mut(&mut self, target: Target) -> &mut PreviewOptions {
        match target {
            Target::Area => &mut self.area,
            Target::Panel => &mut self.panel,
        }
    }

    /// The area's height: the one set, else the theme's, held between the theme's smallest and
    /// [`MAX_AREA_HEIGHT`].
    pub fn area_height(&self, tokens: &Tokens) -> f64 {
        let min = f64::from(tokens.preview_area_min_height);
        let h = self.area_height.filter(|h| h.is_finite()).unwrap_or(f64::from(tokens.preview_area_height));
        h.min(MAX_AREA_HEIGHT).max(min)
    }
}

/// Which preview a command changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Target {
    Area,
    Panel,
}

impl Target {
    pub fn name(self) -> &'static str {
        match self {
            Target::Area => "area",
            Target::Panel => "panel",
        }
    }

    fn from_args(args: &Args) -> Result<Target, String> {
        match args.string("target")? {
            None | Some("area") => Ok(Target::Area),
            Some("panel") => Ok(Target::Panel),
            Some(other) => Err(args.err(format!("`target` must be \"area\" or \"panel\", got \"{other}\""))),
        }
    }
}

pub const COMMANDS: &[UiCommand] = &[
    (
        "view.togglePreviewArea",
        "Preview",
        &[],
        "",
        "{tab?: id, on?: bool, height?: points} show or hide the Preview Area under the tab's canvas (omitted `on`: toggle); `height` sets the area's height, held to the smallest the theme allows and 4000 → {tab, on, height}",
    ),
    (
        "window.previewPanel",
        "Preview Panel",
        &["Window"],
        "",
        "{on?: bool} open or close the Preview Panel, a window that previews the showing Edit View tab and stays blank while Font View shows (omitted: toggle) → {on}",
    ),
    (
        "preview.invert",
        "Invert",
        &[],
        "",
        "{target?: \"area\" | \"panel\" (default area), on?: bool} light text on a dark ground instead of dark on light (omitted: toggle) → {target, on}",
    ),
    (
        "preview.flip",
        "Flip",
        &[],
        "",
        "{target?: \"area\" | \"panel\" (default area), on?: bool} draw the preview upside down (omitted: toggle) → {target, on}",
    ),
    (
        "preview.setBlur",
        "Blur",
        &[],
        "",
        "{target?: \"area\" | \"panel\" (default area), amount: points} blur the preview by `amount` screen points, held to 0..10; 0 is sharp → {target, amount}",
    ),
    (
        "preview.alwaysCenterActiveGlyph",
        "Always Center Active Glyph",
        &[],
        "",
        "{target?: \"area\" | \"panel\" (default area), on?: bool} on: the edited glyph stays in the middle of the preview; off: the text keeps flush with the left edge (right edge for right-to-left text) where it fits, and fills the width where it does not (omitted: toggle) → {target, on}",
    ),
    (
        "preview.setOffset",
        "Scroll Preview",
        &[],
        "",
        "{target?: \"area\" | \"panel\" (default area), x?: points, dx?: points} shift the preview line sideways to `x` screen points from where it would sit, or by `dx` more → {target, x}",
    ),
    (
        "preview.setInstance",
        "Preview Instance",
        &[],
        "",
        "{target?: \"area\" | \"panel\" (default area), instance: name | index | \"master\" | \"all\"} choose what the preview draws: a static instance (by name, `#n` or index from 0), interpolated as its static instance would be; \"master\" (the dash) for the master Edit View draws, following each master switch; \"all\" for the edited glyph once per visible instance. An instance or the dash also becomes the instance the TrueType Instructor previews → {target, instance, index (null for master and all)}",
    ),
    (
        "preview.showAllInstances",
        "Show All Instances",
        &[],
        "",
        "{target?: \"area\" | \"panel\" (default area)} draw the edited glyph once for every static instance whose eye is on (preview.toggleInstanceVisibility), side by side, instead of the tab's text → {target, instance: \"all\"}",
    ),
];

pub fn run(app: &mut FontcraftApp, id: &str, args: &Args) -> Result<Value, String> {
    match id {
        "view.togglePreviewArea" => {
            let index = super::view::target_tab(app, args)?;
            let tab = app.state.tabs.get(index).map(|t| t.id).ok_or_else(|| args.err("no such tab"))?;
            let height = args.number("height")?;
            let ids: Vec<u64> = app.state.tabs.iter().map(|t| t.id).collect();
            let tokens = &app.tokens;
            let preview = &mut app.state.panels.preview;
            let on = args.toggle(preview.area_tabs.contains(&tab))?;
            // Closed tabs drop out, so the list never outgrows the tab strip.
            preview.area_tabs.retain(|t| *t != tab && ids.contains(t));
            if on {
                preview.area_tabs.push(tab);
            }
            if let Some(h) = height {
                preview.area_height = Some(h);
                preview.area_height = Some(preview.area_height(tokens));
            }
            Ok(json!({"tab": tab, "on": on, "height": preview.area_height(tokens)}))
        }
        "window.previewPanel" => {
            let preview = &mut app.state.panels.preview;
            preview.panel_open = args.toggle(preview.panel_open)?;
            Ok(json!({"on": preview.panel_open}))
        }
        "preview.invert" | "preview.flip" | "preview.alwaysCenterActiveGlyph" => {
            let target = Target::from_args(args)?;
            let options = app.state.panels.preview.options_mut(target);
            let slot = match id {
                "preview.invert" => &mut options.invert,
                "preview.flip" => &mut options.flip,
                _ => &mut options.always_center,
            };
            *slot = args.toggle(*slot)?;
            Ok(json!({"target": target.name(), "on": *slot}))
        }
        "preview.setBlur" => {
            let target = Target::from_args(args)?;
            let amount = args.number("amount")?.ok_or_else(|| args.err("give `amount` (screen points, 0 to 10)"))?.clamp(0.0, MAX_BLUR);
            app.state.panels.preview.options_mut(target).blur = amount;
            Ok(json!({"target": target.name(), "amount": amount}))
        }
        "preview.setOffset" => {
            let target = Target::from_args(args)?;
            let options = app.state.panels.preview.options_mut(target);
            let x = match (args.number("x")?, args.number("dx")?) {
                (Some(_), Some(_)) => return Err(args.err("give `x` or `dx`, not both")),
                (Some(x), None) => x,
                (None, Some(dx)) => options.offset() + dx,
                (None, None) => return Err(args.err("give `x` or `dx` (screen points)")),
            };
            options.offset = x.clamp(-MAX_OFFSET, MAX_OFFSET);
            Ok(json!({"target": target.name(), "x": options.offset}))
        }
        "preview.setInstance" => {
            let target = Target::from_args(args)?;
            let font = app.session.font().map_err(|e| e.to_string())?;
            let given = args.get("instance").ok_or_else(|| args.err("give `instance`: a name, an index, \"master\" or \"all\""))?;
            let index = match given {
                Value::String(s) if s == "master" || s == "all" => None,
                Value::String(s) => Some(instance_index(font, s).ok_or_else(|| args.err(format!("the font has no instance `{s}`")))?),
                Value::Number(n) => Some(
                    n.as_u64()
                        .and_then(|i| usize::try_from(i).ok())
                        .filter(|i| *i < font.instances.len())
                        .ok_or_else(|| args.err(format!("the font has {} instances; there is no instance {n}", font.instances.len())))?,
                ),
                _ => return Err(args.err("`instance` must be a name, an index, \"master\" or \"all\"")),
            };
            let shows = match (index, given.as_str()) {
                (Some(i), _) if !is_static(font, i) => {
                    return Err(args.err(format!("`{}` is a variable font setting; the preview shows static instances", instance_key(font, i))));
                }
                (Some(i), _) => Shows::Instance(instance_key(font, i)),
                (None, Some("all")) => Shows::All,
                (None, _) => Shows::Master,
            };
            match &shows {
                Shows::Master => app.state.tt_instructor.instance = None,
                Shows::Instance(key) => app.state.tt_instructor.instance = Some(key.clone()),
                Shows::All => {}
            }
            let name = shows.name().to_string();
            *app.state.panels.preview.shows_mut(target) = shows;
            Ok(json!({"target": target.name(), "instance": name, "index": index}))
        }
        "preview.showAllInstances" => {
            let target = Target::from_args(args)?;
            *app.state.panels.preview.shows_mut(target) = Shows::All;
            Ok(json!({"target": target.name(), "instance": "all"}))
        }
        other => Err(format!("unknown command `{other}`")),
    }
}

/// The strip's lower and upper edge in font units for a master
/// ([`fontcraft_engine::cmd::preview::preview_range`]).
pub fn vertical_range(font: &Font, master: &Master) -> (f64, f64) {
    preview_range(font, master)
}

/// Where a preview draws the tab's line: font units to screen points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    /// The strip.
    pub rect: Rect,
    /// Screen points per font unit.
    pub scale: f64,
    /// The screen x of the line's origin and the screen y of its baseline, before flipping.
    pub x0: f64,
    pub baseline: f64,
    pub flip: bool,
}

impl Placement {
    /// The screen point of a font point.
    pub fn to_screen(&self, x: f64, y: f64) -> Pos2 {
        let p = pos2((self.x0 + x * self.scale) as f32, (self.baseline - y * self.scale) as f32);
        if self.flip { self.turn(p) } else { p }
    }

    /// The font point under a screen point.
    pub fn to_font(&self, p: Pos2) -> (f64, f64) {
        let p = if self.flip { self.turn(p) } else { p };
        ((f64::from(p.x) - self.x0) / self.scale, (self.baseline - f64::from(p.y)) / self.scale)
    }

    /// `p` turned half a circle about the strip's centre.
    fn turn(&self, p: Pos2) -> Pos2 {
        pos2(self.rect.min.x + self.rect.max.x - p.x, self.rect.min.y + self.rect.max.y - p.y)
    }
}

/// The line's extent along x in font units: from 0 rightwards, or ending at 0 for right-to-left
/// text.
fn extent(layout: &Layout) -> (f64, f64) {
    let length = layout.lines.first().map_or(0.0, |l| l.length);
    let length = if length.is_finite() { length.max(0.0) } else { 0.0 };
    if layout.direction == Direction::Rtl { (-length, 0.0) } else { (0.0, length) }
}

/// Where the line goes in `rect`: scaled so `range` (lower, upper edge) fills the height, and
/// placed along x by the options. `edited` is the item of the edited glyph. `None` for an empty
/// strip or a range that encloses nothing.
pub fn place(layout: &Layout, edited: Option<usize>, rect: Rect, range: (f64, f64), options: &PreviewOptions, margin: f64) -> Option<Placement> {
    let focus = edited.and_then(|i| layout.placed.get(i)).map(|p| p.x + p.advance / 2.0);
    place_line(extent(layout), focus, layout.direction == Direction::Rtl, rect, range, options, margin)
}

/// [`place`] for a line reaching over `(left, right)` in font units; `focus` is the x the
/// middle keeps (the line's centre when `None`).
fn place_line(
    (left, right): (f64, f64),
    focus: Option<f64>,
    rtl: bool,
    rect: Rect,
    range: (f64, f64),
    options: &PreviewOptions,
    margin: f64,
) -> Option<Placement> {
    let (low, high) = range;
    let span = high - low;
    let (w, h) = (f64::from(rect.width()), f64::from(rect.height()));
    if !(span > 0.0 && w > 0.0 && h > 0.0 && span.is_finite()) {
        return None;
    }
    let scale = h / span;
    let focus = focus.filter(|f| f.is_finite()).unwrap_or((left + right) / 2.0);
    let centred = w / 2.0 - focus * scale;
    let margin = if margin.is_finite() { margin.max(0.0) } else { 0.0 };
    let start = if options.always_center {
        centred
    } else if (right - left) * scale <= w - 2.0 * margin {
        if rtl { w - margin - right * scale } else { margin - left * scale }
    } else {
        // Too long to fit: as near the middle as the edges allow, never leaving a gap at either.
        centred.max(w - margin - right * scale).min(margin - left * scale)
    };
    let x0 = f64::from(rect.min.x) + start + options.offset();
    let baseline = f64::from(rect.min.y) + high * scale;
    (x0.is_finite() && baseline.is_finite()).then_some(Placement { rect, scale, x0, baseline, flip: options.flip })
}

/// The item whose glyph is under screen point `p`, if any.
pub fn hit(layout: &Layout, tab: &Tab, placement: &Placement, p: Pos2) -> Option<usize> {
    let (x, _) = placement.to_font(p);
    layout
        .placed
        .iter()
        .enumerate()
        .position(|(i, placed)| placed.glyph.is_some() && tab.item_glyph(i).is_some() && x >= placed.x && x < placed.x + placed.advance)
}

/// The direction a preview lays its line out in: the tab's, with vertical text on a horizontal
/// line.
fn direction(tab: &Tab) -> Direction {
    match tab.direction {
        WritingDirection::Rtl => Direction::Rtl,
        WritingDirection::Ltr | WritingDirection::Ttb => Direction::Ltr,
    }
}

/// The one-line layout of a tab's text, in the tab's master with its direction and kerning
/// mode. `None` without a document or a master.
pub fn line_layout(session: &mut Session, tab: &Tab) -> Option<Layout> {
    let index = if tab.kerning.shows_kerning() { session.kern_index().ok() } else { None };
    let font = session.font().ok()?;
    let master = view::tab_master(font, tab)?.id.clone();
    let options = Options {
        direction: direction(tab),
        kerning: tab.kerning,
        single_line: true,
        index: index.as_deref(),
        layers: Some(&tab.layers),
        ..Default::default()
    };
    fontcraft_textlayout::layout(font, &master, &tab.text, &options).ok()
}

/// What a picture is drawn with.
pub struct Picture<'a> {
    pub font: &'a Font,
    pub tab: &'a Tab,
    pub layout: &'a Layout,
    pub placement: &'a Placement,
    /// Pixels per screen point.
    pub ppp: f32,
    /// The document's id and revision, for the composite cache.
    pub doc: (u64, u64),
    pub options: &'a PreviewOptions,
    /// Text and ground before inverting.
    pub ink: Color32,
    pub paper: Color32,
}

/// The strip as an image the size of its rect in pixels: the glyphs filled, inverted, turned
/// and blurred as the options say. `None` for an empty or oversized strip.
pub fn picture(p: &Picture, outlines: &mut CompositeCache) -> Option<ColorImage> {
    picture_with(p, outlines, None)
}

/// [`picture`], with each glyph whose shown layer is a colour layer drawn in colour from
/// `colour`'s files (`color.preview.previewArea`); without `colour` every glyph is filled.
pub fn picture_with(p: &Picture, outlines: &mut CompositeCache, mut colour: Option<&mut crate::canvas::color::Sources>) -> Option<ColorImage> {
    let rect = p.placement.rect;
    let s = p.placement.scale;
    // Glyphs whose advance lies more than an em beyond the strip cannot reach into it.
    let reach = f64::from(p.font.units_per_em) * s;
    let (min_x, max_x) = (f64::from(rect.min.x) - reach, f64::from(rect.max.x) + reach);
    let mut filled = BezPath::new();
    let mut coloured = Vec::new();
    for placed in &p.layout.placed {
        let Some(name) = placed.glyph.as_deref() else { continue };
        let (a, b) = (p.placement.x0 + placed.x * s, p.placement.x0 + (placed.x + placed.advance) * s);
        if b < min_x || a > max_x {
            continue;
        }
        if colour.is_some()
            && let Some(layer) = crate::canvas::color::preview_layer(p.font, p.tab, name)
        {
            coloured.push((name, layer, placed.x, placed.y));
            continue;
        }
        let Some(layer) = view::shown_layer(p.font, p.tab, name) else { continue };
        let Some(composite) = outlines.get(p.doc, p.font, name, layer) else { continue };
        let mut outline = composite.outline.clone();
        outline.apply_affine(Affine::translate((placed.x, placed.y)));
        filled.extend(outline.elements().iter().copied());
    }
    finish(&filled, p.placement, p.ppp, p.options, p.ink, p.paper, &mut |px, to_pixels| {
        if let Some(src) = colour.as_deref_mut() {
            for (name, layer, x, y) in &coloured {
                crate::canvas::color::paint_preview_glyph(p.font, name, layer, src, px, to_pixels * Affine::translate((*x, *y)));
            }
        }
    })
}

/// One glyph of the line of all instances: the instance's view, its master id and the glyph's
/// x in font units.
#[derive(Clone)]
pub struct AllItem {
    pub view: Arc<Font>,
    pub master: String,
    pub x: f64,
}

/// The line of all instances: the edited glyph once per visible instance.
#[derive(Clone, Default)]
pub struct AllLine {
    pub glyph: String,
    pub items: Vec<AllItem>,
    /// The line's length in font units.
    pub length: f64,
}

/// The line of all instances for `glyph` at `instances`, from the instance views of the active
/// document. An instance the glyph cannot be interpolated at is left out.
pub fn all_line(session: &mut Session, glyph: &str, instances: &[usize]) -> AllLine {
    let mut line = AllLine { glyph: glyph.to_string(), ..AllLine::default() };
    let asked = [glyph.to_string()];
    for &i in instances {
        let Ok(view) = session.instance_view(i, &asked) else { continue };
        let master = view_master_id(i);
        let Some(width) = view.glyph(glyph).and_then(|g| g.layer(&master)).map(|l| l.width) else { continue };
        line.items.push(AllItem { view, master, x: line.length });
        line.length += if width.is_finite() { width.max(0.0) } else { 0.0 };
    }
    line
}

/// [`picture`] of the line of all instances.
pub fn picture_all(
    line: &AllLine,
    placement: &Placement,
    ppp: f32,
    doc: (u64, u64),
    style: (&PreviewOptions, Color32, Color32),
    outlines: &mut CompositeCache,
) -> Option<ColorImage> {
    let mut filled = BezPath::new();
    for item in &line.items {
        let Some(layer) = item.view.glyph(&line.glyph).and_then(|g| g.layer(&item.master)) else { continue };
        let Some(composite) = outlines.get(doc, &item.view, &line.glyph, layer) else { continue };
        let mut outline = composite.outline.clone();
        outline.apply_affine(Affine::translate((item.x, 0.0)));
        filled.extend(outline.elements().iter().copied());
    }
    let (options, ink, paper) = style;
    finish(&filled, placement, ppp, options, ink, paper, &mut |_, _| {})
}

/// `filled` (font units on the line) rasterised into the strip, inverted, `over` painted on it
/// (given the pixels and the map from font units on the line to them), turned and blurred.
#[allow(clippy::too_many_arguments)]
fn finish(
    filled: &BezPath,
    placement: &Placement,
    ppp: f32,
    options: &PreviewOptions,
    ink: Color32,
    paper: Color32,
    over: &mut dyn FnMut(&mut crate::canvas::color::Pixels, Affine),
) -> Option<ColorImage> {
    let rect = placement.rect;
    let ppp = if ppp.is_finite() && ppp > 0.0 { ppp } else { 1.0 };
    let (w, h) = ((rect.width() * ppp).round(), (rect.height() * ppp).round());
    if !(w >= 1.0 && h >= 1.0) {
        return None;
    }
    let (width, height) = (w as u32, h as u32);
    let s = placement.scale;
    let to_pixels = Affine::scale(f64::from(ppp))
        * Affine::translate((placement.x0 - f64::from(rect.min.x), placement.baseline - f64::from(rect.min.y)))
        * Affine::scale_non_uniform(s, -s);
    let (ink, paper) = if options.invert { (paper, ink) } else { (ink, paper) };
    let style = Style { ink: ink.to_srgba_unmultiplied(), background: paper.to_srgba_unmultiplied(), anti_alias: true };
    let image = render_path(filled, to_pixels, width, height, &style).ok()?;
    let mut pixels = image.pixels;
    over(&mut crate::canvas::color::Pixels { data: &mut pixels, width, height }, to_pixels);
    if options.flip {
        pixels = turn_pixels(&pixels);
    }
    blur_pixels(&mut pixels, width as usize, height as usize, blur_radius(options.blur() * f64::from(ppp)));
    Some(ColorImage::from_rgba_premultiplied([width as usize, height as usize], &pixels))
}

/// A preview's layouts and texture, and what they were made from, kept in egui's memory.
#[derive(Clone, Default)]
struct Cache {
    layout_key: Option<u64>,
    layout: Option<Arc<Layout>>,
    /// The line at an instance: the instance view it was laid out in and the layout.
    instance_key: Option<u64>,
    instance: Option<(Arc<Font>, Arc<Layout>)>,
    /// The line of all instances.
    all_key: Option<u64>,
    all: Option<Arc<AllLine>>,
    picture_key: Option<u64>,
    texture: Option<TextureHandle>,
}

fn cache_id(target: Target) -> egui::Id {
    egui::Id::new(("fontcraft.preview", target))
}

fn load_cache(ctx: &egui::Context, target: Target) -> Cache {
    ctx.data(|d| d.get_temp::<Cache>(cache_id(target))).unwrap_or_default()
}

fn save_cache(ctx: &egui::Context, target: Target, cache: Cache) {
    ctx.data_mut(|d| d.insert_temp(cache_id(target), cache));
}

/// The texture a preview drew last.
#[cfg(test)]
pub(crate) fn texture_id(ctx: &egui::Context, target: Target) -> Option<egui::TextureId> {
    ctx.data(|d| d.get_temp::<Cache>(cache_id(target))).and_then(|c| c.texture.map(|t| t.id()))
}

/// The line of all instances a preview drew last.
#[cfg(test)]
pub(crate) fn drawn_all(ctx: &egui::Context, target: Target) -> Option<Arc<AllLine>> {
    ctx.data(|d| d.get_temp::<Cache>(cache_id(target))).and_then(|c| c.all)
}

/// The instance view and layout a preview drew last.
#[cfg(test)]
pub(crate) fn drawn_instance(ctx: &egui::Context, target: Target) -> Option<(Arc<Font>, Arc<Layout>)> {
    ctx.data(|d| d.get_temp::<Cache>(cache_id(target))).and_then(|c| c.instance)
}

fn hash(parts: impl Hash, floats: &[f64]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    parts.hash(&mut hasher);
    for f in floats {
        f.to_bits().hash(&mut hasher);
    }
    hasher.finish()
}

/// The tab's one-line layout for `target`, from the cache while nothing it depends on changed.
fn cached_layout(app: &mut FontcraftApp, ctx: &egui::Context, target: Target, tab: &Tab) -> Option<Arc<Layout>> {
    let doc = app.session.active().map(|d| (d.id, d.revision))?;
    let key = hash((doc, tab.id, tab.text.revision(), &tab.master, &tab.layers, tab.direction, tab.kerning), &[]);
    let mut cache = load_cache(ctx, target);
    if cache.layout_key == Some(key)
        && let Some(layout) = &cache.layout
    {
        return Some(layout.clone());
    }
    let layout = line_layout(&mut app.session, tab).map(Arc::new);
    cache.layout_key = Some(key);
    cache.layout.clone_from(&layout);
    save_cache(ctx, target, cache);
    layout
}

/// The glyphs a layout draws, each once, in order.
fn layout_glyphs(layout: &Layout) -> Vec<String> {
    let mut seen = HashSet::new();
    layout.placed.iter().filter_map(|p| p.glyph.as_deref()).filter(|g| seen.insert(*g)).map(str::to_string).collect()
}

/// The tab's text laid out on one line in the view of instance `instance`, with the instance's
/// widths and kerning. `None` when the view cannot be built.
pub fn instance_layout(session: &mut Session, tab: &Tab, master_layout: &Layout, instance: usize) -> Option<(Arc<Font>, Arc<Layout>)> {
    let view = session.instance_view(instance, &layout_glyphs(master_layout)).ok()?;
    let options = Options { direction: direction(tab), kerning: tab.kerning, single_line: true, ..Default::default() };
    let layout = fontcraft_textlayout::layout(&view, &view_master_id(instance), &tab.text, &options).ok()?;
    Some((view, Arc::new(layout)))
}

fn cached_instance(
    app: &mut FontcraftApp,
    ctx: &egui::Context,
    target: Target,
    tab: &Tab,
    master_layout: &Layout,
    instance: usize,
) -> Option<(Arc<Font>, Arc<Layout>)> {
    let doc = app.session.active().map(|d| (d.id, d.revision))?;
    let key = hash((doc, tab.id, tab.text.revision(), tab.direction, tab.kerning, instance), &[]);
    let mut cache = load_cache(ctx, target);
    if cache.instance_key == Some(key) {
        return cache.instance;
    }
    let line = instance_layout(&mut app.session, tab, master_layout, instance);
    cache.instance_key = Some(key);
    cache.instance.clone_from(&line);
    save_cache(ctx, target, cache);
    line
}

fn cached_all(app: &mut FontcraftApp, ctx: &egui::Context, target: Target, glyph: &str, instances: &[usize]) -> Option<Arc<AllLine>> {
    let doc = app.session.active().map(|d| (d.id, d.revision))?;
    let key = hash((doc, glyph, instances), &[]);
    let mut cache = load_cache(ctx, target);
    if cache.all_key == Some(key) {
        return cache.all;
    }
    let line = Arc::new(all_line(&mut app.session, glyph, instances));
    cache.all_key = Some(key);
    cache.all = Some(Arc::clone(&line));
    save_cache(ctx, target, cache);
    Some(line)
}

/// What a frame's strip is drawn from, after its line was placed.
enum Line {
    /// The tab's text, in the master (`None` view) or in an instance's view.
    Text {
        view: Option<(usize, Arc<Font>)>,
        layout: Arc<Layout>,
    },
    All(Arc<AllLine>),
}

/// Draws a preview of `tab` in `rect` and takes its input: dragging shifts the line, a
/// double-click edits the glyph under the pointer, the context menu holds Always Center Active
/// Glyph.
pub(crate) fn strip(app: &mut FontcraftApp, ui: &mut egui::Ui, rect: Rect, target: Target, tab: &Tab, pending: &mut Pending) {
    let options = *app.state.panels.preview.options(target);
    let (ink, paper) = (app.tokens.preview_ink, app.tokens.preview_paper);
    let (ground, _) = if options.invert { (ink, paper) } else { (paper, ink) };
    ui.painter_at(rect).rect_filled(rect, 0.0, ground);
    let response = ui.interact(rect, widgets::id(("preview.strip", target.name())), Sense::click_and_drag());
    let ctx = ui.ctx().clone();
    let margin = f64::from(app.tokens.preview_margin);
    let Some(layout) = cached_layout(app, &ctx, target, tab) else { return };
    let (range, shown) = {
        let Ok(font) = app.session.font() else { return };
        let Some(master) = view::tab_master(font, tab) else { return };
        (vertical_range(font, master), drawn(font, app.state.panels.preview.shows(target)))
    };
    let line = match &shown {
        Drawn::Master => Line::Text { view: None, layout },
        Drawn::Instance(i) => match cached_instance(app, &ctx, target, tab, &layout, *i) {
            Some((view, layout)) => Line::Text { view: Some((*i, view)), layout },
            None => return,
        },
        Drawn::All(list) => {
            let Some(glyph) = tab.edited_glyph() else { return };
            match cached_all(app, &ctx, target, glyph, list) {
                Some(line) => Line::All(line),
                None => return,
            }
        }
    };
    let placement = match &line {
        Line::Text { layout, .. } => place(layout, tab.edited_index(), rect, range, &options, margin),
        Line::All(all) => place_line((0.0, all.length), None, false, rect, range, &options, margin),
    };
    let Some(placement) = placement else { return };
    let doc = app.session.active().map(|d| (d.id, d.revision)).unwrap_or_default();
    let ppp = ctx.pixels_per_point();
    let what = match &shown {
        Drawn::Master => hash((0u8, &tab.master, &tab.layers), &[]),
        Drawn::Instance(i) => hash((1u8, *i), &[]),
        Drawn::All(list) => hash((2u8, list, tab.edited_glyph()), &[]),
    };
    let key = hash(
        // The colour glyphs read image and svg assets, which change without a document revision.
        (
            doc,
            tab.id,
            tab.text.revision(),
            what,
            tab.direction,
            tab.kerning,
            options.invert,
            options.flip,
            ink.to_array(),
            paper.to_array(),
            app.images.revision,
        ),
        &[
            placement.scale,
            placement.x0,
            placement.baseline,
            f64::from(rect.min.x),
            f64::from(rect.min.y),
            f64::from(rect.width()),
            f64::from(rect.height()),
            f64::from(ppp),
            options.blur(),
        ],
    );
    let mut cache = load_cache(&ctx, target);
    if cache.picture_key != Some(key) {
        let image = match &line {
            Line::Text { view: None, layout } => {
                let Ok(font) = app.session.font() else { return };
                let p = Picture { font, tab, layout, placement: &placement, ppp, doc, options: &options, ink, paper };
                let assets = app.session.assets();
                let mut colour = crate::canvas::color::Sources {
                    assets: assets.as_ref(),
                    svgs: &mut app.color_ui.svgs,
                    revision: app.images.revision,
                    text: if options.invert { paper } else { ink },
                };
                picture_with(&p, &mut app.fill_cache.outlines, Some(&mut colour))
            }
            Line::Text { view: Some((i, view)), layout } => {
                // The instance view's one master stands in for the tab's.
                let mut shown_tab = tab.clone();
                shown_tab.master = Some(view_master_id(*i));
                shown_tab.layers.clear();
                let p = Picture { font: view, tab: &shown_tab, layout, placement: &placement, ppp, doc, options: &options, ink, paper };
                picture(&p, &mut app.fill_cache.outlines)
            }
            Line::All(all) => picture_all(all, &placement, ppp, doc, (&options, ink, paper), &mut app.fill_cache.outlines),
        };
        cache.picture_key = Some(key);
        cache.texture = image.map(|image| ctx.load_texture(format!("preview-{}", target.name()), image, TextureOptions::LINEAR));
        save_cache(&ctx, target, cache.clone());
    }
    if let Some(texture) = &cache.texture {
        ui.painter_at(rect).image(texture.id(), rect, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
    }
    if response.dragged_by(egui::PointerButton::Primary) {
        let dx = f64::from(response.drag_delta().x);
        if dx != 0.0 {
            pending.push(("preview.setOffset", json!({"target": target.name(), "dx": if options.flip { -dx } else { dx }})));
        }
    }
    if let Line::Text { layout, .. } = &line
        && response.double_clicked()
        && let Some(at) = response.interact_pointer_pos()
        && let Some(index) = hit(layout, tab, &placement, at)
    {
        pending.push(("editView.setCursor", json!({"tab": tab.id, "index": index})));
    }
    response.context_menu(|ui| {
        let mut on = options.always_center;
        if ui.checkbox(&mut on, "Always Center Active Glyph").clicked() {
            pending.push(("preview.alwaysCenterActiveGlyph", json!({"target": target.name(), "on": on})));
            ui.close();
        }
    });
}

/// What a preview's instance pop-up lists.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct InstanceMenu {
    pub shows: Shows,
    /// The button's label: the dash, the instance's name or Show All Instances.
    pub label: String,
    /// Each static instance: its position, name and eye.
    pub instances: Vec<(usize, String, bool)>,
}

/// The instance pop-up of `target` for the active document.
pub fn instance_menu(app: &FontcraftApp, target: Target) -> InstanceMenu {
    let shows = app.state.panels.preview.shows(target).clone();
    let Ok(font) = app.session.font() else { return InstanceMenu { shows, label: DASH.to_string(), instances: Vec::new() } };
    let instances: Vec<(usize, String, bool)> =
        font.instances.iter().enumerate().filter(|(i, _)| is_static(font, *i)).map(|(i, inst)| (i, instance_key(font, i), inst.visible)).collect();
    let label = match drawn(font, &shows) {
        Drawn::Master => DASH.to_string(),
        Drawn::Instance(i) => instance_key(font, i),
        Drawn::All(_) => SHOW_ALL.to_string(),
    };
    InstanceMenu { shows, label, instances }
}

/// The pop-up entry that follows Edit View's master.
pub const DASH: &str = "-";
const SHOW_ALL: &str = "Show All Instances";

/// The id of a preview's instance pop-up button.
pub fn instance_button_id(target: Target) -> egui::Id {
    widgets::id(("preview.instanceMenu", target.name()))
}

/// The instance pop-up: the dash, each static instance with its eye, and Show All Instances.
fn instance_popup(ui: &mut egui::Ui, target: Target, menu: &InstanceMenu, pending: &mut Pending) {
    let t = target.name();
    let button = crate::panels::attributes::menu_button(ui, instance_button_id(target), &menu.label, "The instance the preview shows");
    egui::Popup::menu(&button).id(instance_button_id(target).with("popup")).show(|ui| {
        if widgets::button(
            ui,
            widgets::id(("preview.instance", t, "master")),
            DASH,
            "Follow the master Edit View shows",
            true,
            menu.shows == Shows::Master,
        ) {
            pending.push(("preview.setInstance", json!({"target": t, "instance": "master"})));
            ui.close();
        }
        for (index, name, visible) in &menu.instances {
            ui.horizontal(|ui| {
                let eye_hover =
                    if *visible { "Leave this instance out of Show All Instances" } else { "Include this instance in Show All Instances" };
                if widgets::icon_button(ui, widgets::id(("preview.instanceEye", t, *index)), Icon::Preview, eye_hover, true, *visible) {
                    pending.push(("preview.toggleInstanceVisibility", json!({ "instance": *index })));
                }
                let chosen = menu.shows == Shows::Instance(name.clone());
                if widgets::button(ui, widgets::id(("preview.instance", t, *index)), name, "Preview this instance", true, chosen) {
                    pending.push(("preview.setInstance", json!({"target": t, "instance": *index})));
                    ui.close();
                }
            });
        }
        ui.separator();
        if widgets::button(
            ui,
            widgets::id(("preview.instance", t, "all")),
            SHOW_ALL,
            "The edited glyph in every instance",
            true,
            menu.shows == Shows::All,
        ) {
            pending.push(("preview.showAllInstances", json!({ "target": t })));
            ui.close();
        }
    });
}

/// A preview's controls row: the instance pop-up, Invert, Flip and the blur slider.
pub(crate) fn controls(ui: &mut egui::Ui, target: Target, options: &PreviewOptions, menu: &InstanceMenu, pending: &mut Pending) {
    ui.horizontal_centered(|ui| {
        let t = target.name();
        instance_popup(ui, target, menu, pending);
        if widgets::button(ui, widgets::id(("preview.invert", t)), "Invert", "Light text on a dark ground", true, options.invert) {
            pending.push(("preview.invert", json!({ "target": t })));
        }
        if widgets::button(ui, widgets::id(("preview.flip", t)), "Flip", "Turn the preview upside down", true, options.flip) {
            pending.push(("preview.flip", json!({ "target": t })));
        }
        let mut amount = options.blur();
        let slider = egui::Slider::new(&mut amount, 0.0..=MAX_BLUR).show_value(false);
        if ui.add(slider).on_hover_text("Blur").changed() {
            pending.push(("preview.setBlur", json!({"target": t, "amount": amount})));
        }
    });
}

/// Draws the Preview Area under the showing tab's canvas while that tab has it open: the
/// separator (drag to change the height), the strip and the controls. Call before the canvas,
/// which takes the space left.
pub fn area(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    if crate::panels::hinting_waterfall::area(app, ui, pending) {
        return;
    }
    let Some(tab) = app.state.tab() else { return };
    if !app.state.panels.preview.area_tabs.contains(&tab.id) {
        return;
    }
    let tab = tab.clone();
    let height = app.state.panels.preview.area_height(&app.tokens);
    let options = *app.state.panels.preview.options(Target::Area);
    let menu = instance_menu(app, Target::Area);
    let (separator, row, fill, line) = (app.tokens.preview_separator, app.tokens.preview_controls_height, app.tokens.app_bar, app.tokens.border);
    let frame = egui::Frame::NONE.fill(fill);
    egui::Panel::bottom("edit_view_preview_area").resizable(false).exact_size(height as f32).frame(frame).show(ui, |ui| {
        let full = ui.max_rect();
        ui.allocate_rect(full, Sense::hover());
        let grab = Rect::from_min_size(full.min, vec2(full.width(), separator));
        let controls_rect = Rect::from_min_max(pos2(full.min.x, (full.max.y - row).max(grab.max.y)), full.max);
        let strip_rect = Rect::from_min_max(pos2(full.min.x, grab.max.y), pos2(full.max.x, controls_rect.min.y));
        let drag = ui.interact(grab, widgets::id("previewArea.separator"), Sense::drag());
        ui.painter().hline(grab.x_range(), grab.center().y, egui::Stroke::new(1.0, line));
        if drag.hovered() || drag.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
        }
        let dy = f64::from(drag.drag_delta().y);
        if drag.dragged() && dy != 0.0 {
            pending.push(("view.togglePreviewArea", json!({"tab": tab.id, "on": true, "height": height - dy})));
        }
        if strip_rect.height() > 0.0 {
            strip(app, ui, strip_rect, Target::Area, &tab, pending);
        }
        ui.scope_builder(egui::UiBuilder::new().max_rect(controls_rect), |ui| controls(ui, Target::Area, &options, &menu, pending));
    });
}

/// The Preview button for the bottom bar: shows or hides the showing tab's Preview Area.
pub fn bottom_bar_button(app: &FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    let Some(tab) = app.state.tab() else { return };
    let on = app.state.panels.preview.area_tabs.contains(&tab.id);
    if widgets::icon_button(ui, widgets::id("bottomBar.preview"), Icon::Preview, "Show or hide the Preview Area", true, on) {
        pending.push(("view.togglePreviewArea", json!({ "tab": tab.id })));
    }
}
