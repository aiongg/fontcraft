//! The view models of the Kerning Pairs Preview and the Kerning Groups Editor: the pairs of one master in the
//! preview's order with the glyphs each is drawn by, the combinations of a selected group pair, and the groups,
//! members and glyph outlines the editor shows.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use craft::behavior::dock::PanelState;
use craft::ui::contract::{PlainData, Tracked};
use fontcraft_engine::model::{Font, metric};
use fontcraft_spacing::kerning::pairs::stored_pairs;
use fontcraft_spacing::kerning::{Direction, Position, Side, facing, group_list, members};
use serde_json::Value;

use super::ProjectCx;
use super::font_view::{CellArt, flatten};
use crate::shell::dock::PanelId;
use crate::shell::kerning_tools::{DEFAULT_SIZE, EditorSide, GroupsView, PreviewSort, preview_master, row_id};

/// Most pairs the preview lists.
pub const MAX_ROWS: usize = 200_000;
/// Most glyph combinations one group pair lists.
pub const MAX_COMBINATIONS: usize = 2_500;
/// Most members the stacked view draws over each other.
pub const MAX_STACKED: usize = 500;
/// Most glyph outlines one panel holds.
const MAX_ART: usize = 20_000;
/// Most proposed groups the editor lists.
const MAX_PROPOSED: usize = 10_000;

/// One listed pair: its sides as commands spell them, its value and the glyphs drawn for it.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct PreviewRow {
    pub left: String,
    pub right: String,
    pub value: f64,
    /// The glyphs drawn: the side's glyph, or its group's first member (empty when the group has no member on the
    /// side facing the pair).
    pub first: String,
    pub second: String,
    /// The value differs from the one the preview found when it opened, or the pair is new since.
    pub changed: bool,
    /// The value when the preview opened; `None` for a pair new since, or one that did not change.
    pub was: Option<f64>,
}

/// One glyph combination a pair with a group side covers, and the stored pair that applies to it.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct Combination {
    pub first: String,
    pub second: String,
    pub value: f64,
    /// The pair that applies, as commands spell it; `None` when none does.
    pub pair: Option<(String, String)>,
    /// The lookup step that found it.
    pub step: Option<String>,
}

/// What the preview shows.
#[derive(PlainData, Clone, Debug, PartialEq)]
pub struct PairsPreviewModel {
    pub has_font: bool,
    pub master: Option<String>,
    pub glyph: Option<String>,
    pub sort: PreviewSort,
    /// Pixels per em.
    pub size: u32,
    pub units_per_em: f32,
    pub rows: Vec<PreviewRow>,
    /// The position of the selected pair in `rows`.
    pub selected: Option<usize>,
    /// The selected pair, listed or not.
    pub selected_pair: Option<(String, String)>,
    /// The row a jump scrolls to.
    pub reveal: Option<usize>,
    /// The combinations of a selected pair with a group side.
    pub combinations: Vec<Combination>,
    /// Why the combinations are not listed.
    pub combinations_note: Option<String>,
    /// The outlines of the glyphs drawn, by name.
    pub art: BTreeMap<String, Arc<CellArt>>,
    /// The document revision, master, glyph and sort the rows and outlines came from.
    pub asked: Option<(u64, u64, String, Option<String>, PreviewSort)>,
}

impl Default for PairsPreviewModel {
    fn default() -> Self {
        PairsPreviewModel {
            has_font: false,
            master: None,
            glyph: None,
            sort: PreviewSort::default(),
            size: DEFAULT_SIZE,
            units_per_em: 1000.0,
            rows: Vec::new(),
            selected: None,
            selected_pair: None,
            reveal: None,
            combinations: Vec::new(),
            combinations_note: None,
            art: BTreeMap::new(),
            asked: None,
        }
    }
}

impl PairsPreviewModel {
    /// The advance width of `glyph`, in font units.
    pub fn width(&self, glyph: &str) -> f32 {
        self.art.get(glyph).map_or(0.0, |a| a.width)
    }
}

/// A group of the listed side and its member count.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct GroupEntry {
    pub name: String,
    pub count: usize,
}

/// A group `kerning.buildGroups` proposes.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct ProposedGroup {
    /// "left" or "right".
    pub side: String,
    pub name: String,
    pub glyphs: Vec<String>,
}

/// A member in the stacked view and where it is drawn, in font units from the alignment line.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct StackedGlyph {
    pub name: String,
    pub offset: f64,
}

