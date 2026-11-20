//! The glyph properties the inspector (`fontView.inspector`) and the Selection Info panel (`window.selectionInfo`)
//! show for a set of glyphs, and the commands their edits run.
//!
//! A field whose value differs between the glyphs is mixed: it shows *Multiple Values*, and a typed value goes to
//! every glyph in one command. The metric fields read the glyphs' layers in Font View's master (the current master):
//! the metrics key that governs a metric (the layer's own key written with `==`), else the measured value.
//!
//! | Field | Edit runs |
//! |---|---|
//! | Name | `glyph.rename` (one glyph only) |
//! | Unicode | `glyph.setUnicodes` |
//! | Production Name, Script, Category, Subcategory, Case, Direction, Sort Name | `glyph.setProductionName`, `glyph.setScript`, `glyph.setCategory`, `glyph.setCase`, `glyph.setDirection`, `glyph.setSortName`; an empty field clears it |
//! | Note | `glyph.setNote` |
//! | Glyph and layer colour | `glyph.setColorLabel`, `glyph.setLayerColorLabel` on Font View's master |
//! | Left, right and width metrics keys | `spacing.setLeftSidebearing`, `spacing.setRightSidebearing`, `spacing.setWidth` with the key |
//! | Left and right kerning groups | `kerning.setLeftGroup`, `kerning.setRightGroup`; an empty field clears the group |
//! | LSB, RSB, width, TSB, BSB | the `spacing.set*` command of the field |
//! | vertical origin, vertical width | `spacing.setVerticalOrigin`, `spacing.setVerticalWidth` |
//! | Exports | `glyph.setExport` |
//! | Tags | `glyph.addTag`, `glyph.removeTag` |

use std::sync::Arc;

use craft::behavior::dock::PanelState;
use craft::ui::contract::{PlainData, Tracked};
use fontcraft_engine::model::{ColorLabel, Font, Glyph, GlyphProps, Layer, PropSource, glyph_props, keeps_alternates, metric};
use fontcraft_spacing::{self as spacing, Field, Scope};
use serde_json::{Value, json};

use super::ProjectCx;
use crate::app::Msg;
use crate::app::models::font_view::COLOR_LABEL_NAMES;
use crate::shell::dock::PanelId;

/// Most glyphs a summary describes; a larger selection is described by its first ones.
pub const MAX_SUMMARY_GLYPHS: usize = 100_000;
/// Longest tag, in characters.
pub const MAX_TAG_CHARS: usize = 255;
/// Decimals a measured metric shows.
const DECIMALS: i32 = 2;

/// A text property of the summary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PropField {
    Name,
    Unicode,
    ProductionName,
    Script,
    Category,
    SubCategory,
    Case,
    Direction,
    SortName,
    Note,
    LeftGroup,
    RightGroup,
    LeftKey,
    RightKey,
    WidthKey,
    /// The glyph's colour label: its index, the `#RRGGBBAA` of a custom colour, or empty.
    Color,
    /// The colour label of the glyph's layer in Font View's master.
    LayerColor,
}

impl PropField {
    pub const ALL: [PropField; 17] = [
        PropField::Name,
        PropField::Unicode,
        PropField::ProductionName,
        PropField::Script,
        PropField::Category,
        PropField::SubCategory,
        PropField::Case,
        PropField::Direction,
        PropField::SortName,
        PropField::Note,
        PropField::LeftGroup,
        PropField::RightGroup,
        PropField::LeftKey,
        PropField::RightKey,
        PropField::WidthKey,
        PropField::Color,
        PropField::LayerColor,
    ];

    pub fn index(self) -> usize {
        PropField::ALL.iter().position(|f| *f == self).unwrap_or(0)
    }

