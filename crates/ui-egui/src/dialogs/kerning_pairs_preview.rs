//! The Kerning Pairs Preview window (`window.kerningPairsPreview`): every stored pair of one
//! master in left-to-right text, drawn kerned with its value underneath, for reviewing a whole
//! font's kerning and correcting outliers in place. The shell command lives in
//! `shell::kerning_review`.
//!
//! | Input | Does |
//! |---|---|
//! | Sort and Size menus | `window.kerningPairsPreview {sort}`, `{size}` |
//! | click a pair | selects it (`{select}`); a pair with a group side lists every glyph combination it covers below (`kerning.previewGroupCombinations`) |
//! | click a combination | selects the stored pair that applies to it |
//! | drag the second glyph sideways | `kerning.setPair` with the value changed by the distance dragged, in font units |
//! | Left, Right (Shift: by 10) on the selected pair | `kerning.setPair` by 1 unit |
//! | typing a character | scrolls to the first pair whose sorted-by glyph has it (`{jump}`) |
//! | double-click a pair | `kerning.showPairInEditView`, which shows it large in Edit View |
//!
//! A group side is drawn by its first member in glyph order. Values that differ from the ones the
//! window found when it opened (or that are new since) are drawn in the accent colour.

use std::collections::HashMap;

use egui::{Align2, FontId, Key, Pos2, Rect, Sense, Shape, Stroke, pos2, vec2};
use fontcraft_geom::Affine;
use fontcraft_model::Font;
use fontcraft_spacing::kerning::pairs::stored_pairs;
use fontcraft_spacing::kerning::{Direction, Position, Side, facing, members};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::app::FontcraftApp;
use crate::panels::Pending;
use crate::shell::Args;

/// The render sizes the window offers, in pixels per em.
pub const SIZES: [u32; 6] = [24, 36, 48, 72, 96, 144];
pub const DEFAULT_SIZE: u32 = 48;
/// Most rows a reply lists; `count` gives the full number.
pub const MAX_REPLY_ROWS: usize = 1_000;
/// Most glyph combinations one group pair lists.
pub const MAX_COMBINATIONS: usize = 2_500;
/// Characters of untrusted text quoted in a message.
const MAX_QUOTED: usize = 64;

/// How the window orders the pairs.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
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
    pub fn name(self) -> &'static str {
        match self {
            PreviewSort::First => "first",
            PreviewSort::Second => "second",
            PreviewSort::Value => "value",
        }
    }

    pub fn from_name(name: &str) -> Option<PreviewSort> {
        [PreviewSort::First, PreviewSort::Second, PreviewSort::Value].into_iter().find(|s| s.name() == name)
    }

    fn label(self) -> &'static str {
        match self {
            PreviewSort::First => "First Glyph",
            PreviewSort::Second => "Second Glyph",
            PreviewSort::Value => "Size of Value",
        }
    }
}

/// One listed pair: its sides as commands spell them, its value and the glyphs drawn for it.
#[derive(Clone, Debug, PartialEq)]
pub struct PreviewRow {
    pub left: String,
    pub right: String,
    pub value: f64,
    /// The glyphs drawn: the side's glyph, or its group's first member (empty when the group has
    /// no member on the side facing the pair).
    pub first: String,
    pub second: String,
}

impl PreviewRow {
    fn id(&self) -> String {
        row_id(&self.left, &self.right)
    }
}

/// A row's name: its sides joined by a tab, which no side may hold.
pub fn row_id(left: &str, right: &str) -> String {
    format!("{left}\t{right}")
}

/// One glyph combination a pair with a group side covers, and the stored pair that applies to it.
#[derive(Clone, Debug, PartialEq)]
pub struct Combination {
    pub first: String,
    pub second: String,
    pub value: f64,
    /// The pair that applies, as commands spell it; `None` when none does.
    pub pair: Option<(String, String)>,
    pub step: Option<&'static str>,
}

/// What the cached rows were computed for: the document, its revision and the options.
type CacheKey = (u64, u64, String, Option<String>, PreviewSort);

