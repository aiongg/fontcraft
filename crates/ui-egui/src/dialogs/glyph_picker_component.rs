//! The glyph picker of the component commands (`editView.chooseGlyph`): one pop-up with a search
//! field and a result list, which Add Component, the base glyph name of the component Info box,
//! Add Corner Component and the name of the corner Info box all open. Its keys are those of the
//! Find Glyphs pop-up the Text tool uses.
//!
//! | Purpose | Opened by | Lists | Return runs |
//! |---|---|---|---|
//! | `addComponent` | Glyph › Add Component, the canvas context menu (`dialog.componentPicker`) | glyphs by name or Unicode value | `glyph.addComponent` |
//! | `addComponentAllMasters` | Glyph › Add Component (all masters) | the same | `glyph.addComponentAllMasters` |
//! | `baseGlyph` | the base glyph name of the component Info box (`infoBox.componentGlyphPicker`) | the same | `component.setBaseGlyph` |
//! | `addCorner` | Add Corner Component in the node context menu (`dialog.cornerPicker`) | corner glyphs (`_corner.` names) | `cornerComponent.add` |
//! | `cornerGlyph` | the name of the corner Info box | corner glyphs | `cornerComponent.setGlyph` |
//!
//! | In the pop-up | Does |
//! |---|---|
//! | typing in the field | searches again; the first result is highlighted |
//! | click | highlights the result |
//! | Up, Down | highlights the result above or below |
//! | Return | runs the purpose's command with the highlighted glyph and closes the pop-up |
//! | Escape | closes the pop-up; nothing changes |
//!
//! What the command acts on (glyph, layer, and the component, nodes or corners) is fixed when the
//! pop-up opens, from the parameters or else from the Edit View selection, so a pick acts on what
//! was selected when the person asked for the picker.

use egui::{Rect, Sense, vec2};
use fontcraft_engine::model::corner::{CornerTarget, corner_hint, is_corner};
use fontcraft_engine::model::{Layer, Shape};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::app::FontcraftApp;
use crate::canvas::view;
use crate::shell::{Args, UiCommand};

type Pending = Vec<(&'static str, Value)>;

/// The command that drives the picker.
pub const COMMAND: &str = "editView.chooseGlyph";
/// Longest query, in characters: four bytes each stay within the search's byte limit.
const MAX_QUERY_CHARS: usize = 64;
/// Most results one search lists (the search's own limit).
const MAX_RESULTS: usize = 1000;
/// Most entries in a `points` or `corners` list given to the picker.
const MAX_TARGET_ITEMS: usize = 4096;
/// Results the pop-up shows before it scrolls.
const VISIBLE_ROWS: f32 = 14.0;
/// Width of the pop-up, in points.
const WIDTH: f32 = 320.0;
/// Characters of untrusted text quoted in a message.
const MAX_QUOTED: usize = 64;
/// The name prefix of corner glyphs.
pub const CORNER_PREFIX: &str = "_corner.";

pub const COMMANDS: &[UiCommand] = &[(
    COMMAND,
    "Choose Glyph",
    &[],
    "",
    "{tab?: id, purpose?: \"addComponent\" (default) | \"addComponentAllMasters\" | \"baseGlyph\" | \"addCorner\" | \"cornerGlyph\", glyph?: name, layer?: id, component?: index, points?: [[shape, node]], corners?: [indices], query?: string, select?: name, pick?: bool, open?: bool (default true)} in Edit View, show the glyph picker of a component command. Component purposes search the font as glyph.search does by name or Unicode value; corner purposes list the corner glyphs (names starting with _corner.) whose names contain every word of the query. The purpose and what it acts on are fixed when the picker opens: glyph and layer default to the edited ones, component to the selected component (baseGlyph), points to the selected nodes (addCorner), corners to those on the selected nodes (cornerGlyph). select highlights a result; pick runs the purpose's command (glyph.addComponent, glyph.addComponentAllMasters, component.setBaseGlyph, cornerComponent.add, cornerComponent.setGlyph) with the highlighted glyph and closes the picker; a command that fails leaves the picker open → {open, purpose, query, results: [names], total, selected} | with pick, the command's result",
)];

/// What a pick does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Purpose {
    #[default]
    AddComponent,
    AddComponentAllMasters,
    BaseGlyph,
    AddCorner,
    CornerGlyph,
}

