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
//! | Location (`location`) | the text interpolated at the location the sliders set (`view.setLocation`) |
//! | Show All Instances (`all`) | the edited glyph once for each static instance whose eye is on |
//! | any, while Animate Axis runs (`shell::axis_sweep`) | the text interpolated at the choice's location, its axis where the sweep is |
//!
//! Variable font settings are not instances here: the pop-up lists static instances only. The model holds the line in
//! font units; the view places it in the strip it gets ([`place`]) and fills it.
//!
//! The line is shaped with the tab's features, script and language as Edit View's text is, an instance's line through
//! the instance's own preview font once it is built (`preview_shaping`). While Edit View draws in colour
//! (`canvas::color::mode`), so does the line: the colour layers of the master, or interpolated at the instance or the
//! location; a layered font's shown masters in their Master Colors whatever the line shows. Invert draws colour
//! glyphs for a dark ground: the text colour is the strip's, and Master Color Dark stands in for Master Color.

use std::collections::HashSet;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use craft::behavior::dock::PanelState;
use craft::ui::contract::{PlainData, Tracked};
use fontcraft_engine::Session;
use fontcraft_engine::cmd::preview::{MAX_RENDER_GLYPHS, preview_range};
use fontcraft_engine::cmd::preview_line::MAX_FEATURES;
use fontcraft_engine::cmd::stroke::OpenCorners;
use fontcraft_engine::interp_cache::view_master_id;
use fontcraft_engine::model::Font;
use fontcraft_engine::prelude::*;
use fontcraft_geom::{Affine, BezPath};
use fontcraft_textlayout::{Direction, Item, Layout, Options, PreviewShaper, Shaping, Text, tag};

use super::ProjectCx;
use super::preview_shaping::{InstanceKey, LineColors, PreviewCache};
use super::text::ShaperSlot;
use crate::app::ViewState;
use crate::canvas::color::{self, ColorArt, ColorFiles, MAX_TEXT_COLOR_GLYPHS, Mode, PlacedColor};
use crate::canvas::text::{self, WritingDirection};
use crate::shell::axis_sweep::location_of;
use crate::shell::dock::PanelId;
use crate::shell::preview::{PreviewOptions, Shows, Target};

/// Most glyphs one line draws; a longer text draws its first ones.
pub const MAX_LINE_GLYPHS: usize = 2000;
/// Most different glyphs a line draws while an axis is animated: every step interpolates them.
pub const MAX_SWEEP_GLYPHS: usize = 200;

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
    /// The glyphs drawn in colour, placed on the line; `outline` does not hold them.
    #[plain_data(assume = "colour drawings shared between the glyphs of one name")]
    pub colors: Vec<PlacedColor>,
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
    pub location_chosen: bool,
    /// The font has axes: the pop-up offers Location.
    pub location_offered: bool,
    /// The chosen instance's name; `None` for the dash and Show All Instances.
    pub instance_label: Option<String>,
}

/// What the lines are made with besides the session and the view.
pub struct LineCx<'a> {
    pub cache: &'a mut PreviewCache,
    pub files: &'a mut ColorFiles,
    /// Edit View's shaper.
    pub shaper: &'a ShaperSlot,
}

/// Projects the Preview Area's line while it is open and the Preview Panel's while the panel shows; a hidden preview
/// holds nothing.
pub fn project_both(area: &mut Tracked<PreviewModel>, panel: &mut Tracked<PreviewModel>, with: &mut LineCx<'_>, cx: &mut ProjectCx<'_>) {
    let panel_shows = cx.view.workspace.layout().panel_state(&PanelId::Preview) != PanelState::Hidden;
    for (model, target, shown) in [(area, Target::Area, cx.view.preview.area_open), (panel, Target::Panel, panel_shows)] {
        let value = if shown { build(cx.session, cx.view, target, with).unwrap_or_default() } else { PreviewModel::default() };
        super::set(model, value);
    }
}

/// Asks for a frame while a preview font is built or a long text shaped off the frame.
pub fn logic(cache: &PreviewCache, ctx: &egui::Context) {
    if let Some(wait) = cache.shaping.wake() {
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(wait));
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
    /// A location, by the bits of its internal coordinates (one per font axis).
    Location(Vec<u64>),
}

