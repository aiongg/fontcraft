//! The view models of the glyph sections: Related Glyphs & Characters, Selections, Measurements and Reference Font.
//! A hidden panel's model is left empty, so its queries run only while it shows.

use craft::behavior::dock::PanelState;
use craft::ui::contract::{PlainData, Tracked};
use fontcraft_engine::cmd::related::{RelatedChar, related_glyphs};
use fontcraft_engine::model::LayerSide;
use fontcraft_tools::Selection;
use serde_json::{Value, json};

use super::glyph::shown_glyph;
use super::{ProjectCx, Stamp};
use crate::canvas::tools::{self, Tools};
use crate::shell::dock::PanelId;
use crate::shell::masters;

/// Rows of metrics or stems the Measurements panel lists; a font may define 256 of each.
pub const MAX_MEASUREMENT_ROWS: usize = 64;
/// Stored selections the Selections panel lists (a glyph keeps at most 256).
pub const MAX_STORED_ROWS: usize = 256;

fn shows(cx: &ProjectCx<'_>, panel: PanelId) -> bool {
    cx.view.workspace.layout().panel_state(&panel) != PanelState::Hidden
}

/// The layer the glyph panels show ([`masters::panel_layer`]).
fn shown_layer(cx: &ProjectCx<'_>) -> Option<String> {
    let font = cx.session.font().ok()?;
    masters::panel_layer(font, cx.view).map(|l| l.id.clone())
}

/// The master the glyph panels show ([`masters::panel_master`]).
fn shown_master(cx: &ProjectCx<'_>) -> Option<String> {
    let font = cx.session.font().ok()?;
    masters::panel_master(font, cx.view).map(|m| m.id.clone())
}

/// Which list of related glyphs a section shows.
#[derive(PlainData, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RelatedKind {
    /// Glyphs with the same base name and another suffix.
    #[default]
    Alternates,
    /// The glyphs this glyph's components refer to.
    Components,
    /// The glyphs with a component of this glyph.
    UsedBy,
    /// The characters the glyph's characters decompose into.
    DecomposesInto,
    /// The characters whose decomposition contains the glyph's characters.
    ContainedIn,
}

/// One entry of a related section.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct RelatedEntry {
    pub label: String,
    /// The glyph a click shows; `None` for a character the font has no glyph for.
    pub glyph: Option<String>,
}

#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct RelatedSection {
    pub kind: RelatedKind,
    pub entries: Vec<RelatedEntry>,
}

/// What is connected to the shown glyph: the sections with entries, in display order.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct RelatedModel {
    pub glyph: Option<String>,
    pub sections: Vec<RelatedSection>,
}

fn glyph_entry(name: &str) -> RelatedEntry {
    RelatedEntry { label: name.to_string(), glyph: Some(name.to_string()) }
}

fn char_entry(c: &RelatedChar) -> RelatedEntry {
    let shown = char::from_u32(c.code_point).filter(|ch| !ch.is_control()).map(String::from).unwrap_or_default();
    let label = match &c.glyph {
        Some(name) => format!("{shown} {name}"),
        None => format!("{shown} U+{:04X}", c.code_point),
    };
    RelatedEntry { label: label.trim_start().to_string(), glyph: c.glyph.clone() }
}

pub fn project_related(model: &mut Tracked<RelatedModel>, cx: &mut ProjectCx<'_>) {
    let glyph = shown_glyph(cx);
    let found = glyph.as_deref().filter(|_| shows(cx, PanelId::RelatedGlyphs)).and_then(|g| {
        let font = cx.session.font().ok()?.clone();
        let data = cx.session.glyph_data().ok()?;
        related_glyphs(&font, &data, g)
    });
    let Some(r) = found else {
        super::set(model, RelatedModel { glyph, sections: Vec::new() });
        return;
    };
    let lists = [
        (RelatedKind::Alternates, r.alternates.iter().map(|n| glyph_entry(n)).collect::<Vec<_>>()),
        (RelatedKind::Components, r.components.iter().map(|n| glyph_entry(n)).collect()),
        (RelatedKind::UsedBy, r.used_by.iter().map(|n| glyph_entry(n)).collect()),
        (RelatedKind::DecomposesInto, r.decomposes_into.iter().map(char_entry).collect()),
        (RelatedKind::ContainedIn, r.decomposed_from.iter().map(char_entry).collect()),
    ];
    let sections = lists.into_iter().filter(|(_, e)| !e.is_empty()).map(|(kind, entries)| RelatedSection { kind, entries }).collect();
    super::set(model, RelatedModel { glyph, sections });
}