impl Purpose {
    const ALL: [(&'static str, Purpose); 5] = [
        ("addComponent", Purpose::AddComponent),
        ("addComponentAllMasters", Purpose::AddComponentAllMasters),
        ("baseGlyph", Purpose::BaseGlyph),
        ("addCorner", Purpose::AddCorner),
        ("cornerGlyph", Purpose::CornerGlyph),
    ];

    pub fn name(self) -> &'static str {
        Self::ALL.iter().find(|(_, p)| *p == self).map_or("addComponent", |(n, _)| n)
    }

    /// The command a pick runs.
    pub fn command(self) -> &'static str {
        match self {
            Purpose::AddComponent => "glyph.addComponent",
            Purpose::AddComponentAllMasters => "glyph.addComponentAllMasters",
            Purpose::BaseGlyph => "component.setBaseGlyph",
            Purpose::AddCorner => "cornerComponent.add",
            Purpose::CornerGlyph => "cornerComponent.setGlyph",
        }
    }

    /// The parameter of that command the picked glyph goes into.
    fn key(self) -> &'static str {
        match self {
            Purpose::AddComponent | Purpose::AddComponentAllMasters => "component",
            Purpose::BaseGlyph => "base",
            Purpose::AddCorner | Purpose::CornerGlyph => "corner",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Purpose::AddComponent | Purpose::AddComponentAllMasters => "Choose Component",
            Purpose::BaseGlyph => "Choose Base Glyph",
            Purpose::AddCorner | Purpose::CornerGlyph => "Choose Corner Component",
        }
    }

    pub fn corners_only(self) -> bool {
        matches!(self, Purpose::AddCorner | Purpose::CornerGlyph)
    }
}

/// One result as the picker lists it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Found {
    pub name: String,
    /// The first code point in hexadecimal, or empty.
    pub unicode: String,
}

/// What a search was run for: the purpose's kind of list, the query and the font it read.
#[derive(Clone, Debug, PartialEq)]
struct Key {
    corners: bool,
    query: String,
    /// The document's id and revision: every change, and every other document, gives a new one.
    font: Option<(u64, u64)>,
}

/// The picker's state.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PickerState {
    pub open: bool,
    /// The tab the picker belongs to; it shows only while that tab does.
    pub tab: Option<u64>,
    pub purpose: Purpose,
    /// The targets of the purpose's command, fixed when the picker opened.
    pub targets: Map<String, Value>,
    pub query: String,
    /// The highlighted result.
    pub selected: Option<String>,
    #[serde(skip)]
    key: Option<Key>,
    #[serde(skip)]
    pub results: Vec<Found>,
    /// How many glyphs matched, also beyond the listed ones.
    #[serde(skip)]
    pub total: usize,
    /// Why the last search or pick was refused.
    #[serde(skip)]
    pub error: Option<String>,
    /// The picker opened this frame: the field takes the keyboard.
    #[serde(skip)]
    pub fresh: bool,
    /// The results laid out in the last frame with their rectangles: tests click them.
    #[serde(skip)]
    pub laid_out: Vec<(String, Rect)>,
}

impl PickerState {
    fn contains(&self, name: &str) -> bool {
        self.results.iter().any(|r| r.name == name)
    }
}

/// The id of the search field, for focus.
pub fn field_id() -> egui::Id {
    egui::Id::new("editView.chooseGlyph.query")
}

fn quoted(s: &str) -> String {
    let mut out: String = s.chars().take(MAX_QUOTED).collect();
    if out.len() < s.len() {
        out.push('…');
    }
    out
}

fn key(app: &FontcraftApp, corners: bool, query: &str) -> Key {
    let font = app.session.active().map(|d| (d.id, d.revision));
    Key { corners, query: query.to_string(), font }
}

