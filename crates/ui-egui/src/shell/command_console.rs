//! The Command Console (`window.macroPanel`): a one-line command input with completion of command
//! ids and glyph names, the command log (`fontcraft_engine::log`) and module log lines; history
//! with Up and Down; the setting that also writes the log to standard error
//! (`addons.useSystemConsoleForScriptOutput`). The panel is drawn by
//! `crate::panels::command_console`.
//!
//! A line is either a command id followed by its JSON parameters (`glyph.newGlyph {"name": "a"}`)
//! or a run-style line as the command-line tool takes it (`run --cmd ID --params JSON`, the pair
//! repeated for several commands; a leading `fontcraft-cli` is allowed). Every command a line runs
//! goes through [`FontcraftApp::run`], so the console can do exactly what a menu or an agent can.
//!
//! The log pane shows the session's command log: engine commands run from any surface, module
//! log lines, and the console's own lines (shell commands it ran, lines it could not read). Clear
//! empties the pane only; the session's log keeps its entries for `app.info`.
//!
//! Console input is untrusted: a line holds at most [`MAX_LINE_BYTES`], the history
//! [`MAX_HISTORY`] lines and [`MAX_HISTORY_BYTES`] in all, and a run-style line
//! [`MAX_LINE_COMMANDS`] commands.

use std::io::Write;
use std::sync::LazyLock;

use fontcraft_engine::log::{CommandLog, LogEntry};
use fontcraft_model::Font;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{Args, UiCommand};
use crate::app::FontcraftApp;
use crate::panels::Pending;

/// Longest line the console runs, in bytes.
pub const MAX_LINE_BYTES: usize = 1 << 20;
/// Most lines the history keeps; the oldest are dropped.
pub const MAX_HISTORY: usize = 1_000;
/// Most bytes the history keeps in all; the oldest lines are dropped.
pub const MAX_HISTORY_BYTES: usize = 16 << 20;
/// Most commands one run-style line names.
pub const MAX_LINE_COMMANDS: usize = 1_000;
/// Most completion suggestions listed.
pub const MAX_SUGGESTIONS: usize = 50;
/// Most text Copy puts on the clipboard, in bytes.
pub const MAX_COPY_BYTES: usize = 16 << 20;
/// Longest command id the console accepts, in bytes.
const MAX_ID_BYTES: usize = 256;
/// The setting that also writes the log to standard error.
pub const SYSTEM_CONSOLE_SETTING: &str = "addons.useSystemConsoleForScriptOutput";
/// What each line written to standard error starts with.
const ECHO_PREFIX: &str = "fontcraft: ";

pub const COMMANDS: &[UiCommand] = &[(
    "window.macroPanel",
    "Macro Panel",
    &["Window"],
    "Cmd+Alt+M",
    "{open?: bool, line?: string, clear?: bool, copy?: bool} show or hide the Command Console (with no parameters: show it). line: run it as Return in the console does: a command id then JSON parameters (`glyph.newGlyph {\"name\": \"a.alt\"}`), or a run-style line (`run --cmd ID --params JSON`, the pair repeated for several engine commands, which run as one step: if one fails, none takes effect); at most 1 MiB. The line joins the history, and each command it runs, with its result or error, joins the log. clear: empty the log pane (the session's command log keeps its entries). copy: put the log pane's text on the clipboard → {open, history: lines in the history, shown: entries in the log pane, result?: the result of the line's command (a list for several)}",
)];

/// This module's view and window state, kept in `UiState::panels`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct CommandConsoleState {
    /// The console window is showing.
    pub open: bool,
    /// The text in the input line.
    #[serde(skip)]
    pub input: String,
    /// The lines run, oldest first.
    #[serde(skip)]
    pub history: Vec<String>,
    /// While walking the history with Up and Down: the line showing.
    #[serde(skip)]
    pub recall: Option<usize>,
    /// What was typed before the walk started; Down past the newest line gives it back.
    #[serde(skip)]
    pub draft: String,
    /// The log pane shows the entries with this sequence number or more (Clear moves it).
    #[serde(skip)]
    pub shown_from: u64,
    /// The first log entry not yet considered for standard error.
    #[serde(skip)]
    pub echoed: u64,
    /// The completion of the input line, and the line and document revision it was made for.
    #[serde(skip)]
    pub completion: Option<(String, u64, Completion)>,
}

