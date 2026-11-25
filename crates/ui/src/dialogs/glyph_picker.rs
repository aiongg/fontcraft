//! The glyph picker of the component commands (`editView.chooseGlyph`): a search field over a result list. Add
//! Component, the base glyph name of the component Info box, Add Corner Component, the corner Info box's name and the
//! cap, segment component and brush equivalents all open it. The attachment pickers are this one module, each listing
//! only the glyphs of its kind's prefix (`_corner.`, `_cap.`, `_segment.`, `_brush.`).
//!
//! | Purpose | Opened by | Lists | OK runs |
//! |---|---|---|---|
//! | `addComponent` | Glyph › Add Component (`dialog.componentPicker`) | glyphs by name or Unicode value | `glyph.addComponent` |
//! | `addComponentAllMasters` | Glyph › Add Component (all masters) | the same | `glyph.addComponentAllMasters` |
//! | `baseGlyph` | the base glyph name of the component Info box (`infoBox.componentGlyphPicker`) | the same | `component.setBaseGlyph` |
//! | `addCorner` | Add Corner Component (`dialog.cornerPicker`) | corner glyphs (`_corner.` names) | `cornerComponent.add` |
//! | `cornerGlyph` | the name of the corner Info box | corner glyphs | `cornerComponent.setGlyph` |
//! | `addCap` | Add Cap Component (`dialog.capPicker`) | cap glyphs (`_cap.` names) | `capComponent.add` |
//! | `capGlyph` | the name of the cap Info box | cap glyphs | `capComponent.setGlyph` |
//! | `addSegment` | Add Segment Component (`dialog.segmentPicker`) | segment glyphs (`_segment.` names) | `segmentComponent.add` |
//! | `segmentGlyph` | the name of the segment component Info box | segment glyphs | `segmentComponent.setGlyph` |
//! | `addBrush` | Add Brush (`dialog.brushPicker`) | brush glyphs (`_brush.` names) | `brush.add` |
//! | `brushGlyph` | the name of the brush Info box | brush glyphs | `brush.setGlyph` |
//!
//! Typing searches again and highlights the first result; OK (Return) runs the purpose's command with the highlighted
//! glyph and closes the picker; Cancel (Escape) changes nothing. What the command acts on is fixed when the picker
//! opens, from the parameters or else from the canvas selection.

use std::borrow::Cow;