/// The results for `key`, how many matched, and the refusal.
fn search(app: &mut FontcraftApp, key: &Key) -> (Vec<Found>, usize, Option<String>) {
    if key.corners {
        return corner_glyphs(app, &key.query);
    }
    let params = json!({ "query": key.query, "mode": "all", "limit": MAX_RESULTS });
    match app.session.execute("glyph.search", &params) {
        Ok(v) => {
            let found: Vec<Found> = v
                .get("results")
                .and_then(Value::as_array)
                .map(|list| {
                    list.iter()
                        .filter_map(|r| {
                            let name = r.get("name").and_then(Value::as_str)?.to_string();
                            let unicode = r.pointer("/unicodes/0").and_then(Value::as_str).unwrap_or_default().to_string();
                            Some(Found { name, unicode })
                        })
                        .collect()
                })
                .unwrap_or_default();
            let total = v.get("total").and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok()).unwrap_or(found.len());
            (found, total, None)
        }
        Err(e) => (Vec::new(), 0, Some(e.to_string())),
    }
}

/// The corner glyphs of the font, in glyph order, whose names contain every word of `query`
/// (ignoring case); an empty query lists them all.
fn corner_glyphs(app: &FontcraftApp, query: &str) -> (Vec<Found>, usize, Option<String>) {
    let Ok(font) = app.session.font() else { return (Vec::new(), 0, Some("no document is open".into())) };
    let terms: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
    let mut found = Vec::new();
    let mut total = 0usize;
    for glyph in font.glyphs.iter() {
        let name = glyph.name.as_str();
        if !name.starts_with(CORNER_PREFIX) || name.len() <= CORNER_PREFIX.len() {
            continue;
        }
        let lower = name.to_lowercase();
        if !terms.iter().all(|t| lower.contains(t.as_str())) {
            continue;
        }
        total += 1;
        if found.len() < MAX_RESULTS {
            found.push(Found { name: name.to_string(), unicode: String::new() });
        }
    }
    (found, total, None)
}

/// Searches again when the query, the purpose's list or the font changed; a new search
/// highlights its first result.
pub fn refresh(app: &mut FontcraftApp) {
    let st = &app.state.panels.component_picker;
    let k = key(app, st.purpose.corners_only(), &st.query);
    if st.key.as_ref() == Some(&k) {
        return;
    }
    let (results, total, error) = search(app, &k);
    let st = &mut app.state.panels.component_picker;
    st.selected = results.first().map(|r| r.name.clone());
    (st.results, st.total, st.error, st.key) = (results, total, error, Some(k));
}

/// The corner components (hint indices) of `layer` that sit on the nodes `points` (model order).
pub fn corners_on_nodes(layer: &Layer, points: impl IntoIterator<Item = (usize, usize)>) -> Vec<usize> {
    let points: Vec<(usize, usize)> = points.into_iter().collect();
    layer
        .info
        .hints
        .iter()
        .enumerate()
        .filter(|(_, h)| is_corner(h))
        .filter_map(|(i, h)| {
            let corner = corner_hint(h).ok().flatten()?;
            let CornerTarget::Node { shape, node } = corner.target else { return None };
            let Some(Shape::Path(path)) = layer.shapes.get(shape) else { return None };
            // The hint counts a closed path's nodes with the start node last.
            let len = path.nodes.len();
            let model = if path.closed && len > 0 { (node + 1) % len } else { node };
            points.contains(&(shape, model)).then_some(i)
        })
        .collect()
}

