//! Font › Kerning › Auto Kern…, Kern to Distance…, Remove Pairs… and Vertical Kerning from Horizontal….
//!
//! - Auto Kern and Kern to Distance take their pairs from one place ([`Source`]): the rows selected in the Kerning
//!   window, the glyphs selected in Font View against each other or against a typed list (glyph names, `@group`, names
//!   of filters), the pair list of Pairs & Phrases, or every stored pair. Preview runs the command with `apply: false`
//!   and lists left, right, master, gap, the value now and the new one; Apply runs it again and leaves the unticked
//!   rows as they are (`skip`), in one undo step.
//! - Remove Pairs lists what `kerning.removePairsByRule` would delete, with the count; Apply deletes it.
//! - Vertical Kerning from Horizontal says how many vertical pairs it deletes before it runs.
//!
//! Apply does what the preview showed: it is off until a preview ran with the settings as they are, and refused when
//! the font changed since.

use std::borrow::Cow;
use std::collections::HashSet;

use craft::behavior::{ColumnKey, ColumnWidth, GesturePhase, keyed};
use craft::ui::choice::{Checkbox, Segmented};
use craft::ui::contract::{Bind, CollectionEvent, ColumnSpec, Entry, PlainData, Show as _, ShowCollection as _, ShowContainer as _, ToggleColumn};
use craft::ui::dialog::{DialogHost, DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use craft::ui::display::{Shade, Text};
use craft::ui::form::Field;
use craft::ui::layout_components::Row;
use craft::ui::list::{ListRow, ToggleCell, ToggleState};
use craft::ui::patterns::form_dialogs::FormDialog;
use craft::ui::pickers::Select;
use craft::ui::runtime::env::{Env, Label, WordSet as _};
use craft::ui::sections::{DialogRole, FooterButton};
use craft::ui::table::Table;
use craft::ui::text::{NumberField, TextField};
use egui::Ui;
use serde_json::{Map, Value, json};

use crate::app::FontCraftApp;
use crate::panels::kerning_window::format_value;
use crate::shell::kerning_window::{Batch, MasterChoice};

craft::ui::words! {
    /// The words of the kerning batch dialogs.
    pub struct KerningBatchWords {
        auto_title: "fontcraftKerningBatch.autoTitle" = "Auto Kern";
        distance_title: "fontcraftKerningBatch.distanceTitle" = "Kern to Distance";
        remove_title: "fontcraftKerningBatch.removeTitle" = "Remove Pairs";
        vertical_title: "fontcraftKerningBatch.verticalTitle" = "Vertical Kerning from Horizontal";
        vertical_message: "fontcraftKerningBatch.verticalMessage" = "Replaces the vertical kerning with the kerning of the glyphs' horizontal forms. Vertical pairs deleted:";
        pairs_from: "fontcraftKerningBatch.pairsFrom" = "Pairs";
        from_window: "fontcraftKerningBatch.fromWindow" = "Rows Selected in the Kerning Window";
        from_font_view: "fontcraftKerningBatch.fromFontView" = "Selected Glyphs with Each Other";
        from_typed: "fontcraftKerningBatch.fromTyped" = "Selected Glyphs with a List";
        from_list: "fontcraftKerningBatch.fromList" = "Pairs & Phrases List";
        from_all: "fontcraftKerningBatch.fromAll" = "Every Stored Pair";
        typed: "fontcraftKerningBatch.typed" = "List";
        typed_hint: "fontcraftKerningBatch.typedHint" = "Glyph names, @groups or a filter";
        order: "fontcraftKerningBatch.order" = "Selected Glyphs";
        first: "fontcraftKerningBatch.first" = "First";
        second: "fontcraftKerningBatch.second" = "Second";
        both: "fontcraftKerningBatch.both" = "Both";
        direction: "fontcraftKerningBatch.direction" = "Direction";
        ltr: "fontcraftKerningBatch.ltr" = "Left to Right";
        rtl: "fontcraftKerningBatch.rtl" = "Right to Left";
        vertical: "fontcraftKerningBatch.vertical" = "Top to Bottom";
        masters: "fontcraftKerningBatch.masters" = "Masters";
        every_master: "fontcraftKerningBatch.everyMaster" = "Every Master";
        separation: "fontcraftKerningBatch.separation" = "Target Gap";
        minimum: "fontcraftKerningBatch.minimum" = "Smallest Value";
        round: "fontcraftKerningBatch.round" = "Round To";
        only_closer: "fontcraftKerningBatch.onlyCloser" = "Only Move Closer";
        touching: "fontcraftKerningBatch.touching" = "Measure the Closest Points";
        keep_existing: "fontcraftKerningBatch.keepExisting" = "Keep Stored Pairs";
        distance: "fontcraftKerningBatch.distance" = "Distance";
        smaller_than: "fontcraftKerningBatch.smallerThan" = "Smaller Than";
        exceptions_within: "fontcraftKerningBatch.exceptionsWithin" = "Exceptions Within";
        only_selected: "fontcraftKerningBatch.onlySelected" = "Only Pairs of the Selected Glyphs";
        preview: "fontcraftKerningBatch.preview" = "Preview";
        apply: "fontcraftKerningBatch.apply" = "Apply";
        close: "fontcraftKerningBatch.close" = "Close";
        results: "fontcraftKerningBatch.results" = "Kerning pairs";
        use_row: "fontcraftKerningBatch.useRow" = "Apply";
        left: "fontcraftKerningBatch.left" = "Left";
        right: "fontcraftKerningBatch.right" = "Right";
        master: "fontcraftKerningBatch.master" = "Master";
        gap: "fontcraftKerningBatch.gap" = "Gap";
        now: "fontcraftKerningBatch.now" = "Now";
        new_value: "fontcraftKerningBatch.new" = "New";
        class_value: "fontcraftKerningBatch.classValue" = "Class Value";
        pairs_change: "fontcraftKerningBatch.pairsChange" = "pairs change";
        pairs_go: "fontcraftKerningBatch.pairsGo" = "pairs are removed";
        skipped: "fontcraftKerningBatch.skipped" = "skipped";
        listed: "fontcraftKerningBatch.listed" = "listed";
        done: "fontcraftKerningBatch.done" = "Applied.";
        stale: "fontcraftKerningBatch.stale" = "The font changed since the preview; preview again.";
    }
}

/// Most pairs the dialogs hand to a command.
const MAX_PAIRS: usize = 200_000;
/// Most rows a dialog lists; the rest are applied with the ticked ones.
const MAX_ROWS: usize = 20_000;
/// Most characters of the typed list.
const MAX_TYPED: usize = 10_000;
/// The picked index of the first extra footer button (`FormOutput::picked`).
const FIRST_EXTRA: usize = 2;
/// The picked index of OK, which a held OK reports instead of `accepted`.
const OK: usize = 0;
/// Rows the list shows before it scrolls: the dialog body around it scrolls too.
const LIST_ROWS: u16 = 12;

const USE: ColumnKey = ColumnKey::new("use");
const LEFT: ColumnKey = ColumnKey::new("left");
const RIGHT: ColumnKey = ColumnKey::new("right");
const MASTER: ColumnKey = ColumnKey::new("master");
const GAP: ColumnKey = ColumnKey::new("gap");
const NOW: ColumnKey = ColumnKey::new("now");
const NEW: ColumnKey = ColumnKey::new("new");

/// Where the pairs come from.
#[derive(PlainData, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Source {
    /// The rows selected in the Kerning window.
    #[default]
    Window,
    /// The glyphs selected in Font View, each with each.
    FontView,
    /// The glyphs selected in Font View with a typed list.
    Typed,
    /// The pair list of Pairs & Phrases: its selected rows, else all.
    List,
    /// Every stored pair of the masters.
    All,
}