/// The window's state, kept in `KerningReviewState`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PreviewState {
    pub open: bool,
    /// `None`: the showing tab's master, else the first.
    pub master: Option<String>,
    /// Only the pairs naming this glyph, or a group it belongs to on the facing side.
    pub glyph: Option<String>,
    pub sort: PreviewSort,
    pub size: u32,
    /// The selected pair, as commands spell its sides.
    pub selected: Option<(String, String)>,
    /// The values when the window opened, by row id, and the master they belong to.
    #[serde(skip)]
    pub opened_with: Option<(String, HashMap<String, f64>)>,
    #[serde(skip)]
    pub scroll_to: Option<usize>,
    /// Where the second glyph of each pair in view was drawn, by row id.
    #[serde(skip)]
    pub laid_out: Vec<(String, Rect)>,
    /// The pair being dragged and the press position.
    #[serde(skip)]
    pub drag: Option<(String, Pos2)>,
    #[serde(skip)]
    cache: Option<(CacheKey, Vec<PreviewRow>)>,
}

impl Default for PreviewState {
    fn default() -> Self {
        Self {
            open: false,
            master: None,
            glyph: None,
            sort: PreviewSort::default(),
            size: DEFAULT_SIZE,
            selected: None,
            opened_with: None,
            scroll_to: None,
            laid_out: Vec::new(),
            drag: None,
            cache: None,
        }
    }
}

fn quoted(s: &str) -> String {
    s.chars().take(MAX_QUOTED).collect()
}

