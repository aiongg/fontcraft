//! The Kerning Pairs Preview and the Kerning Groups Editor: their shell commands and view state.
//!
//! | Command | Does |
//! |---|---|
//! | `window.kerningPairsPreview {open?, master?, glyph?, sort?, size?, select?, jump?}` | shows or hides the preview and sets its options |
//! | `dialog.kerningGroupsEditor {open?, side?, group?, view?, propose?, apply?}` | shows or hides the editor, sets its side, group and view, proposes groups and applies the proposal |
//!
//! What each panel lists is its projection's (`app::models::kerning_tools`). Every parameter is read and checked
//! before anything changes, so a refused command changes nothing.

use std::collections::BTreeMap;

use craft::ui::contract::PlainData;
use craft::ui::runtime::commands::CommandRef;
use fontcraft_spacing::kerning::pairs::stored_pairs;
use fontcraft_spacing::kerning::{Direction, GroupSide, Side, check_group_name};
use serde_json::{Map, Value, json};

use super::commands::{Module, UiCommand};
use super::dock::PanelId;
use super::params::{flag, quoted, string};
use crate::app::FontCraftApp;

pub const MODULE: Module = Module { commands: COMMANDS, run };

/// The render sizes the preview offers, in pixels per em.
pub const SIZES: [u32; 6] = [24, 36, 48, 72, 96, 144];
pub const DEFAULT_SIZE: u32 = 48;
/// Default limit a proposal is asked with, in font units.
pub const DEFAULT_MAX_DIFFERENCE: f64 = 20.0;
/// Largest limit the editor's field takes, in font units.
pub const MAX_DIFFERENCE: f64 = 1_000.0;

const COMMANDS: &[UiCommand] = &[
    UiCommand {
        id: "window.kerningPairsPreview",
        label: "Kerning Pairs Preview",
        menu: &["Window"],
        shortcut: None,
        params: "{open?: bool (default true), master?: id|null (default: the current master), glyph?: name|null (only the pairs naming this glyph or a group it belongs to on the side facing the pair; null: every pair), sort?: \"first\"|\"second\"|\"value\" (first or second glyph in glyph order, or size of the value, largest first), size?: 24|36|48|72|96|144 (pixels per em), select?: {left, right}|null, jump?: one character (scroll to the first pair whose sorted-by glyph has it)} show or hide the Kerning Pairs Preview: every stored left-to-right pair of the master drawn kerned with its value underneath, a group side drawn by its first member. Values that differ from the ones found when the window opened are marked changed. A selected pair with a group side lists every glyph combination it covers, each with the stored pair that applies to it",
    },
    UiCommand {
        id: "dialog.kerningGroupsEditor",
        label: "Kerning Groups Editor",
        menu: &["Window"],
        shortcut: None,
        params: "{open?: bool (default true), side?: \"left\"|\"right\", group?: name|null (the current group; a new name starts an empty group), view?: \"cells\"|\"list\"|\"stacked\", maxDifference?: units (the limit Propose Groups asks with), propose?: {the parameters of kerning.buildGroups without apply}|null, apply?: bool} show or hide the Kerning Groups Editor: the groups of one side, the members of the current group as cells, as a list, or stacked (every member drawn over the others, aligned on the origin for a left group and on the advance width for a right group), and every glyph of the font to drag in. Members change through kerning.setLeftGroup and kerning.setRightGroup. propose runs kerning.buildGroups without storing and shows its proposal; null discards it; apply stores the proposal shown, refused when the font changed since",
    },
];

/// How the preview orders the pairs.
#[derive(PlainData, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PreviewSort {
    /// By the first glyph in glyph order, then the second.
    #[default]
    First,
    /// By the second glyph in glyph order, then the first.
    Second,
    /// By the size of the value, largest first.
    Value,
}

impl PreviewSort {
    pub const ALL: [PreviewSort; 3] = [PreviewSort::First, PreviewSort::Second, PreviewSort::Value];

    pub fn name(self) -> &'static str {
        match self {
            PreviewSort::First => "first",
            PreviewSort::Second => "second",
            PreviewSort::Value => "value",
        }
    }

    pub fn from_name(name: &str) -> Option<PreviewSort> {
        PreviewSort::ALL.into_iter().find(|s| s.name() == name)
    }
}

/// The side whose groups the editor lists.
#[derive(PlainData, Clone, Copy, Debug, Default, PartialEq, Eq)]
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