/// Where the selected glyphs stand in the pairs with a typed list.
#[derive(PlainData, Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Order {
    #[default]
    First,
    Second,
    Both,
}

/// The fields of the dialogs.
#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub source: Source,
    pub typed: String,
    pub order: Order,
    /// "ltr", "rtl" or, to remove pairs, "vertical".
    pub direction: String,
    /// A master id; empty: every master.
    pub master: String,
    pub separation: Option<f64>,
    pub minimum: Option<f64>,
    pub round: Option<f64>,
    pub only_closer: bool,
    pub touching: bool,
    pub keep_existing: bool,
    pub distance: Option<f64>,
    pub smaller_than: Option<f64>,
    pub exceptions_within: Option<f64>,
    /// Remove Pairs: only pairs of the glyphs selected in Font View.
    pub only_selected: bool,
}

impl Default for Settings {
    /// The commands' defaults.
    fn default() -> Self {
        Settings {
            source: Source::Window,
            typed: String::new(),
            order: Order::First,
            direction: "ltr".into(),
            master: String::new(),
            separation: None,
            minimum: Some(0.0),
            round: Some(5.0),
            only_closer: false,
            touching: false,
            keep_existing: true,
            distance: None,
            smaller_than: None,
            exceptions_within: None,
            only_selected: false,
        }
    }
}

/// One pair a command would change.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Change {
    pub left: String,
    pub right: String,
    /// A master id.
    pub master: String,
    pub gap: Option<f64>,
    pub now: Option<f64>,
    /// `None`: the pair is removed.
    pub new: Option<f64>,
    /// Remove Pairs: what applies once an exception is gone.
    pub class_value: Option<f64>,
}

/// What a preview found, and what it was made with.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Report {
    settings: Settings,
    /// The document and its revision.
    document: (u64, u64),
    /// The command's parameters but `apply` and `skip`.
    params: Map<String, Value>,
    pub rows: Vec<Change>,
    /// The pairs that change, listed or not.
    pub count: usize,
    pub skipped: usize,
    /// Rows left as they are.
    pub unticked: HashSet<usize>,
}

/// What a button asked for, for the module to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ask {
    Preview,
    Apply,
}

