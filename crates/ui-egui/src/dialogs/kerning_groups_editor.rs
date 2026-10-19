//! The Kerning Groups Editor (`dialog.kerningGroupsEditor`): the kerning groups of one side, the
//! members of the current group as cells, as a list or stacked, and every glyph of the font below
//! for dragging into the group. The shell command lives in `shell::kerning_review`.
//!
//! | Input | Does |
//! |---|---|
//! | Left, Right | the side whose groups are listed (`{side}`) |
//! | Cells, List, Stacked | how the members are drawn (`{view}`) |
//! | click a group | makes it the current group (`{group}`) |
//! | name field and + | starts a new, empty group of that name |
//! | − under the groups | clears the current group from every member (`kerning.setLeftGroup` or `kerning.setRightGroup` with `null`) |
//! | drop a glyph from the font on the members | adds it to the current group |
//! | − beside a member (List) | takes it out of the group |
//! | Propose, Apply, Discard | `kerning.buildGroups` without applying, its proposal shown for review; Apply stores it |
//!
//! The stacked view draws every member over the others, aligned where the group's side meets the
//! pair: a left group (the glyph comes second in left-to-right text) on the glyphs' origin, a
//! right group on their advance width, so a member whose shape or sidebearing does not fit stands
//! out.

use egui::{Align2, FontId, Pos2, Rect, RichText, Sense, Stroke, pos2, vec2};
use fontcraft_model::Font;
use fontcraft_spacing::kerning::{GroupSide, check_group_name, group_list, members};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::kerning_pairs_preview::paint_outline;
use crate::app::FontcraftApp;
use crate::panels::Pending;
use crate::shell::Args;

/// Most members the stacked view draws over each other.
pub const MAX_STACKED: usize = 500;
/// Default limit a proposal is asked with, in font units.
pub const DEFAULT_MAX_DIFFERENCE: f64 = 20.0;
/// Characters of untrusted text quoted in a message.
const MAX_QUOTED: usize = 64;
/// Glyph cells per row in the font's glyph list.
const GLYPHS_PER_ROW: usize = 8;

/// The side whose groups the editor lists.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EditorSide {
    /// Groups of the glyphs' left sides: the glyph comes second in left-to-right text.
    #[default]
    Left,
    /// Groups of the glyphs' right sides: the glyph comes first.
    Right,
}

impl EditorSide {
    pub fn name(self) -> &'static str {
        match self {
            EditorSide::Left => "left",
            EditorSide::Right => "right",
        }
    }

    pub fn from_name(name: &str) -> Option<EditorSide> {
        [EditorSide::Left, EditorSide::Right].into_iter().find(|s| s.name() == name)
    }

    pub fn group_side(self) -> GroupSide {
        match self {
            EditorSide::Left => GroupSide::Left,
            EditorSide::Right => GroupSide::Right,
        }
    }

    /// The engine command that sets this side's group.
    pub fn set_command(self) -> &'static str {
        match self {
            EditorSide::Left => "kerning.setLeftGroup",
            EditorSide::Right => "kerning.setRightGroup",
        }
    }
}

/// How the members are drawn.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GroupsView {
    #[default]
    Cells,
    List,
    Stacked,
}

impl GroupsView {
    pub fn name(self) -> &'static str {
        match self {
            GroupsView::Cells => "cells",
            GroupsView::List => "list",
            GroupsView::Stacked => "stacked",
        }
    }

    pub fn from_name(name: &str) -> Option<GroupsView> {
        [GroupsView::Cells, GroupsView::List, GroupsView::Stacked].into_iter().find(|v| v.name() == name)
    }

    fn label(self) -> &'static str {
        match self {
            GroupsView::Cells => "Cells",
            GroupsView::List => "List",
            GroupsView::Stacked => "Stacked",
        }
    }
}

/// A `kerning.buildGroups` proposal under review: the parameters it was asked with, the document
/// and revision it describes, and its reply.
#[derive(Clone, Debug, PartialEq)]
pub struct Proposal {
    pub params: Map<String, Value>,
    pub document: (u64, u64),
    pub reply: Value,
}

