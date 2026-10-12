//! The glyph inspector (`fontView.inspector`): the properties of the glyphs selected in Font
//! View, at the bottom of a column at Font View's left.
//!
//! | Row | Shows | Editing runs |
//! |---|---|---|
//! | Name | the glyph name | `glyph.rename` (one glyph only) |
//! | Metrics | LSB, RSB and width, or TSB, BSB, vertical origin and vertical width ([`super::metrics`]) | the `spacing.set*` command of the field |
//! | Kerning | the left and right kerning groups | `kerning.setLeftGroup`, `kerning.setRightGroup`; an empty field clears the group |
//! | Exports | a check box, a line when the glyphs differ | `glyph.setExport` |
//! | Unicode | the code points, with the chevron of the extra rows | `glyph.setUnicodes` |
//! | Production Name, Script, Category, Subcategory | the extra rows (`fontView.toggleInspectorDetails`) | `glyph.setProductionName`, `glyph.setScript`, `glyph.setCategory` |
//! | Tags | the tags as capsules, and a field for new ones | `glyph.addTag` on a comma, Return or leaving the field; a capsule's cross runs `glyph.removeTag` |
//!
//! With several glyphs selected a field whose value differs between them shows a grey
//! *Multiple Values* (`fontView.inspector.mixedText`), and a typed value goes to every selected
//! glyph in one command. The Selection Info window ([`crate::panels::selection_info`]) uses the
//! same summary, fields and commands.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use egui::{Align2, CornerRadius, Pos2, Rect, RichText, Sense, Shape, Stroke, StrokeKind, vec2};
use fontcraft_model::PropSource;
use serde_json::{Value, json};

use super::cache::Row;
use super::columns::{CellCx, Column};
use super::metrics::{self, MetricField};
use crate::app::FontcraftApp;
use crate::icons::Icon;
use crate::panels::widgets;
use crate::shell::{Args, UiCommand};
use crate::theme::Tokens;