use craft::behavior::{ColumnKey, ColumnWidth, Selection, keyed};
use craft::ui::contract::{Bind, ColumnSpec, SelectMode, Show as _, ShowCollection as _};
use craft::ui::dialog::{DialogHost, DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use craft::ui::display::{Shade, Text};
use craft::ui::list::ListRow;
use craft::ui::patterns::form_dialogs::FormDialog;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::Env;
use craft::ui::table::Table;
use craft::ui::text::TextField;
use egui::Ui;
use fontcraft_engine::model::attach;
use serde_json::{Map, Value, json};

use crate::app::models::Stamp;
use crate::app::models::components::{caps_on_nodes, corners_on_nodes, segments_on_nodes, selected_components};
use crate::app::{FontCraftApp, Msg};
use crate::canvas::tools::scene;
use crate::canvas::tools::targets::{selected_extra_nodes, selected_segments};
use crate::shell::commands::{Module, UiCommand};
use crate::shell::masters::shown_layer;
use crate::shell::params::{flag, quoted, string};

craft::ui::words! {
    /// The words of the component glyph picker.
    pub struct GlyphPickerWords {
        component: "fontcraftGlyphPicker.component" = "Choose Component";
        base: "fontcraftGlyphPicker.base" = "Choose Base Glyph";
        corner: "fontcraftGlyphPicker.corner" = "Choose Corner Component";
        cap: "fontcraftGlyphPicker.cap" = "Choose Cap Component";
        segment: "fontcraftGlyphPicker.segment" = "Choose Segment Component";
        brush: "fontcraftGlyphPicker.brush" = "Choose Brush";
        search: "fontcraftGlyphPicker.search" = "Search";
        results: "fontcraftGlyphPicker.results" = "Glyphs";
        name: "fontcraftGlyphPicker.name" = "Name";
        unicode: "fontcraftGlyphPicker.unicode" = "Unicode";
        choose: "fontcraftGlyphPicker.choose" = "Choose";
        nothing: "fontcraftGlyphPicker.nothing" = "No glyph matches";
    }
}

/// The command that drives the picker.
pub const COMMAND: &str = "editView.chooseGlyph";
/// Longest query, in characters: four bytes each stay within the search's byte limit.
const MAX_QUERY_CHARS: usize = 64;
/// Most results one search lists (the search's own limit).
const MAX_RESULTS: usize = 1000;
/// Most entries in a target list given to the picker.
const MAX_TARGET_ITEMS: usize = 4096;
/// Rows the result table shows before it scrolls.
const LIST_ROWS: u16 = 12;

pub const SHELL: Module = Module { commands: COMMANDS, run };

const COMMANDS: &[UiCommand] = &[UiCommand {
    id: COMMAND,
    label: "Choose Glyph",
    menu: &[],
    shortcut: None,
    params: "{purpose?: \"addComponent\" (default) | \"addComponentAllMasters\" | \"baseGlyph\" | \"addCorner\" | \"cornerGlyph\" | \"addCap\" | \"capGlyph\" | \"addSegment\" | \"segmentGlyph\" | \"addBrush\" | \"brushGlyph\", glyph?: name, layer?: id, component?: index, components?: [indices], points?: [[shape, node]], extraNodes?: [[x, y]], segments?: [[shape, node]], corners?: [indices], caps?: [indices], segmentComponents?: [indices], brushes?: [indices], query?: string, select?: name, pick?: bool, open?: bool (default true)} show the glyph picker of a component command. Component purposes search the font as glyph.search does; the corner, cap, segment and brush purposes list the glyphs named _corner.…, _cap.…, _segment.… or _brush.… whose names hold every word of the query. The purpose and its targets are fixed when the picker opens: glyph and layer default to the edited ones, the components to the selected ones, points and extraNodes to the selected nodes and extra nodes, segments to the selected segments (a brush falls back to the selected nodes), and corners, caps, segment components and brushes to the selected ones or those on the selected nodes. select highlights a result; pick runs the purpose's command with the highlighted glyph and closes the picker",
}];

/// What a pick does.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Purpose {
    #[default]
    AddComponent,
    AddComponentAllMasters,
    BaseGlyph,
    AddCorner,
    CornerGlyph,
    AddCap,
    CapGlyph,
    AddSegment,
    SegmentGlyph,
    AddBrush,
    BrushGlyph,
}

impl Purpose {
    const ALL: [(&'static str, Purpose); 11] = [
        ("addComponent", Purpose::AddComponent),
        ("addComponentAllMasters", Purpose::AddComponentAllMasters),
        ("baseGlyph", Purpose::BaseGlyph),
        ("addCorner", Purpose::AddCorner),
        ("cornerGlyph", Purpose::CornerGlyph),
        ("addCap", Purpose::AddCap),
        ("capGlyph", Purpose::CapGlyph),
        ("addSegment", Purpose::AddSegment),
        ("segmentGlyph", Purpose::SegmentGlyph),
        ("addBrush", Purpose::AddBrush),
        ("brushGlyph", Purpose::BrushGlyph),
    ];

    /// The command a pick runs.
    pub fn command(self) -> &'static str {
        match self {
            Purpose::AddComponent => "glyph.addComponent",
            Purpose::AddComponentAllMasters => "glyph.addComponentAllMasters",
            Purpose::BaseGlyph => "component.setBaseGlyph",
            Purpose::AddCorner => "cornerComponent.add",
            Purpose::CornerGlyph => "cornerComponent.setGlyph",
            Purpose::AddCap => "capComponent.add",
            Purpose::CapGlyph => "capComponent.setGlyph",
            Purpose::AddSegment => "segmentComponent.add",
            Purpose::SegmentGlyph => "segmentComponent.setGlyph",
            Purpose::AddBrush => "brush.add",
            Purpose::BrushGlyph => "brush.setGlyph",
        }
    }