/// The editor's state, kept in `KerningReviewState`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct GroupsEditorState {
    pub open: bool,
    pub side: EditorSide,
    /// The current group; it may have no member yet.
    pub group: Option<String>,
    pub view: GroupsView,
    /// The limit the Propose button asks with.
    pub max_difference: f64,
    #[serde(skip)]
    pub proposal: Option<Proposal>,
    /// The name field for a new group.
    #[serde(skip)]
    pub new_name: String,
}

impl Default for GroupsEditorState {
    fn default() -> Self {
        Self {
            open: false,
            side: EditorSide::default(),
            group: None,
            view: GroupsView::default(),
            max_difference: DEFAULT_MAX_DIFFERENCE,
            proposal: None,
            new_name: String::new(),
        }
    }
}

fn quoted(s: &str) -> String {
    s.chars().take(MAX_QUOTED).collect()
}

/// The master the editor draws: the showing tab's, else the first.
fn shown_master(app: &FontcraftApp) -> Option<String> {
    let font = app.session.font().ok()?;
    app.state
        .tab()
        .and_then(|t| t.master.clone())
        .filter(|m| font.masters.iter().any(|x| x.id == *m))
        .or_else(|| font.masters.first().map(|m| m.id.clone()))
}

/// Where each member is drawn in the stacked view, in font units from the shared alignment line:
/// 0 for a left group (aligned on the origin), minus the advance width for a right group.
pub fn stacked_offsets(font: &Font, master: &str, side: EditorSide, glyphs: &[&str]) -> Vec<(String, f64)> {
    glyphs
        .iter()
        .take(MAX_STACKED)
        .map(|name| {
            let width = font.glyph(name).and_then(|g| g.layer(master)).map_or(0.0, |l| l.width);
            let offset = match side {
                EditorSide::Left => 0.0,
                EditorSide::Right => -width,
            };
            (name.to_string(), offset)
        })
        .collect()
}

/// `dialog.kerningGroupsEditor`.
pub fn window(app: &mut FontcraftApp, args: &Args) -> Result<Value, String> {
    let open = args.flag("open")?.unwrap_or(true);
    let side = match args.string("side")? {
        Some(s) => Some(EditorSide::from_name(s).ok_or_else(|| args.err(format!("`side` must be left or right, got `{}`", quoted(s))))?),
        None => None,
    };
    let view = match args.string("view")? {
        Some(v) => Some(GroupsView::from_name(v).ok_or_else(|| args.err(format!("`view` must be cells, list or stacked, got `{}`", quoted(v))))?),
        None => None,
    };
    let group = match args.get("group") {
        Some(Value::Null) => Some(None),
        Some(_) => {
            let name = args.string("group")?.unwrap_or_default();
            let name = name.strip_prefix('@').unwrap_or(name);
            check_group_name(name).map_err(|e| args.err(format!("`group`: {e}")))?;
            Some(Some(name.to_string()))
        }
        None => None,
    };
    let apply = args.flag("apply")?.unwrap_or(false);
    let propose = match args.get("propose") {
        Some(Value::Null) => Some(None),
        Some(Value::Object(m)) => {
            let mut m = m.clone();
            if m.contains_key("apply") {
                return Err(args.err("`propose` takes the parameters of kerning.buildGroups without `apply`"));
            }
            m.insert("apply".into(), json!(false));
            Some(Some(m))
        }
        Some(_) => return Err(args.err("`propose` must be an object (the parameters of kerning.buildGroups) or null")),
        None => None,
    };
    if apply && propose.is_some() {
        return Err(args.err("give `propose` or `apply`, not both"));
    }
    let document = app.session.doc().map(|d| (d.id, d.revision)).map_err(|e| e.to_string())?;
    if apply {
        let proposal = app.state.panels.kerning_review.groups.proposal.clone().ok_or_else(|| args.err("there is no proposal to apply"))?;
        if proposal.document != document {
            return Err(args.err("the font changed since the proposal was made; propose again"));
        }
        let mut params = proposal.params;
        params.insert("apply".into(), json!(true));
        app.run("kerning.buildGroups", &Value::Object(params)).map_err(|e| args.err(e))?;
        app.state.panels.kerning_review.groups.proposal = None;
    }
    if let Some(Some(params)) = &propose {
        let reply = app.run("kerning.buildGroups", &Value::Object(params.clone())).map_err(|e| args.err(e))?;
        app.state.panels.kerning_review.groups.proposal = Some(Proposal { params: params.clone(), document, reply });
    }
    let st = &mut app.state.panels.kerning_review.groups;
    if propose == Some(None) {
        st.proposal = None;
    }
    st.open = open;
    if let Some(s) = side
        && s != st.side
    {
        st.side = s;
        st.group = None;
    }
    if let Some(v) = view {
        st.view = v;
    }
    if let Some(g) = group {
        st.group = g;
    }
    Ok(reply(app))
}