/// A direction through the history.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Recall {
    /// Up: an older line.
    Previous,
    /// Down: a newer line, then what was typed.
    Next,
}

impl CommandConsoleState {
    /// Adds a line to the history, unless it repeats the newest one; drops the oldest lines past
    /// the limits. Ends a walk through the history.
    pub fn remember(&mut self, line: &str) {
        self.recall = None;
        self.draft.clear();
        let line = line.trim();
        if line.is_empty() || line.len() > MAX_LINE_BYTES || self.history.last().is_some_and(|l| l == line) {
            return;
        }
        self.history.push(line.to_string());
        let mut bytes: usize = self.history.iter().map(String::len).sum();
        let mut drop = 0;
        while drop < self.history.len() && (self.history.len() - drop > MAX_HISTORY || bytes > MAX_HISTORY_BYTES) {
            bytes -= self.history.get(drop).map_or(0, String::len);
            drop += 1;
        }
        self.history.drain(..drop);
    }

    /// Shows an older or newer line of the history in the input line (Up and Down).
    pub fn recall(&mut self, direction: Recall) {
        let len = self.history.len();
        let next = match (direction, self.recall) {
            (Recall::Previous, None) if len > 0 => {
                self.draft = std::mem::take(&mut self.input);
                Some(len - 1)
            }
            (Recall::Previous, Some(i)) => Some(i.saturating_sub(1)),
            (Recall::Next, Some(i)) if i + 1 < len => Some(i + 1),
            (Recall::Next, Some(_)) => {
                self.recall = None;
                self.input = std::mem::take(&mut self.draft);
                return;
            }
            _ => return,
        };
        if let Some(line) = next.and_then(|i| self.history.get(i)) {
            self.input = line.clone();
            self.recall = next;
        }
    }
}

/// What a completion suggests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SuggestionKind {
    Command,
    Glyph,
}

/// One suggestion.
#[derive(Clone, Debug, PartialEq)]
pub struct Suggestion {
    pub text: String,
    pub kind: SuggestionKind,
}

/// The completion of an input line: the suggestions for the word being typed, where that word
/// starts, and the parameters doc of the command the line names.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Completion {
    /// Byte position in the line where the word being completed starts.
    pub start: usize,
    pub items: Vec<Suggestion>,
    /// The command the line names (or the first suggested one) and its parameters doc.
    pub doc: Option<(&'static str, &'static str)>,
}

/// Every command id with its parameters doc, sorted by id.
static COMMAND_DOCS: LazyLock<Vec<(&'static str, &'static str)>> = LazyLock::new(|| {
    let mut v: Vec<(&'static str, &'static str)> =
        fontcraft_engine::command_specs().iter().map(|c| (c.id, c.params)).chain(super::UI_COMMANDS.iter().map(|c| (c.0, c.4))).collect();
    v.sort_unstable_by(|a, b| a.0.cmp(b.0));
    v.dedup_by(|a, b| a.0 == b.0);
    v
});

fn command_doc(id: &str) -> Option<(&'static str, &'static str)> {
    let list = &*COMMAND_DOCS;
    list.binary_search_by(|c| c.0.cmp(id)).ok().and_then(|i| list.get(i)).copied()
}

/// Whether `hay` contains `needle`, ignoring ASCII case.
fn contains_ignore_case(hay: &str, needle: &str) -> bool {
    let (h, n) = (hay.as_bytes(), needle.as_bytes());
    n.is_empty() || h.windows(n.len()).any(|w| w.eq_ignore_ascii_case(n))
}

