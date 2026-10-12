//! The command log: the last [`MAX_LOG_ENTRIES`] commands run from any surface, with their result
//! or error, and module log lines. The Command Console and `app.info` read it. Plan:
//! `docs/plans/M13.md` § 3 (command log), § 4 › M13.9.
//!
//! The hook in `Session::execute` adds an entry for every top-level command it runs, queries
//! included; previews of an interaction are not logged. Results and parameters are kept cut at
//! [`MAX_LOG_RESULT_BYTES`] of JSON, so one huge result cannot hold memory for the whole session.

use std::collections::VecDeque;
use std::io::Write;

use serde::Serialize;
use serde_json::Value;

/// Most entries kept; the oldest are dropped.
pub const MAX_LOG_ENTRIES: usize = 10_000;
/// Longest result or parameter JSON kept per entry, in bytes; longer ones are replaced by a
/// cut string ending in `…`.
pub const MAX_LOG_RESULT_BYTES: usize = 64 << 10;
/// Longest command id or module id kept, in bytes.
pub const MAX_LOG_NAME_BYTES: usize = 256;

/// What an entry records.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum LogKind {
    /// A command and its outcome: `result` on success, `error` on failure.
    Command {
        command: String,
        params: Value,
        #[serde(skip_serializing_if = "Option::is_none")]
        result: Option<Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<String>,
    },
    /// A line a module logged (a filter's `log`).
    Module { module: String, line: String },
}

/// One log entry.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct LogEntry {
    /// Increases by one per entry over the session; readers ask for entries after a number.
    pub seq: u64,
    #[serde(flatten)]
    pub kind: LogKind,
}

impl LogEntry {
    /// The entry as one line of text, as the console shows it and standard error receives it:
    /// `id {params} → result`, `id {params} failed: error`, or `[module] line`. Line breaks
    /// inside an error or a module line become spaces.
    pub fn to_line(&self) -> String {
        match &self.kind {
            LogKind::Command { command, params, result, error } => {
                let mut out = command.clone();
                if !is_empty_params(params) {
                    out.push(' ');
                    out.push_str(&compact(params));
                }
                match (result, error) {
                    (_, Some(error)) => {
                        out.push_str(" failed: ");
                        out.push_str(&one_line(error));
                    }
                    (Some(result), None) if !result.is_null() => {
                        out.push_str(" → ");
                        out.push_str(&compact(result));
                    }
                    _ => out.push_str(" → done"),
                }
                out
            }
            LogKind::Module { module, line } => format!("[{module}] {}", one_line(line)),
        }
    }

    /// Whether the entry records a failure.
    pub fn is_error(&self) -> bool {
        matches!(&self.kind, LogKind::Command { error: Some(_), .. })
    }
}

fn is_empty_params(params: &Value) -> bool {
    match params {
        Value::Null => true,
        Value::Object(m) => m.is_empty(),
        _ => false,
    }
}

/// A kept value as text: a cut value is already a string and shows as it is.
fn compact(v: &Value) -> String {
    match v {
        Value::String(s) if s.ends_with('…') => s.clone(),
        other => other.to_string(),
    }
}

fn one_line(s: &str) -> String {
    s.chars().map(|c| if c == '\n' || c == '\r' { ' ' } else { c }).collect()
}

/// `s` cut to at most `max` bytes at a character boundary, with `…` when cut.
fn cut_str(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    let mut out = s.get(..end).unwrap_or_default().to_string();
    out.push('…');
    out
}

/// A writer that keeps at most `limit` bytes and fails past them, so serializing a huge value
/// stops early.
struct Capped {
    buf: Vec<u8>,
    limit: usize,
}