/// How the editor draws the members.
#[derive(PlainData, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum GroupsView {
    #[default]
    Cells,
    List,
    Stacked,
}

impl GroupsView {
    pub const ALL: [GroupsView; 3] = [GroupsView::Cells, GroupsView::List, GroupsView::Stacked];

    pub fn name(self) -> &'static str {
        match self {
            GroupsView::Cells => "cells",
            GroupsView::List => "list",
            GroupsView::Stacked => "stacked",
        }
    }

    pub fn from_name(name: &str) -> Option<GroupsView> {
        GroupsView::ALL.into_iter().find(|v| v.name() == name)
    }
}

/// The preview's options and what it found when it opened.
#[derive(Clone, Debug, PartialEq)]
pub struct PreviewState {
    /// `None`: the current master.
    pub master: Option<String>,
    /// Only the pairs naming this glyph, or a group it belongs to on the facing side.
    pub glyph: Option<String>,
    pub sort: PreviewSort,
    pub size: u32,
    /// The selected pair, as commands spell its sides.
    pub selected: Option<(String, String)>,
    /// The master and its values when the preview opened, by row id.
    pub opened_with: Option<(String, BTreeMap<String, f64>)>,
    /// The character a `jump` asked for; the projection scrolls to its first pair.
    pub jump: Option<char>,
}

impl Default for PreviewState {
    fn default() -> Self {
        PreviewState { master: None, glyph: None, sort: PreviewSort::default(), size: DEFAULT_SIZE, selected: None, opened_with: None, jump: None }
    }
}

/// A `kerning.buildGroups` proposal under review: the parameters it was asked with, the document and revision it
/// describes, and its reply.
#[derive(Clone, Debug, PartialEq)]
pub struct Proposal {
    pub params: Map<String, Value>,
    pub document: (u64, u64),
    pub reply: Value,
}

/// The editor's side, group, view and the proposal under review.
#[derive(Clone, Debug, PartialEq)]
pub struct GroupsState {
    pub side: EditorSide,
    /// The current group; it may have no member yet.
    pub group: Option<String>,
    pub view: GroupsView,
    /// The limit Propose Groups asks with.
    pub max_difference: f64,
    pub proposal: Option<Proposal>,
}

impl Default for GroupsState {
    fn default() -> Self {
        GroupsState { side: EditorSide::default(), group: None, view: GroupsView::default(), max_difference: DEFAULT_MAX_DIFFERENCE, proposal: None }
    }
}

/// The view state of both panels.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct KerningToolsView {
    pub preview: PreviewState,
    pub groups: GroupsState,
}

/// A row's name: its sides joined by a tab, which no side may hold.
pub fn row_id(left: &str, right: &str) -> String {
    format!("{left}\t{right}")
}

/// The master the preview shows: its own choice when the font has it, else the current master, else the first.
pub fn preview_master(font: &fontcraft_engine::model::Font, view: &crate::app::ViewState) -> Option<String> {
    view.kerning_tools
        .preview
        .master
        .clone()
        .filter(|m| font.masters.iter().any(|x| x.id == *m))
        .or_else(|| super::masters::current_master(font, &view.layers).map(|m| m.id.clone()))
}

fn run(app: &mut FontCraftApp, command: &CommandRef) -> Result<(), String> {
    match command.id.as_str() {
        "window.kerningPairsPreview" => preview(app, &command.params),
        "dialog.kerningGroupsEditor" => groups_editor(app, &command.params),
        other => Err(format!("no kerning tools command `{other}`")),
    }
}

/// `{left, right}` at `key` as command spellings; `Some(None)` for an explicit null.
fn side_pair(p: &Value, key: &str) -> Result<Option<Option<(String, String)>>, String> {
    let Some(v) = p.get(key) else { return Ok(None) };
    if v.is_null() {
        return Ok(Some(None));
    }
    let side = |k: &str| -> Result<String, String> {
        let text = v.get(k).and_then(Value::as_str).ok_or_else(|| format!("`{key}.{k}` (a glyph name or @group) is required"))?;
        Side::parse(text).map(|s| s.command()).map_err(|e| format!("`{key}.{k}`: {e}"))
    };
    Ok(Some(Some((side("left")?, side("right")?))))
}