    /// The field's text for one glyph.
    fn text(self, cx: &GlyphCx<'_>) -> String {
        let glyph = cx.glyph;
        let prop =
            |pick: fn(&GlyphProps) -> &fontcraft_engine::model::Prop| cx.props.as_deref().and_then(|p| pick(p).value.clone()).unwrap_or_default();
        let key = |k: &Option<String>| k.as_deref().map(str::trim).unwrap_or_default().to_string();
        match self {
            PropField::Name => glyph.name.clone(),
            PropField::Unicode => glyph.unicodes.iter().map(|u| format!("{u:04X}")).collect::<Vec<_>>().join(" "),
            PropField::ProductionName => prop(|p| &p.production_name),
            PropField::Script => prop(|p| &p.script),
            PropField::Category => prop(|p| &p.category),
            PropField::SubCategory => prop(|p| &p.sub_category),
            PropField::Case => prop(|p| &p.case),
            PropField::Direction => prop(|p| &p.direction),
            PropField::SortName => prop(|p| &p.sort_name),
            PropField::Note => glyph.info.note.clone(),
            PropField::LeftGroup => glyph.kern_groups.left.clone().unwrap_or_default(),
            PropField::RightGroup => glyph.kern_groups.right.clone().unwrap_or_default(),
            PropField::LeftKey => key(&glyph.info.metrics_keys.left),
            PropField::RightKey => key(&glyph.info.metrics_keys.right),
            PropField::WidthKey => key(&glyph.info.metrics_keys.width),
            PropField::Color => color_value(glyph.info.color),
            PropField::LayerColor => color_value(glyph.layer(cx.master).and_then(|l| l.info.color)),
        }
    }
}

/// A colour label as the colour fields hold it: the index, `#RRGGBBAA`, or empty for none.
pub fn color_value(color: Option<ColorLabel>) -> String {
    match color {
        None => String::new(),
        Some(ColorLabel::Index(i)) => i.to_string(),
        Some(ColorLabel::Rgba([r, g, b, a])) => {
            let byte = |v: f64| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
            format!("#{:02X}{:02X}{:02X}{:02X}", byte(r), byte(g), byte(b), byte(a))
        }
    }
}

/// The colour label names with their values, none first.
pub fn color_choices() -> Vec<(String, &'static str)> {
    COLOR_LABEL_NAMES.iter().enumerate().map(|(i, n)| (i.to_string(), *n)).collect()
}

/// One metric field.
#[derive(PlainData, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum MetricField {
    Lsb,
    Rsb,
    Width,
    Tsb,
    Bsb,
    VertOrigin,
    VertWidth,
}

/// The fields of each orientation, in the order the inspector lays them out.
pub const HORIZONTAL: [MetricField; 3] = [MetricField::Lsb, MetricField::Rsb, MetricField::Width];
pub const VERTICAL: [MetricField; 4] = [MetricField::Tsb, MetricField::Bsb, MetricField::VertOrigin, MetricField::VertWidth];

/// The fields of one orientation.
pub fn metric_fields(vertical: bool) -> &'static [MetricField] {
    if vertical { &VERTICAL } else { &HORIZONTAL }
}

impl MetricField {
    /// The metric the field reads and its command sets.
    pub fn field(self) -> Field {
        match self {
            MetricField::Lsb => Field::Left,
            MetricField::Rsb => Field::Right,
            MetricField::Width => Field::Width,
            MetricField::Tsb => Field::Top,
            MetricField::Bsb => Field::Bottom,
            MetricField::VertOrigin => Field::VertOrigin,
            MetricField::VertWidth => Field::VertWidth,
        }
    }

    /// The command that sets the field.
    pub fn command(self) -> &'static str {
        match self {
            MetricField::Lsb => "spacing.setLeftSidebearing",
            MetricField::Rsb => "spacing.setRightSidebearing",
            MetricField::Width => "spacing.setWidth",
            MetricField::Tsb => "spacing.setTopSidebearing",
            MetricField::Bsb => "spacing.setBottomSidebearing",
            MetricField::VertOrigin => "spacing.setVerticalOrigin",
            MetricField::VertWidth => "spacing.setVerticalWidth",
        }
    }

    /// What the field shows for `glyph` in `master`: its key, else its value (empty for a sidebearing of an empty
    /// layer and for a glyph without that master's layer).
    pub fn text(self, font: &Font, glyph: &Glyph, master: &str) -> String {
        let Some(layer) = glyph.layer(master) else { return String::new() };
        let field = self.field();
        match spacing::effective_key(glyph, layer, field) {
            Some((scope, stored)) => key_text(font, field, scope, stored),
            None => measured(font, layer, self).map(format_number).unwrap_or_default(),
        }
    }
}

