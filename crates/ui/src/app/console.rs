//! The command surfaces' state and the Command Console's lines.
//!
//! A console line is either a command id followed by its JSON parameters (`glyph.newGlyph {"name": "a"}`) or a
//! run-style line as the command-line tool takes it (`run --cmd ID --params JSON`, the pair repeated; a leading
//! `fontcraft-cli` is allowed). [`run_line`] runs it as a menu or an agent would run the same commands, and every
//! outcome lands in the session's command log, which the console's pane shows.
//!
//! Console input is untrusted: a line holds at most [`MAX_LINE_BYTES`], the history [`MAX_HISTORY`] lines and
//! [`MAX_HISTORY_BYTES`] in all, and a run-style line [`MAX_LINE_COMMANDS`] commands.

use craft::ui::menu::search::HelpEntry;
use fontcraft_engine::log::CommandLog;
use serde_json::{Value, json};

use super::catalog::{self, Kind};
use super::{FontCraftApp, Msg, ViewMsg};

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
/// Deepest JSON nesting the completion reads into.
const MAX_COMPLETION_DEPTH: usize = 64;
/// The console's own shell command.
pub const CONSOLE_COMMAND: &str = "window.macroPanel";

/// The state of the command surfaces: the console, the command palette and the About dialog.
#[derive(Default)]
pub struct CommandSurfaces {
    pub console: ConsoleState,
    /// The command palette is open.
    pub palette_open: bool,
    /// The palette's entries, made when it opens.
    pub palette: Vec<HelpEntry>,
    pub about: About,
}

/// Where the About dialog stands.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum About {
    #[default]
    Closed,
    /// `app.about` ran: the dialog opens on the next pass.
    Open,
    /// The dialog is on the host.
    Shown,
}

/// The Command Console's state: the input line, the history, where the pane starts, and the Macro Panel's editor
/// (`shell::macro_editor`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ConsoleState {
    /// The text in the input line.
    pub input: String,
    /// The lines run, oldest first.
    pub history: Vec<String>,
    /// The pane shows the log entries with this sequence number or more (Clear moves it).
    pub shown_from: u64,
    /// Text Copy asked for, waiting for the shell to put it on the clipboard.
    pub copy_out: Option<String>,
    /// The editor's text: one step per line, as the input line takes them.
    pub editor: String,
    /// The title, description, arguments and required modules of the macro file opened last; its steps are the
    /// editor's lines, so this holds none.
    pub editor_macro: fontcraft_engine::macros::Macro,
}

impl ConsoleState {
    /// Adds a line to the history, unless it repeats the newest one; drops the oldest lines past the limits.
    pub fn remember(&mut self, line: &str) {
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
}

/// What a completion suggests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SuggestionKind {
    Command,
    Glyph,
    /// A line of the history, offered while the input line is empty.
    History,
}

/// One suggestion.
#[derive(Clone, Debug, PartialEq)]
pub struct Suggestion {
    pub text: String,
    pub kind: SuggestionKind,
}

/// The completion of an input line: the suggestions for the word being typed, where that word starts, and the
/// parameters doc of the command the line names.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Completion {
    /// Byte position in the line where the word being completed starts.
    pub start: usize,
    pub items: Vec<Suggestion>,
    /// The command the line names (or the first suggested one) and its parameters doc.
    pub doc: Option<(&'static str, &'static str)>,
}

impl Suggestion {
    /// The text that replaces the word being completed: a command id with a space after it, a glyph name escaped
    /// for JSON with its closing quote, a history line as it is.
    pub fn insert(&self) -> Option<String> {
        match self.kind {
            SuggestionKind::Command => Some(format!("{} ", self.text)),
            SuggestionKind::Glyph => serde_json::to_string(&self.text).ok().and_then(|q| q.get(1..).map(str::to_string)),
            SuggestionKind::History => Some(self.text.clone()),
        }
    }
}

/// Every command id with its parameters doc, sorted by id: engine commands and shell commands.
pub fn command_docs() -> Vec<(&'static str, &'static str)> {
    let shell = crate::shell::commands::MODULES.iter().flat_map(|m| m.commands.iter().map(|c| (c.id, c.params)));
    let mut docs: Vec<(&'static str, &'static str)> = fontcraft_engine::command_specs().iter().map(|c| (c.id, c.params)).chain(shell).collect();
    docs.sort_unstable_by(|a, b| a.0.cmp(b.0));
    docs.dedup_by(|a, b| a.0 == b.0);
    docs
}

