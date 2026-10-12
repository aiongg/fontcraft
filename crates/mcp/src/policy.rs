//! What an MCP client may run: `--read-only`, and `--allow` and `--deny` patterns over command ids.
//!
//! Whether a command is a query and whether it writes to disk come from the command catalogue
//! (`query`, `writes_files`, `file_params`): the engine declares them for every command. Which
//! files a command may reach is the engine's business: the automation roots
//! (`fontcraft_engine::file_access`), which the server puts in force around every request.
//!
//! Every check fails closed: a command the policy cannot place (an unknown id under
//! `--read-only`, a macro whose steps it cannot see) is refused, never run.

use serde_json::Value;

/// Most `--allow` or `--deny` patterns.
pub const MAX_PATTERNS: usize = 256;
/// Longest pattern, in bytes.
pub const MAX_PATTERN_LEN: usize = 256;
/// Deepest nesting of `command.batch` inside `command.batch`.
pub const MAX_BATCH_NESTING: usize = 8;

/// Commands that run other commands or open files the server cannot see in their parameters
/// (macro files, recent-document lists, URLs). They are refused whenever the policy restricts
/// anything, and by the engine while automation roots are in force.
pub use fontcraft_engine::file_access::command::INDIRECT_COMMANDS;

/// The batch command: its steps are checked one by one.
pub const BATCH_COMMAND: &str = "command.batch";

/// What the policy knows about one command, from its catalogue entry.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CommandFacts {
    /// Reads and changes nothing in the session (it may still write files).
    pub query: bool,
    /// Writes to disk.
    pub writes_files: bool,
    /// Declares a parameter it writes a file to, or one whose access this server does not know.
    pub writes_param: bool,
    /// The entry's file declarations could not be read; the command counts as one that writes.
    pub unreadable: bool,
}

impl CommandFacts {
    /// Reads one entry of the command catalogue; `None` when it has no id.
    pub fn from_entry(entry: &Value) -> Option<(String, Self)> {
        let id = entry.get("id")?.as_str()?.to_string();
        let query = entry.get("query").and_then(Value::as_bool).unwrap_or(false);
        let writes_files = entry.get("writes_files").and_then(Value::as_bool).unwrap_or(false);
        let (writes_param, unreadable) = match entry.get("file_params") {
            None | Some(Value::Null) => (false, false),
            // An access this server does not know counts as a write.
            Some(Value::Array(list)) => (list.iter().any(|f| f.get("access").and_then(Value::as_str) != Some("read")), false),
            Some(_) => (false, true),
        };
        Some((id, Self { query, writes_files, writes_param, unreadable }))
    }

    /// Whether the command may write to disk.
    pub fn writes(&self) -> bool {
        self.writes_files || self.unreadable || self.writes_param
    }
}

#[derive(Clone, Debug, Default)]
pub struct Policy {
    read_only: bool,
    allow: Vec<String>,
    deny: Vec<String>,
}

impl Policy {
    /// Admits every command.
    pub fn unrestricted() -> Self {
        Self::default()
    }

    /// Admits only commands that declare themselves queries and write no files.
    pub fn read_only(mut self, on: bool) -> Self {
        self.read_only = on;
        self
    }

    /// Adds an `--allow` pattern: once there is one, a command must match one of them.
    pub fn allow(mut self, pattern: &str) -> Result<Self, String> {
        check_pattern(pattern, self.allow.len())?;
        self.allow.push(pattern.to_string());
        Ok(self)
    }

    /// Adds a `--deny` pattern: a command that matches one is refused, even when an allow pattern
    /// matches it too.
    pub fn deny(mut self, pattern: &str) -> Result<Self, String> {
        check_pattern(pattern, self.deny.len())?;
        self.deny.push(pattern.to_string());
        Ok(self)
    }

    /// Whether any command could be refused.
    pub fn is_restricted(&self) -> bool {
        self.read_only || !self.allow.is_empty() || !self.deny.is_empty()
    }

    /// Whether the checks need the command catalogue (queries, file declarations).
    pub(crate) fn needs_catalogue(&self) -> bool {
        self.read_only
    }

    /// One paragraph for `initialize`'s instructions, or `None` when nothing is restricted.
    pub fn describe(&self) -> Option<String> {
        let mut parts = Vec::new();
        if self.read_only {
            parts.push("only commands that read (queries) and write no files run".to_string());
        }
        if !self.allow.is_empty() {
            parts.push(format!("only commands matching {} run", self.allow.join(", ")));
        }
        if !self.deny.is_empty() {
            parts.push(format!("commands matching {} are refused", self.deny.join(", ")));
        }
        if self.is_restricted() {
            parts.push("macros and other commands that run steps this server cannot see are refused".to_string());
        }
        (!parts.is_empty()).then(|| format!("This server is restricted: {}.", parts.join("; ")))
    }