/// The master the window shows: its own choice when the font has it, else the showing tab's,
/// else the first.
pub fn shown_master(app: &FontcraftApp) -> Option<String> {
    let font = app.session.font().ok()?;
    let has = |m: &String| font.masters.iter().any(|x| x.id == *m);
    app.state
        .panels
        .kerning_review
        .preview
        .master
        .clone()
        .filter(has)
        .or_else(|| app.state.tab().and_then(|t| t.master.clone()).filter(has))
        .or_else(|| font.masters.first().map(|m| m.id.clone()))
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

/// The rows for these options, in their order.
pub fn rows(font: &Font, master: &str, glyph: Option<&str>, sort: PreviewSort) -> Vec<PreviewRow> {
    let order: HashMap<&str, usize> = font.glyphs.iter().enumerate().map(|(i, g)| (g.name.as_str(), i)).collect();
    let mut out: Vec<(usize, PreviewRow)> = stored_pairs(font, Direction::Ltr, master)
        .into_iter()
        .filter_map(|p| {
            let (left, right) = (p.left(), p.right());
            if let Some(g) = glyph
                && !names_glyph(font, &left, Position::First, g)
                && !names_glyph(font, &right, Position::Second, g)
            {
                return None;
            }
            let row = PreviewRow {
                first: drawn(font, &left, Position::First),
                second: drawn(font, &right, Position::Second),
                left: left.command(),
                right: right.command(),
                value: p.value,
            };
            Some((p.index, row))
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

/// The rows the window shows, computed again only when the font or the options change.
pub fn current_rows(app: &mut FontcraftApp) -> Vec<PreviewRow> {
    let Some(master) = shown_master(app) else { return Vec::new() };
    let Ok(doc) = app.session.doc() else { return Vec::new() };
    let pv = &app.state.panels.kerning_review.preview;
    let key: CacheKey = (doc.id, doc.revision, master.clone(), pv.glyph.clone(), pv.sort);
    if let Some((k, rows)) = &pv.cache
        && *k == key
    {
        return rows.clone();
    }
    let computed = rows(&doc.font, &master, pv.glyph.as_deref(), pv.sort);
    app.state.panels.kerning_review.preview.cache = Some((key, computed.clone()));
    computed
}

/// Every glyph combination the pair covers, with the pair that applies to each.
pub fn combinations(app: &mut FontcraftApp, left: &str, right: &str) -> Result<Vec<Combination>, String> {
    let master = shown_master(app).ok_or_else(|| "the font has no master".to_string())?;
    let index = app.session.kern_index().map_err(|e| e.to_string())?;
    let font = app.session.font().map_err(|e| e.to_string())?;
    let side_glyphs = |text: &str, position: Position| -> Result<Vec<String>, String> {
        match Side::parse(text).map_err(|e| e.to_string())? {
            Side::Glyph(name) => Ok(vec![name]),
            Side::Group(group) => Ok(members(font, facing(Direction::Ltr, position), &group).into_iter().map(str::to_string).collect()),
        }
    };
    let lefts = side_glyphs(left, Position::First)?;
    let rights = side_glyphs(right, Position::Second)?;
    if lefts.len().saturating_mul(rights.len()) > MAX_COMBINATIONS {
        return Err(format!("the pair covers {} combinations; at most {MAX_COMBINATIONS} are listed", lefts.len().saturating_mul(rights.len())));
    }
    let mut out = Vec::new();
    for l in &lefts {
        for r in &rights {
            let applied = index.lookup(Direction::Ltr, &master, l, r).map_err(|e| e.to_string())?;
            out.push(Combination {
                first: l.clone(),
                second: r.clone(),
                value: applied.map_or(0.0, |a| a.value),
                pair: applied.map(|a| (Side::from_stored(a.first).command(), Side::from_stored(a.second).command())),
                step: applied.map(|a| a.step.name()),
            });
        }
    }
    Ok(out)
}

/// Whether a side spelled as commands spell it is a group.
fn is_group(text: &str) -> bool {
    text.starts_with('@')
}

/// The values of the master's pairs now, by row id.
fn snapshot(font: &Font, master: &str) -> HashMap<String, f64> {
    stored_pairs(font, Direction::Ltr, master).into_iter().map(|p| (row_id(&p.left().command(), &p.right().command()), p.value)).collect()
}

/// The value a row had when the window opened, when it differs from its value now. A pair that
/// is new since gives `Some(None)`.
fn change(pv: &PreviewState, row: &PreviewRow) -> Option<Option<f64>> {
    let (_, before) = pv.opened_with.as_ref()?;
    match before.get(&row.id()) {
        Some(was) if *was == row.value => None,
        Some(was) => Some(Some(*was)),
        None => Some(None),
    }
}

/// The first row whose sorted-by glyph is encoded by `c`, else whose name starts with it.
fn jump_target(font: &Font, rows: &[PreviewRow], sort: PreviewSort, c: char) -> Option<usize> {
    let key = |r: &PreviewRow| if sort == PreviewSort::Second { r.second.clone() } else { r.first.clone() };
    let encoded = |name: &str| font.glyph(name).is_some_and(|g| g.unicodes.contains(&u32::from(c)));
    rows.iter().position(|r| encoded(&key(r))).or_else(|| rows.iter().position(|r| key(r).starts_with(c)))
}

fn side_pair(args: &Args, key: &str) -> Result<Option<Option<(String, String)>>, String> {
    let Some(v) = args.get(key) else { return Ok(None) };
    if v.is_null() {
        return Ok(Some(None));
    }
    let side = |k: &str| -> Result<String, String> {
        let text = v.get(k).and_then(Value::as_str).ok_or_else(|| args.err(format!("`{key}.{k}` (a glyph name or @group) is required")))?;
        Side::parse(text).map(|s| s.command()).map_err(|e| args.err(format!("`{key}.{k}`: {e}")))
    };
    Ok(Some(Some((side("left")?, side("right")?))))
}

/// `window.kerningPairsPreview`.
pub fn window(app: &mut FontcraftApp, args: &Args) -> Result<Value, String> {
    let open = args.flag("open")?.unwrap_or(true);
    let master = match args.get("master") {
        Some(Value::Null) => Some(None),
        Some(_) => {
            let id = args.string("master")?.unwrap_or_default();
            let font = app.session.font().map_err(|e| e.to_string())?;
            if !font.masters.iter().any(|m| m.id == id) {
                return Err(args.err(format!("the font has no master `{}`", quoted(id))));
            }
            Some(Some(id.to_string()))
        }
        None => None,
    };
    let glyph = match args.get("glyph") {
        Some(Value::Null) => Some(None),
        Some(_) => {
            let name = args.string("glyph")?.unwrap_or_default();
            if app.session.font().map_err(|e| e.to_string())?.glyph(name).is_none() {
                return Err(args.err(format!("the font has no glyph `{}`", quoted(name))));
            }
            Some(Some(name.to_string()))
        }
        None => None,
    };
    let sort = match args.string("sort")? {
        Some(s) => Some(PreviewSort::from_name(s).ok_or_else(|| args.err(format!("`sort` must be first, second or value, got `{}`", quoted(s))))?),
        None => None,
    };
    let size = match args.index("size")? {
        Some(s) => {
            let s = u32::try_from(s).ok().filter(|s| SIZES.contains(s));
            Some(s.ok_or_else(|| args.err(format!("`size` must be one of {SIZES:?}")))?)
        }
        None => None,
    };
    let select = side_pair(args, "select")?;
    let jump = match args.string("jump")? {
        Some(text) => {
            let mut chars = text.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => Some(c),
                _ => return Err(args.err("`jump` must be one character")),
            }
        }
        None => None,
    };
    // Every parameter is read and checked before anything changes.
    let pv = &mut app.state.panels.kerning_review.preview;
    if !open {
        pv.open = false;
        pv.opened_with = None;
        pv.drag = None;
        return Ok(reply(app));
    }
    pv.open = true;
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
    let master = shown_master(app).unwrap_or_default();
    let fresh = app.state.panels.kerning_review.preview.opened_with.as_ref().is_none_or(|(m, _)| *m != master);
    if fresh {
        let font = app.session.font().map_err(|e| e.to_string())?;
        let taken = snapshot(font, &master);
        app.state.panels.kerning_review.preview.opened_with = Some((master, taken));
    }
    if let Some(c) = jump {
        let rows = current_rows(app);
        let font = app.session.font().map_err(|e| e.to_string())?;
        let at = jump_target(font, &rows, app.state.panels.kerning_review.preview.sort, c);
        app.state.panels.kerning_review.preview.scroll_to = at;
    }
    let mut out = reply(app);
    if let Some(m) = out.as_object_mut() {
        if jump.is_some() {
            m.insert("scrollTo".into(), json!(app.state.panels.kerning_review.preview.scroll_to));
        }
        let selected = app.state.panels.kerning_review.preview.selected.clone();
        if let Some((l, r)) = selected.filter(|(l, r)| is_group(l) || is_group(r)) {
            let found = combinations(app, &l, &r).map_err(|e| args.err(e))?;
            let list: Vec<Value> = found
                .iter()
                .map(|c| {
                    json!({"first": c.first, "second": c.second, "value": c.value, "step": c.step,
                           "pair": c.pair.as_ref().map(|(l, r)| json!({"left": l, "right": r}))})
                })
                .collect();
            m.insert("combinations".into(), Value::Array(list));
        }
    }
    Ok(out)
}

/// The window as `window.kerningPairsPreview` returns it.
pub fn reply(app: &mut FontcraftApp) -> Value {
    let rows = if app.state.panels.kerning_review.preview.open { current_rows(app) } else { Vec::new() };
    let master = shown_master(app);
    let pv = &app.state.panels.kerning_review.preview;
    let listed: Vec<Value> = rows
        .iter()
        .take(MAX_REPLY_ROWS)
        .map(|r| json!({"left": r.left, "right": r.right, "value": r.value, "changed": change(pv, r).is_some()}))
        .collect();
    let changed: Vec<Value> = rows
        .iter()
        .filter_map(|r| change(pv, r).map(|was| json!({"left": r.left, "right": r.right, "value": r.value, "was": was})))
        .take(MAX_REPLY_ROWS)
        .collect();
    json!({
        "open": pv.open,
        "master": master,
        "glyph": pv.glyph,
        "sort": pv.sort.name(),
        "size": pv.size,
        "count": rows.len(),
        "rows": listed,
        "selected": pv.selected.as_ref().map(|(l, r)| json!({"left": l, "right": r})),
        "changed": changed,
    })
}

/// Draws `glyph` of `master` with its origin at `origin` (screen pixels, on the baseline),
/// `scale` pixels per unit, as outlines. Returns its advance in pixels.
pub fn paint_outline(painter: &egui::Painter, font: &Font, glyph: &str, master: &str, origin: Pos2, scale: f32, stroke: Stroke) -> f32 {
    let Some(g) = font.glyph(glyph) else { return 0.0 };
    let width = g.layer(master).map_or(0.0, |l| l.width);
    if let Ok(path) = font.outline(glyph, master) {
        let s = f64::from(scale);
        let to_screen = Affine::translate((f64::from(origin.x), f64::from(origin.y))) * Affine::scale_non_uniform(s, -s);
        for contour in crate::canvas::flatten(&(to_screen * path)) {
            painter.add(Shape::closed_line(contour, stroke));
        }
    }
    if width.is_finite() { (width as f32) * scale } else { 0.0 }
}

/// The scroll area's id, which holds the keyboard for the arrow keys and typed characters.
pub fn list_id() -> egui::Id {
    egui::Id::new("window.kerningPairsPreview.list")
}

/// Draws the window while it is open.
pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    if !app.state.panels.kerning_review.preview.open {
        return;
    }
    let rows = current_rows(app);
    let Some(master) = shown_master(app) else { return };
    let Ok(font) = app.session.doc().map(|d| d.font.clone()) else { return };
    let FontcraftApp { state, tokens, .. } = app;
    let tokens: &crate::theme::Tokens = tokens;
    let pv = &mut state.panels.kerning_review.preview;
    let upm = f32::from(font.units_per_em.max(1));
    let size = pv.size as f32;
    let scale = size / upm;
    let row_height = size * 1.3 + tokens.list_row_height;
    let mut laid_out = Vec::new();
    let mut open = true;
    egui::Window::new("Kerning Pairs Preview")
        .id(egui::Id::new("window.kerningPairsPreview"))
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_size(vec2(tokens.palette_width * 2.0, tokens.list_row_height * 30.0))
        .default_pos(pos2(ui.ctx().content_rect().center().x, tokens.dialog_top))
        .show(ui.ctx(), |ui| {
            ui.horizontal(|ui| {
                ui.menu_button(format!("Sort: {}", pv.sort.label()), |ui| {
                    for s in [PreviewSort::First, PreviewSort::Second, PreviewSort::Value] {
                        if ui.radio(pv.sort == s, s.label()).clicked() {
                            pending.push(("window.kerningPairsPreview", json!({"sort": s.name()})));
                            ui.close();
                        }
                    }
                });
                ui.menu_button(format!("Size: {}", pv.size), |ui| {
                    for s in SIZES {
                        if ui.radio(pv.size == s, s.to_string()).clicked() {
                            pending.push(("window.kerningPairsPreview", json!({"size": s})));
                            ui.close();
                        }
                    }
                });
                let scope = pv.glyph.as_deref().map_or("every pair".to_string(), |g| format!("pairs of {g}"));
                ui.label(egui::RichText::new(format!("{} ({scope})", rows.len())).color(tokens.text_dim));
                if pv.glyph.is_some() && ui.button("Show All").clicked() {
                    pending.push(("window.kerningPairsPreview", json!({"glyph": null})));
                }
            });
            let list = ui.interact(ui.max_rect(), list_id(), Sense::focusable_noninteractive());
            let filter = egui::EventFilter { horizontal_arrows: true, ..Default::default() };
            ui.memory_mut(|m| m.set_focus_lock_filter(list_id(), filter));
            if list.has_focus() {
                keys(ui, pv, &rows, &master, pending);
            }
            let mut area = egui::ScrollArea::vertical().id_salt("rows").auto_shrink([false, true]).max_height(row_height * 8.0);
            if let Some(at) = pv.scroll_to.take() {
                area = area.vertical_scroll_offset(at as f32 * row_height);
            }
            area.show_rows(ui, row_height, rows.len(), |ui, range| {
                for row in rows.get(range).unwrap_or_default() {
                    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), row_height), Sense::click());
                    let id = row.id();
                    let selected = pv.selected.as_ref().is_some_and(|(l, r)| *l == row.left && *r == row.right);
                    let painter = ui.painter_at(rect);
                    if selected {
                        painter.rect_filled(rect, tokens.radius, tokens.row_selected);
                    }
                    let baseline = rect.top() + size;
                    let ink = Stroke::new(1.0, tokens.glyph_fill);
                    let start = rect.left() + tokens.dialog_spacing;
                    let x = start + paint_outline(&painter, &font, &row.first, &master, pos2(start, baseline), scale, ink);
                    let dragged =
                        pv.drag.as_ref().filter(|(d, _)| *d == id).and_then(|(_, from)| ui.input(|i| i.pointer.interact_pos()).map(|p| p.x - from.x));
                    let second_x = x + row.value as f32 * scale + dragged.unwrap_or(0.0);
                    let advance = paint_outline(&painter, &font, &row.second, &master, pos2(second_x, baseline), scale, ink);
                    let target = Rect::from_min_max(pos2(second_x, rect.top()), pos2(second_x + advance.max(tokens.list_row_height), baseline));
                    laid_out.push((id.clone(), target));
                    let colour = if change(pv, row).is_some() { tokens.accent } else { tokens.text_dim };
                    let value = dragged.map_or(row.value, |d| row.value + f64::from((d / scale).round()));
                    painter.text(
                        pos2(start, rect.bottom() - tokens.list_row_height / 2.0),
                        Align2::LEFT_CENTER,
                        format!("{} {}  {value}", row.left, row.right),
                        FontId::proportional(tokens.cell_label_size),
                        colour,
                    );
                    let drag = ui.interact(target, egui::Id::new(("window.kerningPairsPreview.drag", &id)), Sense::click_and_drag());
                    if drag.drag_started()
                        && let Some(from) = ui.input(|i| i.pointer.press_origin())
                    {
                        pv.drag = Some((id.clone(), from));
                    }
                    if drag.drag_stopped() {
                        let units = dragged.map_or(0.0, |d| f64::from((d / scale).round()));
                        if units != 0.0 {
                            pending.push((
                                "kerning.setPair",
                                json!({"left": row.left, "right": row.right, "value": row.value + units, "master": master}),
                            ));
                        }
                        pv.drag = None;
                    }
                    if response.double_clicked() || drag.double_clicked() {
                        pending.push(("kerning.showPairInEditView", json!({"left": row.left, "right": row.right})));
                    } else if response.clicked() || drag.clicked() || drag.drag_started() {
                        list.request_focus();
                        if !selected {
                            pending.push(("window.kerningPairsPreview", json!({"select": {"left": row.left, "right": row.right}})));
                        }
                    }
                }
            });
            if let Some((l, r)) = pv.selected.clone().filter(|(l, r)| is_group(l) || is_group(r)) {
                ui.separator();
                ui.label(egui::RichText::new(format!("Combinations of {l} {r}")).strong());
                combination_strip(ui, Parts { font: &font, master: &master, tokens, scale }, &l, &r, pending);
            }
        });
    pv.laid_out = laid_out;
    if !open {
        pending.push(("window.kerningPairsPreview", json!({"open": false})));
    }
}