/// One stored selection: its name and how many of its items do not resolve on the layer.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct StoredEntry {
    pub name: String,
    pub missing: u64,
}

/// The edit view's selection as `edit.storeSelection` takes it.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct SelectionLists {
    /// `[shape, node]` pairs.
    pub points: Vec<[u64; 2]>,
    pub shapes: Vec<u64>,
    pub anchors: Vec<String>,
}

impl SelectionLists {
    pub fn is_empty(&self) -> bool {
        self.points.is_empty() && self.shapes.is_empty() && self.anchors.is_empty()
    }
}

/// The shown glyph's stored selections in the layer the canvas shows.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct SelectionsModel {
    pub glyph: Option<String>,
    pub layer: Option<String>,
    /// The side of the layer the edit view edits, as the commands spell it.
    pub side: String,
    pub entries: Vec<StoredEntry>,
    /// The edit view's selection, which Store and Update keep.
    pub selection: SelectionLists,
}

/// What the Selections panel was last projected at.
pub type SelectionsStamp = (Stamp, Selection);

pub fn project_selections(
    model: &mut Tracked<SelectionsModel>,
    seen: &mut Option<SelectionsStamp>,
    cx: &mut ProjectCx<'_>,
    tools: &Tools,
    stamp: Stamp,
) {
    let key = (stamp, tools.selection.clone());
    if seen.as_ref() == Some(&key) {
        return;
    }
    *seen = Some(key);
    let (glyph, layer) = (shown_glyph(cx), shown_layer(cx));
    // Font View's cells draw the foreground, whatever the canvas behind it edits.
    let side = if cx.view.tabs.font_view_shows() { LayerSide::Foreground } else { tools::side(cx.view) }.as_str().to_string();
    let mut value = SelectionsModel { glyph: glyph.clone(), layer: layer.clone(), side: side.clone(), ..SelectionsModel::default() };
    if let (Some(glyph), Some(layer), true) = (glyph, layer, shows(cx, PanelId::Selections)) {
        let found = cx.session.query("edit.getStoredSelections", &json!({ "glyph": glyph, "layer": layer, "side": side })).unwrap_or(Value::Null);
        value.selection = canvas_selection(cx, tools, &glyph, &layer).unwrap_or_default();
        let list = found.get("selections").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
        value.entries = list
            .iter()
            .take(MAX_STORED_ROWS)
            .filter_map(|e| {
                let name = e.get("name")?.as_str()?.to_string();
                Some(StoredEntry { name, missing: e.get("missing").and_then(Value::as_u64).unwrap_or(0) })
            })
            .collect();
    }
    super::set(model, value);
}

/// The canvas selection, when the canvas edits `layer` of `glyph`.
fn canvas_selection(cx: &ProjectCx<'_>, tools: &Tools, glyph: &str, layer: &str) -> Option<SelectionLists> {
    if cx.view.tabs.font_view_shows() {
        return None;
    }
    let font = cx.session.font().ok()?;
    let scene = tools::scene(font, cx.session.glyph_data().ok().as_deref(), cx.view, None)?;
    if scene.glyph != glyph || scene.layer_id != layer {
        return None;
    }
    let mut selection = tools.selection.clone();
    // An undo or another edit may have taken some of it away since the canvas last drew.
    selection.retain_valid(&scene);
    Some(SelectionLists {
        points: selection.points.iter().map(|&(shape, node)| [shape as u64, node as u64]).collect(),
        shapes: selection.shapes.iter().map(|&shape| shape as u64).collect(),
        anchors: selection.anchors.iter().cloned().collect(),
    })
}

