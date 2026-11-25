//! What the last export reported: the rows of the Export Results window. Rows: `dialog.exportResults`,
//! `file.exportResults`.
//!
//! An export the person ran keeps its errors, skipped fonts, warnings and files here. The window
//! (`dialogs/export_results.rs`) opens when there is anything but files; an export that only wrote files says how many
//! in the status line. An export a request names (`engine.execute`, MCP) opens nothing: its reply or error says it all.

use std::path::PathBuf;

use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::WordSet as _;
use fontcraft_engine::EngineError;
use serde_json::{Map, Value, json};

use super::{FontCraftApp, Msg};
use crate::shell::commands::{Module, UiCommand};
use crate::shell::font_info_tabs::EntryKey;
use crate::shell::params;

pub const COMMANDS: &[UiCommand] = &[UiCommand {
    id: "file.exportResults",
    label: "Export Results…",
    menu: &[],
    shortcut: None,
    params: "{show?: bool (default true), reveal?: row index} open or close the window that lists what the person's last export reported; reveal shows what a row is about: its glyph in a new tab (in the layer the row names), its feature code in Font Info, or its file in the system's file manager. Refused when no export reported anything. ui.inspect reports it as exportResults: {open, title, rows: [{group: \"error\" | \"skipped\" | \"warning\" | \"file\", subject, text, reveals: \"glyph\" | \"features\" | \"file\" | null}]}",
}];

pub const MODULE: Module = Module { commands: COMMANDS, run };

/// The commands whose reply and errors the window lists.
const EXPORTS: [&str; 7] = [
    "file.exportStatic",
    "file.exportVariable",
    "file.exportAll",
    "file.exportUfo",
    "file.exportMetrics",
    "file.exportWithProfiles",
    "project.export",
];

/// Most rows the window lists.
pub const MAX_ROWS: usize = 5_000;
/// Longest text of a row, in characters.
const MAX_TEXT: usize = 2_000;
/// Deepest nesting of a reply's documents and profiles.
const MAX_DEPTH: usize = 4;

craft::ui::words! {
    /// What the status line says about an export.
    pub struct ExportStatusWords {
        wrote: "fontcraftExportStatus.wrote" = "wrote";
        file: "fontcraftExportStatus.file" = "file";
        files: "fontcraftExportStatus.files" = "files";
        skipped: "fontcraftExportStatus.skipped" = "skipped";
        warnings: "fontcraftExportStatus.warnings" = "warnings";
    }
}

/// The groups of the list, in its order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Group {
    Error,
    Skipped,
    Warning,
    File,
}

impl Group {
    fn id(self) -> &'static str {
        match self {
            Group::Error => "error",
            Group::Skipped => "skipped",
            Group::Warning => "warning",
            Group::File => "file",
        }
    }
}

/// What a row shows when asked.
#[derive(Clone, Debug, PartialEq)]
pub enum Target {
    Glyph { doc: u64, glyph: String, layer: Option<String> },
    Feature { doc: u64, entry: EntryKey, line: usize, column: usize },
    File(PathBuf),
}

impl Target {
    fn id(&self) -> &'static str {
        match self {
            Target::Glyph { .. } => "glyph",
            Target::Feature { .. } => "features",
            Target::File(_) => "file",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub group: Group,
    /// The glyph, instance, entry or file the row is about; may be empty.
    pub subject: String,
    pub text: String,
    pub target: Option<Target>,
}

/// The last export's rows, kept by the app between frames.
#[derive(Clone, Debug, Default)]
pub struct ExportResults {
    pub rows: Vec<Row>,
    /// The export command's label.
    pub title: String,
    pub open: bool,
    /// Tells one export's rows from the next one's.
    pub serial: u64,
}

impl ExportResults {
    /// The whole list as text, one row a line under its group.
    pub fn text(&self, name: impl Fn(Group) -> String) -> String {
        let mut out = String::new();
        let mut last = None;
        for row in &self.rows {
            if last != Some(row.group) {
                out.push_str(&name(row.group));
                out.push('\n');
                last = Some(row.group);
            }
            out.push_str("  ");
            if !row.subject.is_empty() {
                out.push_str(&row.subject);
                out.push_str(": ");
            }
            out.push_str(&row.text);
            out.push('\n');
        }
        out
    }
}

fn cut(text: &str) -> String {
    text.chars().take(MAX_TEXT).collect()
}

fn items<'a>(v: &'a Value, key: &str) -> impl Iterator<Item = &'a Value> {
    v.get(key).and_then(Value::as_array).into_iter().flatten()
}