/// Up to [`MAX_SUGGESTIONS`] names matching `prefix`: those starting with it first (in list
/// order), then those containing it in any ASCII case.
fn matching<'a>(names: impl Iterator<Item = &'a str> + Clone, prefix: &str, kind: SuggestionKind) -> Vec<Suggestion> {
    let mut out: Vec<Suggestion> =
        names.clone().filter(|n| n.starts_with(prefix)).take(MAX_SUGGESTIONS).map(|n| Suggestion { text: n.to_string(), kind }).collect();
    if out.len() < MAX_SUGGESTIONS && !prefix.is_empty() {
        let room = MAX_SUGGESTIONS - out.len();
        out.extend(
            names.filter(|n| !n.starts_with(prefix) && contains_ignore_case(n, prefix)).take(room).map(|n| Suggestion { text: n.to_string(), kind }),
        );
    }
    out
}

/// Where the end of `json` (the parameters typed so far) stands: `Some(start)` when it is inside
/// a JSON string that holds a value (after `:` or inside a list), with the byte position where
/// the string's text starts; `None` when it is outside a string or inside an object key.
fn open_value_string(json: &str) -> Option<usize> {
    let mut stack: Vec<u8> = Vec::new();
    let mut in_string: Option<(usize, bool)> = None;
    let mut escaped = false;
    let mut last_significant = 0u8;
    for (i, b) in json.bytes().enumerate() {
        if in_string.is_some() {
            if escaped {
                escaped = false;
            } else if b == b'\\' {
                escaped = true;
            } else if b == b'"' {
                in_string = None;
                last_significant = b'"';
            }
            continue;
        }
        match b {
            b'"' => {
                let is_value = last_significant == b':' || stack.last() == Some(&b'[');
                in_string = Some((i + 1, is_value));
            }
            b'{' | b'[' => {
                // Nesting deeper than any parameters need is not completed.
                if stack.len() >= 64 {
                    return None;
                }
                stack.push(b);
                last_significant = b;
            }
            b'}' | b']' => {
                stack.pop();
                last_significant = b;
            }
            b' ' | b'\t' | b'\n' | b'\r' => {}
            other => last_significant = other,
        }
    }
    match in_string {
        Some((start, true)) => Some(start),
        _ => None,
    }
}

/// Splits `line` at whitespace (no quoting): each word with its byte position.
fn words(line: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let mut start = None;
    for (i, c) in line.char_indices() {
        match (c.is_whitespace(), start) {
            (true, Some(s)) => {
                out.push((s, line.get(s..i).unwrap_or_default()));
                start = None;
            }
            (false, None) => start = Some(i),
            _ => {}
        }
    }
    if let Some(s) = start {
        out.push((s, line.get(s..).unwrap_or_default()));
    }
    out
}

/// Whether `words` start a run-style line.
fn is_run_line(words: &[(usize, &str)]) -> bool {
    matches!(words, [(_, "run"), ..] | [(_, "fontcraft-cli"), (_, "run"), ..])
}