/// The targets of `purpose` from the parameters, else from the showing tab's selection.
fn targets(app: &FontcraftApp, args: &Args, purpose: Purpose) -> Result<Map<String, Value>, String> {
    let font = app.session.font().map_err(|e| e.to_string())?;
    let tab = app.state.tab();
    let mut m = Map::new();
    let glyph = match args.string("glyph")? {
        Some(g) => g.to_string(),
        None => tab.and_then(|t| t.edited_glyph()).map(str::to_string).ok_or_else(|| args.err("no glyph is being edited: give `glyph`"))?,
    };
    let g = font.glyph(&glyph).ok_or_else(|| args.err(format!("no glyph is named `{}`", quoted(&glyph))))?;
    // The layer the edited glyph shows in the tab, else the glyph's first master layer.
    let edited = tab.filter(|t| t.edited_glyph() == Some(glyph.as_str())).and_then(|t| view::active_layer(font, t));
    let layer = match args.string("layer")? {
        Some(id) => g.layers.iter().find(|l| l.id == id).ok_or_else(|| args.err(format!("glyph `{glyph}` has no layer `{}`", quoted(id))))?,
        None => match edited {
            Some(l) => l,
            None => font.masters.first().and_then(|m| g.layer(&m.id)).ok_or_else(|| args.err(format!("glyph `{glyph}` has no master layer")))?,
        },
    };
    m.insert("glyph".into(), json!(glyph));
    m.insert("layer".into(), json!(layer.id));
    let selection = tab.filter(|_| edited.is_some_and(|l| l.id == layer.id)).map(|t| &t.selection);
    let list = |key: &str| -> Result<Option<Value>, String> {
        match args.get(key) {
            None => Ok(None),
            Some(Value::Array(a)) if a.len() <= MAX_TARGET_ITEMS => Ok(Some(Value::Array(a.clone()))),
            Some(_) => Err(args.err(format!("`{key}` must be a list of at most {MAX_TARGET_ITEMS} entries"))),
        }
    };
    match purpose {
        Purpose::AddComponent | Purpose::AddComponentAllMasters => {}
        Purpose::BaseGlyph => {
            let index = match args.index("component")? {
                Some(i) => usize::try_from(i).map_err(|_| args.err("`component` is too large"))?,
                None => selection
                    .and_then(|s| s.shapes.iter().copied().find(|&i| matches!(layer.shapes.get(i), Some(Shape::Component(_)))))
                    .ok_or_else(|| args.err("select a component, or give `component`"))?,
            };
            if !matches!(layer.shapes.get(index), Some(Shape::Component(_))) {
                return Err(args.err(format!("shape {index} is not a component of the layer")));
            }
            m.insert("component".into(), json!(index));
        }
        Purpose::AddCorner => {
            let points = match list("points")? {
                Some(v) => v,
                None => {
                    let points: Vec<Value> = selection.map(|s| s.points.iter().map(|&(a, b)| json!([a, b])).collect()).unwrap_or_default();
                    if points.is_empty() {
                        return Err(args.err("select the nodes to add a corner component to, or give `points`"));
                    }
                    Value::Array(points)
                }
            };
            m.insert("points".into(), points);
        }
        Purpose::CornerGlyph => {
            let corners = match list("corners")? {
                Some(v) => v,
                None => {
                    let on = corners_on_nodes(layer, selection.map(|s| s.points.iter().copied().collect::<Vec<_>>()).unwrap_or_default());
                    if on.is_empty() {
                        return Err(args.err("select a node that holds a corner component, or give `corners`"));
                    }
                    json!(on)
                }
            };
            m.insert("corners".into(), corners);
        }
    }
    Ok(m)
}

fn purpose_arg(args: &Args) -> Result<Option<Purpose>, String> {
    match args.string("purpose")? {
        None => Ok(None),
        Some(p) => Purpose::ALL.iter().find(|(n, _)| *n == p).map(|(_, v)| Some(*v)).ok_or_else(|| {
            let names: Vec<&str> = Purpose::ALL.iter().map(|(n, _)| *n).collect();
            args.err(format!("`purpose` must be one of {}, got \"{}\"", names.join(", "), quoted(p)))
        }),
    }
}