/// One of the batch dialogs.
pub struct BatchDialog {
    id: DialogId,
    pub kind: Batch,
    pub settings: Settings,
    pub choices: Vec<MasterChoice>,
    /// Vertical Kerning: the vertical pairs each master has.
    pub vertical_pairs: Vec<(String, usize)>,
    pub report: Option<Report>,
    /// What the last button did, or why it did not.
    pub note: String,
    pub request: Option<Ask>,
    open: bool,
}

/// The engine command a dialog runs.
pub fn command(kind: Batch) -> &'static str {
    match kind {
        Batch::AutoKern => "kerning.autoKern",
        Batch::Distance => "kerning.kernToDistance",
        Batch::Remove => "kerning.removePairsByRule",
        Batch::Vertical => "kerning.verticalFromHorizontal",
    }
}

fn selected_glyphs(app: &FontCraftApp) -> Result<Vec<String>, String> {
    let selected = app.view.font_view.selected.clone();
    if selected.is_empty() { Err("select glyphs in Font View first".into()) } else { Ok(selected) }
}

/// The sides a typed list names: the glyphs of a filter it names, else each word as a glyph, an `@group` or a filter.
fn typed_sides(app: &mut FontCraftApp, text: &str) -> Result<Vec<String>, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("type glyph names, @groups or the name of a filter".into());
    }
    if text.chars().nth(MAX_TYPED).is_some() {
        return Err(format!("the list holds at most {MAX_TYPED} characters"));
    }
    let filter = |app: &mut FontCraftApp, name: &str| -> Option<Vec<String>> {
        let found = app.session.query("fontView.queryGlyphs", &json!({"entries": [["filters", name]]})).ok()?;
        Some(found.get("glyphs")?.as_array()?.iter().filter_map(Value::as_str).map(str::to_string).collect())
    };
    let is_side = |app: &FontCraftApp, word: &str| word.starts_with('@') || app.session.font().is_ok_and(|f| f.glyph(word).is_some());
    let mut out: Vec<String> = Vec::new();
    if !is_side(app, text)
        && let Some(glyphs) = filter(app, text)
    {
        out = glyphs;
    } else {
        for word in text.split(|c: char| c.is_whitespace() || c == ',').filter(|w| !w.is_empty()) {
            if is_side(app, word) {
                out.push(word.to_string());
            } else {
                let glyphs = filter(app, word);
                out.extend(glyphs.ok_or_else(|| format!("no glyph, group or filter is named `{word}`"))?);
            }
        }
    }
    let mut seen = HashSet::new();
    out.retain(|side| seen.insert(side.clone()));
    if out.is_empty() { Err("the filter holds no glyph".into()) } else { Ok(out) }
}

fn product(firsts: &[String], seconds: &[String], out: &mut Vec<(String, String)>) -> Result<(), String> {
    if out.len().saturating_add(firsts.len().saturating_mul(seconds.len())) > MAX_PAIRS {
        return Err(format!("the selection gives more than {MAX_PAIRS} pairs"));
    }
    out.extend(firsts.iter().flat_map(|l| seconds.iter().map(move |r| (l.clone(), r.clone()))));
    Ok(())
}

/// The pairs `settings` name; `None` when the command takes every stored pair by itself.
pub fn pairs(app: &mut FontCraftApp, kind: Batch, settings: &Settings) -> Result<Option<Vec<(String, String)>>, String> {
    let mut out: Vec<(String, String)> = Vec::new();
    match settings.source {
        Source::Window => {
            let model = &app.models.kerning_window;
            if model.direction != settings.direction {
                return Err("the Kerning window lists another writing direction".into());
            }
            out.extend(model.selected.iter().filter_map(|&i| model.rows.get(i)).map(|r| (r.left.clone(), r.right.clone())));
            if out.is_empty() {
                return Err("select rows in the Kerning window first".into());
            }
        }
        Source::FontView => {
            let selected = selected_glyphs(app)?;
            product(&selected, &selected, &mut out)?;
        }
        Source::Typed => {
            let selected = selected_glyphs(app)?;
            let typed = typed_sides(app, &settings.typed)?;
            if settings.order != Order::Second {
                product(&selected, &typed, &mut out)?;
            }
            if settings.order != Order::First {
                product(&typed, &selected, &mut out)?;
            }
        }
        Source::List => out = crate::shell::kerning_review::listed_pairs(app)?,
        Source::All if kind == Batch::Distance => return Ok(None),
        Source::All => {
            let font = app.session.font().map_err(|e| e.to_string())?;
            let masters: Vec<String> =
                font.masters.iter().map(|m| m.id.clone()).filter(|id| settings.master.is_empty() || *id == settings.master).collect();
            let mut seen = HashSet::new();
            for master in masters {
                let params = json!({"master": master, "direction": settings.direction});
                let listed = app.session.query("kerning.listPairs", &params).map_err(|e| e.to_string())?;
                for row in listed.get("pairs").and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default() {
                    let side = |key: &str| row.get(key).and_then(Value::as_str).map(str::to_string);
                    if let (Some(l), Some(r)) = (side("left"), side("right"))
                        && seen.insert((l.clone(), r.clone()))
                    {
                        out.push((l, r));
                    }
                }
            }
            if out.is_empty() {
                return Err("the masters store no pair in this direction".into());
            }
        }
    }
    let mut seen = HashSet::new();
    out.retain(|pair| seen.insert(pair.clone()));
    if out.len() > MAX_PAIRS {
        return Err(format!("the pairs are more than {MAX_PAIRS}"));
    }
    Ok(Some(out))
}