/// The measured value of `field` in `layer`; a vertical width the layer does not store is the master's ascender
/// to its descender.
fn measured(font: &Font, layer: &Layer, field: MetricField) -> Option<f64> {
    let value = spacing::metric(font, layer, field.field()).ok().flatten();
    let value = match (field, value) {
        (MetricField::VertWidth, None) => default_vertical_width(font, &layer.master),
        (_, v) => v,
    };
    value.filter(|n| n.is_finite())
}

fn default_vertical_width(font: &Font, master: &str) -> Option<f64> {
    let upm = f64::from(font.units_per_em);
    let m = font.masters.iter().find(|m| m.id == master)?;
    Some(font.metric_pos(m, metric::ASCENDER, upm * 0.8) - font.metric_pos(m, metric::DESCENDER, -upm * 0.2))
}

/// A stored key as the field shows it: `==` for a key of the layer only.
fn key_text(font: &Font, field: Field, scope: Scope, stored: &str) -> String {
    match spacing::parse_stored(stored, scope, field, |n| font.glyph(n).is_some()) {
        Ok(key) => key.display(),
        Err(_) => match scope {
            Scope::Local => format!("=={}", stored.trim().trim_start_matches('=')),
            Scope::Glyph => stored.trim().to_string(),
        },
    }
}

/// A number with at most two decimals and no trailing zeros.
pub fn format_number(n: f64) -> String {
    let scale = 10f64.powi(DECIMALS);
    let rounded = (n * scale).round() / scale;
    let rounded = if rounded == 0.0 { 0.0 } else { rounded };
    let text = format!("{rounded:.2}");
    text.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// What a field shows for the glyphs it describes.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct Shown {
    /// The common value (empty while mixed or with no glyphs).
    pub text: String,
    /// The glyphs differ: the field shows *Multiple Values*.
    pub mixed: bool,
}

impl Shown {
    /// The common value of `values`; stops at the first that differs.
    pub fn of(values: impl IntoIterator<Item = String>) -> Shown {
        let mut values = values.into_iter();
        let Some(first) = values.next() else { return Shown::default() };
        if values.any(|v| v != first) { Shown { text: String::new(), mixed: true } } else { Shown { text: first, mixed: false } }
    }
}

/// `Some(v)` when every value is `v` (`Some(false)` with none), `None` when they differ.
fn common(mut values: impl Iterator<Item = bool>) -> Option<bool> {
    let Some(first) = values.next() else { return Some(false) };
    if values.any(|v| v != first) { None } else { Some(first) }
}

/// The properties of a set of glyphs.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct GlyphPropsModel {
    /// A font is open.
    pub has_font: bool,
    /// The glyphs described, in the order given; names the font lacks are left out.
    pub glyphs: Vec<String>,
    /// Font View's master: the layer the metric and layer colour fields read and edit.
    pub master: String,
    /// The text fields, by [`PropField::index`].
    pub fields: Vec<Shown>,
    /// The metric fields of the shown orientation, in [`metric_fields`] order.
    pub metrics: Vec<Shown>,
    /// Whether the glyphs export; `None` when some do and some do not.
    pub export: Option<bool>,
    /// The tags, when every glyph has the same set; `None` otherwise.
    pub tags: Option<Vec<String>>,
    /// Whether the glyphs store their own sort name; `None` when some do.
    pub sort_name_stored: Option<bool>,
    /// The inspector: the vertical metric fields, the extra rows, only the title row.
    pub vertical: bool,
    pub details: bool,
    pub hidden: bool,
}

impl GlyphPropsModel {
    /// What the field `field` shows.
    pub fn field(&self, field: PropField) -> Shown {
        self.fields.get(field.index()).cloned().unwrap_or_default()
    }
}

/// One glyph as a summary reads it.
struct GlyphCx<'a> {
    glyph: &'a Glyph,
    props: Option<Arc<GlyphProps>>,
    master: &'a str,
}