    /// Decides one command by id. `facts` is its catalogue entry (`None`: not in the catalogue,
    /// or the catalogue was not read because no check needs it).
    pub fn admit(&self, id: &str, facts: Option<&CommandFacts>) -> Result<(), String> {
        if let Some(p) = self.deny.iter().find(|p| glob(p, id)) {
            return Err(format!("`{id}` is refused by this server (--deny {p})"));
        }
        if !self.allow.is_empty() && !self.allow.iter().any(|p| glob(p, id)) {
            return Err(format!("`{id}` is not among the commands this server allows (--allow {})", self.allow.join(", ")));
        }
        if self.read_only {
            match facts {
                None => return Err(format!("`{id}` is not a known query, and this server is read-only")),
                Some(f) if !f.query => return Err(format!("`{id}` changes documents, files or settings, and this server is read-only")),
                Some(f) if f.writes() => return Err(format!("`{id}` writes files, and this server is read-only")),
                Some(_) => {}
            }
        }
        if self.is_restricted() && INDIRECT_COMMANDS.contains(&id) {
            return Err(format!(
                "`{id}` runs steps or opens files this server cannot check against its restrictions; run the steps one by one, or with command.batch"
            ));
        }
        Ok(())
    }
}

fn check_pattern(pattern: &str, count: usize) -> Result<(), String> {
    if count >= MAX_PATTERNS {
        return Err(format!("at most {MAX_PATTERNS} patterns"));
    }
    if pattern.is_empty() || pattern.len() > MAX_PATTERN_LEN {
        return Err(format!("a pattern has 1 to {MAX_PATTERN_LEN} bytes"));
    }
    Ok(())
}

/// Shell-style match over a whole command id: `*` is any run of characters, `?` one character,
/// everything else itself. Linear backtracking over the last `*`, so the work is at most the
/// product of the two lengths.
pub fn glob(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let (mut pi, mut ti) = (0usize, 0usize);
    let mut star: Option<(usize, usize)> = None;
    while ti < t.len() {
        match p.get(pi) {
            Some('*') => {
                star = Some((pi, ti));
                pi += 1;
            }
            Some(c) if *c == '?' || Some(c) == t.get(ti) => {
                pi += 1;
                ti += 1;
            }
            _ => match star {
                Some((sp, st)) => {
                    pi = sp + 1;
                    ti = st + 1;
                    star = Some((sp, st + 1));
                }
                None => return false,
            },
        }
    }
    p.get(pi..).unwrap_or_default().iter().all(|c| *c == '*')
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn facts(query: bool, writes_files: bool, writes_param: bool) -> CommandFacts {
        CommandFacts { query, writes_files, writes_param, unreadable: false }
    }

    #[test]
    fn glob_matches_whole_ids() {
        assert!(glob("glyph.*", "glyph.newGlyph"));
        assert!(!glob("glyph.*", "font.glyph.x"));
        assert!(glob("*.list*", "font.master.list"));
        assert!(glob("kerning.?etPair", "kerning.setPair"));
        assert!(!glob("file.save", "file.saveAs"));
        assert!(glob("*", ""));
        assert!(!glob("a*b*c", &"ab".repeat(200)));
    }

    #[test]
    fn deny_beats_allow() {
        let p = Policy::unrestricted().allow("glyph.*").unwrap().deny("glyph.remove*").unwrap();
        assert!(p.admit("glyph.newGlyph", None).is_ok());
        assert!(p.admit("glyph.removeGlyph", None).unwrap_err().contains("--deny"));
        assert!(p.admit("font.info.set", None).unwrap_err().contains("--allow"));
    }

    #[test]
    fn read_only_admits_only_queries_that_write_nothing_and_restrictions_refuse_macros() {
        let p = Policy::unrestricted().read_only(true);
        assert!(p.admit("glyph.getInfo", Some(&facts(true, false, false))).is_ok());
        assert!(p.admit("glyph.newGlyph", Some(&facts(false, false, false))).is_err());
        assert!(p.admit("made.up", None).is_err());
        // A query that writes files: flagged, or with a parameter it writes to.
        assert!(p.admit("tables.delete", Some(&facts(true, true, false))).unwrap_err().contains("writes files"));
        assert!(p.admit("font.dumpTables", Some(&facts(true, false, true))).is_err());
        assert!(p.admit("tables.list", Some(&facts(true, false, false))).is_ok());
        assert!(Policy::unrestricted().admit("script.run", None).is_ok());
        assert!(Policy::unrestricted().deny("x").unwrap().admit("script.run", None).is_err());
    }

    #[test]
    fn catalogue_entries_say_whether_a_command_writes() {
        let (_, read) =
            CommandFacts::from_entry(&json!({"id": "a.b", "query": true, "file_params": [{"name": "font", "access": "read", "form": "path"}]}))
                .unwrap();
        assert_eq!(read, facts(true, false, false));
        let (_, odd) = CommandFacts::from_entry(&json!({"id": "a.b", "file_params": [{"name": "x", "access": "?", "form": "?"}]})).unwrap();
        assert!(odd.writes(), "an access this server does not know counts as a write");
        assert!(CommandFacts::from_entry(&json!({"id": "a.b", "file_params": "x"})).unwrap().1.writes());
    }
}