/// One metric of the master: its height and, except for the italic angle, its overshoot zone.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct MetricRow {
    pub index: u64,
    /// The metric's name, else its label; empty when it has neither.
    pub label: String,
    pub position: Option<f64>,
    pub zone: Option<f64>,
    /// The metric has a zone (the italic angle has none).
    pub has_zone: bool,
}

/// One stem definition of the master and its width.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct StemRow {
    pub index: u64,
    /// Empty when the stem has no name.
    pub name: String,
    pub horizontal: bool,
    pub value: Option<f64>,
}

/// The design values of the master the glyph panels show.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct MeasurementsModel {
    pub master: Option<String>,
    pub metrics: Vec<MetricRow>,
    pub stems: Vec<StemRow>,
}

pub fn project_measurements(model: &mut Tracked<MeasurementsModel>, cx: &mut ProjectCx<'_>) {
    let master = shown_master(cx);
    let mut value = MeasurementsModel { master: master.clone(), ..MeasurementsModel::default() };
    if let (Some(master), true) = (master, shows(cx, PanelId::Measurements)) {
        let number = |v: &Value, key: &str| v.get(key).and_then(Value::as_f64).filter(|n| n.is_finite());
        let text = |v: &Value, key: &str| v.get(key).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string);
        let metrics = cx.session.query("font.metrics.list", &json!({ "master": master })).unwrap_or(Value::Null);
        let stems = cx.session.query("font.stems.list", &json!({ "master": master })).unwrap_or(Value::Null);
        let rows = |v: &Value, key: &str| v.get(key).and_then(Value::as_array).cloned().unwrap_or_default();
        value.metrics = rows(&metrics, "metrics")
            .iter()
            .take(MAX_MEASUREMENT_ROWS)
            .filter_map(|m| {
                Some(MetricRow {
                    index: m.get("index")?.as_u64()?,
                    label: text(m, "name").or_else(|| text(m, "label")).unwrap_or_default(),
                    position: number(m, "position"),
                    zone: number(m, "zone"),
                    has_zone: m.get("zone").is_some_and(|z| !z.is_null()),
                })
            })
            .collect();
        value.stems = rows(&stems, "stems")
            .iter()
            .take(MAX_MEASUREMENT_ROWS)
            .filter_map(|s| {
                Some(StemRow {
                    index: s.get("index")?.as_u64()?,
                    name: text(s, "name").unwrap_or_default(),
                    horizontal: s.get("orientation").and_then(Value::as_str) == Some("horizontal"),
                    value: number(s, "value"),
                })
            })
            .collect();
    }
    super::set(model, value);
}

/// A listed reference font.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct ReferenceEntry {
    pub path: String,
    /// The file name, else the path.
    pub name: String,
    pub selected: bool,
}

/// The reference fonts and how the selected one is shown (`view.setReferenceFont`).
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct ReferenceFontModel {
    pub fonts: Vec<ReferenceEntry>,
    pub show: bool,
    pub character: String,
    pub language: String,
    /// A font is selected but not read.
    pub unread: bool,
}

pub fn project_reference_font(model: &mut Tracked<ReferenceFontModel>, cx: &mut ProjectCx<'_>) {
    let state = &cx.view.reference_font;
    let fonts = state
        .paths
        .iter()
        .map(|path| {
            let name = std::path::Path::new(path).file_name().and_then(|n| n.to_str()).unwrap_or(path).to_string();
            ReferenceEntry { path: path.clone(), name, selected: state.selected.as_deref() == Some(path.as_str()) }
        })
        .collect();
    super::set(
        model,
        ReferenceFontModel {
            fonts,
            show: state.show,
            character: state.character.clone(),
            language: state.language.clone(),
            unread: state.selected.is_some() && state.loaded.0.is_none(),
        },
    );
}