/// The inspector's model: the Font View selection, while the sidebar shows.
pub fn project_inspector(model: &mut Tracked<GlyphPropsModel>, cx: &mut ProjectCx<'_>) {
    let shows = cx.view.workspace.layout.panel_state(&PanelId::Sidebar) != PanelState::Hidden;
    let glyphs = cx.view.font_view.selected.clone();
    project(model, cx, shows, &glyphs);
}

/// Selection Info's model, while its panel shows: the Font View selection while Font View shows, else the glyph
/// the glyph panels show.
pub fn project_selection_info(model: &mut Tracked<GlyphPropsModel>, cx: &mut ProjectCx<'_>) {
    let shows = cx.view.workspace.layout.panel_state(&PanelId::SelectionInfo) != PanelState::Hidden;
    let font_view = cx.view.workspace.layout.panel_state(&PanelId::FontView) != PanelState::Hidden;
    let glyphs = if font_view {
        cx.view.font_view.selected.clone()
    } else {
        let first = || cx.session.font().ok().and_then(|f| f.glyphs.iter().next().map(|g| g.name.clone()));
        cx.view.glyph.clone().or_else(first).into_iter().collect()
    };
    project(model, cx, shows, &glyphs);
}

fn project(model: &mut Tracked<GlyphPropsModel>, cx: &mut ProjectCx<'_>, shows: bool, names: &[String]) {
    let Ok(font) = cx.session.font().cloned() else {
        super::set(model, GlyphPropsModel::default());
        return;
    };
    let side = &cx.view.sidebar;
    let base = GlyphPropsModel {
        has_font: true,
        vertical: side.inspector_vertical,
        details: side.inspector_details,
        hidden: side.inspector_hidden,
        ..GlyphPropsModel::default()
    };
    if !shows {
        super::set(model, base);
        return;
    }
    let master = crate::shell::masters::current_master(&font, &cx.view.layers).map(|m| m.id.clone()).unwrap_or_default();
    let keep = keeps_alternates(&font);
    let data = cx.session.glyph_data().ok();
    let cache = cx.session.effective_props().ok().filter(|c| data.as_deref().is_some_and(|d| c.is_for(d, keep)));
    let glyphs: Vec<&Glyph> = names.iter().take(MAX_SUMMARY_GLYPHS).filter_map(|n| font.glyph(n)).map(Arc::as_ref).collect();
    let rows: Vec<GlyphCx<'_>> = glyphs
        .iter()
        .map(|g| {
            let props = cache.as_ref().and_then(|c| c.get(g).cloned()).or_else(|| data.as_deref().map(|d| Arc::new(glyph_props(g, d, keep))));
            GlyphCx { glyph: g, props, master: &master }
        })
        .collect();
    let fields = PropField::ALL.iter().map(|f| Shown::of(rows.iter().map(|r| f.text(r)))).collect();
    let metrics = metric_fields(base.vertical).iter().map(|f| Shown::of(rows.iter().map(|r| f.text(&font, r.glyph, &master)))).collect();
    // Tags hold no comma, so the joined lists compare like the lists.
    let tags = match Shown::of(rows.iter().map(|r| r.glyph.info.tags.join(","))) {
        Shown { mixed: true, .. } => None,
        _ => Some(rows.first().map(|r| r.glyph.info.tags.clone()).unwrap_or_default()),
    };
    let value = GlyphPropsModel {
        glyphs: rows.iter().map(|r| r.glyph.name.clone()).collect(),
        export: common(rows.iter().map(|r| r.glyph.export)),
        sort_name_stored: common(rows.iter().map(|r| r.props.as_deref().is_some_and(|p| matches!(p.sort_name.source, PropSource::Stored)))),
        fields,
        metrics,
        tags,
        master,
        ..base
    };
    super::set(model, value);
}

/// An edit of a property field.
#[derive(Clone, Debug, PartialEq)]
pub enum Edit {
    /// A text field committed this text.
    Text(PropField, String),
    /// A metric field committed this number or key.
    Metric(MetricField, String),
    Export(bool),
    AddTags(Vec<String>),
    RemoveTag(String),
    /// The sort name's check box was turned off.
    ClearSortName,
}