/// The command's parameters for `settings`, without `apply`.
pub fn params(app: &mut FontCraftApp, kind: Batch, settings: &Settings) -> Result<Map<String, Value>, String> {
    let mut p = Map::new();
    if !settings.master.is_empty() {
        p.insert("masters".into(), json!([settings.master]));
    }
    if kind == Batch::Vertical {
        return Ok(p);
    }
    p.insert("direction".into(), json!(settings.direction));
    if matches!(kind, Batch::AutoKern | Batch::Distance)
        && let Some(pairs) = pairs(app, kind, settings)?
    {
        p.insert("pairs".into(), json!(pairs));
    }
    let mut number = |key: &str, value: Option<f64>| {
        if let Some(v) = value.filter(|v| v.is_finite()) {
            p.insert(key.into(), json!(v));
        }
    };
    match kind {
        Batch::AutoKern => {
            number("separation", settings.separation);
            number("minKern", settings.minimum);
            number("round", settings.round);
            p.insert("onlyCloser".into(), json!(settings.only_closer));
            p.insert("touching".into(), json!(settings.touching));
            p.insert("keepExisting".into(), json!(settings.keep_existing));
        }
        Batch::Distance => number("distance", Some(settings.distance.ok_or("type the distance")?)),
        Batch::Remove => {
            number("smallerThan", settings.smaller_than);
            number("exceptionsWithin", settings.exceptions_within);
            if settings.only_selected {
                p.insert("glyphs".into(), json!(selected_glyphs(app)?));
            }
        }
        Batch::Vertical => {}
    }
    Ok(p)
}

/// The rows a reply lists that change something, and how many of its pairs were skipped.
fn changes(kind: Batch, reply: &Value) -> (Vec<Change>, usize) {
    let key = if kind == Batch::Remove { "removed" } else { "pairs" };
    let listed = reply.get(key).and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default();
    let rows = listed
        .iter()
        .filter_map(|row| {
            let text = |k: &str| row.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
            let number = |k: &str| row.get(k).and_then(Value::as_f64).filter(|v| v.is_finite());
            let (value, previous) = (number("value"), number("previous"));
            let (now, new) = match kind {
                Batch::Remove => (value, None),
                Batch::AutoKern => match text("result").as_str() {
                    "set" => (previous, value),
                    "removed" => (previous, None),
                    _ => return None,
                },
                _ if row.get("changed").and_then(Value::as_bool) != Some(true) => return None,
                _ => (previous, value),
            };
            Some(Change {
                left: text("left"),
                right: text("right"),
                master: text("master"),
                gap: number("gap"),
                now,
                new,
                class_value: number("classValue"),
            })
        })
        .collect();
    (rows, reply.get("skipped").and_then(Value::as_array).map_or(0, Vec::len))
}

