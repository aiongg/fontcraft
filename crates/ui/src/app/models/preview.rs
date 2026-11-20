//! The two previews' lines: what the Preview Area under the edit view and the Preview Panel draw.
//!
//! A preview draws the edit view's text filled, on one line without wrapping (`preview.singleLine`), in the text's
//! direction (vertical text on a horizontal line) and kerning mode. While the canvas shows a glyph the text does not
//! hold, the line is that glyph alone. What the line shows is each preview's own choice (`preview.setInstance`):
//!
//! | Choice | The line |
//! |---|---|
//! | the dash (`master`) | the text in the current master and the picked layers, following every master switch |
//! | an instance | the text laid out in the document's instance view: the instance's widths and kerning |
//! | Show All Instances (`all`) | the edited glyph once for each static instance whose eye is on |
//!
//! Variable font settings are not instances here: the pop-up lists static instances only. The model holds the line in
//! font units; the view places it in the strip it gets ([`place`]) and fills it.

use std::collections::HashSet;
use std::hash::{Hash, Hasher};

use craft::behavior::dock::PanelState;
use craft::ui::contract::{PlainData, Tracked};
use fontcraft_engine::Session;
use fontcraft_engine::cmd::preview::preview_range;
use fontcraft_engine::interp_cache::view_master_id;
use fontcraft_engine::model::Font;
use fontcraft_engine::prelude::*;
use fontcraft_geom::{Affine, BezPath};
use fontcraft_textlayout::{Direction, Item, Layout, Options, Text};

use super::ProjectCx;
use crate::app::ViewState;
use crate::canvas::text::{self, WritingDirection};
use crate::shell::dock::PanelId;
use crate::shell::preview::{PreviewOptions, Shows, Target};

/// Most glyphs one line draws; a longer text draws its first ones.
pub const MAX_LINE_GLYPHS: usize = 2000;

/// One glyph of the line the double-click can hit: the text item and its advance along the line, in font units.
#[derive(PlainData, Clone, Debug, PartialEq)]
pub struct HitItem {
    pub item: usize,
    pub x: f64,
    pub advance: f64,
}

/// One static instance in the pop-up.
#[derive(PlainData, Clone, Debug, PartialEq)]
pub struct InstanceEntry {
    /// The position in the font's instance list.
    pub index: usize,
    /// Its name, or `#n` when the name does not find it.
    pub name: String,
    /// Its eye: shown by Show All Instances.
    pub visible: bool,
    /// The preview shows it.
    pub chosen: bool,
}

/// What one preview draws and its controls show.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct PreviewModel {
    /// There is a line to draw; the strip is blank otherwise.
    pub shown: bool,
    /// Moves with whatever the line is made from: the texture key.
    pub key: u64,
    /// Every glyph of the line, placed, in font units on the line (baseline at y = 0).
    #[plain_data(assume = "a kurbo path: owned points")]
    pub outline: BezPath,
    /// The line's extent along x: from 0 rightwards, or ending at 0 for right-to-left text.
    pub extent: [f64; 2],
    /// The middle of the edited glyph, which Always Center Active Glyph keeps in the middle; `None`: the line's.
    pub focus: Option<f64>,
    pub rtl: bool,
    /// The strip's lower and upper edge in font units (`Preview Descender`, `Preview Ascender`).
    pub range: [f64; 2],
    /// The glyphs a double-click edits; empty while the line is not the edit view's text.
    pub hits: Vec<HitItem>,
    pub options: PreviewOptions,
    /// The instance pop-up: what it shows, each static instance, and which choice is on.
    pub instances: Vec<InstanceEntry>,
    pub master_chosen: bool,
    pub all_chosen: bool,
    /// The chosen instance's name; `None` for the dash and Show All Instances.
    pub instance_label: Option<String>,
}

