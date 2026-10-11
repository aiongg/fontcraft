//! The command log: the last [`MAX_LOG_ENTRIES`] commands run from any surface, with their result
//! or error, and module log lines (M13.9 fills the bodies; the Command Console and `app.info`
//! read it). Plan: `docs/plans/M13.md` § 3 (command log), § 4 › M13.9.
//!
//! The hook in `Session::execute` adds an entry for every top-level command it runs, queries
//! included; previews of an interaction are not logged. Results are kept cut at
//! [`MAX_LOG_RESULT_BYTES`] of JSON.

use std::collections::VecDeque;

use serde::Serialize;
use serde_json::Value;

/// Most entries kept; the oldest are dropped.
pub const MAX_LOG_ENTRIES: usize = 10_000;
/// Longest result or parameter JSON kept per entry, in bytes; longer ones are replaced by a
/// cut string ending in `…`.
pub const MAX_LOG_RESULT_BYTES: usize = 64 << 10;

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
        let _ = (command, params, outcome);
        // M13.9: cut long JSON, push, drop past MAX_LOG_ENTRIES.
    }

    /// Logs a line from module `module`.
    pub fn push_module_line(&mut self, module: &str, line: &str) {
        let _ = (module, line);
        // M13.9.
    }

    /// The entries with a sequence number above `seq`, oldest first.
    pub fn since(&self, seq: u64) -> impl Iterator<Item = &LogEntry> {
        self.entries.iter().filter(move |e| e.seq > seq)
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