impl BatchDialog {
    pub fn new(kind: Batch, app: &mut FontCraftApp) -> BatchDialog {
        let choices: Vec<MasterChoice> = app
            .session
            .font()
            .map(|f| f.masters.iter().map(|m| MasterChoice { id: m.id.clone(), name: m.name.clone() }).collect())
            .unwrap_or_default();
        let window = &app.models.kerning_window;
        let rows_selected = !window.selected.is_empty();
        let direction = match window.direction.as_str() {
            "rtl" => "rtl",
            "vertical" if kind == Batch::Remove => "vertical",
            _ => "ltr",
        };
        let source = if rows_selected {
            Source::Window
        } else if app.view.font_view.selected.len() > 1 {
            Source::FontView
        } else {
            Source::All
        };
        let shown = app.session.font().ok().and_then(|f| crate::shell::masters::current_master(f, &app.view.layers)).map(|m| m.id.clone());
        let vertical_pairs = choices
            .iter()
            .filter(|_| kind == Batch::Vertical)
            .map(|m| {
                let listed = app.session.query("kerning.listPairs", &json!({"master": m.id, "direction": "vertical"})).ok();
                let total = listed.as_ref().and_then(|v| v.get("total")).and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok());
                (m.id.clone(), total.unwrap_or(0))
            })
            .collect();
        let settings = Settings {
            source,
            direction: direction.into(),
            // Vertical kerning and the rules default to every master, as their commands do.
            master: if matches!(kind, Batch::AutoKern | Batch::Distance) { shown.unwrap_or_default() } else { String::new() },
            ..Settings::default()
        };
        BatchDialog {
            id: dialog_id("kerning.batch"),
            kind,
            settings,
            choices,
            vertical_pairs,
            report: None,
            note: String::new(),
            request: None,
            open: true,
        }
    }

    /// The preview that Apply would store, when it was made with the settings as they are.
    fn current(&self) -> Option<&Report> {
        self.report.as_ref().filter(|r| r.settings == self.settings)
    }

    /// Whether Apply has something to do.
    pub fn can_apply(&self) -> bool {
        self.kind == Batch::Vertical || self.current().is_some_and(|r| r.count > r.unticked.len())
    }

    /// Runs the command without storing anything and lists what it would change.
    pub fn preview(&mut self, app: &mut FontCraftApp) {
        self.report = None;
        self.note = match self.ask(app) {
            Ok(report) => {
                self.report = Some(report);
                String::new()
            }
            Err(e) => e,
        };
    }

    fn ask(&self, app: &mut FontCraftApp) -> Result<Report, String> {
        let params = params(app, self.kind, &self.settings)?;
        let document = app.session.active().map(|d| (d.id, d.revision)).ok_or("no font is open")?;
        let mut asked = params.clone();
        asked.insert("apply".into(), json!(false));
        let reply = app.session.execute(command(self.kind), &Value::Object(asked)).map_err(|e| e.to_string())?;
        let (mut rows, skipped) = changes(self.kind, &reply);
        let count = rows.len();
        rows.truncate(MAX_ROWS);
        Ok(Report { settings: self.settings.clone(), document, params, rows, count, skipped, unticked: HashSet::new() })
    }

    /// Runs the command as previewed, the unticked rows left as they are.
    pub fn apply(&mut self, app: &mut FontCraftApp) {
        let words = KerningBatchWords::english();
        let params = if self.kind == Batch::Vertical {
            params(app, self.kind, &self.settings)
        } else {
            match self.current() {
                None => return,
                Some(r) if app.session.active().map(|d| (d.id, d.revision)) != Some(r.document) => Err(words.stale.as_str().to_string()),
                Some(r) => {
                    let mut params = r.params.clone();
                    let skip: Vec<Value> = r
                        .unticked
                        .iter()
                        .filter_map(|&i| r.rows.get(i))
                        .map(|c| json!({"left": c.left, "right": c.right, "master": c.master}))
                        .collect();
                    if !skip.is_empty() {
                        params.insert("skip".into(), Value::Array(skip));
                    }
                    Ok(params)
                }
            }
        };
        self.report = None;
        self.note = match params.and_then(|p| app.run_command(command(self.kind), Value::Object(p))) {
            Ok(_) => words.done.as_str().to_string(),
            Err(e) => e,
        };
    }

    fn summary(&self, words: &KerningBatchWords) -> String {
        let Some(r) = self.current() else { return self.note.clone() };
        let what = if self.kind == Batch::Remove { &words.pairs_go } else { &words.pairs_change };
        let mut line = format!("{} {}", r.count.saturating_sub(r.unticked.len()), what.as_str());
        if r.skipped > 0 {
            line.push_str(&format!(" · {} {}", r.skipped, words.skipped.as_str()));
        }
        if r.count > r.rows.len() {
            line.push_str(&format!(" · {} {}", r.rows.len(), words.listed.as_str()));
        }
        line
    }
}

fn number(ui: &mut Ui, label: &Label, value: &mut Option<f64>) {
    let _ = Field::new(label.clone()).show(ui, |ui, _| {
        NumberField::new(label.clone(), Bind::bound(value)).show(ui);
    });
}

fn check(ui: &mut Ui, label: &Label, on: &mut bool) {
    if let Some(v) = Checkbox::new(Bind::given(*on)).label(label.clone()).show(ui).events.committed() {
        *on = *v;
    }
}

fn pairs_part(ui: &mut Ui, words: &KerningBatchWords, s: &mut Settings) {
    let sources = [
        Entry::new(Source::Window, words.from_window.clone()),
        Entry::new(Source::FontView, words.from_font_view.clone()),
        Entry::new(Source::Typed, words.from_typed.clone()),
        Entry::new(Source::List, words.from_list.clone()),
        Entry::new(Source::All, words.from_all.clone()),
    ];
    let _ = Field::new(words.pairs_from.clone()).show(ui, |ui, _| {
        Select::new(words.pairs_from.clone(), Bind::bound(&mut s.source), &sources).show(ui);
    });
    if s.source != Source::Typed {
        return;
    }
    let _ = Field::new(words.typed.clone()).show(ui, |ui, _| {
        TextField::new(words.typed.clone(), Bind::bound(&mut s.typed)).placeholder(words.typed_hint.clone()).limit(MAX_TYPED).show(ui);
    });
    let orders =
        [Entry::new(Order::First, words.first.clone()), Entry::new(Order::Second, words.second.clone()), Entry::new(Order::Both, words.both.clone())];
    let _ = Field::new(words.order.clone()).show(ui, |ui, _| {
        Segmented::new(words.order.clone(), Bind::bound(&mut s.order), &orders).show(ui);
    });
}