/// Projects the Preview Area's line while it is open and the Preview Panel's while the panel shows; a hidden preview
/// holds nothing.
pub fn project_both(area: &mut Tracked<PreviewModel>, panel: &mut Tracked<PreviewModel>, cx: &mut ProjectCx<'_>) {
    let panel_shows = cx.view.workspace.layout.panel_state(&PanelId::Preview) != PanelState::Hidden;
    for (model, target, shown) in [(area, Target::Area, cx.view.preview.area_open), (panel, Target::Panel, panel_shows)] {
        let value = if shown { build(cx.session, cx.view, target).unwrap_or_default() } else { PreviewModel::default() };
        super::set(model, value);
    }
}

/// Whether the instance at `index` is a static instance, not a variable font setting.
pub fn is_static(font: &Font, index: usize) -> bool {
    font.instances.get(index).is_some_and(|i| i.kind.as_deref() != Some("variable"))
}

/// The position of the instance `name` names: by name, or `#n`.
pub fn instance_index(font: &Font, name: &str) -> Option<usize> {
    match name.strip_prefix('#') {
        Some(n) => n.parse::<usize>().ok().filter(|&i| i < font.instances.len()),
        None => font.instances.iter().position(|i| i.name == name),
    }
}

/// How [`Shows::Instance`] names the instance at `index`: its name when that finds it, else `#index`.
pub fn instance_key(font: &Font, index: usize) -> String {
    match font.instances.get(index) {
        Some(i) if !i.name.is_empty() && instance_index(font, &i.name) == Some(index) => i.name.clone(),
        _ => format!("#{index}"),
    }
}

/// What a preview draws: [`Shows`] resolved against the font. An instance that is gone draws the master.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Drawn {
    Master,
    Instance(usize),
    /// The static instances whose eye is on, in font order.
    All(Vec<usize>),
}

pub fn drawn(font: &Font, shows: &Shows) -> Drawn {
    match shows {
        Shows::Master => Drawn::Master,
        Shows::Instance(key) => instance_index(font, key).filter(|i| is_static(font, *i)).map_or(Drawn::Master, Drawn::Instance),
        Shows::All => {
            Drawn::All((0..font.instances.len()).filter(|i| is_static(font, *i) && font.instances.get(*i).is_some_and(|x| x.visible)).collect())
        }
    }
}

/// The text the previews lay out and the item of the edited glyph: the edit view's text while it shows the edited
/// glyph, else that glyph alone. The flag says whether it is the edit view's text.
fn line_text(font: &Font, view: &ViewState) -> Option<(Text, usize, bool)> {
    if text::shows_text(font, view) {
        return Some((view.text.text.clone(), view.text.edited_index()?, true));
    }
    let shown = crate::canvas::tools::edited_glyph(font, view)?;
    let typed = font.glyph(shown).and_then(|g| g.unicodes.first().copied()).and_then(char::from_u32);
    let mut alone = Text::new();
    alone.replace(0..0, vec![Item::Glyph { name: shown.into(), typed }]).ok()?;
    Some((alone, 0, false))
}

fn direction(view: &ViewState) -> Direction {
    match view.text.direction {
        WritingDirection::Rtl => Direction::Rtl,
        WritingDirection::Ltr | WritingDirection::Ttb => Direction::Ltr,
    }
}