/// A name at `key` the font must hold, checked by `has`; `Some(None)` for an explicit null.
fn named(p: &Value, key: &str, what: &str, has: impl Fn(&str) -> bool) -> Result<Option<Option<String>>, String> {
    match p.get(key) {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(_) => {
            let name = string(p, key)?.unwrap_or_default();
            if !has(name) {
                return Err(format!("the font has no {what} `{}`", quoted(name)));
            }
            Ok(Some(Some(name.to_string())))
        }
    }
}

fn preview(app: &mut FontCraftApp, p: &Value) -> Result<(), String> {
    let open = flag(p, "open")?.unwrap_or(true);
    let font = app.session.font().map_err(|e| e.to_string())?;
    let master = named(p, "master", "master", |id| font.masters.iter().any(|m| m.id == id))?;
    let glyph = named(p, "glyph", "glyph", |name| font.glyph(name).is_some())?;
    let sort = match string(p, "sort")? {
        Some(s) => Some(PreviewSort::from_name(s).ok_or_else(|| format!("`sort` must be first, second or value, got `{}`", quoted(s)))?),
        None => None,
    };
    let size = match p.get("size") {
        None => None,
        Some(v) => Some(
            v.as_u64().and_then(|s| u32::try_from(s).ok()).filter(|s| SIZES.contains(s)).ok_or_else(|| format!("`size` must be one of {SIZES:?}"))?,
        ),
    };
    let select = side_pair(p, "select")?;
    let jump = match string(p, "jump")? {
        Some(text) => {
            let mut chars = text.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => Some(c),
                _ => return Err("`jump` must be one character".into()),
            }
        }
        None => None,
    };
    crate::shell::dock::show_window(&mut app.view.workspace, PanelId::KerningPairsPreview, open, app.view.canvas.rect);
    let pv = &mut app.view.kerning_tools.preview;
    if !open {
        pv.opened_with = None;
        pv.jump = None;
        return Ok(());
    }
    if let Some(m) = master {
        pv.master = m;
    }
    if let Some(g) = glyph {
        pv.glyph = g;
    }
    if let Some(s) = sort {
        pv.sort = s;
    }
    if let Some(s) = size {
        pv.size = s;
    }
    if let Some(s) = select {
        pv.selected = s;
    }
    pv.jump = jump;
    let font = app.session.font().map_err(|e| e.to_string())?;
    let shown = preview_master(font, &app.view).unwrap_or_default();
    if app.view.kerning_tools.preview.opened_with.as_ref().is_none_or(|(m, _)| *m != shown) {
        let values =
            stored_pairs(font, Direction::Ltr, &shown).into_iter().map(|p| (row_id(&p.left().command(), &p.right().command()), p.value)).collect();
        app.view.kerning_tools.preview.opened_with = Some((shown, values));
    }
    Ok(())
}

fn groups_editor(app: &mut FontCraftApp, p: &Value) -> Result<(), String> {
    let open = flag(p, "open")?.unwrap_or(true);
    let side = match string(p, "side")? {
        Some(s) => Some(EditorSide::from_name(s).ok_or_else(|| format!("`side` must be left or right, got `{}`", quoted(s)))?),
        None => None,
    };
    let view = match string(p, "view")? {
        Some(v) => Some(GroupsView::from_name(v).ok_or_else(|| format!("`view` must be cells, list or stacked, got `{}`", quoted(v)))?),
        None => None,
    };
    let group = match p.get("group") {
        None => None,
        Some(Value::Null) => Some(None),
        Some(_) => {
            let name = string(p, "group")?.unwrap_or_default();
            let name = name.strip_prefix('@').unwrap_or(name);
            check_group_name(name).map_err(|e| format!("`group`: {e}"))?;
            Some(Some(name.to_string()))
        }
    };
    let max_difference = match p.get("maxDifference") {
        None => None,
        Some(v) => Some(
            v.as_f64()
                .filter(|n| n.is_finite() && (0.0..=MAX_DIFFERENCE).contains(n))
                .ok_or_else(|| format!("`maxDifference` must be a number from 0 to {MAX_DIFFERENCE}"))?,
        ),
    };
    let apply = flag(p, "apply")?.unwrap_or(false);
    let propose = match p.get("propose") {
        None => None,
        Some(Value::Null) => Some(None),
        Some(Value::Object(m)) => {
            if m.contains_key("apply") {
                return Err("`propose` takes the parameters of kerning.buildGroups without `apply`".into());
            }
            let mut m = m.clone();
            m.insert("apply".into(), json!(false));
            Some(Some(m))
        }
        Some(_) => return Err("`propose` must be an object (the parameters of kerning.buildGroups) or null".into()),
    };
    if apply && propose.is_some() {
        return Err("give `propose` or `apply`, not both".into());
    }
    let document = app.session.active().map(|d| (d.id, d.revision)).ok_or("no font is open")?;
    if apply {
        let proposal = app.view.kerning_tools.groups.proposal.clone().ok_or("there is no proposal to apply")?;
        if proposal.document != document {
            return Err("the font changed since the proposal was made; propose again".into());
        }
        let mut params = proposal.params;
        params.insert("apply".into(), json!(true));
        app.session.execute("kerning.buildGroups", &Value::Object(params)).map_err(|e| e.to_string())?;
        app.view.kerning_tools.groups.proposal = None;
    }
    if let Some(Some(params)) = &propose {
        let reply = app.session.execute("kerning.buildGroups", &Value::Object(params.clone())).map_err(|e| e.to_string())?;
        app.view.kerning_tools.groups.proposal = Some(Proposal { params: params.clone(), document, reply });
    }
    crate::shell::dock::show_window(&mut app.view.workspace, PanelId::KerningGroupsEditor, open, app.view.canvas.rect);
    let st = &mut app.view.kerning_tools.groups;
    if propose == Some(None) {
        st.proposal = None;
    }
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
    if let Some(d) = max_difference {
        st.max_difference = d;
    }
    Ok(())
}