/// The editor as `dialog.kerningGroupsEditor` returns it.
pub fn reply(app: &FontcraftApp) -> Value {
    let st = &app.state.panels.kerning_review.groups;
    let master = shown_master(app).unwrap_or_default();
    let Ok(font) = app.session.font() else { return json!({"open": st.open}) };
    let groups: Vec<Value> = group_list(font, st.side.group_side()).iter().map(|(name, glyphs)| json!({"name": name, "glyphs": glyphs})).collect();
    let member_list: Vec<&str> = st.group.as_deref().map(|g| members(font, st.side.group_side(), g)).unwrap_or_default();
    let stacked: Vec<Value> =
        stacked_offsets(font, &master, st.side, &member_list).into_iter().map(|(name, offset)| json!({"name": name, "offset": offset})).collect();
    json!({
        "open": st.open,
        "side": st.side.name(),
        "groups": groups,
        "group": st.group,
        "members": member_list,
        "view": st.view.name(),
        "master": master,
        "stacked": {"aligned": if st.side == EditorSide::Left { "origin" } else { "advance" }, "glyphs": stacked},
        "proposal": st.proposal.as_ref().map(|p| p.reply.clone()),
    })
}

/// Draws the editor while it is open.
pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    if !app.state.panels.kerning_review.groups.open {
        return;
    }
    let Some(master) = shown_master(app) else { return };
    let Ok(font) = app.session.doc().map(|d| d.font.clone()) else { return };
    let FontcraftApp { state, tokens, .. } = app;
    let tokens: &crate::theme::Tokens = tokens;
    let st = &mut state.panels.kerning_review.groups;
    let side = st.side.group_side();
    let groups = group_list(&font, side);
    let member_list: Vec<String> = st.group.as_deref().map(|g| members(&font, side, g).into_iter().map(str::to_string).collect()).unwrap_or_default();
    let mut open = true;
    egui::Window::new("Kerning Groups Editor")
        .id(egui::Id::new("dialog.kerningGroupsEditor"))
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_size(vec2(tokens.palette_width * 2.5, tokens.list_row_height * 30.0))
        .default_pos(pos2(ui.ctx().content_rect().center().x, tokens.dialog_top))
        .show(ui.ctx(), |ui| {
            ui.horizontal(|ui| {
                for s in [EditorSide::Left, EditorSide::Right] {
                    let label = if s == EditorSide::Left { "Left Groups" } else { "Right Groups" };
                    if ui.selectable_label(st.side == s, label).clicked() && st.side != s {
                        pending.push(("dialog.kerningGroupsEditor", json!({"side": s.name()})));
                    }
                }
                ui.separator();
                for v in [GroupsView::Cells, GroupsView::List, GroupsView::Stacked] {
                    if ui.selectable_label(st.view == v, v.label()).clicked() && st.view != v {
                        pending.push(("dialog.kerningGroupsEditor", json!({"view": v.name()})));
                    }
                }
            });
            ui.separator();
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(tokens.palette_width * 0.6);
                    egui::ScrollArea::vertical().id_salt("groups").max_height(tokens.list_row_height * 12.0).show(ui, |ui| {
                        for (name, glyphs) in &groups {
                            let current = st.group.as_deref() == Some(*name);
                            if ui.selectable_label(current, format!("{name} ({})", glyphs.len())).clicked() && !current {
                                pending.push(("dialog.kerningGroupsEditor", json!({"group": name})));
                            }
                        }
                        if let Some(g) = st.group.as_deref().filter(|g| !groups.iter().any(|(n, _)| n == g)) {
                            let _ = ui.selectable_label(true, format!("{g} (new)"));
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut st.new_name).desired_width(tokens.field_width).hint_text("new group"));
                        if ui.button("+").clicked() && !st.new_name.trim().is_empty() {
                            pending.push(("dialog.kerningGroupsEditor", json!({"group": st.new_name.trim()})));
                            st.new_name.clear();
                        }
                        if ui
                            .add_enabled(!member_list.is_empty(), egui::Button::new("−"))
                            .on_hover_text("Clear this group from every member")
                            .clicked()
                        {
                            pending.push((st.side.set_command(), json!({"glyphs": member_list, "group": null})));
                        }
                    });
                });
                ui.separator();
                ui.vertical(|ui| {
                    let (_, dropped) = ui.dnd_drop_zone::<String, _>(egui::Frame::NONE, |ui| {
                        ui.set_min_size(vec2(tokens.cell_size * 4.0, tokens.cell_size * 1.5));
                        members_view(ui, tokens, &font, &master, st, &member_list, pending);
                    });
                    if let (Some(glyph), Some(group)) = (dropped, st.group.as_deref()) {
                        pending.push((st.side.set_command(), json!({"glyphs": [glyph.as_str()], "group": group})));
                    }
                });
            });
            ui.separator();
            proposal_view(ui, tokens, st, pending);
            ui.separator();
            ui.label(RichText::new("Glyphs (drag onto the members)").color(tokens.text_dim));
            glyph_list(ui, tokens, &font, &master);
        });
    if !open {
        pending.push(("dialog.kerningGroupsEditor", json!({"open": false})));
    }
}