/// Completes the end of `line`: command ids for the first word (or after `--cmd`), glyph names
/// inside a JSON string value of the parameters. `font` gives the glyph names.
pub fn complete(font: Option<&Font>, line: &str) -> Completion {
    if line.len() > MAX_LINE_BYTES {
        return Completion::default();
    }
    let ws = words(line);
    let ends_in_space = line.chars().last().is_some_and(char::is_whitespace);
    let current = if ends_in_space { None } else { ws.last().copied() };
    let commands = || COMMAND_DOCS.iter().map(|c| c.0);
    let glyphs = |prefix: &str, start: usize, doc| {
        let items = match font {
            Some(font) => matching(font.glyphs.iter().map(|g| g.name.as_str()), prefix, SuggestionKind::Glyph),
            None => Vec::new(),
        };
        Completion { start, items, doc }
    };
    let id_completion = |(start, prefix): (usize, &str)| {
        let items = matching(commands(), prefix, SuggestionKind::Command);
        let doc = command_doc(prefix).or_else(|| items.first().and_then(|s| command_doc(&s.text)));
        Completion { start, items, doc }
    };
    if is_run_line(&ws) {
        let before = if current.is_some() { ws.len().saturating_sub(1) } else { ws.len() };
        let prior = ws.get(..before).unwrap_or_default();
        if prior.last().is_some_and(|w| w.1 == "--cmd") {
            return id_completion(current.unwrap_or((line.len(), "")));
        }
        let named = prior.iter().rposition(|w| w.1 == "--cmd").and_then(|i| prior.get(i + 1)).and_then(|w| command_doc(w.1));
        let params_at = ws.iter().rposition(|w| w.1 == "--params").and_then(|i| ws.get(i + 1)).map(|w| w.0);
        if let Some(at) = params_at {
            let tail = line.get(at..).unwrap_or_default();
            // The parameters may be quoted for the shell; the JSON starts at the first brace.
            if let Some(brace) = tail.find('{')
                && let Some(start) = open_value_string(tail.get(brace..).unwrap_or_default())
            {
                let start = at + brace + start;
                return glyphs(line.get(start..).unwrap_or_default(), start, named);
            }
        }
        return Completion { start: line.len(), items: Vec::new(), doc: named };
    }
    match ws.as_slice() {
        [] => id_completion((line.len(), "")),
        [only] if current.is_some() => id_completion(*only),
        [(_, id), ..] => {
            let doc = command_doc(id);
            let after_id = ws.get(1).map_or(line.len(), |w| w.0);
            let tail = line.get(after_id..).unwrap_or_default();
            match open_value_string(tail) {
                Some(start) => {
                    let start = after_id + start;
                    glyphs(line.get(start..).unwrap_or_default(), start, doc)
                }
                None => Completion { start: line.len(), items: Vec::new(), doc },
            }
        }
    }
}

/// `line` with suggestion `index` of `completion` in place of the word being completed: a
/// command id is followed by a space, a glyph name (escaped for JSON) by its closing quote.
pub fn apply(line: &str, completion: &Completion, index: usize) -> Option<String> {
    let s = completion.items.get(index)?;
    let mut out = line.get(..completion.start)?.to_string();
    match s.kind {
        SuggestionKind::Command => {
            out.push_str(&s.text);
            out.push(' ');
        }
        SuggestionKind::Glyph => {
            let quoted = serde_json::to_string(&s.text).ok()?;
            out.push_str(quoted.get(1..)?);
        }
    }
    Some(out)
}

/// Splits a run-style line into words as a shell does: whitespace separates, single quotes keep
/// their text as it is, double quotes and backslashes escape.
fn shell_words(line: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut word: Option<String> = None;
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            c if c.is_whitespace() => {
                if let Some(w) = word.take() {
                    out.push(w);
                }
            }
            '\'' => {
                let w = word.get_or_insert_with(String::new);
                loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some(c) => w.push(c),
                        None => return Err("a single quote is not closed".into()),
                    }
                }
            }
            '"' => {
                let w = word.get_or_insert_with(String::new);
                loop {
                    match chars.next() {
                        Some('"') => break,
                        Some('\\') => match chars.next() {
                            Some(c @ ('"' | '\\' | '$' | '`')) => w.push(c),
                            Some(c) => {
                                w.push('\\');
                                w.push(c);
                            }
                            None => return Err("a double quote is not closed".into()),
                        },
                        Some(c) => w.push(c),
                        None => return Err("a double quote is not closed".into()),
                    }
                }
            }
            '\\' => {
                let w = word.get_or_insert_with(String::new);
                if let Some(c) = chars.next() {
                    w.push(c);
                }
            }
            c => word.get_or_insert_with(String::new).push(c),
        }
    }
    if let Some(w) = word {
        out.push(w);
    }
    Ok(out)
}

/// Reads a command's parameters: a JSON object.
fn parse_params(text: &str) -> Result<Value, String> {
    let v: Value = serde_json::from_str(text).map_err(|e| format!("the parameters are not JSON: {e}"))?;
    if v.is_object() { Ok(v) } else { Err("the parameters must be a JSON object".into()) }
}

fn checked_id(id: &str) -> Result<String, String> {
    if id.len() > MAX_ID_BYTES {
        return Err(format!("a command id has at most {MAX_ID_BYTES} bytes"));
    }
    Ok(id.to_string())
}