/// The command an edit runs on the glyphs `model` describes; none for an edit that changes nothing.
pub fn command(model: &GlyphPropsModel, edit: &Edit) -> Option<Msg> {
    let glyphs = &model.glyphs;
    if glyphs.is_empty() {
        return None;
    }
    let or_null = |t: &str| if t.is_empty() { Value::Null } else { json!(t) };
    let list = |t: &str| t.split(|c: char| c.is_whitespace() || c == ',').filter(|s| !s.is_empty()).map(str::to_string).collect::<Vec<_>>();
    let run = |id: &str, params: Value| Some(Msg::run(id, params));
    match edit {
        Edit::Text(field, text) => {
            let text = text.trim();
            match field {
                PropField::Name => {
                    let [glyph] = glyphs.as_slice() else { return None };
                    if text.is_empty() || text == glyph {
                        return None;
                    }
                    run("glyph.rename", json!({"glyph": glyph, "newName": text}))
                }
                PropField::Unicode => run("glyph.setUnicodes", json!({"glyphs": glyphs, "unicodes": list(text)})),
                PropField::ProductionName => run("glyph.setProductionName", json!({"glyphs": glyphs, "productionName": or_null(text)})),
                PropField::Script => run("glyph.setScript", json!({"glyphs": glyphs, "script": or_null(text)})),
                PropField::Category => run("glyph.setCategory", json!({"glyphs": glyphs, "category": or_null(text)})),
                PropField::SubCategory => run("glyph.setCategory", json!({"glyphs": glyphs, "subCategory": or_null(text)})),
                PropField::Case => run("glyph.setCase", json!({"glyphs": glyphs, "case": or_null(text)})),
                PropField::Direction => run("glyph.setDirection", json!({"glyphs": glyphs, "direction": or_null(text)})),
                PropField::SortName => run("glyph.setSortName", json!({"glyphs": glyphs, "sortName": or_null(text)})),
                PropField::Note => run("glyph.setNote", json!({"glyphs": glyphs, "note": text})),
                PropField::LeftGroup => run("kerning.setLeftGroup", json!({"glyphs": glyphs, "group": or_null(text)})),
                PropField::RightGroup => run("kerning.setRightGroup", json!({"glyphs": glyphs, "group": or_null(text)})),
                PropField::LeftKey | PropField::RightKey | PropField::WidthKey => {
                    let metric = match field {
                        PropField::LeftKey => MetricField::Lsb,
                        PropField::RightKey => MetricField::Rsb,
                        _ => MetricField::Width,
                    };
                    metric_command(model, metric, text)
                }
                PropField::Color => run("glyph.setColorLabel", json!({"glyphs": glyphs, "color": color_param(text)})),
                PropField::LayerColor => {
                    run("glyph.setLayerColorLabel", json!({"glyphs": glyphs, "layer": model.master, "color": color_param(text)}))
                }
            }
        }
        Edit::Metric(field, text) => metric_command(model, *field, text.trim()),
        Edit::Export(on) => run("glyph.setExport", json!({"glyphs": glyphs, "export": on})),
        Edit::AddTags(tags) => match tags.as_slice() {
            [] => None,
            [tag] => run("glyph.addTag", json!({"glyphs": glyphs, "tag": tag})),
            _ => run("glyph.addTag", json!({"glyphs": glyphs, "tags": tags})),
        },
        Edit::RemoveTag(tag) => run("glyph.removeTag", json!({"glyphs": glyphs, "tag": tag})),
        Edit::ClearSortName => run("glyph.setSortName", json!({"glyphs": glyphs, "sortName": null})),
    }
}

/// The command a typed number or key runs on `model`'s glyphs in its master's layer; none for an empty text.
fn metric_command(model: &GlyphPropsModel, field: MetricField, text: &str) -> Option<Msg> {
    if text.is_empty() {
        return None;
    }
    let value = match text.parse::<f64>() {
        Ok(n) if n.is_finite() => json!(n),
        _ => json!(text),
    };
    Some(Msg::run(field.command(), json!({"glyphs": model.glyphs, "layer": model.master, "value": value})))
}

/// The `color` parameter of a colour field's value: the index, or null for none.
fn color_param(text: &str) -> Value {
    match text.parse::<u64>() {
        Ok(i) => json!(i),
        Err(_) if text.is_empty() => Value::Null,
        Err(_) => json!(text),
    }
}