/// What the combination strip draws with.
struct Parts<'a> {
    font: &'a Font,
    master: &'a str,
    tokens: &'a crate::theme::Tokens,
    scale: f32,
}

/// Every combination of the selected group pair, drawn kerned by the pair that applies; a click
/// selects that pair.
fn combination_strip(ui: &mut egui::Ui, parts: Parts<'_>, left: &str, right: &str, pending: &mut Pending) {
    let Parts { font, master, tokens, scale } = parts;
    let side_glyphs = |text: &str, position: Position| -> Vec<String> {
        match Side::parse(text) {
            Ok(Side::Glyph(name)) => vec![name],
            Ok(Side::Group(group)) => members(font, facing(Direction::Ltr, position), &group).into_iter().map(str::to_string).collect(),
            Err(_) => Vec::new(),
        }
    };
    let (lefts, rights) = (side_glyphs(left, Position::First), side_glyphs(right, Position::Second));
    if lefts.len().saturating_mul(rights.len()) > MAX_COMBINATIONS {
        ui.label(egui::RichText::new(format!("more than {MAX_COMBINATIONS} combinations")).color(tokens.text_dim));
        return;
    }
    let index = fontcraft_spacing::kerning::KernIndex::new(font);
    let small = scale * 0.75;
    let height = font.units_per_em as f32 * small * 1.3 + tokens.list_row_height;
    egui::ScrollArea::horizontal().id_salt("combinations").show(ui, |ui| {
        ui.horizontal(|ui| {
            for l in &lefts {
                for r in &rights {
                    let applied = index.lookup(Direction::Ltr, master, l, r).ok().flatten();
                    let value = applied.map_or(0.0, |a| a.value);
                    let wl = font.glyph(l).and_then(|g| g.layer(master)).map_or(0.0, |x| x.width) as f32;
                    let wr = font.glyph(r).and_then(|g| g.layer(master)).map_or(0.0, |x| x.width) as f32;
                    let width = ((wl + wr + value as f32) * small).max(tokens.cell_size) + tokens.dialog_spacing * 2.0;
                    let (rect, response) = ui.allocate_exact_size(vec2(width, height), Sense::click());
                    let painter = ui.painter_at(rect);
                    let ink = Stroke::new(1.0, tokens.glyph_fill);
                    let baseline = rect.top() + font.units_per_em as f32 * small;
                    let x = rect.left() + tokens.dialog_spacing;
                    let adv = paint_outline(&painter, font, l, master, pos2(x, baseline), small, ink);
                    paint_outline(&painter, font, r, master, pos2(x + adv + value as f32 * small, baseline), small, ink);
                    let label = applied.map_or("no pair".to_string(), |a| format!("{} ({})", a.value, a.step.name()));
                    painter.text(
                        pos2(rect.center().x, rect.bottom() - tokens.list_row_height / 2.0),
                        Align2::CENTER_CENTER,
                        label,
                        FontId::proportional(tokens.cell_label_size),
                        tokens.text_dim,
                    );
                    if response.clicked()
                        && let Some(a) = applied
                    {
                        let pair = json!({"left": Side::from_stored(a.first).command(), "right": Side::from_stored(a.second).command()});
                        pending.push(("window.kerningPairsPreview", json!({"select": pair})));
                    }
                }
            }
        });
    });
}

/// The arrow keys change the selected pair's value; a typed character scrolls to its pairs.
fn keys(ui: &egui::Ui, pv: &PreviewState, rows: &[PreviewRow], master: &str, pending: &mut Pending) {
    let (left, right, shift, typed) = ui.input(|i| {
        let typed: Option<String> = i.events.iter().find_map(|e| match e {
            egui::Event::Text(t) if t.chars().count() == 1 => Some(t.clone()),
            _ => None,
        });
        (i.key_pressed(Key::ArrowLeft), i.key_pressed(Key::ArrowRight), i.modifiers.shift, typed)
    });
    if let Some(t) = typed {
        pending.push(("window.kerningPairsPreview", json!({"jump": t})));
    }
    let step = if shift { 10.0 } else { 1.0 };
    let delta = match (left, right) {
        (true, false) => -step,
        (false, true) => step,
        _ => return,
    };
    let Some((l, r)) = &pv.selected else { return };
    let value = rows.iter().find(|row| row.left == *l && row.right == *r).map_or(0.0, |row| row.value);
    pending.push(("kerning.setPair", json!({"left": l, "right": r, "value": value + delta, "master": master})));
}