    /// The parameter of that command the picked glyph goes into.
    fn key(self) -> &'static str {
        match self {
            Purpose::AddComponent | Purpose::AddComponentAllMasters => "component",
            Purpose::BaseGlyph => "base",
            Purpose::AddCorner | Purpose::CornerGlyph => "corner",
            Purpose::AddCap | Purpose::CapGlyph => "cap",
            Purpose::AddSegment | Purpose::SegmentGlyph => "segmentGlyph",
            Purpose::AddBrush | Purpose::BrushGlyph => "brush",
        }
    }

    /// The name prefix of the glyphs the purpose lists; `None`: every glyph.
    fn prefix(self) -> Option<&'static str> {
        match self {
            Purpose::AddCorner | Purpose::CornerGlyph => Some("_corner."),
            Purpose::AddCap | Purpose::CapGlyph => Some("_cap."),
            Purpose::AddSegment | Purpose::SegmentGlyph => Some("_segment."),
            Purpose::AddBrush | Purpose::BrushGlyph => Some("_brush."),
            _ => None,
        }
    }
}

/// One result as the picker lists it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Found {
    pub name: String,
    /// The first code point in hexadecimal, or empty.
    pub unicode: String,
}

/// The picker's state, in the app's view state while it is open.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PickerState {
    pub purpose: Purpose,
    /// The targets of the purpose's command, fixed when the picker opened.
    pub targets: Map<String, Value>,
    pub query: String,
    /// The highlighted result.
    pub selected: Option<String>,
    pub results: Vec<Found>,
    /// How many glyphs matched, also beyond the listed ones.
    pub total: usize,
    /// Why the last search was refused.
    pub error: Option<String>,
    /// What the results were searched for.
    searched: Option<(Purpose, String, Stamp)>,
}

impl PickerState {
    /// The command a pick of the highlighted result runs.
    pub fn pick(&self) -> Result<Msg, String> {
        let name = self.selected.clone().ok_or("no result is highlighted")?;
        let mut params = self.targets.clone();
        params.insert(self.purpose.key().into(), json!(name));
        Ok(Msg::run(self.purpose.command(), Value::Object(params)))
    }
}

/// Searches again when the purpose's list, the query or the font changed; a new search highlights its first result.
fn refresh(app: &mut FontCraftApp, st: &mut PickerState) {
    let key = (st.purpose, st.query.clone(), Stamp::of(&app.session, 0));
    if st.searched.as_ref() == Some(&key) {
        return;
    }
    let (results, total, error) = search(app, st.purpose, &st.query);
    st.selected = results.first().map(|r| r.name.clone());
    (st.results, st.total, st.error, st.searched) = (results, total, error, Some(key));
}