type Pending = Vec<(&'static str, Value)>;

/// The placeholder of a field whose glyphs differ.
pub const MULTIPLE: &str = "Multiple Values";
/// Shown instead of [`MULTIPLE`] in a field too narrow for it.
const MULTIPLE_SHORT: &str = "–";
/// Longest text the Tags field takes, in characters.
const MAX_TAG_CHARS: usize = 255;
/// The salt of the inspector's controls (`widgets::id((SALT, …))`).
pub const SALT: &str = "fontView.inspector";

pub const COMMANDS: &[UiCommand] = &[
    (
        "fontView.toggleInspector",
        "Show / Hide Glyph Inspector",
        &[],
        "",
        "{visible?: bool (omitted: toggle)} show or hide the glyph inspector at the bottom left of Font View → {visible}",
    ),
    (
        "fontView.toggleInspectorDetails",
        "Show / Hide Extra Glyph Properties",
        &[],
        "",
        "{expanded?: bool (omitted: toggle)} show or hide the inspector rows for production name, script, category and subcategory → {expanded}",
    ),
    (
        "edit.infoForSelection",
        "Info for Selection",
        &["Edit"],
        "Cmd+Alt+I",
        "{open?: bool (default true)} open the Selection Info window on the selected glyphs (in Edit View: the edited glyph); false closes it → {open}",
    ),
];

pub fn run(app: &mut FontcraftApp, id: &str, args: &Args) -> Result<Value, String> {
    let panels = &mut app.state.panels;
    match id {
        "fontView.toggleInspector" => {
            let visible = args.flag("visible")?.unwrap_or(panels.inspector_hidden);
            panels.inspector_hidden = !visible;
            Ok(json!({ "visible": visible }))
        }
        "fontView.toggleInspectorDetails" => {
            let expanded = args.flag("expanded")?.unwrap_or(!panels.inspector_details);
            panels.inspector_details = expanded;
            Ok(json!({ "expanded": expanded }))
        }
        "edit.infoForSelection" => {
            let open = args.flag("open")?.unwrap_or(true);
            panels.selection_info = open;
            Ok(json!({ "open": open }))
        }
        other => Err(format!("unknown command `{other}`")),
    }
}

/// What a field shows for the glyphs it describes.
#[derive(Clone, Debug, PartialEq)]
pub enum Shown {
    /// Every glyph has this value (empty with no glyphs).
    Same(String),
    /// The glyphs differ: the field shows *Multiple Values*.
    Mixed,
}

impl Shown {
    /// The common value of `values`; stops at the first that differs.
    pub fn of(values: impl IntoIterator<Item = String>) -> Shown {
        let mut values = values.into_iter();
        let Some(first) = values.next() else { return Shown::Same(String::new()) };
        if values.any(|v| v != first) { Shown::Mixed } else { Shown::Same(first) }
    }

    /// The text a field starts from: empty while mixed.
    pub fn text(&self) -> &str {
        match self {
            Shown::Same(s) => s,
            Shown::Mixed => "",
        }
    }
}

/// `Some(v)` when every value is `v` (`Some(false)` with none), `None` when they differ.
fn common(mut values: impl Iterator<Item = bool>) -> Option<bool> {
    let Some(first) = values.next() else { return Some(false) };
    if values.any(|v| v != first) { None } else { Some(first) }
}

/// The text properties the inspector and Selection Info show.
pub const TEXT_COLUMNS: [Column; 14] = [
    Column::Name,
    Column::Unicode,
    Column::ProductionName,
    Column::Script,
    Column::Category,
    Column::SubCategory,
    Column::Case,
    Column::Direction,
    Column::SortName,
    Column::Lsb,
    Column::Rsb,
    Column::Width,
    Column::LeftGroup,
    Column::RightGroup,
];

/// The properties of a set of glyphs, as the inspector and Selection Info show them.
#[derive(Clone, Debug, PartialEq)]
pub struct Summary {
    /// The glyphs described, in the order given; names the font lacks are left out.
    pub glyphs: Vec<String>,
    fields: HashMap<Column, Shown>,
    /// Whether the glyphs export; `None` when some do and some do not.
    pub export: Option<bool>,
    /// The tags, when every glyph has the same set; `None` otherwise.
    pub tags: Option<Vec<String>>,
    /// Whether the glyphs store their own sort name; `None` when some do.
    pub sort_name_stored: Option<bool>,
    /// The metric fields of the orientation the inspector shows, in Font View's master.
    metrics: HashMap<MetricField, Shown>,
}

impl Summary {
    /// What the field of `column` shows.
    pub fn field(&self, column: Column) -> Shown {
        self.fields.get(&column).cloned().unwrap_or_else(|| Shown::Same(String::new()))
    }

    /// What the metric field `field` shows (empty for a field of the other orientation).
    pub fn metric(&self, field: MetricField) -> Shown {
        self.metrics.get(&field).cloned().unwrap_or_else(|| Shown::Same(String::new()))
    }
}

/// The properties of `glyphs` from Font View's cache, which must be fresh.
pub fn summarize(app: &FontcraftApp, glyphs: &[String]) -> Summary {
    let cache = &app.font_view.cache;
    let rows: Vec<&Row> = match app.session.font() {
        Ok(_) => glyphs.iter().filter_map(|n| cache.row_of(n)).collect(),
        Err(_) => Vec::new(),
    };
    let fields = match app.session.font() {
        Ok(font) => {
            let cx = CellCx { font, master: &cache.master };
            TEXT_COLUMNS.iter().map(|&c| (c, Shown::of(rows.iter().map(|r| c.text(&cx, r))))).collect()
        }
        Err(_) => HashMap::new(),
    };
    let metrics = match app.session.font() {
        Ok(font) => metrics::fields(app.state.panels.inspector_vertical)
            .iter()
            .map(|&f| (f, Shown::of(rows.iter().map(|r| f.text(font, &r.glyph, &cache.master)))))
            .collect(),
        Err(_) => HashMap::new(),
    };
    // Tags hold no comma, so the joined lists compare like the lists.
    let tags = match Shown::of(rows.iter().map(|r| r.glyph.info.tags.join(","))) {
        Shown::Mixed => None,
        Shown::Same(_) => Some(rows.first().map(|r| r.glyph.info.tags.clone()).unwrap_or_default()),
    };
    Summary {
        glyphs: rows.iter().map(|r| r.glyph.name.clone()).collect(),
        fields,
        export: common(rows.iter().map(|r| r.glyph.export)),
        tags,
        sort_name_stored: common(rows.iter().map(|r| matches!(r.props.sort_name.source, PropSource::Stored))),
        metrics,
    }
}

/// [`summarize`], kept in egui's memory under `salt` until the font, Font View's master or the
/// glyphs change.
pub fn cached_summary(app: &FontcraftApp, ctx: &egui::Context, salt: &str, glyphs: &[String]) -> Arc<Summary> {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    app.font_view.cache.generation.hash(&mut h);
    app.font_view.cache.master.hash(&mut h);
    app.state.panels.inspector_vertical.hash(&mut h);
    app.session.active().map(|d| (d.id, d.revision)).hash(&mut h);
    glyphs.hash(&mut h);
    let key = h.finish();
    let id = widgets::id((salt, "summary"));
    if let Some((k, summary)) = ctx.data(|d| d.get_temp::<(u64, Arc<Summary>)>(id))
        && k == key
    {
        return summary;
    }
    let summary = Arc::new(summarize(app, glyphs));
    ctx.data_mut(|d| d.insert_temp(id, (key, summary.clone())));
    summary
}

/// An edit of a property field.
#[derive(Clone, Debug, PartialEq)]
pub enum Edit {
    /// A text field committed this text.
    Text(Column, String),
    /// A metric field committed this number or key.
    Metric(MetricField, String),
    Export(bool),
    AddTags(Vec<String>),
    RemoveTag(String),
    /// The sort name's check box was turned off.
    ClearSortName,
}

/// The commands an edit runs on `glyphs`: one per edit.
pub fn commands(app: &FontcraftApp, glyphs: &[String], edit: &Edit) -> Vec<(&'static str, Value)> {
    if glyphs.is_empty() {
        return Vec::new();
    }
    match edit {
        Edit::Text(Column::Name, _) if glyphs.len() != 1 => Vec::new(),
        Edit::Text(column @ (Column::LeftGroup | Column::RightGroup), text) => {
            let id = if *column == Column::LeftGroup { "kerning.setLeftGroup" } else { "kerning.setRightGroup" };
            let text = text.trim();
            let group = if text.is_empty() { Value::Null } else { json!(text) };
            vec![(id, json!({"glyphs": glyphs, "group": group}))]
        }
        Edit::Metric(field, text) => metrics::edit_command(*field, glyphs, &app.font_view.cache.master, text).into_iter().collect(),
        Edit::Text(column, text) => {
            let Some(row) = glyphs.first().and_then(|g| app.font_view.cache.row_of(g)) else { return Vec::new() };
            if *column == Column::Name && (text.trim().is_empty() || text.trim() == row.glyph.name) {
                return Vec::new();
            }
            let Some((id, mut params)) = column.edit_command_in(Some(&app.font_view.cache.master), row, text) else { return Vec::new() };
            if let Some(m) = params.as_object_mut()
                && m.contains_key("glyphs")
            {
                m.insert("glyphs".into(), json!(glyphs));
            }
            vec![(id, params)]
        }
        Edit::Export(on) => vec![("glyph.setExport", json!({"glyphs": glyphs, "export": on}))],
        Edit::AddTags(tags) => match tags.as_slice() {
            [] => Vec::new(),
            [tag] => vec![("glyph.addTag", json!({"glyphs": glyphs, "tag": tag}))],
            _ => vec![("glyph.addTag", json!({"glyphs": glyphs, "tags": tags}))],
        },
        Edit::RemoveTag(tag) => vec![("glyph.removeTag", json!({"glyphs": glyphs, "tag": tag}))],
        Edit::ClearSortName => vec![("glyph.setSortName", json!({"glyphs": glyphs, "sortName": null}))],
    }
}

/// The id of the field of `column` in the controls made with `salt`.
pub fn field_id(salt: &str, column: Column) -> egui::Id {
    widgets::id((salt, column.id()))
}

/// A text field under the panel field rules; *Multiple Values* in grey while the glyphs differ
/// and the field is not being edited. Returns the committed text.
pub fn text_field(ui: &mut egui::Ui, tokens: &Tokens, id: egui::Id, shown: &Shown, width: f32, enabled: bool) -> Option<String> {
    let committed = ui.add_enabled_ui(enabled, |ui| widgets::text(ui, id, shown.text(), width)).inner;
    if *shown == Shown::Mixed
        && !ui.ctx().memory(|m| m.has_focus(id))
        && let Some(response) = ui.ctx().read_response(id)
    {
        placeholder(ui, tokens, response.rect);
    }
    committed.filter(|_| enabled)
}

/// Paints *Multiple Values* (or a dash where it does not fit) over a field's rectangle.
fn placeholder(ui: &egui::Ui, tokens: &Tokens, rect: Rect) {
    let font = egui::TextStyle::Body.resolve(ui.style());
    let pad = ui.spacing().button_padding.x;
    let fits = ui.painter().layout_no_wrap(MULTIPLE.to_string(), font.clone(), tokens.text_dim).size().x + 2.0 * pad <= rect.width();
    let text = if fits { MULTIPLE } else { MULTIPLE_SHORT };
    ui.painter_at(rect).text(rect.left_center() + vec2(pad, 0.0), Align2::LEFT_CENTER, text, font, tokens.text_dim);
}

/// A labelled text row of a property grid. Returns the edit it commits.
#[allow(clippy::too_many_arguments)]
pub fn text_row(ui: &mut egui::Ui, tokens: &Tokens, salt: &str, label: &str, column: Column, s: &Summary, width: f32, enabled: bool) -> Option<Edit> {
    ui.label(RichText::new(label).color(tokens.text_dim));
    let edit = text_field(ui, tokens, field_id(salt, column), &s.field(column), width, enabled).map(|t| Edit::Text(column, t));
    ui.end_row();
    edit
}

/// A check box with a stable id: checked, empty, or a line for `None` (mixed). True when
/// clicked.
pub fn check_box(ui: &mut egui::Ui, tokens: &Tokens, id: egui::Id, state: Option<bool>, enabled: bool, hover: &str) -> bool {
    let side = ui.spacing().icon_width;
    let (rect, _) = ui.allocate_exact_size(vec2(side, side), Sense::hover());
    let response = ui.interact(rect, id, if enabled { Sense::click() } else { Sense::hover() });
    let painter = ui.painter();
    let marked = state != Some(false);
    let fill = if marked && enabled { tokens.accent } else { tokens.panel_darker };
    painter.rect(rect, CornerRadius::from(tokens.radius), fill, Stroke::new(1.0, tokens.border), StrokeKind::Inside);
    let stroke = Stroke::new(1.5, if enabled { tokens.selected_text } else { tokens.text_dim });
    let at = |x: f32, y: f32| Pos2::new(rect.left() + rect.width() * x, rect.top() + rect.height() * y);
    match state {
        Some(true) => {
            painter.add(Shape::line(vec![at(0.22, 0.52), at(0.42, 0.72), at(0.78, 0.3)], stroke));
        }
        None => {
            painter.line_segment([at(0.25, 0.5), at(0.75, 0.5)], stroke);
        }
        Some(false) => {}
    }
    enabled && response.on_hover_text(hover).clicked()
}

/// A disclosure chevron with a stable id. True when clicked.
pub fn chevron(ui: &mut egui::Ui, tokens: &Tokens, id: egui::Id, expanded: bool, hover: &str) -> bool {
    let side = ui.spacing().interact_size.y;
    let (rect, _) = ui.allocate_exact_size(vec2(side, side), Sense::hover());
    let response = ui.interact(rect, id, Sense::click());
    let c = rect.center();
    let s = side * 0.2;
    let points = if expanded {
        vec![c + vec2(-s, -s * 0.6), c + vec2(s, -s * 0.6), c + vec2(0.0, s * 0.8)]
    } else {
        vec![c + vec2(-s * 0.6, -s), c + vec2(-s * 0.6, s), c + vec2(s * 0.8, 0.0)]
    };
    let color = if response.hovered() { tokens.text } else { tokens.text_dim };
    ui.painter().add(Shape::convex_polygon(points, color, Stroke::NONE));
    response.on_hover_text(hover).clicked()
}

/// The Tags field (`fontView.inspector.tagsField`): the shared tags as capsules with a cross,
/// *Multiple Values* when the glyphs' sets differ, and a field whose text becomes tags on a
/// comma, on Return and on leaving it (Escape drops it).
pub fn tags_field(ui: &mut egui::Ui, tokens: &Tokens, salt: &str, tags: Option<&[String]>, width: f32, enabled: bool) -> Vec<Edit> {
    let mut edits = Vec::new();
    if let Some(tags) = tags.filter(|t| !t.is_empty()) {
        ui.horizontal_wrapped(|ui| {
            ui.set_max_width(width);
            for tag in tags {
                egui::Frame::NONE
                    .fill(tokens.accent)
                    .corner_radius(CornerRadius::from(tokens.radius))
                    .inner_margin(egui::Margin::symmetric(4, 0))
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            ui.spacing_mut().item_spacing.x = 2.0;
                            ui.label(RichText::new(tag).color(tokens.selected_text));
                            if enabled && widgets::flat_button(ui, widgets::id((salt, "removeTag", tag)), "×", "Remove Tag") {
                                edits.push(Edit::RemoveTag(tag.clone()));
                            }
                        });
                    });
            }
        });
    }
    let id = widgets::id((salt, "tags"));
    let key = id.with("text");
    let ctx = ui.ctx().clone();
    let mut text: String = ctx.data(|d| d.get_temp::<String>(key)).unwrap_or_default();
    let hint = if tags.is_none() { MULTIPLE } else { "" };
    let edit = egui::TextEdit::singleline(&mut text).id(id).desired_width(width).char_limit(MAX_TAG_CHARS).hint_text(hint);
    let response = ui.add_enabled(enabled, edit);
    let mut added: Vec<String> = Vec::new();
    if let Some((done, rest)) = text.rsplit_once(',') {
        added.extend(done.split(',').map(str::trim).filter(|t| !t.is_empty()).map(str::to_string));
        text = rest.to_string();
    }
    if response.lost_focus() {
        if !ui.input(|i| i.key_pressed(egui::Key::Escape)) && !text.trim().is_empty() {
            added.push(text.trim().to_string());
        }
        text.clear();
    }
    if text.is_empty() {
        ctx.data_mut(|d| d.remove::<String>(key));
    } else {
        ctx.data_mut(|d| d.insert_temp(key, text));
    }
    let mut seen = std::collections::HashSet::new();
    added.retain(|t| seen.insert(t.clone()));
    if enabled && !added.is_empty() {
        edits.push(Edit::AddTags(added));
    }
    edits
}