/// What the editor shows.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct GroupsEditorModel {
    pub has_font: bool,
    pub side: EditorSide,
    pub view: GroupsView,
    pub group: Option<String>,
    /// The current group has no member yet.
    pub is_new: bool,
    pub groups: Vec<GroupEntry>,
    pub members: Vec<String>,
    pub stacked: Vec<StackedGlyph>,
    /// Every glyph of the font, in glyph order.
    pub glyphs: Vec<String>,
    pub master: String,
    /// The master's ascender and descender, in font units: the extent a cell shows.
    pub ascender: f32,
    pub descender: f32,
    pub art: BTreeMap<String, Arc<CellArt>>,
    pub max_difference: f64,
    /// The proposal under review.
    pub proposal: Option<Vec<ProposedGroup>>,
    /// Glyphs the proposal left out.
    pub skipped: usize,
    /// The document revision and master the outlines came from.
    pub asked: Option<(u64, u64, String)>,
}

/// The glyph a side is drawn by at `position`: itself, or its group's first member.
fn drawn(font: &Font, side: &Side, position: Position) -> String {
    match side {
        Side::Glyph(name) => name.clone(),
        Side::Group(group) => members(font, facing(Direction::Ltr, position), group).first().map_or(String::new(), |s| s.to_string()),
    }
}

/// Whether a side names `glyph`, or the group `glyph` has on the side facing the pair.
fn names_glyph(font: &Font, side: &Side, position: Position, glyph: &str) -> bool {
    match side {
        Side::Glyph(name) => name == glyph,
        Side::Group(group) => font.glyph(glyph).is_some_and(|g| facing(Direction::Ltr, position).of(&g.kern_groups) == Some(group.as_str())),
    }
}

/// The preview's rows for these options, in their order, without the changed marks.
pub fn preview_rows(font: &Font, master: &str, glyph: Option<&str>, sort: PreviewSort) -> Vec<PreviewRow> {
    let order: HashMap<&str, usize> = font.glyphs.iter().enumerate().map(|(i, g)| (g.name.as_str(), i)).collect();
    let mut out: Vec<(usize, PreviewRow)> = stored_pairs(font, Direction::Ltr, master)
        .into_iter()
        .filter(|p| glyph.is_none_or(|g| names_glyph(font, &p.left(), Position::First, g) || names_glyph(font, &p.right(), Position::Second, g)))
        .take(MAX_ROWS)
        .map(|p| {
            let (left, right) = (p.left(), p.right());
            let row = PreviewRow {
                first: drawn(font, &left, Position::First),
                second: drawn(font, &right, Position::Second),
                left: left.command(),
                right: right.command(),
                value: p.value,
                changed: false,
                was: None,
            };
            (p.index, row)
        })
        .collect();
    let at = |name: &str| order.get(name).copied().unwrap_or(usize::MAX);
    match sort {
        PreviewSort::First => out.sort_by_key(|(i, r)| (at(&r.first), at(&r.second), *i)),
        PreviewSort::Second => out.sort_by_key(|(i, r)| (at(&r.second), at(&r.first), *i)),
        PreviewSort::Value => out.sort_by(|(i, a), (j, b)| b.value.abs().total_cmp(&a.value.abs()).then(i.cmp(j))),
    }
    out.into_iter().map(|(_, r)| r).collect()
}

/// The first row whose sorted-by glyph is encoded by `c`, else whose name starts with it.
pub fn jump_target(font: &Font, rows: &[PreviewRow], sort: PreviewSort, c: char) -> Option<usize> {
    fn key(r: &PreviewRow, sort: PreviewSort) -> &str {
        if sort == PreviewSort::Second { r.second.as_str() } else { r.first.as_str() }
    }
    let encoded = |name: &str| font.glyph(name).is_some_and(|g| g.unicodes.contains(&u32::from(c)));
    rows.iter().position(|r| encoded(key(r, sort))).or_else(|| rows.iter().position(|r| key(r, sort).starts_with(c)))
}

/// The glyphs a side covers at `position`: itself, or its group's members.
fn side_glyphs(font: &Font, text: &str, position: Position) -> Vec<String> {
    match Side::parse(text) {
        Ok(Side::Glyph(name)) => vec![name],
        Ok(Side::Group(group)) => members(font, facing(Direction::Ltr, position), &group).into_iter().map(str::to_string).collect(),
        Err(_) => Vec::new(),
    }
}