/// The results for `query`, how many matched, and the refusal.
fn search(app: &mut FontCraftApp, purpose: Purpose, query: &str) -> (Vec<Found>, usize, Option<String>) {
    if let Some(prefix) = purpose.prefix() {
        let Ok(font) = app.session.font() else { return (Vec::new(), 0, Some("no document is open".into())) };
        let terms: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        let mut found = Vec::new();
        let mut total = 0usize;
        for glyph in font.glyphs.iter() {
            let name = glyph.name.as_str();
            if !name.starts_with(prefix) || name.len() <= prefix.len() {
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
        return (found, total, None);
    }
    match app.session.query("glyph.search", &json!({ "query": query, "mode": "all", "limit": MAX_RESULTS })) {
        Ok(v) => {
            let found: Vec<Found> = v
                .get("results")
                .and_then(Value::as_array)
                .map(|list| {
                    list.iter()
                        .take(MAX_RESULTS)
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

/// The targets of `purpose` from `p`, else from the canvas selection.
fn targets(app: &FontCraftApp, p: &Value, purpose: Purpose) -> Result<Map<String, Value>, String> {
    let font = app.session.font().map_err(|e| e.to_string())?;
    let shown = scene(font, app.session.glyph_data().ok().as_deref(), &app.view, app.tools.focus.as_ref());
    let glyph = match string(p, "glyph")? {
        Some(g) => g.to_string(),
        None => shown.as_ref().map(|s| s.glyph.clone()).ok_or("no glyph is being edited: give `glyph`")?,
    };
    let g = font.glyph(&glyph).ok_or_else(|| format!("no glyph is named `{}`", quoted(&glyph)))?;
    let edited = shown.as_ref().filter(|s| s.glyph == glyph).and_then(|s| g.layers.iter().find(|l| l.id == s.layer_id));
    let layer = match string(p, "layer")? {
        Some(id) => g.layers.iter().find(|l| l.id == id).ok_or_else(|| format!("glyph `{glyph}` has no layer `{}`", quoted(id)))?,
        None => match edited {
            Some(l) => l,
            None => shown_layer(font, &app.view.layers, g).ok_or_else(|| format!("glyph `{glyph}` has no master layer"))?,
        },
    };
    let mut m = Map::new();
    m.insert("glyph".into(), json!(glyph));
    m.insert("layer".into(), json!(layer.id));
    // The selection belongs to the layer the canvas shows.
    let selection = edited.filter(|l| l.id == layer.id).map(|_| &app.tools.selection);
    let points: Vec<(usize, usize)> = selection.map(|s| s.points.iter().copied().collect()).unwrap_or_default();
    let list = |key: &str| -> Result<Option<Value>, String> {
        match p.get(key).filter(|v| !v.is_null()) {
            None => Ok(None),
            Some(Value::Array(a)) if a.len() <= MAX_TARGET_ITEMS => Ok(Some(Value::Array(a.clone()))),
            Some(_) => Err(format!("`{key}` must be a list of at most {MAX_TARGET_ITEMS} entries")),
        }
    };
    let need = |found: Value, what: &str| if found.as_array().is_some_and(|a| !a.is_empty()) { Ok(found) } else { Err(what.to_string()) };
    match purpose {
        Purpose::AddComponent | Purpose::AddComponentAllMasters => {}
        Purpose::BaseGlyph => {
            let of_value = |v: &Value| v.as_u64().and_then(|i| usize::try_from(i).ok()).ok_or("a component must be an index");
            let picked: Vec<usize> = match (list("components")?, p.get("component").filter(|v| !v.is_null())) {
                (Some(_), Some(_)) => return Err("give `component` or `components`, not both".into()),
                (Some(given), None) => given.as_array().into_iter().flatten().map(of_value).collect::<Result<_, _>>()?,
                (None, Some(v)) => vec![of_value(v)?],
                (None, None) => selection.map(|s| selected_components(layer, s)).unwrap_or_default(),
            };
            if picked.is_empty() {
                return Err("select a component, or give `component` or `components`".into());
            }
            if let Some(i) = picked.iter().find(|&&i| !matches!(layer.shapes.get(i), Some(fontcraft_engine::model::Shape::Component(_)))) {
                return Err(format!("shape {i} is not a component of the layer"));
            }
            m.insert("components".into(), json!(picked));
        }
        Purpose::AddCorner => {
            let (given, extra) = (list("points")?, list("extraNodes")?);
            let (nodes, extra) = if given.is_some() || extra.is_some() {
                (given, extra)
            } else {
                let extra = match (shown.as_ref(), selection) {
                    (Some(scene), Some(sel)) => selected_extra_nodes(scene, sel),
                    _ => Vec::new(),
                };
                (Some(Value::Array(points.iter().map(|&(a, b)| json!([a, b])).collect())), Some(json!(extra)))
            };
            let filled = |v: Option<Value>| v.filter(|v| v.as_array().is_some_and(|a| !a.is_empty()));
            let (nodes, extra) = (filled(nodes), filled(extra));
            if nodes.is_none() && extra.is_none() {
                return Err("select the nodes to attach it to, or give `points` or `extraNodes`".into());
            }
            for (key, v) in [("points", nodes), ("extraNodes", extra)] {
                if let Some(v) = v {
                    m.insert(key.into(), v);
                }
            }
        }
        Purpose::AddCap => {
            let given = list("points")?;
            let found = given.unwrap_or_else(|| Value::Array(points.iter().map(|&(a, b)| json!([a, b])).collect()));
            m.insert("points".into(), need(found, "select the nodes to attach it to, or give `points`")?);
        }
        Purpose::CornerGlyph => {
            let found = list("corners")?.unwrap_or_else(|| json!(corners_on_nodes(layer, &points)));
            m.insert("corners".into(), need(found, "select a node that holds a corner component, or give `corners`")?);
        }
        Purpose::CapGlyph => {
            let mut on: Vec<usize> = selected_kind(layer, selection, attach::is_cap);
            on.extend(caps_on_nodes(layer, &points).into_iter().map(|(i, _)| i));
            on.sort_unstable();
            on.dedup();
            let found = list("caps")?.unwrap_or_else(|| json!(on));
            m.insert("caps".into(), need(found, "select a cap component or one of its nodes, or give `caps`")?);
        }
        Purpose::AddSegment | Purpose::AddBrush => {
            let segments = match list("segments")? {
                Some(given) => given,
                None => {
                    let found = match (shown.as_ref(), selection) {
                        (Some(scene), Some(sel)) => selected_segments(scene, sel),
                        _ => Vec::new(),
                    };
                    Value::Array(found.iter().map(|&(a, b)| json!([a, b])).collect())
                }
            };
            let has_segments = segments.as_array().is_some_and(|a| !a.is_empty());
            if purpose == Purpose::AddSegment {
                m.insert("segments".into(), need(segments, "select the segments to attach it to, or give `segments`")?);
            } else if has_segments {
                m.insert("segments".into(), segments);
            } else {
                let given = list("points")?;
                let found = given.unwrap_or_else(|| Value::Array(points.iter().map(|&(a, b)| json!([a, b])).collect()));
                m.insert("points".into(), need(found, "select the segments or their end nodes to attach it to, or give `segments` or `points`")?);
            }
        }
        Purpose::SegmentGlyph | Purpose::BrushGlyph => {
            let brush = purpose == Purpose::BrushGlyph;
            let (key, what) = if brush { ("brushes", "a brush") } else { ("segmentComponents", "a segment component") };
            let mut on: Vec<usize> = selected_kind(layer, selection, |h| if brush { attach::is_brush(h) } else { attach::is_segment(h) });
            on.extend(segments_on_nodes(layer, &points, brush).into_iter().map(|(i, _)| i));
            on.sort_unstable();
            on.dedup();
            let found = list(key)?.unwrap_or_else(|| json!(on));
            m.insert(key.into(), need(found, &format!("select {what} or the node that ends its segment, or give `{key}`"))?);
        }
    }
    Ok(m)
}

/// The selected hints of `layer` that `is` accepts, in selection order.
fn selected_kind(
    layer: &fontcraft_engine::model::Layer,
    selection: Option<&fontcraft_tools::Selection>,
    is: impl Fn(&fontcraft_engine::model::Hint) -> bool,
) -> Vec<usize> {
    selection.map(|s| s.hints.iter().copied().filter(|&i| layer.info.hints.get(i).is_some_and(&is)).collect()).unwrap_or_default()
}

fn purpose_arg(p: &Value) -> Result<Option<Purpose>, String> {
    match string(p, "purpose")? {
        None => Ok(None),
        Some(name) => Purpose::ALL.iter().find(|(n, _)| *n == name).map(|(_, v)| Some(*v)).ok_or_else(|| {
            let names: Vec<&str> = Purpose::ALL.iter().map(|(n, _)| *n).collect();
            format!("`purpose` must be one of {}, got \"{}\"", names.join(", "), quoted(name))
        }),
    }
}

fn has_targets(p: &Value) -> bool {
    ["glyph", "layer", "component", "components", "points", "extraNodes", "segments", "corners", "caps", "segmentComponents", "brushes"]
        .iter()
        .any(|k| p.get(*k).is_some_and(|v| !v.is_null()))
}

fn run(app: &mut FontCraftApp, command: &CommandRef) -> Result<(), String> {
    if command.id != COMMAND {
        return Err(format!("no picker command `{}`", command.id));
    }
    let p = &command.params;
    if flag(p, "open")? == Some(false) {
        app.view.components.picker = None;
        return Ok(());
    }
    let given = purpose_arg(p)?;
    let current = app.view.components.picker.clone();
    // An open picker keeps its purpose and targets unless others are asked for. Everything is checked on a copy.
    let mut next = match current {
        Some(st) if given.is_none_or(|g| g == st.purpose) && !has_targets(p) => st,
        _ => {
            let purpose = given.unwrap_or_default();
            PickerState { purpose, targets: targets(app, p, purpose)?, ..PickerState::default() }
        }
    };
    if let Some(q) = string(p, "query")? {
        if q.chars().count() > MAX_QUERY_CHARS {
            return Err(format!("`query` holds at most {MAX_QUERY_CHARS} characters"));
        }
        next.query = q.to_string();
    }
    refresh(app, &mut next);
    if let Some(e) = &next.error {
        return Err(e.clone());
    }
    if let Some(name) = string(p, "select")? {
        if !next.results.iter().any(|r| r.name == name) {
            return Err(format!("`{}` is not among the results", quoted(name)));
        }
        next.selected = Some(name.to_string());
    }
    if flag(p, "pick")? == Some(true) {
        app.sent.push(next.pick()?);
        app.view.components.picker = None;
        return Ok(());
    }
    app.view.components.picker = Some(next);
    Ok(())
}

/// A command fired from a menu without the glyph to add opens the picker instead of running.
pub fn fired(app: &mut FontCraftApp, id: &str, params: &Value) -> bool {
    let purpose = match id {
        "glyph.addComponent" => Purpose::AddComponent,
        "glyph.addComponentAllMasters" => Purpose::AddComponentAllMasters,
        "cornerComponent.add" => Purpose::AddCorner,
        "capComponent.add" => Purpose::AddCap,
        "segmentComponent.add" => Purpose::AddSegment,
        "brush.add" => Purpose::AddBrush,
        _ => return false,
    };
    if params.get(purpose.key()).is_some_and(|v| !v.is_null()) {
        return false;
    }
    match targets(app, params, purpose) {
        Ok(targets) => {
            let mut st = PickerState { purpose, targets, ..PickerState::default() };
            refresh(app, &mut st);
            app.view.components.picker = Some(st);
        }
        Err(e) => app.status = e,
    }
    true
}

/// What the person did in the picker this pass.
#[derive(Clone, Debug, Default, PartialEq)]
struct Intent {
    query: Option<String>,
    highlight: Option<String>,
}

/// The picker on the window's dialog host: shows the app's state and hands back what was typed and highlighted.
struct GlyphPicker {
    id: DialogId,
    shown: PickerState,
    intent: Intent,
    open: bool,
}

impl StackDialog for GlyphPicker {
    type Result = ();

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<Self::Result>> {
        let words = Env::of(ui).words::<GlyphPickerWords>();
        let title = match self.shown.purpose {
            Purpose::AddComponent | Purpose::AddComponentAllMasters => words.component.clone(),
            Purpose::BaseGlyph => words.base.clone(),
            Purpose::AddCorner | Purpose::CornerGlyph => words.corner.clone(),
            Purpose::AddCap | Purpose::CapGlyph => words.cap.clone(),
            Purpose::AddSegment | Purpose::SegmentGlyph => words.segment.clone(),
            Purpose::AddBrush | Purpose::BrushGlyph => words.brush.clone(),
        };
        let GlyphPicker { shown, intent, open, .. } = self;
        let positions: Vec<usize> = (0..shown.results.len()).collect();
        let lead = shown.selected.as_ref().and_then(|s| shown.results.iter().position(|r| &r.name == s));
        let mut selection = lead.map_or_else(Selection::default, Selection::single);
        let out = FormDialog::new(title, Bind::bound(&mut selection)).opened(Bind::bound(open)).ok_label(words.choose.clone()).show(
            ui,
            |ui, _, selection| {
                let typed = TextField::search(words.search.clone(), Bind::given(shown.query.clone())).show(ui);
                if let Some(q) = typed.events.value().filter(|q| **q != shown.query) {
                    intent.query = Some(q.chars().take(MAX_QUERY_CHARS).collect());
                }
                if let Some(e) = &shown.error {
                    Text::new(e.clone()).show(ui);
                } else if shown.results.is_empty() {
                    Text::new(words.nothing.clone()).shade(Shade::Muted).show(ui);
                } else {
                    let columns = vec![
                        ColumnSpec::new(ColumnKey::new("name"), words.name.clone(), ColumnWidth::Content),
                        ColumnSpec::new(ColumnKey::new("unicode"), words.unicode.clone(), ColumnWidth::Content),
                    ];
                    let list = &shown.results;
                    let source = keyed(&positions, |at| *at).with_text(|at| Cow::Owned(list.get(*at).map_or_else(String::new, |r| r.name.clone())));
                    let _ = Table::<usize>::new(words.results.clone(), columns)
                        .select(SelectMode::Single)
                        .selection(Bind::bound(selection))
                        .max_rows(LIST_ROWS)
                        .show_items(ui, &source, &mut |at, icx| {
                            let Some(found) = list.get(*at) else { return ListRow::new("") };
                            ListRow::new(found.name.clone()).cells(icx.cells()).cell(ColumnKey::new("unicode"), found.unicode.clone())
                        });
                }
                !selection.items.is_empty()
            },
        );
        let picked = |s: &Selection<usize>| s.items.first().and_then(|at| shown.results.get(*at)).map(|r| r.name.clone());
        if let Some(name) = picked(&selection).filter(|n| Some(n) != shown.selected.as_ref()) {
            intent.highlight = Some(name);
        }
        match (out.accepted, out.closed) {
            (Some(accepted), _) => {
                intent.highlight = picked(&accepted).or_else(|| shown.selected.clone());
                Some(DialogResult::Ok(()))
            }
            (None, Some(_)) => Some(DialogResult::Cancel),
            (None, None) => None,
        }
    }
}

/// Opens the picker while the app asks for it, hands typing and highlighting to the app's state, and runs the pick.
pub fn run_dialog(host: &mut DialogHost, app: &mut FontCraftApp) {
    let Some(mut st) = app.view.components.picker.clone() else {
        // Closed by its command: the dialog closes on its next pass.
        if let Some(picker) = host.get_mut::<GlyphPicker>() {
            picker.open = false;
        }
        let _ = host.take::<GlyphPicker>();
        return;
    };
    if host.get_mut::<GlyphPicker>().is_none() {
        host.open(GlyphPicker { id: dialog_id("component.glyphPicker"), shown: st.clone(), intent: Intent::default(), open: true });
    }
    let mut answer = None;
    if let Some(picker) = host.get_mut::<GlyphPicker>() {
        let intent = std::mem::take(&mut picker.intent);
        if let Some(q) = intent.query {
            st.query = q;
        }
        refresh(app, &mut st);
        if let Some(h) = intent.highlight.filter(|h| st.results.iter().any(|r| &r.name == h)) {
            st.selected = Some(h);
        }
        picker.shown = st.clone();
    }
    match host.take::<GlyphPicker>() {
        Some(DialogResult::Ok(())) => answer = Some(true),
        Some(_) => answer = Some(false),
        None => {}
    }
    match answer {
        Some(true) => {
            match st.pick() {
                Ok(msg) => app.sent.push(msg),
                Err(e) => app.status = e,
            }
            app.view.components.picker = None;
        }
        Some(false) => app.view.components.picker = None,
        None => {
            if app.view.components.picker.as_ref() != Some(&st) {
                app.view.components.picker = Some(st);
                app.view_changed();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use craft::host::Host;
    use craft::ui::runtime::commands::CommandRef;
    use fontcraft_engine::Session;
    use serde_json::json;

    use crate::app::{APP, FontCraftApp, Msg};

    /// The picker searches as the query changes and a pick posts the purpose's command with the highlighted glyph.
    #[test]
    fn a_pick_adds_the_highlighted_component() {
        let mut app = FontCraftApp::with_edit_tab(Session::with_document(), Arc::new(Host::builder(APP).build()));
        for name in ["acutecomb", "Aacute"] {
            app.run_now(Msg::run("glyph.newGlyph", json!({ "name": name })), false).expect("a new glyph");
        }
        app.view.glyph = Some("Aacute".into());
        let open = CommandRef::with_params(super::COMMAND, json!({"purpose": "addComponent", "query": "acute"}));
        app.apply_msg(Msg::Shell(open)).expect("the picker opens");
        let st = app.view.components.picker.clone().expect("an open picker");
        assert!(st.results.iter().any(|r| r.name == "acutecomb"), "{:?}", st.results);
        let pick = CommandRef::with_params(super::COMMAND, json!({"select": "acutecomb", "pick": true}));
        app.apply_msg(Msg::Shell(pick)).expect("the pick");
        assert!(app.view.components.picker.is_none());
        let layer = st.targets.get("layer").cloned().unwrap_or_default();
        assert_eq!(app.sent, vec![Msg::run("glyph.addComponent", json!({"glyph": "Aacute", "layer": layer, "component": "acutecomb"}))]);
    }
}