fn members_view(
    ui: &mut egui::Ui,
    tokens: &crate::theme::Tokens,
    font: &Font,
    master: &str,
    st: &GroupsEditorState,
    list: &[String],
    pending: &mut Pending,
) {
    if st.group.is_none() {
        ui.label(RichText::new("Choose or start a group").color(tokens.text_dim));
        return;
    }
    let ink = Stroke::new(1.0, tokens.glyph_fill);
    match st.view {
        GroupsView::Cells => {
            ui.horizontal_wrapped(|ui| {
                for name in list {
                    glyph_cell(ui, tokens, font, master, name);
                }
            });
        }
        GroupsView::List => {
            for name in list {
                ui.horizontal(|ui| {
                    ui.label(name);
                    if ui.small_button("−").on_hover_text("Take this glyph out of the group").clicked() {
                        pending.push((st.side.set_command(), json!({"glyphs": [name], "group": null})));
                    }
                });
            }
        }
        GroupsView::Stacked => {
            let upm = f32::from(font.units_per_em.max(1));
            let height = tokens.cell_size * 3.0;
            let scale = height / upm * 0.8;
            let (rect, _) = ui.allocate_exact_size(vec2(tokens.cell_size * 6.0, height), Sense::hover());
            let painter = ui.painter_at(rect);
            painter.rect_stroke(rect, tokens.radius, Stroke::new(1.0, tokens.border), egui::StrokeKind::Inside);
            let names: Vec<&str> = list.iter().map(String::as_str).collect();
            // The alignment line: the origin of a left group near the left edge, the advance of a
            // right group near the right edge.
            let line_x = if st.side == EditorSide::Left { rect.left() + rect.width() * 0.2 } else { rect.right() - rect.width() * 0.2 };
            let baseline = rect.bottom() - height * 0.15;
            painter.line_segment([pos2(line_x, rect.top()), pos2(line_x, rect.bottom())], Stroke::new(1.0, tokens.metric_line));
            for (name, offset) in stacked_offsets(font, master, st.side, &names) {
                paint_outline(&painter, font, &name, master, pos2(line_x + offset as f32 * scale, baseline), scale, ink);
            }
        }
    }
}