pub fn run(app: &mut FontcraftApp, id: &str, args: &Args) -> Result<Value, String> {
    if id != COMMAND {
        return Err(args.err("unknown command"));
    }
    let index = crate::shell::view::target_tab(app, args)?;
    let tab = app.state.tabs.get(index).map(|t| t.id).ok_or_else(|| args.err("no such tab"))?;
    let current = &app.state.panels.component_picker;
    let given = purpose_arg(args)?;
    // A picker that is open for this tab keeps its purpose and targets unless others are asked for.
    let reopen = !current.open || current.tab != Some(tab) || given.is_some_and(|p| p != current.purpose) || has_targets(args);
    // Everything is checked on a copy before anything changes.
    let mut next = current.clone();
    if reopen {
        let purpose = given.unwrap_or_default();
        next = PickerState { purpose, targets: targets(app, args, purpose)?, tab: Some(tab), fresh: true, ..PickerState::default() };
    }
    if let Some(q) = args.string("query")? {
        if q.chars().count() > MAX_QUERY_CHARS {
            return Err(args.err(format!("`query` holds at most {MAX_QUERY_CHARS} characters")));
        }
        next.query = q.to_string();
    }
    let k = key(app, next.purpose.corners_only(), &next.query);
    if next.key.as_ref() != Some(&k) {
        let (results, total, error) = search(app, &k);
        if let Some(e) = error {
            return Err(args.err(e));
        }
        next.selected = results.first().map(|r| r.name.clone());
        (next.results, next.total, next.error, next.key) = (results, total, None, Some(k));
    }
    if let Some(name) = args.string("select")? {
        if !next.contains(name) {
            return Err(args.err(format!("`{}` is not among the results", quoted(name))));
        }
        next.selected = Some(name.to_string());
    }
    if args.flag("pick")?.unwrap_or(false) {
        let name = next.selected.clone().ok_or_else(|| args.err("no result is highlighted"))?;
        let mut params = next.targets.clone();
        params.insert(next.purpose.key().into(), json!(name));
        let command = next.purpose.command();
        match app.run(command, &Value::Object(params)) {
            Ok(result) => {
                next.open = false;
                app.state.panels.component_picker = next;
                return Ok(result);
            }
            Err(e) => {
                // The picker stays as it was, open, with the message under its list.
                next.open = true;
                next.error = Some(e.clone());
                app.state.panels.component_picker = next;
                return Err(e);
            }
        }
    }
    next.open = args.flag("open")?.unwrap_or(true);
    let st = &mut app.state.panels.component_picker;
    *st = next;
    Ok(reply(st))
}

/// Whether the parameters name what the purpose acts on.
fn has_targets(args: &Args) -> bool {
    ["glyph", "layer", "component", "points", "corners"].iter().any(|k| args.get(k).is_some())
}

fn reply(st: &PickerState) -> Value {
    json!({
        "open": st.open,
        "purpose": st.purpose.name(),
        "query": st.query,
        "results": st.results.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
        "total": st.total,
        "selected": st.selected,
    })
}

/// Glyph › Add Component and its all-masters variant from the menu bar or a shortcut, without
/// parameters, while a glyph is being edited: open the picker for it. `None` for any other
/// command, or in Font View, where the batch form asks instead.
pub fn menu_command(app: &mut FontcraftApp, id: &str) -> Option<Result<Value, String>> {
    let purpose = match id {
        "glyph.addComponent" => Purpose::AddComponent,
        "glyph.addComponentAllMasters" => Purpose::AddComponentAllMasters,
        _ => return None,
    };
    app.state.tab()?.edited_glyph()?;
    Some(app.run(COMMAND, &json!({ "purpose": purpose.name() })))
}

/// What the person did in the pop-up this frame.
enum Event {
    Query(String),
    Click(String),
    Step(bool),
    Pick,
    Close,
}