fn text<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or_default()
}

/// Reads rows out of a reply or an error's data.
struct Reader<'a> {
    app: &'a FontCraftApp,
    rows: Vec<Row>,
    /// The warnings listed, each once.
    warned: std::collections::HashSet<String>,
}

impl Reader<'_> {
    fn push(&mut self, group: Group, subject: &str, text: &str, target: Option<Target>) {
        if self.rows.len() < MAX_ROWS {
            self.rows.push(Row { group, subject: cut(subject), text: cut(text), target });
        }
    }

    /// The document called `title`, else `doc`.
    fn document(&self, v: &Value, key: &str, doc: Option<u64>) -> Option<u64> {
        let title = v.get(key).and_then(Value::as_str);
        match title {
            Some(title) => self.app.session.documents().iter().find(|d| d.title == title).map(|d| d.id),
            None => doc,
        }
    }

    /// The glyph `name` of `doc`, when the document has it.
    fn glyph(&self, doc: Option<u64>, name: &str, layer: Option<&str>) -> Option<Target> {
        let doc = doc?;
        let font = &self.app.session.documents().iter().find(|d| d.id == doc)?.font;
        let glyph = font.glyph(name)?;
        let layer = layer.filter(|l| glyph.layers.iter().any(|known| known.id == **l)).map(str::to_string);
        Some(Target::Glyph { doc, glyph: name.to_string(), layer })
    }

    /// The glyph a compiler message starts with (glyph `NAME` …).
    fn named_glyph(&self, doc: Option<u64>, message: &str) -> Option<Target> {
        let (_, rest) = message.split_once("glyph `")?;
        let (name, _) = rest.split_once('`')?;
        self.glyph(doc, name, None)
    }

    fn error(&mut self, v: &Value, doc: Option<u64>) {
        let doc = self.document(v, "document", doc);
        let message = text(v, "message");
        let entry = v.get("entry").filter(|e| !e.is_null());
        let key = entry.and_then(EntryKey::from_json);
        let number = |key: &str| v.get(key).and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok()).unwrap_or(1);
        let target = match (&key, doc) {
            (Some(key), Some(doc)) => Some(Target::Feature { doc, entry: key.clone(), line: number("line"), column: number("column") }),
            _ => match v.get("glyph").and_then(Value::as_str) {
                Some(glyph) => self.glyph(doc, glyph, v.get("layer").and_then(Value::as_str)),
                None => self.named_glyph(doc, message),
            },
        };
        let subject = match (entry, v.get("glyph").and_then(Value::as_str)) {
            (Some(entry), _) => text(entry, "name").to_string(),
            (None, Some(glyph)) => glyph.to_string(),
            (None, None) => items(v, "instances").filter_map(Value::as_str).collect::<Vec<_>>().join(", "),
        };
        self.push(Group::Error, &subject, message, target);
    }

    fn warning(&mut self, subject: &str, message: &str, doc: Option<u64>) {
        if self.warned.insert(format!("{subject}\n{message}")) {
            let target = self.named_glyph(doc, message);
            self.push(Group::Warning, subject, message, target);
        }
    }

    fn file(&mut self, v: &Value) {
        let Some(path) = v.get("path").and_then(Value::as_str).map(PathBuf::from) else { return };
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let folder = path.parent().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
        self.push(Group::File, &name, &folder, Some(Target::File(path)));
    }

    /// A reply, or one document or profile of it.
    fn reply(&mut self, v: &Value, doc: Option<u64>, depth: usize) {
        if depth > MAX_DEPTH {
            return;
        }
        let doc = self.document(v, "title", doc);
        for e in items(v, "errors") {
            self.error(e, doc);
        }
        for skipped in items(v, "skipped") {
            let instance = text(skipped, "instance").to_string();
            self.push(Group::Skipped, &instance, text(skipped, "reason"), None);
            for g in items(skipped, "glyphs") {
                let target = self.glyph(doc, text(g, "glyph"), g.get("layer").and_then(Value::as_str));
                self.push(Group::Skipped, text(g, "glyph"), text(g, "message"), target);
            }
        }
        for font in items(v, "fonts") {
            let name = font.get("instance").or_else(|| font.get("setting")).and_then(Value::as_str).unwrap_or_default();
            for w in items(font, "warnings").filter_map(Value::as_str) {
                self.warning(name, w, doc);
            }
            for f in items(font, "files") {
                self.file(f);
            }
        }
        for w in items(v, "warnings").filter_map(Value::as_str) {
            self.warning("", w, doc);
        }
        for f in items(v, "files") {
            self.file(f);
        }
        if v.get("fonts").is_none() && v.get("files").is_none() {
            self.file(v);
        }
        for part in items(v, "documents").chain(items(v, "profiles")) {
            self.reply(part, doc, depth.saturating_add(1));
        }
    }
}