/// The outline of `glyph` in `master`, flattened.
fn art_of(font: &Font, glyph: &str, master: &str) -> Option<Arc<CellArt>> {
    let g = font.glyph(glyph)?;
    let width = g.layer(master).map_or(0.0, |l| l.width) as f32;
    let contours = font.outline_of(g, master).map(|p| flatten(&p)).unwrap_or_default();
    Some(Arc::new(CellArt { width: if width.is_finite() { width } else { 0.0 }, contours }))
}

/// Adds the outlines of `names` that `art` lacks, up to [`MAX_ART`].
fn add_art<'a>(art: &mut BTreeMap<String, Arc<CellArt>>, font: &Font, master: &str, names: impl IntoIterator<Item = &'a str>) {
    for name in names {
        if art.len() >= MAX_ART {
            return;
        }
        if name.is_empty() || art.contains_key(name) {
            continue;
        }
        if let Some(a) = art_of(font, name, master) {
            art.insert(name.to_string(), a);
        }
    }
}

fn shows(cx: &ProjectCx<'_>, panel: PanelId) -> bool {
    cx.view.workspace.layout.panel_state(&panel) != PanelState::Hidden
}

/// Lists the pairs again while the preview shows and the document or its options moved.
pub fn project_preview(model: &mut Tracked<PairsPreviewModel>, cx: &mut ProjectCx<'_>) {
    if !shows(cx, PanelId::KerningPairsPreview) {
        return;
    }
    let state = &cx.view.kerning_tools.preview;
    let Some(doc) = cx.session.active().map(|d| (d.id, d.revision)) else {
        super::set(model, PairsPreviewModel { sort: state.sort, size: state.size, ..Default::default() });
        return;
    };
    let group_pair = state.selected.clone().filter(|(l, r)| l.starts_with('@') || r.starts_with('@'));
    let index = match group_pair {
        Some(_) => cx.session.kern_index().ok(),
        None => None,
    };
    let Ok(font) = cx.session.font() else { return };
    let state = &cx.view.kerning_tools.preview;
    let master = preview_master(font, cx.view).unwrap_or_default();
    let mut next = (**model).clone();
    let asked = Some((doc.0, doc.1, master.clone(), state.glyph.clone(), state.sort));
    if next.asked != asked {
        next.rows = preview_rows(font, &master, state.glyph.as_deref(), state.sort);
        if next.asked.as_ref().is_none_or(|(id, revision, m, _, _)| (*id, *revision, m.as_str()) != (doc.0, doc.1, master.as_str())) {
            next.art.clear();
        }
        let drawn: Vec<&str> = next.rows.iter().flat_map(|r| [r.first.as_str(), r.second.as_str()]).collect();
        let mut art = std::mem::take(&mut next.art);
        add_art(&mut art, font, &master, drawn);
        next.art = art;
        next.asked = asked;
    }
    let before = state.opened_with.as_ref().filter(|(m, _)| *m == master).map(|(_, v)| v);
    for row in &mut next.rows {
        let was = before.map(|b| b.get(&row_id(&row.left, &row.right)).copied());
        (row.changed, row.was) = match was {
            Some(Some(v)) if v == row.value => (false, None),
            Some(Some(v)) => (true, Some(v)),
            Some(None) => (true, None),
            None => (false, None),
        };
    }
    next.has_font = true;
    next.master = Some(master.clone());
    next.glyph = state.glyph.clone();
    next.sort = state.sort;
    next.size = state.size;
    next.units_per_em = f32::from(font.units_per_em.max(1));
    next.selected_pair = state.selected.clone();
    next.selected = state.selected.as_ref().and_then(|(l, r)| next.rows.iter().position(|row| row.left == *l && row.right == *r));
    next.reveal = state.jump.and_then(|c| jump_target(font, &next.rows, state.sort, c));
    next.combinations.clear();
    next.combinations_note = None;
    if let Some((l, r)) = &group_pair {
        let (lefts, rights) = (side_glyphs(font, l, Position::First), side_glyphs(font, r, Position::Second));
        let count = lefts.len().saturating_mul(rights.len());
        if count > MAX_COMBINATIONS {
            next.combinations_note = Some(format!("{count} > {MAX_COMBINATIONS}"));
        } else if let Some(index) = &index {
            for first in &lefts {
                for second in &rights {
                    let applied = index.lookup(Direction::Ltr, &master, first, second).ok().flatten();
                    next.combinations.push(Combination {
                        first: first.clone(),
                        second: second.clone(),
                        value: applied.map_or(0.0, |a| a.value),
                        pair: applied.map(|a| (Side::from_stored(a.first).command(), Side::from_stored(a.second).command())),
                        step: applied.map(|a| a.step.name().to_string()),
                    });
                }
            }
            let mut art = std::mem::take(&mut next.art);
            add_art(&mut art, font, &master, lefts.iter().chain(&rights).map(String::as_str));
            next.art = art;
        }
    }
    super::set(model, next);
}