/// [`Shows::Location`] draws the master here: [`build`] puts the location in its place when the font has a designspace.
pub fn drawn(font: &Font, shows: &Shows) -> Drawn {
    match shows {
        Shows::Master | Shows::Location => Drawn::Master,
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

fn build(session: &mut Session, view: &ViewState, target: Target, with: &mut LineCx<'_>) -> Option<PreviewModel> {
    let state = &view.preview;
    let shows = state.shows(target).clone();
    let options = *state.options(target);
    let index = if view.text.kerning.shows_kerning() { session.kern_index().ok() } else { None };
    let doc = session.active().map(|d| (d.id, d.revision))?;
    let at_location = shows == Shows::Location;
    let designspace = if state.sweep.is_some() || at_location { session.designspace().ok() } else { None };
    let font = session.font().ok()?;
    let master = crate::shell::masters::current_master(font, &view.layers)?;
    let (low, high) = preview_range(font, master);
    let master = master.id.clone();
    let (text, edited, is_text) = line_text(font, view)?;
    let chosen = drawn(font, &shows);
    let drawn = match &designspace {
        Some(ds) => Drawn::Location(location_of(font, ds, view, &shows).into_iter().map(f64::to_bits).collect()),
        None => chosen.clone(),
    };
    let instances: Vec<InstanceEntry> = (0..font.instances.len())
        .filter(|i| is_static(font, *i))
        .map(|i| InstanceEntry {
            index: i,
            name: instance_key(font, i),
            visible: font.instances.get(i).is_some_and(|x| x.visible),
            chosen: chosen == Drawn::Instance(i),
        })
        .collect();
    let instance_label = match chosen {
        Drawn::Instance(i) => Some(instance_key(font, i)),
        _ => None,
    };
    let dir = direction(view);
    let shaper = line_shaper(session, view, &drawn, &text, dir, with);
    let font = session.font().ok()?;
    let features = &view.text.features;
    let active = features.active_tags();
    let shaping = shaper.as_deref().map(|shaper| Shaping { shaper, active: &active, localized: features.localized_tags() });
    let layout_options = Options {
        direction: dir,
        kerning: view.text.kerning,
        single_line: true,
        index: index.as_deref(),
        layers: Some(&view.layers.layers),
        shaping,
        cursive: true,
        ..Default::default()
    };
    let master_layout = fontcraft_textlayout::layout(font, &master, &text, &layout_options).ok()?;
    let mut model = PreviewModel {
        shown: true,
        rtl: dir == Direction::Rtl,
        range: [low, high],
        options,
        instances,
        master_chosen: chosen == Drawn::Master && !at_location,
        location_chosen: at_location,
        location_offered: !font.axes.is_empty(),
        all_chosen: matches!(chosen, Drawn::All(_)),
        instance_label,
        ..PreviewModel::default()
    };
    let edited_glyph = text_glyph(&text, edited);
    let mode = color_mode(font, view, options.invert);
    let strip = crate::panels::preview::PreviewTokens::standard();
    let text_color = if options.invert { strip.paper } else { strip.ink }.to_srgba_unmultiplied().map(|c| f32::from(c) / 255.0);
    let doc_font = session.doc().ok().map(|d| Arc::clone(&d.font))?;
    let in_color = matches!(mode, Some(Mode::Layers { .. }));
    let color = LineMode { mode, font: doc_font, text: text_color };
    let single = Options { direction: dir, kerning: view.text.kerning, single_line: true, shaping, cursive: true, ..Default::default() };
    match &drawn {
        Drawn::Master => {
            let shown = crate::shell::masters::shown_glyph(font, view)
                .and_then(|g| crate::shell::masters::shown_layer(font, &view.layers, g).map(|l| (g.name.clone(), l.id.clone())));
            let mut colors = color.own(session, &master, with);
            let art = |item: usize, name: &str, own: Option<&str>| {
                let exact = shown.as_ref().filter(|(glyph, _)| item == edited && glyph == name).map(|(_, layer)| layer.as_str());
                colors.as_mut()?.art(name, exact.or(own))
            };
            let corners = OpenCorners::of(font);
            let outline = |name: &str, own: Option<&str>| {
                crate::app::models::text::item_outline(font, view, &corners, &master, name, own).map(|(outline, fill)| fill.unwrap_or(outline))
            };
            fill_line(&mut model, &text, &master_layout, edited, is_text, outline, art);
        }
        Drawn::Instance(i) => {
            let glyphs = layout_glyphs(&text, &master_layout);
            let instance_view = if in_color { session.instance_color_view(*i, &glyphs) } else { session.instance_view(*i, &glyphs) }.ok()?;
            let at = view_master_id(*i);
            let mut text = text;
            // An instance has no masters' layers: every item shows the glyph in the instance.
            text.clear_layers();
            let layout = fontcraft_textlayout::layout(&instance_view, &at, &text, &single).ok()?;
            let corners = OpenCorners::of(&instance_view);
            let outline = |name: &str, _: Option<&str>| instance_view.outline(name, &at).ok().map(|o| corners.filled(&at, name, &o).unwrap_or(o));
            let mut colors = color.view(session, &instance_view, &at, &drawn, with);
            fill_line(&mut model, &text, &layout, edited, is_text, outline, |_, name, _| colors.as_mut()?.art(name, None));
        }
        Drawn::Location(bits) => {
            let mut glyphs = layout_glyphs(&text, &master_layout);
            glyphs.truncate(MAX_SWEEP_GLYPHS);
            let location: Vec<f64> = bits.iter().copied().map(f64::from_bits).collect();
            let there = if in_color { session.location_color_view(&location, &glyphs) } else { session.location_view(&location, &glyphs) }.ok()?;
            let at = there.masters.first()?.id.clone();
            let mut text = text;
            text.clear_layers();
            let layout = fontcraft_textlayout::layout(&there, &at, &text, &single).ok()?;
            let mut colors = color.view(session, &there, &at, &drawn, with);
            let art = |_: usize, name: &str, _: Option<&str>| colors.as_mut()?.art(name, None);
            fill_line(&mut model, &text, &layout, edited, is_text, |name, _| there.outline(name, &at).ok(), art);
        }
        Drawn::All(list) => {
            let glyph = edited_glyph.clone()?;
            let art = |session: &Session, there: &Arc<Font>, at: &str| color.view(session, there, at, &drawn, with)?.art(&glyph, None);
            all_line(&mut model, session, &glyph, list, in_color, art);
        }
    }
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    (doc, view.text.revision, &shows, &master, &view.layers.layers, view.text.direction.name(), view.text.kerning.name(), &drawn).hash(&mut hasher);
    edited_glyph.hash(&mut hasher);
    (text::features::compile_stamp(session), format!("{features:?}"), shaper.as_ref().map(|s| Arc::as_ptr(s) as usize)).hash(&mut hasher);
    for c in &model.colors {
        (c.art.key, c.at.map(f64::to_bits)).hash(&mut hasher);
    }
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

/// How Edit View draws in colour, and so the line; `None` for the ordinary drawing.
fn color_mode(font: &Font, view: &ViewState, dark: bool) -> Option<Mode> {
    let glyph = crate::shell::masters::shown_glyph(font, view)?;
    let layer = crate::shell::masters::shown_layer(font, &view.layers, glyph)?;
    color::mode(font, glyph, layer, dark)
}

/// Edit View's line as `preview.render` takes it: what a viewer outside the window draws.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ShownLine {
    /// The glyphs, at most [`MAX_RENDER_GLYPHS`], in text order; a placeholder's is the current glyph's.
    pub glyphs: Vec<String>,
    pub master: Option<String>,
    pub features: Vec<String>,
    /// Localized Forms' pair, while it is on.
    pub script: Option<String>,
    pub language: Option<String>,
    pub rtl: bool,
    /// Edit View draws in colour.
    pub color: bool,
}

impl ShownLine {
    /// Adds the line to the parameters of a `preview.render`.
    pub fn add_to(&self, params: &mut serde_json::Map<String, serde_json::Value>) {
        use serde_json::json;
        params.insert("glyphs".into(), json!(self.glyphs));
        params.insert("features".into(), json!(self.features));
        params.insert("direction".into(), json!(if self.rtl { "rtl" } else { "ltr" }));
        params.insert("color".into(), json!(self.color));
        for (key, value) in [("master", &self.master), ("script", &self.script), ("language", &self.language)] {
            if let Some(value) = value {
                params.insert(key.into(), json!(value));
            }
        }
    }
}

/// The line Edit View shows: its text, or the one glyph it shows; `None` while it shows no glyph.
pub fn shown_line(session: &Session, view: &ViewState) -> Option<ShownLine> {
    let font = session.font().ok()?;
    let (text, ..) = line_text(font, view)?;
    let glyphs = (0..text.len()).filter_map(|i| text_glyph(&text, i)).filter(|g| font.glyph(g).is_some()).take(MAX_RENDER_GLYPHS).collect();
    let features = &view.text.features;
    let (script, language) = match features.localized {
        true => {
            // As Edit View shapes: a tag it cannot take is the default.
            let known = |t: &Option<String>, default: &str| t.clone().filter(|t| tag(t).is_some()).unwrap_or_else(|| default.into());
            (Some(known(&features.script, "DFLT")), Some(known(&features.language, "dflt")))
        }
        false => (None, None),
    };
    Some(ShownLine {
        glyphs,
        master: crate::shell::masters::current_master(font, &view.layers).map(|m| m.id.clone()),
        features: features.active.iter().filter(|t| tag(t).is_some()).take(MAX_FEATURES).cloned().collect(),
        script,
        language,
        rtl: direction(view) == Direction::Rtl,
        color: color_mode(font, view, false).is_some(),
    })
}

/// How a line draws in colour.
struct LineMode {
    mode: Option<Mode>,
    /// The document's font.
    font: Arc<Font>,
    /// The text colour, straight RGBA.
    text: [f32; 4],
}

impl LineMode {
    /// The colour drawings of the document's line in `master`.
    fn own<'a>(&self, session: &Session, master: &str, with: &'a mut LineCx<'_>) -> Option<LineColors<'a>> {
        let mode = self.mode.clone()?;
        Some(LineColors::new(session, mode, Arc::clone(&self.font), master, self.text, &mut *with.files, &mut with.cache.arts))
    }

    /// The colour drawings of a line of `there`, the view of `drawn` with the one master `at`; a layered font draws its
    /// own masters there.
    fn view<'a>(&self, session: &Session, there: &Arc<Font>, at: &str, drawn: &Drawn, with: &'a mut LineCx<'_>) -> Option<LineColors<'a>> {
        let (mode, source) = match self.mode.clone()? {
            Mode::Layers { kind, .. } => (Mode::Layers { master: at.to_string(), kind }, there),
            layered @ Mode::Layered { .. } => (layered, &self.font),
        };
        Some(LineColors::new(session, mode, Arc::clone(source), (at, drawn), self.text, &mut *with.files, &mut with.cache.arts))
    }
}

/// What the line is shaped with: the instance's shaper, else Edit View's. `None` while nothing is on or compiled, and
/// while a long text is shaped off the frame.
fn line_shaper(session: &Session, view: &ViewState, drawn: &Drawn, text: &Text, dir: Direction, with: &mut LineCx<'_>) -> Option<Arc<PreviewShaper>> {
    let features = &view.text.features;
    let preview = text::features::preview(session, features)?;
    let font = Arc::clone(&session.doc().ok()?.font);
    let shaping = &mut with.cache.shaping;
    if let Drawn::Instance(i) = drawn
        && let Some(key) = InstanceKey::new(session, &font, *i)
        && let Some(own) = shaping.instance(&key, &font)
        && shaping.ready(&own, &font, text, dir, features)
    {
        return Some(own);
    }
    let shared = with.shaper.ready(&preview)?;
    shaping.ready(&shared, &font, text, dir, features).then_some(shared)
}

/// The glyphs a line needs, each once: the text's own and the ones a layout of it draws.
fn layout_glyphs(text: &Text, layout: &Layout) -> Vec<String> {
    let mut seen = HashSet::new();
    let typed = text.items().iter().filter_map(|i| if let Item::Glyph { name, .. } = i { Some(&**name) } else { None });
    let drawn = layout.placed.iter().flat_map(|p| p.glyph.as_deref().into_iter().chain(p.extra.iter().map(|(g, ..)| &**g)));
    drawn.chain(typed).chain(text.current_glyph()).filter(|g| seen.insert(*g)).take(MAX_LINE_GLYPHS).map(str::to_string).collect()
}

/// The line of `layout`, which laid `text` out: the outlines (`outline` finds a glyph's in the own layer of the item
/// that shows it) or the colour drawings in their place (`art`, by item, glyph and own layer), the extent, the edited
/// glyph's middle and, for the edit view's text, the glyphs a double-click edits.
fn fill_line(
    model: &mut PreviewModel,
    text: &Text,
    layout: &Layout,
    edited: usize,
    is_text: bool,
    outline: impl Fn(&str, Option<&str>) -> Option<BezPath>,
    mut art: impl FnMut(usize, &str, Option<&str>) -> Option<Arc<ColorArt>>,
) {
    let length = layout.lines.first().map_or(0.0, |l| l.length);
    let length = if length.is_finite() { length.max(0.0) } else { 0.0 };
    model.extent = if layout.direction == Direction::Rtl { [-length, 0.0] } else { [0.0, length] };
    model.focus = layout.placed.get(edited).map(|p| p.x + p.advance / 2.0).filter(|f| f.is_finite());
    let mut outlines: std::collections::HashMap<(&str, Option<&str>), Option<BezPath>> = std::collections::HashMap::new();
    for (item, placed) in layout.placed.iter().enumerate().take(MAX_LINE_GLYPHS) {
        let Some(name) = placed.glyph.as_deref() else { continue };
        if is_text {
            model.hits.push(HitItem { item, x: placed.x, advance: placed.advance });
        }
        let first = (name, placed.x, placed.y);
        for (name, x, y) in std::iter::once(first).chain(placed.extra.iter().map(|(g, x, y)| (&**g, *x, *y))) {
            let own = text.layer_of(item, name);
            if model.colors.len() < MAX_TEXT_COLOR_GLYPHS
                && let Some(art) = art(item, name, own)
            {
                model.colors.push(PlacedColor { at: [x, y], art });
                continue;
            }
            if let Some(outline) = outlines.entry((name, own)).or_insert_with(|| outline(name, own)) {
                let mut moved = outline.clone();
                moved.apply_affine(Affine::translate((x, y)));
                model.outline.extend(moved.elements().iter().copied());
            }
        }
    }
}

/// The line of all instances: `glyph` once per instance of `instances`, each from the instance's view (with its colour
/// layers when `in_color`), in `art`'s colour drawing when it gives one. An instance the glyph cannot be interpolated
/// at is left out.
fn all_line(
    model: &mut PreviewModel,
    session: &mut Session,
    glyph: &str,
    instances: &[usize],
    in_color: bool,
    mut art: impl FnMut(&Session, &Arc<Font>, &str) -> Option<Arc<ColorArt>>,
) {
    let asked = [glyph.to_string()];
    let mut x = 0.0;
    let mut drawn: Option<OpenCorners> = None;
    for &i in instances {
        let instance_view = if in_color { session.instance_color_view(i, &asked) } else { session.instance_view(i, &asked) };
        let Ok(instance_view) = instance_view else { continue };
        let at = view_master_id(i);
        let Some(width) = instance_view.glyph(glyph).and_then(|g| g.layer(&at)).map(|l| l.width) else { continue };
        if model.colors.len() < MAX_TEXT_COLOR_GLYPHS
            && let Some(art) = art(session, &instance_view, &at)
        {
            model.colors.push(PlacedColor { at: [x, 0.0], art });
        } else if let Ok(outline) = instance_view.outline(glyph, &at) {
            let corners = drawn.as_ref().map_or_else(|| OpenCorners::of(&instance_view), |c| c.sharing(&instance_view));
            let mut outline = corners.filled(&at, glyph, &outline).unwrap_or(outline);
            drawn = Some(corners);
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