/// The widest of `labels` in the body font.
pub fn label_width(ui: &egui::Ui, tokens: &Tokens, labels: &[&str]) -> f32 {
    let font = egui::TextStyle::Body.resolve(ui.style());
    labels.iter().map(|l| ui.painter().layout_no_wrap((*l).to_string(), font.clone(), tokens.text).size().x).fold(0.0, f32::max)
}

/// The width of the column the inspector sits in, in Palette widths: room for a label and a
/// field.
pub const COLUMN_WIDTH: f32 = 1.25;

/// Draws the inspector at the bottom of `ui`: its title row, and its fields while shown.
pub fn show_in(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    let tokens = app.tokens.clone();
    let visible = !app.state.panels.inspector_hidden;
    let details = app.state.panels.inspector_details;
    let vertical = app.state.panels.inspector_vertical;
    let summary = cached_summary(app, ui.ctx(), SALT, &app.state.selected_glyphs);
    let mut edits = Vec::new();
    let frame = egui::Frame::NONE.fill(tokens.panel).inner_margin(tokens.palette_margin);
    egui::Panel::bottom("fontView.inspector").resizable(false).frame(frame).show(ui, |ui| {
        if widgets::section_header(ui, &tokens, SALT, "Glyph Properties", visible) {
            pending.push(("fontView.toggleInspector", json!({ "visible": !visible })));
        }
        if visible {
            edits = fields(ui, &tokens, &summary, details, vertical, pending);
        }
    });
    for edit in &edits {
        pending.extend(commands(app, &summary.glyphs, edit));
    }
}

