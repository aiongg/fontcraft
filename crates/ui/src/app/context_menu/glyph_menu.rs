//! Font View's glyph menu (`fontView.glyphContextMenu`): a secondary click on a cell or a List View row opens it. A
//! glyph that is not selected becomes the selection first; on a selected glyph the menu acts on the whole selection.
//!
//! It holds the colour swatches (with Alt held at the click: the colour of the layer shown), the Exports check item
//! (`glyph.setExport`, a dash when some of the glyphs export), the Tags submenu (`fontView.contextMenu.tags`), which ends with
//! More Tags… when the font has more tags than [`MAX_MENU_TAGS`], and the Copy Glyph Names submenu, whose text goes to
//! the system clipboard (`app::clipboard`). When the font defines statuses, the Status submenu lists them with their
//! colours, a check on the status every glyph is in and a dash on one some are in, and ends with No Status
//! (`glyph.setStatus`).

use std::collections::{BTreeSet, HashMap};

use craft::ui::runtime::commands::CommandRef;
use fontcraft_engine::cmd::status;
use fontcraft_engine::model::Font;
use serde_json::{Map, Value, json};

use crate::app::{FontCraftApp, Msg};
use crate::shell::masters;

/// Most tags the Tags submenu lists; past this it ends with More Tags….
pub const MAX_MENU_TAGS: usize = 512;
/// Most glyphs one menu acts on.
pub const MAX_MENU_GLYPHS: usize = 100_000;

/// The formats of Copy Glyph Names: the `glyph.copyGlyphNames` format and the item's label.
pub const COPY_FORMATS: [(&str, &str); 6] = [
    ("spaceSeparated", "Space Separated"),
    ("onePerLine", "One per Line"),
    ("commaSeparated", "Comma Separated"),
    ("slashed", "Slashed"),
    ("pythonList", "Python List"),
    ("unicodes", "Unicodes"),
];

/// How many of the menu's glyphs carry a tag, or export: all, some or none.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mark {
    All,
    Some,
    #[default]
    None,
}

impl Mark {
    fn of(count: usize, total: usize) -> Mark {
        match count {
            0 => Mark::None,
            n if n == total => Mark::All,
            _ => Mark::Some,
        }
    }
}

/// An open glyph menu.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GlyphMenu {
    /// The glyphs its items act on.
    pub glyphs: Vec<String>,
    /// Where it opens, in window points.
    pub screen: [f32; 2],
    /// The first [`MAX_MENU_TAGS`] tags the font uses, sorted, with how many of `glyphs` carry each. Empty: no Tags
    /// submenu.
    pub tags: Vec<(String, Mark)>,
    /// The font has more tags than the submenu lists: it ends with More Tags….
    pub more_tags: bool,
    /// How many of `glyphs` export.
    pub exports: Mark,
    /// The layer the swatches colour while Alt is held: the one glyph's shown layer, else the current master's.
    pub layer: Option<String>,
    /// Alt was held when the menu opened: the swatches colour `layer` instead of the glyphs.
    pub alt: bool,
    /// The font's statuses, in order, with how many of `glyphs` are in each. Empty: no Status submenu.
    pub statuses: Vec<StatusItem>,
}

/// A status of the Status submenu.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StatusItem {
    pub code: i64,
    pub name: String,
    /// Red, green, blue and alpha.
    pub color: [u8; 4],
    pub mark: Mark,
}