fn build(session: &mut Session, view: &ViewState, target: Target) -> Option<PreviewModel> {
    let state = &view.preview;
    let shows = state.shows(target).clone();
    let options = *state.options(target);
    let index = if view.text.kerning.shows_kerning() { session.kern_index().ok() } else { None };
    let doc = session.active().map(|d| (d.id, d.revision))?;
    let font = session.font().ok()?;
    let master = crate::shell::masters::current_master(font, &view.layers)?;
    let (low, high) = preview_range(font, master);
    let master = master.id.clone();
    let (text, edited, is_text) = line_text(font, view)?;
    let drawn = drawn(font, &shows);
    let instances: Vec<InstanceEntry> = (0..font.instances.len())
        .filter(|i| is_static(font, *i))
        .map(|i| InstanceEntry {
            index: i,
            name: instance_key(font, i),
            visible: font.instances.get(i).is_some_and(|x| x.visible),
            chosen: drawn == Drawn::Instance(i),
        })
        .collect();
    let instance_label = match drawn {
        Drawn::Instance(i) => Some(instance_key(font, i)),
        _ => None,
    };
    let dir = direction(view);
    let layout_options = Options {
        direction: dir,
        kerning: view.text.kerning,
        single_line: true,
        index: index.as_deref(),
        layers: Some(&view.layers.layers),
        ..Default::default()
    };
    let master_layout = fontcraft_textlayout::layout(font, &master, &text, &layout_options).ok()?;
    let mut model = PreviewModel {
        shown: true,
        rtl: dir == Direction::Rtl,
        range: [low, high],
        options,
        instances,
        master_chosen: drawn == Drawn::Master,
        all_chosen: matches!(drawn, Drawn::All(_)),
        instance_label,
        ..PreviewModel::default()
    };
    let edited_glyph = text_glyph(&text, edited);
    match &drawn {
        Drawn::Master => fill_line(&mut model, font, &master, &master_layout, edited, is_text),
        Drawn::Instance(i) => {
            let glyphs = layout_glyphs(&master_layout);
            let instance_view = session.instance_view(*i, &glyphs).ok()?;
            let at = view_master_id(*i);
            let options = Options { direction: dir, kerning: view.text.kerning, single_line: true, ..Default::default() };
            let layout = fontcraft_textlayout::layout(&instance_view, &at, &text, &options).ok()?;
            fill_line(&mut model, &instance_view, &at, &layout, edited, is_text);
        }
        Drawn::All(list) => {
            let glyph = edited_glyph.clone()?;
            all_line(&mut model, session, &glyph, list);
        }
    }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (doc, view.text.revision, &shows, &master, &view.layers.layers, view.text.direction.name(), view.text.kerning.name(), &drawn).hash(&mut hasher);
    edited_glyph.hash(&mut hasher);
    model.key = hasher.finish();
    Some(model)
}

/// The glyph item `index` of `text` shows; a placeholder shows the current glyph.
fn text_glyph(text: &Text, index: usize) -> Option<String> {
    match text.items().get(index)? {
        Item::Glyph { name, .. } => Some(name.to_string()),
        Item::Placeholder => text.current_glyph().map(str::to_string),
        Item::Missing(_) | Item::LineBreak => None,
    }
}

/// The glyphs a layout draws, each once, in order.
fn layout_glyphs(layout: &Layout) -> Vec<String> {
    let mut seen = HashSet::new();
    layout.placed.iter().filter_map(|p| p.glyph.as_deref()).filter(|g| seen.insert(*g)).take(MAX_LINE_GLYPHS).map(str::to_string).collect()
}

/// The line of `layout` in `font` at master `master`: the outlines, the extent, the edited glyph's middle and, for
/// the edit view's text, the glyphs a double-click edits.
fn fill_line(model: &mut PreviewModel, font: &Font, master: &str, layout: &Layout, edited: usize, is_text: bool) {
    let length = layout.lines.first().map_or(0.0, |l| l.length);
    let length = if length.is_finite() { length.max(0.0) } else { 0.0 };
    model.extent = if layout.direction == Direction::Rtl { [-length, 0.0] } else { [0.0, length] };
    model.focus = layout.placed.get(edited).map(|p| p.x + p.advance / 2.0).filter(|f| f.is_finite());
    let mut outlines: std::collections::HashMap<&str, Option<BezPath>> = std::collections::HashMap::new();
    for (item, placed) in layout.placed.iter().enumerate().take(MAX_LINE_GLYPHS) {
        let Some(name) = placed.glyph.as_deref() else { continue };
        if is_text {
            model.hits.push(HitItem { item, x: placed.x, advance: placed.advance });
        }
        if let Some(outline) = outlines.entry(name).or_insert_with(|| font.outline(name, master).ok()) {
            let mut moved = outline.clone();
            moved.apply_affine(Affine::translate((placed.x, placed.y)));
            model.outline.extend(moved.elements().iter().copied());
        }
    }
}