/// Reads a console line into the commands it runs, in order. An empty line runs nothing.
pub fn parse_line(line: &str) -> Result<Vec<(String, Value)>, String> {
    if line.len() > MAX_LINE_BYTES {
        return Err(format!("a console line has at most {MAX_LINE_BYTES} bytes"));
    }
    let line = line.trim();
    if line.is_empty() {
        return Ok(Vec::new());
    }
    if !is_run_line(&words(line)) {
        let (id, rest) = line.split_once(char::is_whitespace).unwrap_or((line, ""));
        let rest = rest.trim();
        let params = if rest.is_empty() { json!({}) } else { parse_params(rest)? };
        return Ok(vec![(checked_id(id)?, params)]);
    }
    let mut steps: Vec<(String, Value, bool)> = Vec::new();
    let mut it = shell_words(line)?.into_iter().peekable();
    if it.peek().is_some_and(|w| w == "fontcraft-cli") {
        it.next();
    }
    it.next(); // `run`
    while let Some(word) = it.next() {
        match word.as_str() {
            "--cmd" => {
                let id = it.next().ok_or("--cmd needs a command id")?;
                if steps.len() >= MAX_LINE_COMMANDS {
                    return Err(format!("a run line names at most {MAX_LINE_COMMANDS} commands"));
                }
                steps.push((checked_id(&id)?, json!({}), false));
            }
            "--params" => {
                let text = it.next().ok_or("--params needs JSON parameters")?;
                let Some(step) = steps.last_mut() else { return Err("--params comes after the --cmd it belongs to".into()) };
                if step.2 {
                    return Err(format!("`{}` has --params twice", step.0));
                }
                step.1 = parse_params(&text)?;
                step.2 = true;
            }
            "--in" | "--save" | "--export" => {
                return Err(format!("the console does not take `{word}`: open, save and export with the file commands (file.open, file.save…)"));
            }
            other => {
                let shown: String = other.chars().take(64).collect();
                return Err(format!("unexpected `{shown}`: a run line holds --cmd ID and --params JSON pairs"));
            }
        }
    }
    if steps.is_empty() {
        return Err("the run line names no command (--cmd ID)".into());
    }
    Ok(steps.into_iter().map(|(id, params, _)| (id, params)).collect())
}

/// Runs a console line, as Return in the console does: adds it to the history, then runs it
/// through [`FontcraftApp::run`]. A line naming several commands runs them as one `command.batch`
/// (one transaction and one undo step: if one fails, none takes effect), so it may name engine
/// commands only. Commands the session's log does not record itself (shell commands, unknown
/// ids) and lines that cannot be read are logged here, so the log pane shows every outcome.
pub fn run_line(app: &mut FontcraftApp, line: &str) -> Result<Value, String> {
    if line.len() > MAX_LINE_BYTES {
        return Err(format!("a console line has at most {MAX_LINE_BYTES} bytes"));
    }
    app.state.panels.command_console.remember(line);
    let first = line.split_whitespace().next().unwrap_or_default();
    let steps = parse_line(line).and_then(|steps| {
        if steps.len() > 1
            && let Some((id, _)) = steps.iter().find(|(id, _)| super::is_ui_command(id))
        {
            return Err(format!("`{id}` is a window command: a line with several commands runs engine commands only"));
        }
        Ok(steps)
    });
    let batched = steps.as_ref().is_ok_and(|s| s.len() > 1);
    let (id, params) = match steps {
        Ok(mut steps) if steps.len() == 1 => steps.pop().unwrap_or_default(),
        Ok(steps) if steps.is_empty() => return Ok(Value::Null),
        Ok(steps) => {
            let steps: Vec<Value> = steps.into_iter().map(|(id, params)| json!({"command": id, "params": params})).collect();
            ("command.batch".to_string(), json!({"label": "Console", "steps": steps}))
        }
        Err(e) => {
            app.session.extensions_mut().log.push_command(first, &Value::Null, Err(e.clone()));
            return Err(e);
        }
    };
    let before = app.session.extensions().log.next_seq();
    let result = if id == "window.macroPanel" && params.get("line").is_some_and(|l| !l.is_null()) {
        Err("a console line cannot run another console line".to_string())
    } else {
        app.run(&id, &params)
    };
    if app.session.extensions().log.next_seq() == before {
        app.session.extensions_mut().log.push_command(&id, &params, result.as_ref().map_err(Clone::clone));
    }
    match result {
        Ok(v) if batched => Ok(v.get("results").cloned().unwrap_or(v)),
        other => other,
    }
}