/// Draws the pop-up while it is open and its tab is showing. Return queues the pick in `pending`.
pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    let showing = app.state.tab().map(|t| t.id);
    let st = &app.state.panels.component_picker;
    if !st.open || st.tab.is_none() || st.tab != showing {
        return;
    }
    refresh(app);
    let FontcraftApp { state, tokens, .. } = app;
    let st = &mut state.panels.component_picker;
    let mut events = Vec::new();
    let mut laid_out = Vec::new();
    let mut open = true;
    egui::Window::new(st.purpose.title())
        .id(egui::Id::new("editView.chooseGlyph"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(WIDTH)
        .pivot(egui::Align2::CENTER_TOP)
        .default_pos(egui::pos2(ui.ctx().content_rect().center().x, tokens.dialog_top))
        .show(ui.ctx(), |ui| {
            let mut query = st.query.clone();
            let hint = if st.purpose.corners_only() { "Corner glyph name" } else { "Name, character or code point" };
            let field =
                egui::TextEdit::singleline(&mut query).id(field_id()).hint_text(hint).char_limit(MAX_QUERY_CHARS).desired_width(ui.available_width());
            let response = ui.add(field);
            // Up and Down step through the results; without this they move the focus.
            let filter = egui::EventFilter { vertical_arrows: true, horizontal_arrows: true, ..Default::default() };
            ui.memory_mut(|m| m.set_focus_lock_filter(field_id(), filter));
            if st.fresh {
                response.request_focus();
                st.fresh = false;
            }
            if response.changed() {
                events.push(Event::Query(query));
            }
            let (enter, escape, up, down) = ui.input(|i| {
                (
                    i.key_pressed(egui::Key::Enter),
                    i.key_pressed(egui::Key::Escape),
                    i.key_pressed(egui::Key::ArrowUp),
                    i.key_pressed(egui::Key::ArrowDown),
                )
            });
            let focused = response.has_focus() || response.lost_focus();
            if focused && enter {
                events.push(Event::Pick);
            } else if focused && escape {
                events.push(Event::Close);
            } else if response.has_focus() && (up || down) {
                events.push(Event::Step(down));
            }
            ui.add_space(tokens.dialog_spacing);
            let height = tokens.list_row_height;
            egui::ScrollArea::vertical().id_salt("chooseGlyph.v").max_height(height * VISIBLE_ROWS).auto_shrink([false, true]).show_rows(
                ui,
                height,
                st.results.len(),
                |ui, range| {
                    for at in range {
                        let Some(row) = st.results.get(at) else { continue };
                        let selected = st.selected.as_deref() == Some(row.name.as_str());
                        let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::click());
                        laid_out.push((row.name.clone(), rect));
                        let fill = if selected { tokens.row_selected } else { tokens.panel };
                        ui.painter_at(rect).rect_filled(rect, egui::CornerRadius::ZERO, fill);
                        let color = if selected { tokens.selected_text } else { tokens.text };
                        let inner = rect.shrink2(vec2(tokens.cell_inset, 0.0));
                        let font = egui::FontId::proportional(tokens.font_size);
                        ui.painter_at(inner).text(inner.left_center(), egui::Align2::LEFT_CENTER, &row.name, font.clone(), color);
                        if !row.unicode.is_empty() {
                            let dim = if selected { tokens.selected_text } else { tokens.text_dim };
                            ui.painter_at(inner).text(inner.right_center(), egui::Align2::RIGHT_CENTER, &row.unicode, font, dim);
                        }
                        if response.double_clicked() {
                            events.push(Event::Click(row.name.clone()));
                            events.push(Event::Pick);
                        } else if response.clicked() {
                            events.push(Event::Click(row.name.clone()));
                        }
                    }
                },
            );
            let note = match &st.error {
                Some(e) => e.clone(),
                None if st.purpose.corners_only() && st.results.is_empty() && st.query.trim().is_empty() => {
                    "The font has no corner glyph (a name starting with _corner.)".into()
                }
                None if st.query.trim().is_empty() => String::new(),
                None if st.total > st.results.len() => format!("The first {} of {} matches", st.results.len(), st.total),
                None => format!("{} matches", st.total),
            };
            if !note.is_empty() {
                let color = if st.error.is_some() { tokens.error_text } else { tokens.text_dim };
                ui.colored_label(color, note);
            }
        });
    if !open {
        events.push(Event::Close);
    }
    let order: Vec<String> = st.results.iter().map(|r| r.name.clone()).collect();
    for e in events {
        match e {
            Event::Query(q) => st.query = q,
            Event::Click(name) => {
                st.selected = Some(name);
                ui.ctx().memory_mut(|m| m.request_focus(field_id()));
            }
            Event::Step(down) => {
                let at = st.selected.as_ref().and_then(|s| order.iter().position(|n| n == s));
                let to = match (at, down) {
                    (Some(i), true) => (i + 1).min(order.len().saturating_sub(1)),
                    (Some(i), false) => i.saturating_sub(1),
                    (None, _) => 0,
                };
                if let Some(name) = order.get(to) {
                    st.selected = Some(name.clone());
                }
            }
            Event::Pick => {
                if let Some(tab) = st.tab {
                    pending.push((COMMAND, json!({ "tab": tab, "pick": true })));
                }
            }
            Event::Close => st.open = false,
        }
    }
    st.laid_out = laid_out;
}