fn settings_part(ui: &mut Ui, words: &KerningBatchWords, kind: Batch, s: &mut Settings, choices: &[MasterChoice]) {
    if matches!(kind, Batch::AutoKern | Batch::Distance) {
        pairs_part(ui, words, s);
    }
    let _ = Row::new().fill_width().show(ui, |ui, _| {
        if kind != Batch::Vertical {
            let mut directions = vec![Entry::new("ltr".to_string(), words.ltr.clone()), Entry::new("rtl".to_string(), words.rtl.clone())];
            if kind == Batch::Remove {
                directions.push(Entry::new("vertical".to_string(), words.vertical.clone()));
            }
            Segmented::new(words.direction.clone(), Bind::bound(&mut s.direction), &directions).show(ui);
        }
        let masters: Vec<Entry<String>> = std::iter::once(Entry::new(String::new(), words.every_master.clone()))
            .chain(choices.iter().map(|m| Entry::new(m.id.clone(), m.name.clone())))
            .collect();
        let _ = Field::new(words.masters.clone()).show(ui, |ui, _| {
            Select::new(words.masters.clone(), Bind::bound(&mut s.master), &masters).show(ui);
        });
    });
    match kind {
        Batch::AutoKern => {
            let _ = Row::new().fill_width().show(ui, |ui, _| {
                number(ui, &words.separation, &mut s.separation);
                number(ui, &words.minimum, &mut s.minimum);
                number(ui, &words.round, &mut s.round);
            });
            let _ = Row::new().fill_width().show(ui, |ui, _| {
                check(ui, &words.only_closer, &mut s.only_closer);
                check(ui, &words.touching, &mut s.touching);
                check(ui, &words.keep_existing, &mut s.keep_existing);
            });
        }
        Batch::Distance => number(ui, &words.distance, &mut s.distance),
        Batch::Remove => {
            let _ = Row::new().fill_width().show(ui, |ui, _| {
                number(ui, &words.smaller_than, &mut s.smaller_than);
                number(ui, &words.exceptions_within, &mut s.exceptions_within);
            });
            check(ui, &words.only_selected, &mut s.only_selected);
        }
        Batch::Vertical => {}
    }
}

/// The rows of a preview; a tick flipped is recorded in `report`.
fn results(ui: &mut Ui, words: &KerningBatchWords, kind: Batch, report: &mut Report, choices: &[MasterChoice]) {
    let ticks = kind != Batch::Remove;
    let mut columns = Vec::new();
    if ticks {
        columns.push(ColumnSpec::toggle(USE, words.use_row.clone(), ToggleColumn::sweepable()));
    }
    columns.extend([
        ColumnSpec::new(LEFT, words.left.clone(), ColumnWidth::Content),
        ColumnSpec::new(RIGHT, words.right.clone(), ColumnWidth::Content),
        ColumnSpec::new(MASTER, words.master.clone(), ColumnWidth::Content),
        ColumnSpec::new(GAP, if ticks { words.gap.clone() } else { words.class_value.clone() }, ColumnWidth::Content).numeric(),
        ColumnSpec::new(NOW, words.now.clone(), ColumnWidth::Content).numeric(),
        ColumnSpec::new(NEW, words.new_value.clone(), ColumnWidth::Content).numeric(),
    ]);
    let (rows, unticked) = (&report.rows, &report.unticked);
    let positions: Vec<usize> = (0..rows.len()).collect();
    let source = keyed(&positions, |at| *at).with_text(|at| Cow::Owned(rows.get(*at).map_or_else(String::new, |r| r.left.clone())));
    let name = |id: &str| choices.iter().find(|m| m.id == id).map_or_else(|| id.to_string(), |m| m.name.clone());
    let shown = |v: Option<f64>| v.map(format_value).unwrap_or_default();
    let out = Table::<usize>::new(words.results.clone(), columns).max_rows(LIST_ROWS).show_items(ui, &source, &mut |at, icx| {
        let Some(r) = rows.get(*at) else { return ListRow::new("") };
        let row = ListRow::new(r.left.clone())
            .cells(icx.cells())
            .state(icx.row.clone())
            .cell(RIGHT, r.right.clone())
            .cell(MASTER, name(&r.master))
            .cell(GAP, shown(if ticks { r.gap } else { r.class_value }))
            .cell(NOW, shown(r.now))
            .cell(NEW, shown(r.new));
        if ticks { row.toggle(ToggleCell::check(USE, ToggleState::of(!unticked.contains(at)), words.use_row.clone())) } else { row }
    });
    let mut flips = Vec::new();
    for event in &out.events {
        if let CollectionEvent::Toggle(edit) = event
            && matches!(edit.phase(), Some(GesturePhase::Commit) | None)
            && let Some(cell) = edit.value()
            && cell.column == USE
        {
            flips.push((cell.key, cell.on));
        }
    }
    for (at, on) in flips {
        if on {
            report.unticked.remove(&at);
        } else if at < report.rows.len() {
            report.unticked.insert(at);
        }
    }
}