/// The id of the metric field `field` in the inspector.
pub fn metric_field_id(field: MetricField) -> egui::Id {
    widgets::id((SALT, field.id()))
}

/// The id of the button that switches between the horizontal and vertical metric fields.
pub fn orientation_button_id() -> egui::Id {
    widgets::id((SALT, "metricsOrientation"))
}

/// The Metrics rows: the fields of one orientation, with the switch after the first field.
/// Horizontal: one row of LSB, RSB and width. Vertical: TSB and BSB, then vertical origin and
/// vertical width on a second row.
fn metric_rows(ui: &mut egui::Ui, tokens: &Tokens, s: &Summary, width: f32, vertical: bool, enabled: bool, pending: &mut Pending) -> Vec<Edit> {
    let mut edits = Vec::new();
    let spacing = ui.spacing().item_spacing.x;
    let icon = ui.spacing().interact_size.y * 1.25;
    let shown = metrics::fields(vertical);
    let (first, second) = shown.split_at_checked(if vertical { 2 } else { shown.len() }).unwrap_or((shown, &[]));
    let mut field = |ui: &mut egui::Ui, f: MetricField, w: f32| {
        let id = metric_field_id(f);
        if let Some(t) = text_field(ui, tokens, id, &s.metric(f), w, enabled && f.editable()) {
            edits.push(Edit::Metric(f, t));
        }
        if let Some(r) = ui.ctx().read_response(id) {
            r.on_hover_text(f.label());
        }
    };
    ui.label(RichText::new("Metrics").color(tokens.text_dim));
    ui.horizontal(|ui| {
        let n = first.len().max(1) as f32;
        let w = ((width - icon - n * spacing) / n).max(0.0);
        for (i, &f) in first.iter().enumerate() {
            field(ui, f, w);
            if i == 0 {
                let (glyph, hover, next) = if vertical {
                    (Icon::TopToBottom, "Show Horizontal Metrics", "horizontal")
                } else {
                    (Icon::LeftToRight, "Show Vertical Metrics", "vertical")
                };
                if widgets::icon_button(ui, orientation_button_id(), glyph, hover, true, false) {
                    pending.push(("fontView.toggleMetricsOrientation", json!({ "orientation": next })));
                }
            }
        }
    });
    ui.end_row();
    if !second.is_empty() {
        ui.label("");
        ui.horizontal(|ui| {
            let n = second.len() as f32;
            let w = ((width - (n - 1.0) * spacing) / n).max(0.0);
            for &f in second {
                field(ui, f, w);
            }
        });
        ui.end_row();
    }
    edits
}