/// Most rows `ui.inspect` lists for the preview; `count` gives the full number.
const MAX_INSPECTED: usize = 1_000;

/// `ui.inspect` keys: `kerningPairsPreview {open, master, glyph, sort, size, count, rows: [{left, right, value, changed,
/// was}], selected, scrollTo, combinations: [{first, second, value, step, pair}]}` and `kerningGroupsEditor {open, side,
/// groups: [{name, count}], group, members, view, master, stacked: {aligned, glyphs: [{name, offset}]}, proposal,
/// maxDifference}`.
pub fn inspect(app: &FontCraftApp, state: &mut Map<String, Value>) {
    let shows = |panel: PanelId| app.view.workspace.layout().panel_state(&panel) != craft::behavior::dock::PanelState::Hidden;
    let pv = &app.models.kerning_pairs_preview;
    let pair = |(l, r): &(String, String)| json!({"left": l, "right": r});
    let rows: Vec<Value> = pv
        .rows
        .iter()
        .take(MAX_INSPECTED)
        .map(|r| json!({"left": r.left, "right": r.right, "value": r.value, "changed": r.changed, "was": r.was}))
        .collect();
    let combinations: Vec<Value> = pv
        .combinations
        .iter()
        .map(|c| json!({"first": c.first, "second": c.second, "value": c.value, "step": c.step, "pair": c.pair.as_ref().map(pair)}))
        .collect();
    state.insert(
        "kerningPairsPreview".into(),
        json!({
            "open": shows(PanelId::KerningPairsPreview),
            "master": pv.master,
            "glyph": pv.glyph,
            "sort": pv.sort.name(),
            "size": pv.size,
            "count": pv.rows.len(),
            "rows": rows,
            "selected": pv.selected_pair.as_ref().map(pair),
            "scrollTo": pv.reveal,
            "combinations": combinations,
        }),
    );
    let ge = &app.models.kerning_groups_editor;
    let groups: Vec<Value> = ge.groups.iter().map(|g| json!({"name": g.name, "count": g.count})).collect();
    let stacked: Vec<Value> = ge.stacked.iter().map(|s| json!({"name": s.name, "offset": s.offset})).collect();
    let proposal = ge.proposal.as_ref().map(|groups| {
        let listed: Vec<Value> = groups.iter().map(|g| json!({"side": g.side, "name": g.name, "glyphs": g.glyphs})).collect();
        json!({"groups": listed, "skipped": ge.skipped})
    });
    state.insert(
        "kerningGroupsEditor".into(),
        json!({
            "open": shows(PanelId::KerningGroupsEditor),
            "side": ge.side.name(),
            "groups": groups,
            "group": ge.group,
            "members": ge.members,
            "view": ge.view.name(),
            "master": ge.master,
            "stacked": {"aligned": if ge.side == EditorSide::Left { "origin" } else { "advance" }, "glyphs": stacked},
            "proposal": proposal,
            "maxDifference": ge.max_difference,
        }),
    );
}