impl StackDialog for BatchDialog {
    type Result = ();

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<()>> {
        let words = Env::of(ui).words::<KerningBatchWords>();
        let title = match self.kind {
            Batch::AutoKern => &words.auto_title,
            Batch::Distance => &words.distance_title,
            Batch::Remove => &words.remove_title,
            Batch::Vertical => &words.vertical_title,
        };
        let summary = self.summary(&words);
        let can_apply = self.can_apply();
        let kind = self.kind;
        let extra: Vec<FooterButton> =
            if kind == Batch::Vertical { Vec::new() } else { vec![FooterButton::new(words.preview.clone(), DialogRole::Other)] };
        let BatchDialog { settings, choices, vertical_pairs, report, open, .. } = self;
        let mut shown_value = false;
        let shown = FormDialog::new(title.clone(), Bind::bound(&mut shown_value))
            .opened(Bind::bound(open))
            .ok_label(words.apply.clone())
            .cancel_label(words.close.clone())
            .ok_enabled(can_apply)
            .accept_stays_open(true)
            .extra_buttons(extra)
            .show(ui, |ui, _, _| {
                settings_part(ui, &words, kind, settings, choices);
                if kind == Batch::Vertical {
                    let wanted = vertical_pairs.iter().filter(|(id, _)| settings.master.is_empty() || *id == settings.master);
                    let total: usize = wanted.map(|(_, n)| *n).sum();
                    Text::new(format!("{} {total}", words.vertical_message.as_str())).show(ui);
                }
                Text::new(summary.clone()).shade(Shade::Muted).show(ui);
                if let Some(report) = report.as_mut().filter(|r| r.settings == *settings && !r.rows.is_empty()) {
                    results(ui, &words, kind, report, choices);
                }
                true
            });
        if shown.picked == Some(FIRST_EXTRA) {
            self.request = Some(Ask::Preview);
        } else if (shown.picked == Some(OK) || shown.accepted.is_some()) && can_apply {
            self.request = Some(Ask::Apply);
        }
        shown.closed.filter(|_| shown.accepted.is_none()).map(|_| DialogResult::Cancel)
    }
}

/// Opens the dialog a shell command asked for and does what its buttons ask.
pub fn run(host: &mut DialogHost, app: &mut FontCraftApp) {
    let _ = host.take::<BatchDialog>();
    if let Some(kind) = app.view.kerning.batch.take()
        && host.get_mut::<BatchDialog>().is_none()
    {
        host.open(BatchDialog::new(kind, app));
    }
    let Some(dialog) = host.get_mut::<BatchDialog>() else { return };
    match dialog.request.take() {
        Some(Ask::Preview) => dialog.preview(app),
        Some(Ask::Apply) => dialog.apply(app),
        None => {}
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing, clippy::panic)]
mod tests {
    use std::sync::Arc;

    use craft::host::Host;
    use craft::ui::runtime::commands::CommandRef;
    use fontcraft_engine::Session;

    use super::*;
    use crate::app::{APP, Msg};

    /// T, o and c are boxes 500 wide with sidebearings of 50; o and c are in the left group o; T–@o −60, A–V −100.
    fn app() -> FontCraftApp {
        let mut s = Session::with_document();
        let mut run = |id: &str, p: Value| s.execute(id, &p).map(|_| ()).unwrap_or_else(|e| panic!("{id}: {e}"));
        for glyph in ["T", "o", "c"] {
            run("path.addRectangle", json!({"glyph": glyph, "x": 50, "y": 0, "width": 500, "height": 500}));
            run("spacing.setWidth", json!({"glyphs": [glyph], "value": 600}));
        }
        run("kerning.setLeftGroup", json!({"glyphs": ["o", "c"], "group": "o"}));
        run("kerning.setPair", json!({"left": "T", "right": "@o", "value": -60}));
        run("kerning.setPair", json!({"left": "A", "right": "V", "value": -100}));
        FontCraftApp::with_edit_tab(s, Arc::new(Host::builder(APP).build()))
    }

    fn stored(app: &mut FontCraftApp) -> Vec<(String, String, f64)> {
        let listed = app.session.query("kerning.listPairs", &json!({})).unwrap();
        let text = |v: &Value| v.as_str().unwrap().to_string();
        listed["pairs"].as_array().unwrap().iter().map(|p| (text(&p["left"]), text(&p["right"]), p["value"].as_f64().unwrap())).collect()
    }

    fn shell(app: &mut FontCraftApp, id: &str, params: Value) {
        app.sent.push(Msg::Shell(CommandRef::with_params(id, params)));
        app.drain();
        assert!(app.status.is_empty(), "{}", app.status);
    }

    fn named(pairs: &[(&str, &str)]) -> Option<Vec<(String, String)>> {
        Some(pairs.iter().map(|(l, r)| (l.to_string(), r.to_string())).collect())
    }