/// The rows of `v`, a reply or an error's data, in the list's order.
fn rows(app: &FontCraftApp, id: &str, v: &Value) -> Vec<Row> {
    // A project's font is read from its file: the active document is another font.
    let doc = if id == "project.export" { None } else { app.session.active().map(|d| d.id) };
    let mut reader = Reader { app, rows: Vec::new(), warned: Default::default() };
    reader.reply(v, doc, 0);
    let mut rows = reader.rows;
    rows.sort_by_key(|r| r.group);
    rows
}

fn label(id: &str) -> &str {
    fontcraft_engine::find_command(id).map_or(id, |spec| spec.label)
}

/// Keeps `rows` as the last export's; opens the window when there is more than files.
fn keep(app: &mut FontCraftApp, id: &str, rows: Vec<Row>) {
    let results = &mut app.view.export_results;
    results.serial = results.serial.saturating_add(1);
    results.title = label(id).to_string();
    results.open = rows.iter().any(|r| r.group != Group::File);
    results.rows = rows;
    app.view_changed();
}

/// After the person's run of `id`: keeps what an export reported and returns the status line for it.
pub fn after_run(app: &mut FontCraftApp, id: &str, reply: &Value) -> Option<String> {
    if !EXPORTS.contains(&id) {
        return None;
    }
    let rows = rows(app, id, reply);
    let count = |group: Group| rows.iter().filter(|r| r.group == group).count();
    let listed = count(Group::File);
    let files = match reply.get("written").and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok()) {
        Some(written) => written,
        None => listed,
    };
    let skipped = items_skipped(reply, 0);
    let warnings = count(Group::Warning);
    let words = ExportStatusWords::english();
    let file_word = if files == 1 { words.file.as_str() } else { words.files.as_str() };
    let mut status = format!("{}: {} {files} {file_word}", label(id), words.wrote.as_str());
    if skipped > 0 {
        status.push_str(&format!(", {skipped} {}", words.skipped.as_str()));
    }
    if warnings > 0 {
        status.push_str(&format!(", {warnings} {}", words.warnings.as_str()));
    }
    keep(app, id, rows);
    Some(status)
}

/// How many fonts a reply says were skipped.
fn items_skipped(v: &Value, depth: usize) -> usize {
    if depth > MAX_DEPTH {
        return 0;
    }
    let here = items(v, "skipped").count();
    items(v, "documents").chain(items(v, "profiles")).map(|part| items_skipped(part, depth.saturating_add(1))).fold(here, usize::saturating_add)
}