/// The inspector's rows. Returns the edits committed this frame.
fn fields(ui: &mut egui::Ui, tokens: &Tokens, s: &Summary, details: bool, vertical: bool, pending: &mut Pending) -> Vec<Edit> {
    let mut edits = Vec::new();
    let any = !s.glyphs.is_empty();
    let one = s.glyphs.len() == 1;
    let mut labels = vec!["Name", "Metrics", "Kerning", "Exports", "Unicode", "Tags"];
    if details {
        labels.extend(["Production Name", "Script", "Category", "Subcategory"]);
    }
    let spacing = ui.spacing().item_spacing.x;
    let labels_width = label_width(ui, tokens, &labels);
    let width = (ui.available_width() - labels_width - 2.0 * spacing).max(tokens.field_width);
    // The label column is as wide as the longest label shown, and the fields take the rest of
    // the panel.
    egui::Grid::new(widgets::id((SALT, "grid"))).num_columns(2).min_col_width(labels_width).show(ui, |ui| {
        edits.extend(text_row(ui, tokens, SALT, "Name", Column::Name, s, width, one));

        edits.extend(metric_rows(ui, tokens, s, width, vertical, any, pending));

        ui.label(RichText::new("Kerning").color(tokens.text_dim));
        ui.horizontal(|ui| {
            let half = (width - spacing) / 2.0;
            for (column, hover) in [(Column::LeftGroup, "Left Group"), (Column::RightGroup, "Right Group")] {
                if let Some(t) = text_field(ui, tokens, field_id(SALT, column), &s.field(column), half, any) {
                    edits.push(Edit::Text(column, t));
                }
                if let Some(r) = ui.ctx().read_response(field_id(SALT, column)) {
                    r.on_hover_text(hover);
                }
            }
        });
        ui.end_row();

        ui.label(RichText::new("Exports").color(tokens.text_dim));
        if check_box(ui, tokens, widgets::id((SALT, "export")), s.export, any, "Exports") {
            edits.push(Edit::Export(s.export != Some(true)));
        }
        ui.end_row();

        ui.label(RichText::new("Unicode").color(tokens.text_dim));
        ui.horizontal(|ui| {
            let side = ui.spacing().interact_size.y;
            let field = (width - side - spacing).max(tokens.field_width);
            if let Some(t) = text_field(ui, tokens, field_id(SALT, Column::Unicode), &s.field(Column::Unicode), field, any) {
                edits.push(Edit::Text(Column::Unicode, t));
            }
            let hover = if details { "Hide Extra Glyph Properties" } else { "Show Extra Glyph Properties" };
            if chevron(ui, tokens, widgets::id((SALT, "details")), details, hover) {
                pending.push(("fontView.toggleInspectorDetails", json!({ "expanded": !details })));
            }
        });
        ui.end_row();

        if details {
            for column in [Column::ProductionName, Column::Script, Column::Category, Column::SubCategory] {
                edits.extend(text_row(ui, tokens, SALT, column.label(), column, s, width, any));
            }
        }

        ui.label(RichText::new("Tags").color(tokens.text_dim));
        ui.vertical(|ui| edits.extend(tags_field(ui, tokens, SALT, s.tags.as_deref(), width, any)));
        ui.end_row();
    });
    edits
}