    /// Each source gives the pairs it names.
    #[test]
    fn each_source_gives_its_pairs() {
        let mut app = app();
        shell(&mut app, "window.kerning", json!({"select": [{"left": "T", "right": "@o"}]}));
        let mut s = Settings::default();
        assert_eq!(pairs(&mut app, Batch::AutoKern, &s).unwrap(), named(&[("T", "@o")]));
        s.source = Source::All;
        assert_eq!(pairs(&mut app, Batch::AutoKern, &s).unwrap(), named(&[("T", "@o"), ("A", "V")]));
        assert_eq!(pairs(&mut app, Batch::Distance, &s).unwrap(), None, "the command takes every stored pair");

        app.view.font_view.selected = vec!["T".into(), "o".into()];
        s.source = Source::FontView;
        assert_eq!(pairs(&mut app, Batch::AutoKern, &s).unwrap(), named(&[("T", "T"), ("T", "o"), ("o", "T"), ("o", "o")]));
        s.source = Source::Typed;
        s.typed = "A, @o".into();
        assert_eq!(pairs(&mut app, Batch::AutoKern, &s).unwrap(), named(&[("T", "A"), ("T", "@o"), ("o", "A"), ("o", "@o")]));
        s.order = Order::Second;
        assert_eq!(pairs(&mut app, Batch::AutoKern, &s).unwrap(), named(&[("A", "T"), ("A", "o"), ("@o", "T"), ("@o", "o")]));
        app.session.execute("fontView.addListFilter", &json!({"name": "Round Ones", "glyphs": ["c"]})).unwrap();
        s.typed = "Round Ones".into();
        s.order = Order::First;
        assert_eq!(pairs(&mut app, Batch::AutoKern, &s).unwrap(), named(&[("T", "c"), ("o", "c")]));
        s.typed = "nothing".into();
        assert!(pairs(&mut app, Batch::AutoKern, &s).unwrap_err().contains("nothing"));

        shell(&mut app, "kerning.loadPairList", json!({"text": "o c\nc T\n", "list": "pairs"}));
        s.source = Source::List;
        assert_eq!(pairs(&mut app, Batch::AutoKern, &s).unwrap(), named(&[("o", "c"), ("c", "T")]));
    }

    /// Kern to Distance previews without storing; Apply stores the ticked rows in one undo step and leaves the unticked
    /// one; a preview of a font that changed since is not applied.
    #[test]
    fn apply_stores_the_ticked_rows_of_the_preview() {
        let mut app = app();
        app.view.font_view.selected = vec!["T".into(), "o".into()];
        let before = stored(&mut app);
        let mut dialog = BatchDialog::new(Batch::Distance, &mut app);
        dialog.settings.source = Source::FontView;
        dialog.settings.distance = Some(80.0);
        assert!(!dialog.can_apply(), "nothing was previewed");
        dialog.preview(&mut app);
        assert_eq!(dialog.note, "");
        let report = dialog.report.as_mut().unwrap();
        // The boxes stand 100 apart, so every pair gets -20.
        assert_eq!((report.count, report.rows.iter().all(|r| r.new == Some(-20.0))), (4, true), "{report:?}");
        let at = report.rows.iter().position(|r| r.left == "o" && r.right == "o").unwrap();
        report.unticked.insert(at);
        assert_eq!(stored(&mut app), before, "a preview stores nothing");
        dialog.apply(&mut app);
        let after = stored(&mut app);
        let value = |l: &str, r: &str| after.iter().find(|p| p.0 == l && p.1 == r).map(|p| p.2);
        assert_eq!((value("T", "T"), value("T", "o"), value("o", "T"), value("o", "o")), (Some(-20.0), Some(-20.0), Some(-20.0), None));
        app.session.execute("edit.undo", &json!({})).unwrap();
        assert_eq!(stored(&mut app), before, "one undo step");

        dialog.preview(&mut app);
        app.session.execute("kerning.setPair", &json!({"left": "A", "right": "V", "value": 1})).unwrap();
        let changed = stored(&mut app);
        dialog.apply(&mut app);
        assert!(dialog.note.contains("preview again"), "{}", dialog.note);
        assert_eq!(stored(&mut app), changed);
        dialog.preview(&mut app);
        dialog.settings.distance = Some(10.0);
        assert!(!dialog.can_apply(), "the settings changed since the preview");
    }

    /// Remove Pairs shows the count before it deletes; Vertical Kerning counts the pairs it deletes.
    #[test]
    fn remove_pairs_counts_before_it_deletes() {
        let mut app = app();
        let mut dialog = BatchDialog::new(Batch::Remove, &mut app);
        dialog.settings.smaller_than = Some(80.0);
        dialog.preview(&mut app);
        assert_eq!(dialog.report.as_ref().map(|r| (r.count, r.rows[0].now)), Some((1, Some(-60.0))));
        assert_eq!(stored(&mut app).len(), 2);
        dialog.apply(&mut app);
        assert_eq!(stored(&mut app), vec![("A".to_string(), "V".to_string(), -100.0)]);

        app.session.execute("kerning.setPair", &json!({"left": "A", "right": "V", "value": 3, "direction": "vertical"})).unwrap();
        let vertical = BatchDialog::new(Batch::Vertical, &mut app);
        assert_eq!(vertical.vertical_pairs.iter().map(|(_, n)| *n).sum::<usize>(), 1);
        assert!(vertical.can_apply());
    }
}