/// After the person's run of `id` failed: lists what stopped an export, when its error lists anything. Returns whether
/// the window has it.
pub fn after_error(app: &mut FontCraftApp, id: &str, error: &EngineError) -> bool {
    let Some(data) = error.data().filter(|_| EXPORTS.contains(&id)) else { return false };
    let rows = rows(app, id, data);
    if rows.is_empty() {
        return false;
    }
    keep(app, id, rows);
    true
}

/// Where character `column` of line `line` (both from 1) is in `code`, in characters.
fn character(code: &str, line: usize, column: usize) -> usize {
    let before: usize = code.split('\n').take(line.saturating_sub(1)).map(|l| l.chars().count().saturating_add(1)).sum();
    before.saturating_add(column.saturating_sub(1))
}

/// Shows what row `at` is about.
fn reveal(app: &mut FontCraftApp, at: usize) -> Result<(), String> {
    let row = app.view.export_results.rows.get(at).ok_or_else(|| format!("the export results have no row {at}"))?;
    let target = row.target.clone().ok_or("this row names no glyph, feature code or file")?;
    let run = |app: &mut FontCraftApp, id: &str, params: Value| app.apply_msg(FontCraftApp::route(CommandRef::with_params(id, params)));
    match target {
        Target::Glyph { doc, glyph, layer } => {
            super::close::activate(app, doc)?;
            let known = app.session.font().is_ok_and(|f| f.glyph(&glyph).is_some());
            if !known {
                return Err(format!("the font has no glyph {} any more", params::quoted(&glyph)));
            }
            run(app, "view.newTab", json!({ "glyphs": [glyph] }))?;
            let kept =
                |f: &fontcraft_engine::model::Font, layer: &str| f.glyph(&glyph).is_some_and(|g| g.layers.iter().any(|known| known.id == layer));
            match layer.filter(|l| app.session.font().is_ok_and(|f| kept(f, l))) {
                Some(layer) => run(app, "layers.select", json!({ "glyphs": [glyph], "layer": layer })),
                None => Ok(()),
            }
        }
        Target::Feature { doc, entry, line, column } => {
            super::close::activate(app, doc)?;
            let caret = {
                let font = app.session.font().map_err(|e| e.to_string())?;
                let now = entry.resolve(&font.features).ok_or("the feature code has no such entry any more")?;
                let caret = now.snippet(&font.features).map(|s| character(&s.code, line, column)).unwrap_or(0);
                (now, caret)
            };
            run(app, "font.info", json!({ "tab": "features" }))?;
            run(app, "font.info.features.select", json!({ "entries": [caret.0.params()] }))?;
            app.view.font_info.features.caret = Some(caret);
            Ok(())
        }
        Target::File(path) => app.opener.reveal(&path),
    }
}

fn run(app: &mut FontCraftApp, command: &CommandRef) -> Result<(), String> {
    let p = &command.params;
    if app.view.export_results.rows.is_empty() {
        return Err("no export has reported anything".into());
    }
    if let Some(at) = p.get("reveal").filter(|v| !v.is_null()) {
        let at = at.as_u64().and_then(|n| usize::try_from(n).ok()).ok_or("`reveal` is the index of a row")?;
        return reveal(app, at);
    }
    app.view.export_results.open = params::flag(p, "show")?.unwrap_or(true);
    app.view_changed();
    Ok(())
}

/// The message that shows what row `at` is about.
pub fn reveal_msg(at: usize) -> Msg {
    Msg::Shell(CommandRef::with_params("file.exportResults", json!({ "reveal": at })))
}

/// `ui.inspect`'s `exportResults`.
pub fn inspect(app: &FontCraftApp, out: &mut Map<String, Value>) {
    let results = &app.view.export_results;
    let rows: Vec<Value> = results
        .rows
        .iter()
        .map(|r| json!({ "group": r.group.id(), "subject": r.subject, "text": r.text, "reveals": r.target.as_ref().map(Target::id) }))
        .collect();
    out.insert("exportResults".into(), json!({ "open": results.open, "title": results.title, "rows": rows }));
}

#[cfg(test)]
#[path = "export_results_tests.rs"]
mod tests;