/// Where each member is drawn in the stacked view, in font units from the shared alignment line: 0 for a left group
/// (aligned on the origin), minus the advance width for a right group.
pub fn stacked_offsets(font: &Font, master: &str, side: EditorSide, glyphs: &[String]) -> Vec<StackedGlyph> {
    glyphs
        .iter()
        .take(MAX_STACKED)
        .map(|name| {
            let width = font.glyph(name).and_then(|g| g.layer(master)).map_or(0.0, |l| l.width);
            let offset = match side {
                EditorSide::Left => 0.0,
                EditorSide::Right => -width,
            };
            StackedGlyph { name: name.clone(), offset }
        })
        .collect()
}

/// The master's ascender and descender, in font units.
fn frame(font: &Font, master: &str) -> (f32, f32) {
    let upm = f64::from(font.units_per_em);
    let (a, d) = match font.master(master) {
        Ok(m) => (font.metric_pos(m, metric::ASCENDER, upm * 0.8), font.metric_pos(m, metric::DESCENDER, -upm * 0.2)),
        Err(_) => (upm * 0.8, -upm * 0.2),
    };
    (a as f32, d as f32)
}

/// The groups `kerning.buildGroups` proposed and how many glyphs it skipped.
fn proposed(reply: &Value) -> (Vec<ProposedGroup>, usize) {
    let mut out = Vec::new();
    for side in ["left", "right"] {
        for g in reply.get("groups").and_then(|g| g.get(side)).and_then(Value::as_array).into_iter().flatten().take(MAX_PROPOSED) {
            out.push(ProposedGroup {
                side: side.to_string(),
                name: g.get("name").and_then(Value::as_str).unwrap_or_default().to_string(),
                glyphs: g.get("glyphs").and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect(),
            });
        }
    }
    (out, reply.get("skipped").and_then(Value::as_array).map_or(0, Vec::len))
}

/// Reads the groups, members and outlines again while the editor shows.
pub fn project_groups(model: &mut Tracked<GroupsEditorModel>, cx: &mut ProjectCx<'_>) {
    if !shows(cx, PanelId::KerningGroupsEditor) {
        return;
    }
    let state = &cx.view.kerning_tools.groups;
    let (Some(doc), Ok(font)) = (cx.session.active().map(|d| (d.id, d.revision)), cx.session.font()) else {
        super::set(model, GroupsEditorModel { side: state.side, view: state.view, max_difference: state.max_difference, ..Default::default() });
        return;
    };
    let master = crate::shell::masters::current_master(font, &cx.view.layers).map(|m| m.id.clone()).unwrap_or_default();
    let mut next = (**model).clone();
    let side = state.side.group_side();
    let groups = group_list(font, side);
    next.groups = groups.iter().map(|(name, glyphs)| GroupEntry { name: name.to_string(), count: glyphs.len() }).collect();
    next.members = state.group.as_deref().map(|g| members(font, side, g).into_iter().map(str::to_string).collect()).unwrap_or_default();
    next.is_new = state.group.as_deref().is_some_and(|g| !groups.iter().any(|(n, _)| *n == g));
    next.stacked = stacked_offsets(font, &master, state.side, &next.members);
    let asked = Some((doc.0, doc.1, master.clone()));
    if next.asked != asked {
        next.glyphs = font.glyphs.iter().map(|g| g.name.clone()).collect();
        let mut art = BTreeMap::new();
        add_art(&mut art, font, &master, next.glyphs.iter().map(String::as_str));
        next.art = art;
        (next.ascender, next.descender) = frame(font, &master);
        next.asked = asked;
    }
    next.has_font = true;
    next.side = state.side;
    next.view = state.view;
    next.group = state.group.clone();
    next.master = master;
    next.max_difference = state.max_difference;
    (next.proposal, next.skipped) = match &state.proposal {
        Some(p) => {
            let (groups, skipped) = proposed(&p.reply);
            (Some(groups), skipped)
        }
        None => (None, 0),
    };
    super::set(model, next);
}