/// The log entries the pane shows, oldest first.
pub fn shown<'a>(state: &CommandConsoleState, log: &'a CommandLog) -> impl Iterator<Item = &'a LogEntry> {
    log.from_seq(state.shown_from)
}

/// The pane's text, one line per entry, cut at [`MAX_COPY_BYTES`].
pub fn pane_text(state: &CommandConsoleState, log: &CommandLog) -> String {
    let mut out = String::new();
    for entry in shown(state, log) {
        let line = entry.to_line();
        if out.len() + line.len() + 1 > MAX_COPY_BYTES {
            break;
        }
        out.push_str(&line);
        out.push('\n');
    }
    out
}

/// Writes the log entries added since the last call to `out`, one line each, while the setting
/// [`SYSTEM_CONSOLE_SETTING`] is on. The desktop app calls it every frame with standard error;
/// entries added while the setting is off are never written.
pub fn echo(app: &mut FontcraftApp, out: &mut dyn Write) {
    let on = app.session.settings().get_bool(SYSTEM_CONSOLE_SETTING).unwrap_or(false);
    let log = &app.session.extensions().log;
    let state = &mut app.state.panels.command_console;
    if on {
        for entry in log.from_seq(state.echoed) {
            // A closed standard error must not stop the app.
            let _ = writeln!(out, "{ECHO_PREFIX}{}", entry.to_line());
        }
        let _ = out.flush();
    }
    state.echoed = log.next_seq();
}

/// The completion of the current input line, made again only when the line or the document
/// changed.
pub fn completion(app: &mut FontcraftApp) -> Completion {
    let revision = app.session.active().map_or(0, |d| d.revision);
    let state = &mut app.state.panels.command_console;
    if let Some((line, rev, c)) = &state.completion
        && *line == state.input
        && *rev == revision
    {
        return c.clone();
    }
    let c = complete(app.session.font().ok(), &state.input);
    state.completion = Some((state.input.clone(), revision, c.clone()));
    c
}

pub fn run(app: &mut FontcraftApp, id: &str, args: &Args) -> Result<Value, String> {
    if id != "window.macroPanel" {
        return Err(format!("unknown command `{id}`"));
    }
    let open = args.flag("open")?;
    let line = args.string("line")?;
    let clear = args.flag("clear")?.unwrap_or(false);
    let copy = args.flag("copy")?.unwrap_or(false);
    let only_open = line.is_none() && !clear && !copy;
    if let Some(open) = open.or(only_open.then_some(true)) {
        app.state.panels.command_console.open = open;
    }
    if clear {
        app.state.panels.command_console.shown_from = app.session.extensions().log.next_seq();
    }
    let result = match line {
        Some(line) => Some(run_line(app, line).map_err(|e| args.err(e))?),
        None => None,
    };
    if copy {
        app.clipboard_out = Some(pane_text(&app.state.panels.command_console, &app.session.extensions().log));
    }
    let state = &app.state.panels.command_console;
    let mut out = json!({
        "open": state.open,
        "history": state.history.len(),
        "shown": shown(state, &app.session.extensions().log).count(),
    });
    if let (Some(result), Some(map)) = (result, out.as_object_mut()) {
        map.insert("result".into(), result);
    }
    Ok(out)
}

/// Draws this module's windows while they are open; every frame it also hands new log entries
/// to standard error when the setting asks for it.
pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    echo(app, &mut std::io::stderr());
    if !app.state.panels.command_console.open {
        return;
    }
    let completion = completion(app);
    crate::panels::command_console::show(app, ui, &completion, pending);
}