/// The line of all instances: `glyph` once per instance of `instances`, each from the instance's view. An instance
/// the glyph cannot be interpolated at is left out.
fn all_line(model: &mut PreviewModel, session: &mut Session, glyph: &str, instances: &[usize]) {
    let asked = [glyph.to_string()];
    let mut x = 0.0;
    for &i in instances {
        let Ok(instance_view) = session.instance_view(i, &asked) else { continue };
        let at = view_master_id(i);
        let Some(width) = instance_view.glyph(glyph).and_then(|g| g.layer(&at)).map(|l| l.width) else { continue };
        if let Ok(mut outline) = instance_view.outline(glyph, &at) {
            outline.apply_affine(Affine::translate((x, 0.0)));
            model.outline.extend(outline.elements().iter().copied());
        }
        x += if width.is_finite() { width.max(0.0) } else { 0.0 };
    }
    model.extent = [0.0, x];
    model.focus = None;
}

/// Where a preview draws its line in a strip: font units to screen points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    /// The strip, as `[left, top, right, bottom]` in screen points.
    pub rect: [f64; 4],
    /// Screen points per font unit.
    pub scale: f64,
    /// The screen x of the line's origin and the screen y of its baseline, before flipping.
    pub x0: f64,
    pub baseline: f64,
    pub flip: bool,
}

impl Placement {
    /// The screen point of a font point on the line.
    pub fn to_screen(&self, x: f64, y: f64) -> [f64; 2] {
        let p = [self.x0 + x * self.scale, self.baseline - y * self.scale];
        if self.flip { self.turn(p) } else { p }
    }

    /// The font point on the line under a screen point.
    pub fn to_font(&self, p: [f64; 2]) -> [f64; 2] {
        let [x, y] = if self.flip { self.turn(p) } else { p };
        [(x - self.x0) / self.scale, (self.baseline - y) / self.scale]
    }

    /// `p` turned half a circle about the strip's centre.
    fn turn(&self, [x, y]: [f64; 2]) -> [f64; 2] {
        let [left, top, right, bottom] = self.rect;
        [left + right - x, top + bottom - y]
    }
}

/// Where `model`'s line goes in `rect` (`[left, top, right, bottom]`): scaled so the strip's range fills the height,
/// and placed along x by the options. Always Center Active Glyph keeps the edited glyph in the middle; off, the line
/// keeps flush with the left edge (right edge for right-to-left text) where it fits, and fills the width where it
/// does not. `None` for an empty strip or a range that encloses nothing.
pub fn place(model: &PreviewModel, rect: [f64; 4], margin: f64) -> Option<Placement> {
    let [left_edge, top, right_edge, bottom] = rect;
    let [low, high] = model.range;
    let [left, right] = model.extent;
    let span = high - low;
    let (w, h) = (right_edge - left_edge, bottom - top);
    if !(span > 0.0 && w > 0.0 && h > 0.0 && span.is_finite()) {
        return None;
    }
    let scale = h / span;
    let options = &model.options;
    let focus = model.focus.filter(|f| f.is_finite()).unwrap_or((left + right) / 2.0);
    let centred = w / 2.0 - focus * scale;
    let margin = if margin.is_finite() { margin.max(0.0) } else { 0.0 };
    let start = if options.always_center {
        centred
    } else if (right - left) * scale <= w - 2.0 * margin {
        if model.rtl { w - margin - right * scale } else { margin - left * scale }
    } else {
        // Too long to fit: as near the middle as the edges allow, never leaving a gap at either.
        centred.max(w - margin - right * scale).min(margin - left * scale)
    };
    let x0 = left_edge + start + options.offset();
    let baseline = top + high * scale;
    (x0.is_finite() && baseline.is_finite()).then_some(Placement { rect, scale, x0, baseline, flip: options.flip })
}

/// The text item whose glyph is under the screen point `p`, if any.
pub fn hit(model: &PreviewModel, placement: &Placement, p: [f64; 2]) -> Option<usize> {
    let [x, _] = placement.to_font(p);
    model.hits.iter().find(|h| x >= h.x && x < h.x + h.advance).map(|h| h.item)
}