fn glyph_cell(ui: &mut egui::Ui, tokens: &crate::theme::Tokens, font: &Font, master: &str, name: &str) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(tokens.cell_size, tokens.cell_size + tokens.cell_label_height), Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_stroke(rect, tokens.radius, Stroke::new(1.0, tokens.border), egui::StrokeKind::Inside);
    let glyph_rect = Rect::from_min_size(rect.min, vec2(tokens.cell_size, tokens.cell_size));
    crate::master_icons::paint_glyph(&painter, glyph_rect, font, name, master, tokens.glyph_fill);
    painter.text(
        Pos2::new(rect.center().x, rect.bottom() - tokens.cell_label_height / 2.0),
        Align2::CENTER_CENTER,
        name,
        FontId::proportional(tokens.cell_label_size),
        tokens.cell_label,
    );
    response
}

/// The font's glyphs as cells that can be dragged onto the members.
fn glyph_list(ui: &mut egui::Ui, tokens: &crate::theme::Tokens, font: &Font, master: &str) {
    let height = tokens.cell_size + tokens.cell_label_height + tokens.cell_gap;
    let rows = font.glyphs.len().div_ceil(GLYPHS_PER_ROW);
    egui::ScrollArea::vertical().id_salt("glyphs").max_height(height * 3.0).show_rows(ui, height, rows, |ui, range| {
        for row in range {
            ui.horizontal(|ui| {
                let start = row.saturating_mul(GLYPHS_PER_ROW);
                for glyph in font.glyphs.iter().skip(start).take(GLYPHS_PER_ROW) {
                    let name = glyph.name.clone();
                    ui.dnd_drag_source(egui::Id::new(("dialog.kerningGroupsEditor.glyph", &name)), name.clone(), |ui| {
                        glyph_cell(ui, tokens, font, master, &name);
                    });
                }
            });
        }
    });
}

/// The proposal under review, or the controls that ask for one.
fn proposal_view(ui: &mut egui::Ui, tokens: &crate::theme::Tokens, st: &mut GroupsEditorState, pending: &mut Pending) {
    let Some(proposal) = &st.proposal else {
        ui.horizontal(|ui| {
            ui.label("Largest profile difference");
            ui.add(egui::DragValue::new(&mut st.max_difference).range(0.0..=1_000.0).speed(1.0));
            if ui.button("Propose Groups").on_hover_text("Group glyphs whose side profiles match, for review").clicked() {
                pending.push(("dialog.kerningGroupsEditor", json!({"propose": {"maxDifference": st.max_difference, "side": st.side.name()}})));
            }
        });
        return;
    };
    ui.label(RichText::new("Proposed groups").strong());
    egui::ScrollArea::vertical().id_salt("proposal").max_height(tokens.list_row_height * 8.0).show(ui, |ui| {
        for key in ["left", "right"] {
            for g in proposal.reply.get("groups").and_then(|g| g.get(key)).and_then(Value::as_array).into_iter().flatten() {
                let name = g.get("name").and_then(Value::as_str).unwrap_or_default();
                let glyphs: Vec<&str> = g.get("glyphs").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).collect();
                ui.label(format!("{key} {name}: {}", glyphs.join(" ")));
            }
        }
        let skipped = proposal.reply.get("skipped").and_then(Value::as_array).map_or(0, Vec::len);
        if skipped > 0 {
            ui.label(RichText::new(format!("{skipped} skipped")).color(tokens.text_dim));
        }
    });
    ui.horizontal(|ui| {
        if ui.button("Apply").clicked() {
            pending.push(("dialog.kerningGroupsEditor", json!({"apply": true})));
        }
        if ui.button("Discard").clicked() {
            pending.push(("dialog.kerningGroupsEditor", json!({"propose": null})));
        }
    });
}