/// What an item of a glyph menu does.
#[derive(Clone, Debug, PartialEq)]
pub enum GlyphAction {
    /// A swatch (`Some(index)`) or the cross (`None`).
    Color(Option<u8>),
    /// A swatch or the cross with Alt held.
    LayerColor(Option<u8>),
    /// Exports: on for every glyph unless all export, then off.
    Export,
    /// A tag of the Tags submenu: adds it to every glyph.
    Tag(String),
    /// More Tags…: shows the inspector's Tags field.
    MoreTags,
    /// A format of Copy Glyph Names.
    CopyNames(&'static str),
    /// A status of the Status submenu, by its code, or No Status (`None`).
    Status(Option<i64>),
}

/// The glyph menu for `glyphs`, opening at `screen`. Names the font lacks are left out.
pub fn open(app: &FontCraftApp, glyphs: &[String], screen: [f32; 2]) -> GlyphMenu {
    let Ok(font) = app.session.font() else { return GlyphMenu { screen, ..GlyphMenu::default() } };
    let glyphs: Vec<String> = glyphs.iter().filter(|n| font.glyph(n).is_some()).take(MAX_MENU_GLYPHS).cloned().collect();
    let used: BTreeSet<&str> = font.glyphs.iter().flat_map(|g| g.info.tags.iter().map(String::as_str)).collect();
    let mut counts: HashMap<&str, usize> = HashMap::new();
    let mut exporting = 0;
    for g in glyphs.iter().filter_map(|n| font.glyph(n)) {
        exporting += usize::from(g.export);
        for tag in &g.info.tags {
            *counts.entry(tag.as_str()).or_insert(0) += 1;
        }
    }
    let total = glyphs.len();
    let tags = used.iter().take(MAX_MENU_TAGS).map(|tag| (tag.to_string(), Mark::of(counts.get(tag).copied().unwrap_or(0), total))).collect();
    GlyphMenu {
        more_tags: used.len() > MAX_MENU_TAGS,
        exports: Mark::of(exporting, total),
        tags,
        screen,
        layer: menu_layer(app, font, &glyphs),
        statuses: status_items(font, &glyphs),
        glyphs,
        alt: false,
    }
}

/// The Status submenu's statuses: each definition with how many of `glyphs` are in it (their stored status, else the
/// default one).
fn status_items(font: &Font, glyphs: &[String]) -> Vec<StatusItem> {
    let defs = status::definitions(font);
    let default = defs.iter().find(|d| d.is_default).map(|d| d.code);
    let mut counts: HashMap<i64, usize> = HashMap::new();
    for name in glyphs {
        if let Some(code) = status::stored_status(font, name).or(default) {
            *counts.entry(code).or_insert(0) += 1;
        }
    }
    let byte = |c: f64| if c.is_finite() { (c.clamp(0.0, 1.0) * 255.0).round() as u8 } else { 0 };
    defs.iter()
        .map(|d| StatusItem {
            code: d.code,
            name: d.name.clone(),
            color: d.color.map(byte),
            mark: Mark::of(counts.get(&d.code).copied().unwrap_or(0), glyphs.len()),
        })
        .collect()
}

/// The layer the Alt swatches colour: in the edit view for one glyph the layer it shows (the Layers panel's pick, else its
/// layer of the current master); for several and in Font View the current master's.
fn menu_layer(app: &FontCraftApp, font: &Font, glyphs: &[String]) -> Option<String> {
    let layers = &app.view.layers;
    if !app.font_view_shows()
        && let [one] = glyphs
        && let Some(glyph) = font.glyph(one)
        && let Some(layer) = masters::shown_layer(font, layers, glyph)
    {
        return Some(layer.id.clone());
    }
    masters::current_master(font, layers).map(|m| m.id.clone())
}

/// The messages `action` posts for the menu's glyphs.
pub fn steps(menu: &GlyphMenu, action: &GlyphAction) -> Vec<Msg> {
    let color = |c: &Option<u8>| c.map_or(Value::Null, |i| json!(i));
    match action {
        GlyphAction::Color(c) => vec![Msg::run("glyph.setColorLabel", json!({"glyphs": menu.glyphs, "color": color(c)}))],
        GlyphAction::LayerColor(c) => {
            let mut m = Map::new();
            m.insert("glyphs".into(), json!(menu.glyphs));
            if let Some(layer) = &menu.layer {
                m.insert("layer".into(), json!(layer));
            }
            m.insert("color".into(), color(c));
            vec![Msg::run("glyph.setLayerColorLabel", Value::Object(m))]
        }
        GlyphAction::Export => vec![Msg::run("glyph.setExport", json!({"glyphs": menu.glyphs, "export": menu.exports != Mark::All}))],
        GlyphAction::Tag(tag) => vec![Msg::run("glyph.addTag", json!({"glyphs": menu.glyphs, "tag": tag}))],
        GlyphAction::MoreTags => vec![Msg::Shell(CommandRef::new(crate::shell::context_menu::SHOW_TAGS))],
        GlyphAction::CopyNames(format) => vec![Msg::run("glyph.copyGlyphNames", json!({"glyphs": menu.glyphs, "format": format}))],
        GlyphAction::Status(code) => {
            let status = code.map_or(Value::Null, |c| json!(c));
            vec![Msg::run("glyph.setStatus", json!({"glyphs": menu.glyphs, "status": status}))]
        }
    }
}