impl Write for Capped {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        let room = self.limit.saturating_sub(self.buf.len());
        if data.len() > room {
            self.buf.extend_from_slice(data.get(..room).unwrap_or_default());
            return Err(std::io::Error::other("over the log limit"));
        }
        self.buf.extend_from_slice(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// `v` as kept in the log: itself when its JSON fits in [`MAX_LOG_RESULT_BYTES`], else the
/// start of its JSON as a string ending in `…`.
fn kept(v: &Value) -> Value {
    let mut w = Capped { buf: Vec::new(), limit: MAX_LOG_RESULT_BYTES };
    if serde_json::to_writer(&mut w, v).is_ok() {
        return v.clone();
    }
    let text = match std::str::from_utf8(&w.buf) {
        Ok(s) => s.to_string(),
        Err(e) => String::from_utf8_lossy(w.buf.get(..e.valid_up_to()).unwrap_or_default()).into_owned(),
    };
    let mut text = cut_str(&text, MAX_LOG_RESULT_BYTES);
    if !text.ends_with('…') {
        text.push('…');
    }
    Value::String(text)
}

/// The ring buffer of log entries.
#[derive(Clone, Debug, Default)]
pub struct CommandLog {
    entries: VecDeque<LogEntry>,
    next_seq: u64,
}

impl CommandLog {
    pub fn new() -> Self {
        Self::default()
    }

    /// Logs a command and its outcome.
    pub fn push_command(&mut self, command: &str, params: &Value, outcome: Result<&Value, String>) {
        let (result, error) = match outcome {
            Ok(v) => (Some(kept(v)), None),
            Err(e) => (None, Some(cut_str(&e, MAX_LOG_RESULT_BYTES))),
        };
        self.push(LogKind::Command { command: cut_str(command, MAX_LOG_NAME_BYTES), params: kept(params), result, error });
    }

    /// Logs a line from module `module`.
    pub fn push_module_line(&mut self, module: &str, line: &str) {
        self.push(LogKind::Module { module: cut_str(module, MAX_LOG_NAME_BYTES), line: cut_str(line, MAX_LOG_RESULT_BYTES) });
    }

    fn push(&mut self, kind: LogKind) {
        self.entries.push_back(LogEntry { seq: self.next_seq, kind });
        self.next_seq = self.next_seq.saturating_add(1);
        while self.entries.len() > MAX_LOG_ENTRIES {
            self.entries.pop_front();
        }
    }

    /// The entries with a sequence number above `seq`, oldest first.
    pub fn since(&self, seq: u64) -> impl Iterator<Item = &LogEntry> {
        // Sequence numbers are consecutive, so the first wanted entry's position is known.
        let skip = self.entries.front().map_or(0, |first| usize::try_from(seq.saturating_add(1).saturating_sub(first.seq)).unwrap_or(usize::MAX));
        self.entries.iter().skip(skip).filter(move |e| e.seq > seq)
    }

    /// The entries with a sequence number of `seq` or more, oldest first.
    pub fn from_seq(&self, seq: u64) -> impl Iterator<Item = &LogEntry> {
        let skip = self.entries.front().map_or(0, |first| usize::try_from(seq.saturating_sub(first.seq)).unwrap_or(usize::MAX));
        self.entries.iter().skip(skip).filter(move |e| e.seq >= seq)
    }

    /// The entry at `index`, oldest first.
    pub fn get(&self, index: usize) -> Option<&LogEntry> {
        self.entries.get(index)
    }

    /// The position of the first entry with a sequence number of `seq` or more (the length when
    /// there is none).
    pub fn position_of(&self, seq: u64) -> usize {
        self.entries.front().map_or(0, |first| usize::try_from(seq.saturating_sub(first.seq)).unwrap_or(usize::MAX).min(self.entries.len()))
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The sequence number the next entry gets.
    pub fn next_seq(&self) -> u64 {
        self.next_seq
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn the_log_keeps_the_newest_entries_and_cuts_long_results() {
        let mut log = CommandLog::new();
        for i in 0..MAX_LOG_ENTRIES + 5 {
            log.push_command("glyph.newGlyph", &json!({"name": format!("g{i}")}), Ok(&json!({"ok": true})));
        }
        assert_eq!(log.len(), MAX_LOG_ENTRIES);
        assert_eq!(log.next_seq(), (MAX_LOG_ENTRIES + 5) as u64);
        assert_eq!(log.get(0).map(|e| e.seq), Some(5));
        assert_eq!(log.since(MAX_LOG_ENTRIES as u64 + 2).count(), 2);
        assert_eq!(log.from_seq(0).count(), MAX_LOG_ENTRIES);
        assert_eq!(log.position_of(7), 2);

        let big = json!({"text": "é".repeat(MAX_LOG_RESULT_BYTES)});
        log.push_command("font.get", &Value::Null, Ok(&big));
        log.push_command("glyph.delete", &json!({"name": "x"}), Err("no glyph named x\nin the font".into()));
        log.push_module_line("com.example.round", "rounded 12 points");
        let tail: Vec<&LogEntry> = log.since(log.next_seq() - 4).collect();
        let [cut, failed, module] = tail.as_slice() else { panic!("three entries") };
        let LogKind::Command { result: Some(Value::String(text)), .. } = &cut.kind else { panic!("a cut result") };
        assert!(text.len() <= MAX_LOG_RESULT_BYTES + '…'.len_utf8() && text.ends_with('…'));
        assert!(failed.is_error());
        assert_eq!(failed.to_line(), "glyph.delete {\"name\":\"x\"} failed: no glyph named x in the font");
        assert_eq!(module.to_line(), "[com.example.round] rounded 12 points");
        assert_eq!(log.get(log.len() - 4).map(LogEntry::to_line).as_deref(), Some("glyph.newGlyph {\"name\":\"g10004\"} → {\"ok\":true}"));
    }
}