fn command_doc(docs: &[(&'static str, &'static str)], id: &str) -> Option<(&'static str, &'static str)> {
    docs.binary_search_by(|c| c.0.cmp(id)).ok().and_then(|i| docs.get(i)).copied()
}

/// Whether `hay` contains `needle`, ignoring ASCII case.
fn contains_ignore_case(hay: &str, needle: &str) -> bool {
    let (h, n) = (hay.as_bytes(), needle.as_bytes());
    n.is_empty() || h.windows(n.len()).any(|w| w.eq_ignore_ascii_case(n))
}

/// Up to [`MAX_SUGGESTIONS`] names matching `prefix`: those starting with it first (in list order), then those
/// containing it in any ASCII case.
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

/// Where the end of `json` (the parameters typed so far) stands: `Some(start)` inside a JSON string that holds a
/// value (after `:` or inside a list), with the byte position where the string's text starts; `None` outside a
/// string or inside an object key.
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
                if stack.len() >= MAX_COMPLETION_DEPTH {
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

/// Completes the end of `line`: the history (newest first) and then every command id while the line is empty,
/// command ids for the first word (or after `--cmd`), glyph names inside a JSON string value of the parameters.
pub fn complete(docs: &[(&'static str, &'static str)], glyphs: &[String], history: &[String], line: &str) -> Completion {
    if line.len() > MAX_LINE_BYTES {
        return Completion::default();
    }
    let ws = words(line);
    let ends_in_space = line.chars().last().is_some_and(char::is_whitespace);
    let current = if ends_in_space { None } else { ws.last().copied() };
    let commands = || docs.iter().map(|c| c.0);
    let glyph_completion = |prefix: &str, start: usize, doc| Completion {
        start,
        items: matching(glyphs.iter().map(String::as_str), prefix, SuggestionKind::Glyph),
        doc,
    };
    let id_completion = |(start, prefix): (usize, &str)| {
        let items = matching(commands(), prefix, SuggestionKind::Command);
        let doc = command_doc(docs, prefix).or_else(|| items.first().and_then(|s| command_doc(docs, &s.text)));
        Completion { start, items, doc }
    };
    if is_run_line(&ws) {
        let before = if current.is_some() { ws.len().saturating_sub(1) } else { ws.len() };
        let prior = ws.get(..before).unwrap_or_default();
        if prior.last().is_some_and(|w| w.1 == "--cmd") {
            return id_completion(current.unwrap_or((line.len(), "")));
        }
        let named = prior.iter().rposition(|w| w.1 == "--cmd").and_then(|i| prior.get(i + 1)).and_then(|w| command_doc(docs, w.1));
        let params_at = ws.iter().rposition(|w| w.1 == "--params").and_then(|i| ws.get(i + 1)).map(|w| w.0);
        if let Some(at) = params_at {
            let tail = line.get(at..).unwrap_or_default();
            // The parameters may be quoted for the shell; the JSON starts at the first brace.
            if let Some(brace) = tail.find('{')
                && let Some(start) = open_value_string(tail.get(brace..).unwrap_or_default())
            {
                let start = at + brace + start;
                return glyph_completion(line.get(start..).unwrap_or_default(), start, named);
            }
        }
        return Completion { start: line.len(), items: Vec::new(), doc: named };
    }
    match ws.as_slice() {
        [] => {
            let mut completion = id_completion((line.len(), ""));
            let recent = history.iter().rev().take(MAX_SUGGESTIONS).map(|text| Suggestion { text: text.clone(), kind: SuggestionKind::History });
            completion.items.splice(0..0, recent);
            completion.items.truncate(MAX_SUGGESTIONS);
            completion.start = 0;
            completion
        }
        [only] if current.is_some() => id_completion(*only),
        [(_, id), ..] => {
            let doc = command_doc(docs, id);
            let after_id = ws.get(1).map_or(line.len(), |w| w.0);
            let tail = line.get(after_id..).unwrap_or_default();
            match open_value_string(tail) {
                Some(start) => {
                    let start = after_id + start;
                    glyph_completion(line.get(start..).unwrap_or_default(), start, doc)
                }
                None => Completion { start: line.len(), items: Vec::new(), doc },
            }
        }
    }
}

/// Splits a run-style line into words as a shell does: whitespace separates, single quotes keep their text as it
/// is, double quotes and backslashes escape.
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

/// Runs a console line: adds it to the history, then runs its commands. A line naming several commands runs them
/// as one `command.batch` (one transaction and one undo step: if one fails, none takes effect), so it may name
/// engine commands only. Outcomes the session's log does not record itself (shell commands, tools, lines that
/// cannot be read) are logged here, so the pane shows every outcome.
pub fn run_line(app: &mut FontCraftApp, line: &str) -> Result<Value, String> {
    if line.len() > MAX_LINE_BYTES {
        return Err(format!("a console line has at most {MAX_LINE_BYTES} bytes"));
    }
    app.view.commands.console.remember(line);
    let first = line.split_whitespace().next().unwrap_or_default().chars().take(MAX_ID_BYTES).collect::<String>();
    let steps = parse_line(line).and_then(|steps| {
        if steps.len() > 1
            && let Some((id, _)) = steps.iter().find(|(id, _)| catalog::kind(id) != Kind::Engine)
        {
            return Err(format!("`{id}` is a window command: a line with several commands runs engine commands only"));
        }
        Ok(steps)
    });
    let batched = steps.as_ref().is_ok_and(|s| s.len() > 1);
    let (id, params) = match steps {
        Ok(steps) if steps.is_empty() => return Ok(Value::Null),
        Ok(mut steps) if steps.len() == 1 => steps.pop().unwrap_or_default(),
        Ok(steps) => {
            let steps: Vec<Value> = steps.into_iter().map(|(id, params)| json!({"command": id, "params": params})).collect();
            ("command.batch".to_string(), json!({"label": "Console", "steps": steps}))
        }
        Err(e) => {
            app.session.extensions_mut().log.push_command(&first, &Value::Null, Err(e.clone()));
            return Err(e);
        }
    };
    let before = app.session.extensions().log.next_seq();
    let result = match catalog::kind(&id) {
        Kind::Engine => app.session.execute(&id, &params).map_err(|e| e.to_string()),
        Kind::Tool(own) => {
            let command = craft::ui::runtime::commands::CommandRef::with_params(id.clone(), params.clone());
            catalog::chosen_tool(own, &command).map(|tool| {
                app.sent.push(Msg::View(ViewMsg::SelectTool(tool)));
                Value::Null
            })
        }
        Kind::Shell if id == CONSOLE_COMMAND && params.get("line").is_some_and(|l| !l.is_null()) => {
            Err("a console line cannot run another console line".to_string())
        }
        Kind::Shell => {
            let command = craft::ui::runtime::commands::CommandRef::with_params(id.clone(), params.clone());
            crate::shell::commands::run(app, &command).map(|()| Value::Null)
        }
    };
    if app.session.extensions().log.next_seq() == before {
        app.session.extensions_mut().log.push_command(&id, &params, result.as_ref().map_err(Clone::clone));
    }
    match result {
        Ok(v) if batched => Ok(v.get("results").cloned().unwrap_or(v)),
        other => other,
    }
}

/// The pane's text from the log entry `from` on, one line per entry, cut at [`MAX_COPY_BYTES`].
pub fn pane_text(log: &CommandLog, from: u64) -> String {
    let mut out = String::new();
    for entry in log.from_seq(from) {
        let line = entry.to_line();
        if out.len() + line.len() + 1 > MAX_COPY_BYTES {
            break;
        }
        out.push_str(&line);
        out.push('\n');
    }
    out
}

/// The command palette's entries: every engine and shell command that has a menu row, with its menu path.
pub fn palette_entries() -> Vec<HelpEntry> {
    let engine = fontcraft_engine::command_specs().iter().filter(|s| !s.menu.is_empty()).map(|s| (s.id, s.label, s.menu));
    let shell =
        crate::shell::commands::MODULES.iter().flat_map(|m| m.commands.iter()).filter(|c| !c.menu.is_empty()).map(|c| (c.id, c.label, c.menu));
    engine.chain(shell).map(|(id, label, menu)| HelpEntry::new(id, label).with_path(menu.iter().copied())).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A plain line, a run-style line with shell quoting, and the lines the console refuses.
    #[test]
    fn lines_parse_into_commands() {
        assert_eq!(parse_line("glyph.newGlyph {\"name\": \"a\"}").unwrap(), vec![("glyph.newGlyph".to_string(), json!({"name": "a"}))]);
        let run = parse_line("fontcraft-cli run --cmd glyph.newGlyph --params '{\"name\": \"b c\"}' --cmd edit.undo").unwrap();
        assert_eq!(run, vec![("glyph.newGlyph".to_string(), json!({"name": "b c"})), ("edit.undo".to_string(), json!({}))]);
        assert!(parse_line("").unwrap().is_empty());
        assert!(parse_line("glyph.newGlyph [1]").unwrap_err().contains("JSON object"));
        assert!(parse_line("run --params {}").unwrap_err().contains("--cmd"));
        assert!(parse_line("run --cmd a --export x.ttf").unwrap_err().contains("--export"));
        assert!(parse_line("run --cmd 'a").unwrap_err().contains("quote"));
    }

    /// Command ids complete at the start, glyph names inside a string value, and an empty line offers the history.
    #[test]
    fn completion_offers_ids_glyphs_and_history() {
        let docs = command_docs();
        let glyphs = vec!["A".to_string(), "Aacute".to_string(), "b".to_string()];
        let history = vec!["edit.undo".to_string(), "glyph.newGlyph {\"name\": \"x\"}".to_string()];
        let ids = complete(&docs, &glyphs, &history, "glyph.newG");
        assert_eq!(ids.items.first().map(|s| (s.text.as_str(), s.kind)), Some(("glyph.newGlyph", SuggestionKind::Command)));
        assert_eq!(ids.doc.map(|d| d.0), Some("glyph.newGlyph"));
        let names = complete(&docs, &glyphs, &history, "glyph.delete {\"name\": \"Aa");
        assert_eq!(names.items, vec![Suggestion { text: "Aacute".into(), kind: SuggestionKind::Glyph }]);
        assert_eq!(names.items.first().and_then(Suggestion::insert).as_deref(), Some("Aacute\""));
        let empty = complete(&docs, &glyphs, &history, "");
        assert_eq!(empty.items.first().map(|s| (s.text.as_str(), s.kind)), Some(("glyph.newGlyph {\"name\": \"x\"}", SuggestionKind::History)));
        assert!(empty.items.len() <= MAX_SUGGESTIONS);
        assert!(complete(&docs, &glyphs, &history, "glyph.delete {\"na").items.is_empty(), "a key is not completed");
    }

    /// A console line runs through `drain`, joins the history and the pane; a bad line reaches the pane and the
    /// status line; Clear empties the pane; the palette opens over the menu commands.
    #[test]
    fn console_lines_run_log_and_reach_the_pane() {
        use std::sync::Arc;

        use craft::host::Host;
        use craft::ui::runtime::commands::CommandRef;
        use fontcraft_engine::Session;

        use crate::app::APP;

        let mut app = FontCraftApp::with_edit_tab(Session::with_document(), Arc::new(Host::builder(APP).build()));
        let console = |params: Value| Msg::Shell(CommandRef::with_params(CONSOLE_COMMAND, params));
        let line = "glyph.newGlyph {\"name\": \"a.alt\"}";
        app.sent.push(console(json!({})));
        app.sent.push(console(json!({ "line": line })));
        app.drain();
        assert!(app.status.is_empty(), "{}", app.status);
        assert!(app.session.font().unwrap().glyphs.iter().any(|g| g.name == "a.alt"));
        assert_eq!(app.view.commands.console.history, vec![line.to_string()]);
        assert!(app.models.console.lines.last().unwrap().starts_with("glyph.newGlyph"), "{:?}", app.models.console.lines);
        assert!(app.models.console.glyphs.iter().any(|g| g == "a.alt"));

        app.sent.push(console(json!({ "line": "glyph.newGlyph [1]" })));
        app.drain();
        assert!(app.status.contains("JSON object"), "{}", app.status);
        assert!(app.models.console.lines.last().unwrap().contains("failed"), "{:?}", app.models.console.lines);

        app.sent.push(console(json!({ "clear": true, "copy": true })));
        app.drain();
        assert!(app.models.console.lines.is_empty());
        assert_eq!(app.view.commands.console.copy_out.as_deref(), Some(""));

        app.sent.push(Msg::Shell(CommandRef::new("app.commandPalette")));
        app.drain();
        assert!(app.view.commands.palette_open);
        assert!(app.view.commands.palette.iter().any(|e| e.command().id == "edit.undo"));
    }

    /// The history skips repeats and blank lines and drops the oldest past its limit.
    #[test]
    fn history_skips_repeats_and_keeps_its_limit() {
        let mut state = ConsoleState::default();
        state.remember("a");
        state.remember(" a ");
        state.remember("   ");
        assert_eq!(state.history, vec!["a".to_string()]);
        for i in 0..MAX_HISTORY + 5 {
            state.remember(&i.to_string());
        }
        assert_eq!(state.history.len(), MAX_HISTORY);
        assert_eq!(state.history.last().map(String::as_str), Some((MAX_HISTORY + 4).to_string().as_str()));
    }
}
