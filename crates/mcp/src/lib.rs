//! FontCraft MCP server (L6): the [Model Context Protocol](https://modelcontextprotocol.io) over
//! stdio, newline-delimited JSON-RPC 2.0. See `docs/mcp.md`.
//!
//! Two backends answer the tools:
//! - [`Headless`]: an engine session inside this process.
//! - [`Remote`]: a running FontCraft app, reached over its control port (`fontcraft --control`),
//!   so the agent's changes appear in the window.
//!
//! Besides the tools the server offers workflow prompts ([`prompts`]), resources and argument
//! completion ([`resources`]) and log notifications. A [`Policy`] decides which commands and file
//! paths a client may use; everything it cannot place is refused.
//!
//! The server reads one line and writes one reply, preceded by the log notifications the request
//! produced. It never panics on input: malformed JSON, wrong types and unknown methods get
//! JSON-RPC errors, and a bug inside a tool becomes an error result.
#![forbid(unsafe_code)]

mod backend;
pub mod policy;
mod prompts;
mod resources;

use std::collections::HashMap;
use std::io::{BufRead, Write};

pub use backend::{Backend, DEFAULT_ADDR, Headless, Remote};
use fontcraft_engine::guard;
pub use policy::Policy;
use serde_json::{Value, json};

/// Protocol revisions this server speaks, newest first. `2025-03-26` is left out on purpose: that
/// revision requires JSON-RPC batches, which this server does not accept.
pub const PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2024-11-05"];
/// Longest request line accepted.
pub const MAX_LINE: usize = 4 << 20;
/// Most log notifications held for one reply; later ones are dropped.
pub const MAX_PENDING_NOTIFICATIONS: usize = 1_000;
/// Most entries read from the backend's command catalogue.
pub const MAX_CATALOGUE: usize = 100_000;

const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;
const RESOURCE_NOT_FOUND: i64 = -32002;

/// The protocol's log levels (RFC 5424 severities), least severe first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum LogLevel {
    Debug,
    Info,
    Notice,
    Warning,
    Error,
    Critical,
    Alert,
    Emergency,
}

impl LogLevel {
    const ALL: [LogLevel; 8] = [
        LogLevel::Debug,
        LogLevel::Info,
        LogLevel::Notice,
        LogLevel::Warning,
        LogLevel::Error,
        LogLevel::Critical,
        LogLevel::Alert,
        LogLevel::Emergency,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            LogLevel::Debug => "debug",
            LogLevel::Info => "info",
            LogLevel::Notice => "notice",
            LogLevel::Warning => "warning",
            LogLevel::Error => "error",
            LogLevel::Critical => "critical",
            LogLevel::Alert => "alert",
            LogLevel::Emergency => "emergency",
        }
    }

    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|l| l.as_str() == text)
    }
}

pub struct Server {
    backend: Box<dyn Backend>,
    policy: Policy,
    /// Command id → what the policy needs to know about it, read from the backend when the
    /// policy first needs it.
    catalogue: Option<HashMap<String, policy::CommandFacts>>,
    /// Least severe level sent as a notification; `None` until the client sets one.
    log_level: Option<LogLevel>,
    /// Notifications produced while answering the current request.
    pending: Vec<Value>,
}

/// What one request's checks have already looked up.
#[derive(Default)]
struct Seen {
    /// Every open document's file was checked against the roots.
    documents: bool,
    /// The open projects (`project.list`).
    projects: Option<Value>,
    /// An earlier step of the batch opens a project, whose stored places the server has not seen.
    project_opened: bool,
}

/// The tools offered to agents. `run_command` reaches every command; the others save a round trip.
fn tools() -> Value {
    json!([
        {
            "name": "list_commands",
            "description": "List FontCraft commands: id, label, menu path, shortcut, a description of the parameters each takes and what it returns, and whether it is enabled now. Call this first to find the command for a task.",
            "inputSchema": {"type": "object", "properties": {"pattern": {"type": "string", "description": "Only commands whose id or label contains this text (case-insensitive)."}}}
        },
        {
            "name": "run_command",
            "description": "Run any FontCraft command by id with JSON parameters, as listed by list_commands. Changes are undoable (edit.undo). A command that fails changes nothing.",
            "inputSchema": {"type": "object", "required": ["command"], "properties": {"command": {"type": "string", "description": "A command id, for example glyph.newGlyph."}, "params": {"type": "object", "description": "The command's parameters."}}}
        },
        {
            "name": "inspect_document",
            "description": "Summarise the open fonts: title, file path, unsaved changes, revision number (it grows with every change), family name, glyph count, masters, and the undo and redo steps.",
            "inputSchema": {"type": "object", "properties": {}}
        }
    ])
}

fn error(id: &Value, code: i64, message: impl Into<String>) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message.into()}})
}

/// List methods take a cursor only to continue a list; this server returns every list whole.
fn no_cursor(params: &Value) -> Result<(), (i64, String)> {
    match params.get("cursor") {
        None | Some(Value::Null) => Ok(()),
        Some(_) => Err((INVALID_PARAMS, "unknown cursor: this server returns whole lists".into())),
    }
}

impl Server {
    /// A server that admits every command and path.
    pub fn new(backend: Box<dyn Backend>) -> Self {
        Self::with_policy(backend, Policy::unrestricted())
    }

    pub fn with_policy(backend: Box<dyn Backend>, policy: Policy) -> Self {
        Self { backend, policy, catalogue: None, log_level: None, pending: Vec::new() }
    }

    /// The log notifications produced since the last call, as JSON-RPC lines. A transport sends
    /// them before the reply to the request that produced them.
    pub fn take_notifications(&mut self) -> Vec<String> {
        std::mem::take(&mut self.pending).into_iter().map(|n| n.to_string()).collect()
    }

    fn log(&mut self, level: LogLevel, data: Value) {
        if self.log_level.is_some_and(|min| level >= min) && self.pending.len() < MAX_PENDING_NOTIFICATIONS {
            self.pending.push(json!({
                "jsonrpc": "2.0",
                "method": "notifications/message",
                "params": {"level": level.as_str(), "logger": "fontcraft", "data": data},
            }));
        }
    }

    /// Answers one line. `None`: the line was a notification (or blank) and gets no reply.
    pub fn handle_line(&mut self, line: &str) -> Option<String> {
        if line.trim().is_empty() {
            return None;
        }
        let reply = if line.len() > MAX_LINE {
            Some(error(&Value::Null, INVALID_REQUEST, format!("a request line may be at most {MAX_LINE} bytes")))
        } else {
            match serde_json::from_str::<Value>(line) {
                Err(e) => Some(error(&Value::Null, PARSE_ERROR, format!("not valid JSON: {e}"))),
                Ok(request) => self.handle(&request),
            }
        };
        reply.map(|r| r.to_string())
    }

    fn handle(&mut self, request: &Value) -> Option<Value> {
        let Some(obj) = request.as_object() else {
            return Some(error(&Value::Null, INVALID_REQUEST, "a request is a JSON object (batches are not supported)"));
        };
        // A request without an id is a notification: it is never answered.
        let id = obj.get("id").cloned();
        if id.as_ref().is_some_and(|id| !(id.is_string() || id.is_number())) {
            return Some(error(&Value::Null, INVALID_REQUEST, "`id` must be a string or a number"));
        }
        // A response from the client (to a request this server never sends) is ignored.
        if !obj.contains_key("method") && (obj.contains_key("result") || obj.contains_key("error")) {
            return None;
        }
        let Some(method) = obj.get("method").and_then(Value::as_str) else {
            return Some(error(&id.unwrap_or(Value::Null), INVALID_REQUEST, "`method` (a string) is required"));
        };
        let params = obj.get("params").cloned().unwrap_or(Value::Null);
        let id = id?;
        let outcome = guard::catch_panic(|| self.dispatch(method, &params));
        Some(match outcome {
            Ok(Ok(result)) => json!({"jsonrpc": "2.0", "id": id, "result": result}),
            Ok(Err((code, message))) => error(&id, code, message),
            Err(panic) => {
                self.log(LogLevel::Error, json!({"method": method, "internalError": panic}));
                error(&id, -32603, format!("internal error (please report it): {panic}"))
            }
        })
    }

    fn dispatch(&mut self, method: &str, params: &Value) -> Result<Value, (i64, String)> {
        match method {
            "initialize" => {
                let asked = params.get("protocolVersion").and_then(Value::as_str).unwrap_or("");
                let version = PROTOCOL_VERSIONS.iter().find(|v| **v == asked).or(PROTOCOL_VERSIONS.first()).copied().unwrap_or("2025-06-18");
                let mut instructions = format!(
                    "FontCraft font editor ({}). Every action is a command: call list_commands, then run_command. Prompts walk through common workflows; resources read one glyph, command, custom parameter or feature.",
                    self.backend.describe()
                );
                if let Some(restricted) = self.policy.describe() {
                    instructions.push(' ');
                    instructions.push_str(&restricted);
                }
                Ok(json!({
                    "protocolVersion": version,
                    "capabilities": {"tools": {}, "prompts": {}, "resources": {}, "logging": {}, "completions": {}},
                    "serverInfo": {"name": "fontcraft", "version": env!("CARGO_PKG_VERSION")},
                    "instructions": instructions,
                }))
            }
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({"tools": tools()})),
            "tools/call" => {
                let name = params.get("name").and_then(Value::as_str).ok_or((INVALID_PARAMS, "`name` (a tool name) is required".to_string()))?;
                let args = params.get("arguments").cloned().unwrap_or(Value::Null);
                let result = match name {
                    "list_commands" => self.list_commands(args.get("pattern").and_then(Value::as_str)),
                    "run_command" => match args.get("command").and_then(Value::as_str) {
                        Some(command) => self.run_command(command, args.get("params").unwrap_or(&Value::Null)),
                        None => Err("`command` (a command id) is required".to_string()),
                    },
                    "inspect_document" => resources::Reader::document(self),
                    other => return Err((INVALID_PARAMS, format!("unknown tool `{other}`"))),
                };
                // A failed tool is a result the agent reads, not a protocol error.
                Ok(match result {
                    Ok(value) => json!({"content": [{"type": "text", "text": value.to_string()}], "isError": false}),
                    Err(message) => json!({"content": [{"type": "text", "text": message}], "isError": true}),
                })
            }
            "logging/setLevel" => {
                let level = params
                    .get("level")
                    .and_then(Value::as_str)
                    .and_then(LogLevel::parse)
                    .ok_or((INVALID_PARAMS, "`level` is one of debug, info, notice, warning, error, critical, alert, emergency".to_string()))?;
                self.log_level = Some(level);
                Ok(json!({}))
            }
            "prompts/list" => {
                no_cursor(params)?;
                Ok(prompts::list())
            }
            "prompts/get" => {
                let name = params.get("name").and_then(Value::as_str).ok_or((INVALID_PARAMS, "`name` (a prompt name) is required".to_string()))?;
                let prompt = prompts::find(name).ok_or_else(|| (INVALID_PARAMS, format!("unknown prompt `{name}`")))?;
                let refused: Vec<&str> = prompt.commands.iter().copied().filter(|c| self.admit(c).is_err()).collect();
                self.log(LogLevel::Debug, json!({"prompt": name}));
                prompts::get(prompt, params.get("arguments").unwrap_or(&Value::Null), &refused).map_err(|e| (INVALID_PARAMS, e))
            }
            "resources/list" => {
                no_cursor(params)?;
                Ok(resources::list())
            }
            "resources/templates/list" => {
                no_cursor(params)?;
                Ok(resources::templates())
            }
            "resources/read" => {
                let uri = params.get("uri").and_then(Value::as_str).ok_or((INVALID_PARAMS, "`uri` (a resource URI) is required".to_string()))?;
                if uri.len() > resources::MAX_URI_LEN {
                    return Err((INVALID_PARAMS, format!("a resource URI may be at most {} bytes", resources::MAX_URI_LEN)));
                }
                let resource = resources::parse_uri(uri).map_err(|e| (RESOURCE_NOT_FOUND, e))?;
                self.log(LogLevel::Debug, json!({"resource": uri}));
                resources::read(uri, &resource, self).map_err(|e| (RESOURCE_NOT_FOUND, e))
            }
            "completion/complete" => self.complete(params),
            other => Err((METHOD_NOT_FOUND, format!("method `{other}` is not supported"))),
        }
    }

    fn complete(&mut self, params: &Value) -> Result<Value, (i64, String)> {
        let bad = |m: &str| (INVALID_PARAMS, m.to_string());
        let reference = params.get("ref").ok_or_else(|| bad("`ref` is required"))?;
        let argument = params.get("argument").ok_or_else(|| bad("`argument` ({name, value}) is required"))?;
        let arg_name = argument.get("name").and_then(Value::as_str).ok_or_else(|| bad("`argument.name` is required"))?;
        let value = argument.get("value").and_then(Value::as_str).ok_or_else(|| bad("`argument.value` is required"))?;
        if value.len() > resources::MAX_COMPLETION_INPUT {
            return Err(bad("the value to complete is too long"));
        }
        let source = match reference.get("type").and_then(Value::as_str) {
            Some("ref/resource") => {
                let uri = reference.get("uri").and_then(Value::as_str).ok_or_else(|| bad("`ref.uri` is required"))?;
                Some(
                    resources::template_source(uri, arg_name)
                        .ok_or_else(|| (INVALID_PARAMS, format!("no resource template {uri} with the variable `{arg_name}`")))?,
                )
            }
            Some("ref/prompt") => {
                let name = reference.get("name").and_then(Value::as_str).ok_or_else(|| bad("`ref.name` is required"))?;
                let prompt = prompts::find(name).ok_or_else(|| (INVALID_PARAMS, format!("unknown prompt `{name}`")))?;
                let arg = prompt
                    .arguments
                    .iter()
                    .find(|a| a.name == arg_name)
                    .ok_or_else(|| (INVALID_PARAMS, format!("prompt `{name}` has no argument `{arg_name}`")))?;
                arg.completion
            }
            _ => return Err(bad("`ref.type` is ref/resource or ref/prompt")),
        };
        let empty = json!({"completion": {"values": [], "total": 0, "hasMore": false}});
        let Some(source) = source else { return Ok(empty) };
        // Nothing to offer (no document open, or the query is refused) is an empty list.
        Ok(resources::complete(source, value, self).unwrap_or_else(|e| {
            self.log(LogLevel::Debug, json!({"completion": e}));
            empty
        }))
    }

    /// Whether the policy admits a command by id, reading the catalogue when it needs to know
    /// what the command is. Returns the catalogue entry when it was read.
    fn admit(&mut self, id: &str) -> Result<Option<policy::CommandFacts>, String> {
        if !self.policy.is_restricted() {
            return Ok(None);
        }
        let facts = if self.policy.needs_catalogue() { self.facts(id)? } else { None };
        self.policy.admit(id, facts.as_ref())?;
        Ok(facts)
    }

    fn facts(&mut self, id: &str) -> Result<Option<policy::CommandFacts>, String> {
        if self.catalogue.is_none() {
            let all = self.backend.commands(None)?;
            let list = all.get("commands").and_then(Value::as_array).ok_or("the command catalogue has no `commands` list")?;
            if list.len() > MAX_CATALOGUE {
                return Err(format!("the command catalogue has more than {MAX_CATALOGUE} entries"));
            }
            self.catalogue = Some(list.iter().filter_map(policy::CommandFacts::from_entry).collect());
        }
        Ok(self.catalogue.as_ref().and_then(|c| c.get(id).cloned()))
    }

    /// Checks a command and, for `command.batch`, each of its steps; `depth` counts batches.
    /// File parameters in `params` are replaced by the paths that were checked.
    fn check_command(&mut self, id: &str, params: &mut Value, depth: usize, seen: &mut Seen) -> Result<(), String> {
        if depth > policy::MAX_BATCH_NESTING {
            return Err(format!("command.batch nests deeper than {} levels", policy::MAX_BATCH_NESTING));
        }
        let facts = self.admit(id)?;
        if self.policy.restricts_paths() {
            if let Some(place) = policy::implicit_place(id, params) {
                self.check_implicit(id, place, seen)?;
            }
            if id == "project.open" {
                seen.project_opened = true;
            }
            if let Some(facts) = &facts {
                let names_document =
                    facts.files.iter().any(|f| f.form == policy::FileForm::Document && params.get(&f.name).is_some_and(|v| !v.is_null()));
                let open = if names_document { self.open_documents()? } else { Vec::new() };
                self.policy.check_params(facts, params, &open)?;
            }
        }
        if id == policy::BATCH_COMMAND && self.policy.is_restricted() {
            for step in params.get_mut("steps").and_then(Value::as_array_mut).into_iter().flatten() {
                // A step without a command id is refused by the batch itself.
                let Some(command) = step.get("command").and_then(Value::as_str).map(str::to_string) else { continue };
                let mut none = Value::Null;
                let step_params = match step.get_mut("params") {
                    Some(p) => p,
                    None => &mut none,
                };
                self.check_command(&command, step_params, depth + 1, seen)?;
            }
        }
        Ok(())
    }

    /// Checks a place a command writes to, or reads, without a parameter naming it. Which
    /// document or project a step acts on can change inside a batch, so every open one is
    /// checked.
    fn check_implicit(&mut self, id: &str, place: policy::Implicit, seen: &mut Seen) -> Result<(), String> {
        match place {
            policy::Implicit::Hidden(why) => Err(format!("`{id}` {why}: this server restricts file access (--root)")),
            policy::Implicit::Documents => {
                if !seen.documents {
                    let documents = self.backend.inspect()?;
                    let list = documents.get("documents").and_then(Value::as_array).ok_or("cannot list the open documents")?;
                    for path in list.iter().filter_map(|d| d.get("path").and_then(Value::as_str)) {
                        self.policy.check_implicit_path(path, &format!("`{id}` writes to or beside the open document's file"))?;
                    }
                    seen.documents = true;
                }
                Ok(())
            }
            policy::Implicit::ProjectExports if seen.project_opened => {
                Err(format!("`{id}` follows project.open in the same batch, so its destination cannot be checked first; run it on its own"))
            }
            policy::Implicit::ProjectFiles | policy::Implicit::ProjectExports => {
                if seen.projects.is_none() {
                    seen.projects = Some(self.backend.execute("project.list", &json!({}))?);
                }
                let projects =
                    seen.projects.as_ref().and_then(|p| p.get("projects")).and_then(Value::as_array).ok_or("cannot list the open projects")?;
                let keys: &[(&str, &str)] = if place == policy::Implicit::ProjectFiles {
                    &[("path", "writes the project's file")]
                } else {
                    &[("destination", "writes into the project's export destination"), ("font", "reads the project's linked font")]
                };
                for project in projects {
                    for (key, what) in keys {
                        if let Some(path) = project.get(*key).and_then(Value::as_str) {
                            self.policy.check_implicit_path(path, &format!("`{id}` {what}"))?;
                        }
                    }
                }
                Ok(())
            }
        }
    }

    /// The titles and paths of the open documents.
    fn open_documents(&mut self) -> Result<Vec<String>, String> {
        let documents = self.backend.inspect()?;
        let list = documents.get("documents").and_then(Value::as_array).ok_or("cannot list the open documents")?;
        Ok(list.iter().flat_map(|d| [d.get("title"), d.get("path")]).filter_map(|v| v.and_then(Value::as_str).map(str::to_string)).collect())
    }

    /// The parameters to run a command with, once the policy admits it and its paths.
    fn checked(&mut self, id: &str, params: &Value) -> Result<Value, String> {
        let mut params = params.clone();
        self.check_command(id, &mut params, 0, &mut Seen::default())?;
        Ok(params)
    }

    fn run_command(&mut self, id: &str, params: &Value) -> Result<Value, String> {
        let params = match self.checked(id, params) {
            Ok(p) => p,
            Err(reason) => {
                self.log(LogLevel::Warning, json!({"command": id, "refused": reason}));
                return Err(reason);
            }
        };
        let result = self.backend.execute(id, &params);
        match &result {
            Ok(_) => self.log(LogLevel::Info, json!({"command": id, "ok": true})),
            Err(e) => self.log(LogLevel::Warning, json!({"command": id, "error": e})),
        }
        result
    }

    fn list_commands(&mut self, pattern: Option<&str>) -> Result<Value, String> {
        let mut all = self.backend.commands(pattern)?;
        if self.policy.is_restricted() {
            let list = all.get_mut("commands").and_then(Value::as_array_mut).ok_or("the command catalogue has no `commands` list")?;
            let policy = &self.policy;
            list.retain(|c| {
                let id = c.get("id").and_then(Value::as_str).unwrap_or("");
                let facts = policy::CommandFacts::from_entry(c).map(|(_, f)| f);
                policy.admit(id, facts.as_ref()).is_ok()
            });
        }
        Ok(all)
    }

    /// Serves until `input` ends. Only protocol messages go to `output`.
    pub fn serve(&mut self, mut input: impl BufRead, mut output: impl Write) -> std::io::Result<()> {
        use std::io::Read as _;
        loop {
            // Read at most one byte past the limit, so a peer that never ends its line cannot
            // fill memory; the rest of an oversized line is skipped.
            let mut line = Vec::new();
            let read = (&mut input).take(MAX_LINE as u64 + 1).read_until(b'\n', &mut line)?;
            if read == 0 {
                return Ok(());
            }
            let reply = if line.len() > MAX_LINE && !line.ends_with(b"\n") {
                skip_line(&mut input)?;
                Some(error(&Value::Null, INVALID_REQUEST, format!("a request line may be at most {MAX_LINE} bytes")).to_string())
            } else {
                match std::str::from_utf8(&line) {
                    Ok(text) => self.handle_line(text),
                    // A line that is not UTF-8 is a parse error, not the end of the session.
                    Err(_) => Some(error(&Value::Null, PARSE_ERROR, "the request is not valid UTF-8").to_string()),
                }
            };
            for notification in self.take_notifications() {
                writeln!(output, "{notification}")?;
            }
            if let Some(reply) = reply {
                writeln!(output, "{reply}")?;
            }
            output.flush()?;
        }
    }
}

impl resources::Reader for Server {
    fn query(&mut self, command: &str, params: Value) -> Result<Value, String> {
        let params = self.checked(command, &params)?;
        self.backend.execute(command, &params)
    }
    fn document(&mut self) -> Result<Value, String> {
        self.admit("document.inspect")?;
        self.backend.inspect()
    }
    fn admitted_commands(&mut self) -> Result<Vec<Value>, String> {
        let all = self.list_commands(None)?;
        Ok(all.get("commands").and_then(Value::as_array).cloned().unwrap_or_default())
    }
}

/// Discards input up to and including the next newline (or the end).
fn skip_line(input: &mut impl BufRead) -> std::io::Result<()> {
    loop {
        let buffer = input.fill_buf()?;
        if buffer.is_empty() {
            return Ok(());
        }
        match buffer.iter().position(|b| *b == b'\n') {
            Some(i) => {
                input.consume(i + 1);
                return Ok(());
            }
            None => {
                let n = buffer.len();
                input.consume(n);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn server() -> Server {
        Server::new(Box::new(Headless::with_document()))
    }

    fn ask(s: &mut Server, request: Value) -> Value {
        serde_json::from_str(&s.handle_line(&request.to_string()).expect("a reply")).unwrap()
    }

    fn tool(s: &mut Server, name: &str, arguments: Value) -> (bool, String) {
        let r = ask(s, json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": name, "arguments": arguments}}));
        (r["result"]["isError"].as_bool().unwrap(), r["result"]["content"][0]["text"].as_str().unwrap().to_string())
    }

    #[test]
    fn initialize_negotiates_the_version() {
        let mut s = server();
        let r = ask(&mut s, json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2024-11-05"}}));
        assert_eq!(r["result"]["protocolVersion"], "2024-11-05");
        // A revision this server does not speak (unknown, or one that needs batches) gets the newest.
        for asked in ["1999-01-01", "2025-03-26"] {
            let r = ask(&mut s, json!({"jsonrpc": "2.0", "id": 2, "method": "initialize", "params": {"protocolVersion": asked}}));
            assert_eq!(r["result"]["protocolVersion"], PROTOCOL_VERSIONS[0]);
        }
        let r = ask(&mut s, json!({"jsonrpc": "2.0", "id": 2, "method": "initialize", "params": {}}));
        assert_eq!(r["result"]["serverInfo"]["name"], "fontcraft");
        assert!(s.handle_line(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#).is_none());
        assert_eq!(ask(&mut s, json!({"jsonrpc": "2.0", "id": 3, "method": "ping"}))["result"], json!({}));
    }

    #[test]
    fn an_agent_can_find_and_run_commands() {
        let mut s = server();
        let list = ask(&mut s, json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}));
        let names: Vec<&str> = list["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
        assert_eq!(names, ["list_commands", "run_command", "inspect_document"]);

        let (is_error, text) = tool(&mut s, "list_commands", json!({"pattern": "glyph"}));
        assert!(!is_error && text.contains("glyph.newGlyph") && !text.contains("file.save"));
        let (is_error, text) = tool(&mut s, "run_command", json!({"command": "glyph.newGlyph", "params": {"name": "agent", "unicodes": ["E001"]}}));
        assert!(!is_error, "{text}");
        assert_eq!(serde_json::from_str::<Value>(&text).unwrap()["name"], "agent");
        let (_, text) = tool(&mut s, "inspect_document", json!({}));
        assert_eq!(serde_json::from_str::<Value>(&text).unwrap()["documents"][0]["undo"], json!(["New Glyph"]));

        // A failing command is a tool result with isError, and says why.
        let (is_error, text) = tool(&mut s, "run_command", json!({"command": "glyph.newGlyph", "params": {"name": "agent"}}));
        assert!(is_error && text.contains("already exists"), "{text}");
        let (is_error, _) = tool(&mut s, "run_command", json!({}));
        assert!(is_error);
    }

    #[test]
    fn malformed_input_gets_json_rpc_errors() {
        let mut s = server();
        let code = |s: &mut Server, line: &str| serde_json::from_str::<Value>(&s.handle_line(line).unwrap()).unwrap()["error"]["code"].as_i64();
        assert_eq!(code(&mut s, "{not json"), Some(PARSE_ERROR));
        assert_eq!(code(&mut s, "[1,2]"), Some(INVALID_REQUEST));
        assert_eq!(code(&mut s, r#"{"id":1}"#), Some(INVALID_REQUEST));
        assert_eq!(code(&mut s, r#"{"id":1,"method":7}"#), Some(INVALID_REQUEST));
        assert_eq!(code(&mut s, r#"{"id":1,"method":"nope"}"#), Some(METHOD_NOT_FOUND));
        assert_eq!(code(&mut s, r#"{"id":1,"method":"tools/call","params":{"name":"nope"}}"#), Some(INVALID_PARAMS));
        assert_eq!(code(&mut s, r#"{"id":1,"method":"tools/call","params":[]}"#), Some(INVALID_PARAMS));
        assert_eq!(code(&mut s, &format!("{{\"id\":1,\"method\":\"{}\"}}", "x".repeat(MAX_LINE))), Some(INVALID_REQUEST));
        assert_eq!(code(&mut s, r#"{"id":null,"method":"ping"}"#), Some(INVALID_REQUEST));
        assert_eq!(code(&mut s, r#"{"id":{"a":1},"method":"ping"}"#), Some(INVALID_REQUEST));
        assert!(s.handle_line("   ").is_none());
        assert!(s.handle_line(r#"{"method":"nope"}"#).is_none(), "an unknown notification is ignored");
        assert!(s.handle_line(r#"{"jsonrpc":"2.0","id":7,"result":{}}"#).is_none(), "a client response is ignored");
    }

    #[test]
    fn serve_skips_an_oversized_line_without_holding_it() {
        let mut s = server();
        let mut input = vec![b'x'; MAX_LINE * 2];
        input.extend_from_slice(b"\n{\"jsonrpc\":\"2.0\",\"id\":5,\"method\":\"ping\"}\n");
        let mut output = Vec::new();
        s.serve(&input[..], &mut output).unwrap();
        let lines: Vec<Value> = String::from_utf8(output).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(lines.len(), 2);
        assert_eq!((lines[0]["error"]["code"].as_i64(), lines[1]["id"].as_i64()), (Some(INVALID_REQUEST), Some(5)));
    }

    #[test]
    fn window_commands_explain_themselves_headless() {
        let mut s = server();
        let (is_error, text) = tool(&mut s, "run_command", json!({"command": "view.zoomIn"}));
        assert!(is_error && text.contains("needs the running app"), "{text}");
    }

    #[test]
    fn serve_answers_line_by_line_and_survives_bad_bytes() {
        let mut s = server();
        let input: Vec<u8> = [
            &b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"ping\"}\n"[..],
            &b"\xff\xfe\n"[..],
            &b"{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"ping\"}\n"[..],
        ]
        .concat();
        let mut output = Vec::new();
        s.serve(&input[..], &mut output).unwrap();
        let lines: Vec<Value> = String::from_utf8(output).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!(lines.len(), 3);
        assert_eq!((lines[0]["id"].as_i64(), lines[1]["error"]["code"].as_i64(), lines[2]["id"].as_i64()), (Some(1), Some(PARSE_ERROR), Some(2)));
    }

    fn restricted(policy: Policy) -> Server {
        Server::with_policy(Box::new(Headless::with_document()), policy)
    }

    fn listed(s: &mut Server) -> Vec<String> {
        let (_, text) = tool(s, "list_commands", json!({}));
        let all: Value = serde_json::from_str(&text).unwrap();
        all["commands"].as_array().unwrap().iter().map(|c| c["id"].as_str().unwrap().to_string()).collect()
    }

    #[test]
    fn read_only_refuses_changes_with_is_error() {
        let mut s = restricted(Policy::unrestricted().read_only(true));
        let (is_error, text) = tool(&mut s, "run_command", json!({"command": "glyph.newGlyph", "params": {"name": "x"}}));
        assert!(is_error && text.contains("read-only"), "{text}");
        let (is_error, text) = tool(&mut s, "run_command", json!({"command": "glyph.getInfo", "params": {}}));
        assert!(!is_error, "{text}");
        let ids = listed(&mut s);
        assert!(ids.iter().any(|i| i == "glyph.getInfo") && !ids.iter().any(|i| i == "glyph.newGlyph"));
        let (_, text) = tool(&mut s, "inspect_document", json!({}));
        assert_eq!(serde_json::from_str::<Value>(&text).unwrap()["documents"][0]["undo"], json!([]), "nothing ran");
        let r = ask(&mut s, json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}));
        assert!(r["result"]["instructions"].as_str().unwrap().contains("restricted"));
    }

    #[test]
    fn deny_beats_allow_in_tools_and_batches() {
        let policy = Policy::unrestricted().allow("glyph.*").unwrap().allow("command.batch").unwrap().deny("glyph.remove*").unwrap();
        let mut s = restricted(policy);
        let (is_error, text) = tool(&mut s, "run_command", json!({"command": "glyph.newGlyph", "params": {"name": "kept"}}));
        assert!(!is_error, "{text}");
        let (is_error, text) = tool(&mut s, "run_command", json!({"command": "glyph.removeGlyph", "params": {"glyphs": ["kept"]}}));
        assert!(is_error && text.contains("--deny"), "{text}");
        let (is_error, _) = tool(&mut s, "run_command", json!({"command": "font.info.set", "params": {"values": {"familyName": "X"}}}));
        assert!(is_error);
        // A batch is refused as a whole when one step is.
        let batch = json!({"command": "command.batch", "params": {"steps": [
            {"command": "glyph.newGlyph", "params": {"name": "b"}},
            {"command": "glyph.removeGlyph", "params": {"glyphs": ["kept"]}},
        ]}});
        let (is_error, text) = tool(&mut s, "run_command", batch);
        assert!(is_error && text.contains("glyph.removeGlyph"), "{text}");
        let ids = listed(&mut s);
        assert!(ids.iter().any(|i| i == "glyph.newGlyph") && !ids.iter().any(|i| i == "glyph.removeGlyph" || i == "font.info.set"));
        // Macros run steps the server cannot see.
        let (is_error, _) = tool(&mut s, "run_command", json!({"command": "script.run", "params": {}}));
        assert!(is_error);
    }

    #[test]
    fn file_parameters_outside_the_roots_are_refused() {
        let dir = std::env::temp_dir().join(format!("fontcraft-mcp-roots-{}", std::process::id()));
        let root = dir.join("root");
        std::fs::create_dir_all(&root).unwrap();
        let mut s = restricted(Policy::unrestricted().root(&root).unwrap().base_dir(&root));
        let (is_error, text) = tool(&mut s, "run_command", json!({"command": "file.saveAs", "params": {"path": "../outside.fontcraft"}}));
        assert!(is_error && text.contains("--root"), "{text}");
        assert!(!dir.join("outside.fontcraft").exists());
        let (is_error, text) = tool(&mut s, "run_command", json!({"command": "file.saveAs", "params": {"path": "inside.fontcraft"}}));
        assert!(!is_error, "{text}");
        assert!(root.join("inside.fontcraft").exists(), "a relative path resolves inside the root");
        let (is_error, text) = tool(&mut s, "run_command", json!({"command": "file.save", "params": {"force": true}}));
        assert!(!is_error, "the document's own file is inside the root: {text}");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Inputs that once slipped past the policy: each is refused and writes nothing.
    #[test]
    fn undeclared_and_implicit_writes_are_refused() {
        let dir = std::env::temp_dir().join(format!("fontcraft-mcp-writes-{}", std::process::id()));
        let (root, outside) = (dir.join("root"), dir.join("outside"));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        let rooted = || Policy::unrestricted().root(&root).unwrap().base_dir(&root);
        let out = |name: &str| outside.join(name).to_str().unwrap().to_string();

        // A table file after the tag, alone or as a batch step.
        let mut s = restricted(rooted());
        let extract = json!({"font": "f.ttf", "tables": [format!("head={}", out("head.bin"))]});
        let (is_error, text) = tool(&mut s, "run_command", json!({"command": "tables.extract", "params": extract.clone()}));
        assert!(is_error && text.contains("--root"), "{text}");
        let batch = json!({"steps": [{"command": "glyph.newGlyph", "params": {"name": "b"}}, {"command": "tables.extract", "params": extract}]});
        let (is_error, text) = tool(&mut s, "run_command", json!({"command": "command.batch", "params": batch}));
        assert!(is_error && text.contains("--root"), "{text}");
        // A WOFF block extracted outside the roots.
        let (is_error, text) =
            tool(&mut s, "run_command", json!({"command": "woff.decompress", "params": {"font": "f.woff", "extractMetadata": out("meta.xml")}}));
        assert!(is_error && text.contains("--root"), "{text}");
        assert!(!outside.join("head.bin").exists() && !outside.join("meta.xml").exists());

        // A query that writes in place, under --read-only.
        std::fs::write(root.join("f.ttf"), b"not a font").unwrap();
        let mut s = restricted(rooted().read_only(true));
        let params = json!({"font": "f.ttf", "tables": ["DSIG"], "inPlace": true});
        let (is_error, text) = tool(&mut s, "run_command", json!({"command": "tables.delete", "params": params}));
        assert!(is_error && text.contains("read-only"), "{text}");
        assert_eq!(std::fs::read(root.join("f.ttf")).unwrap(), b"not a font");
        assert!(!listed(&mut s).iter().any(|i| i == "tables.delete" || i == "font.dumpTables"));

        // A project whose stored file and destination lie outside the roots.
        let mut h = Headless::with_document();
        let stored = outside.join("p.fontcraftproject");
        h.execute("file.newProject", &json!({})).unwrap();
        h.execute("project.setExportDestination", &json!({"folder": out("fonts")})).unwrap();
        h.execute("project.save", &json!({"path": stored})).unwrap();
        std::fs::remove_file(&stored).unwrap();
        let mut s = Server::with_policy(Box::new(h), rooted());
        let (is_error, text) = tool(&mut s, "run_command", json!({"command": "project.save", "params": {}}));
        assert!(is_error && text.contains("--root"), "{text}");
        assert!(!stored.exists());
        let (is_error, text) = tool(&mut s, "run_command", json!({"command": "project.export", "params": {}}));
        assert!(is_error && text.contains("--root"), "{text}");
        assert!(!outside.join("fonts").exists());
        // A project opened earlier in the same batch: its stored destination is unknown.
        let batch =
            json!({"steps": [{"command": "project.open", "params": {"path": "p.fontcraftproject"}}, {"command": "project.export", "params": {}}]});
        let (is_error, text) = tool(&mut s, "run_command", json!({"command": "command.batch", "params": batch}));
        assert!(is_error && text.contains("on its own"), "{text}");
        // Macros saved into the Macros folder, which the server cannot see.
        let save = json!({"name": "m", "steps": [{"command": "glyph.newGlyph", "params": {"name": "x"}}]});
        let (is_error, text) = tool(&mut s, "run_command", json!({"command": "script.saveActionSet", "params": save}));
        assert!(is_error && text.contains("Macros folder"), "{text}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_file_system_root_alone_restricts_nothing() {
        let mut s = restricted(Policy::unrestricted().root(std::path::Path::new("/")).unwrap());
        let (_, text) = tool(&mut s, "run_command", json!({"command": "script.run", "params": {"macro": "missing"}}));
        assert!(!text.contains("cannot check"), "macros are admitted: {text}");
        let r = ask(&mut s, json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}));
        assert!(!r["result"]["instructions"].as_str().unwrap_or("").contains("This server is restricted"));
    }

    #[test]
    fn set_level_filters_log_notifications() {
        let mut s = server();
        tool(&mut s, "run_command", json!({"command": "glyph.newGlyph", "params": {"name": "a1"}}));
        assert!(s.take_notifications().is_empty(), "no notifications before logging/setLevel");
        let r = ask(&mut s, json!({"jsonrpc": "2.0", "id": 1, "method": "logging/setLevel", "params": {"level": "warning"}}));
        assert_eq!(r["result"], json!({}));
        tool(&mut s, "run_command", json!({"command": "glyph.newGlyph", "params": {"name": "a2"}}));
        assert!(s.take_notifications().is_empty(), "a success is info, below warning");
        tool(&mut s, "run_command", json!({"command": "glyph.newGlyph", "params": {"name": "a2"}}));
        let notes: Vec<Value> = s.take_notifications().iter().map(|n| serde_json::from_str(n).unwrap()).collect();
        assert_eq!(notes.len(), 1);
        assert_eq!((notes[0]["method"].as_str(), notes[0]["params"]["level"].as_str()), (Some("notifications/message"), Some("warning")));
        ask(&mut s, json!({"jsonrpc": "2.0", "id": 2, "method": "logging/setLevel", "params": {"level": "debug"}}));
        tool(&mut s, "run_command", json!({"command": "glyph.newGlyph", "params": {"name": "a3"}}));
        assert_eq!(s.take_notifications().len(), 1);
        let r = ask(&mut s, json!({"jsonrpc": "2.0", "id": 3, "method": "logging/setLevel", "params": {"level": "loud"}}));
        assert_eq!(r["error"]["code"].as_i64(), Some(INVALID_PARAMS));
        // serve writes a request's notifications before its reply.
        let mut output = Vec::new();
        s.serve(&b"{\"jsonrpc\":\"2.0\",\"id\":9,\"method\":\"tools/call\",\"params\":{\"name\":\"run_command\",\"arguments\":{\"command\":\"glyph.newGlyph\",\"params\":{\"name\":\"a4\"}}}}\n"[..], &mut output).unwrap();
        let lines: Vec<Value> = String::from_utf8(output).unwrap().lines().map(|l| serde_json::from_str(l).unwrap()).collect();
        assert_eq!((lines.len(), lines[0]["method"].as_str(), lines[1]["id"].as_i64()), (2, Some("notifications/message"), Some(9)));
    }

    #[test]
    fn every_prompt_names_commands_that_exist() {
        let mut s = server();
        let ids = listed(&mut s);
        for prompt in prompts::PROMPTS {
            for command in prompt.commands {
                assert!(ids.iter().any(|i| i == command), "prompt {} names {command}, which does not exist", prompt.name);
            }
        }
        let list = ask(&mut s, json!({"jsonrpc": "2.0", "id": 1, "method": "prompts/list"}));
        assert_eq!(list["result"]["prompts"].as_array().unwrap().len(), 5);
        let r = ask(
            &mut s,
            json!({"jsonrpc": "2.0", "id": 2, "method": "prompts/get", "params": {"name": "start_family", "arguments": {"family_name": "Fieldstone"}}}),
        );
        assert!(r["result"]["messages"][0]["content"]["text"].as_str().unwrap().contains("Fieldstone"));
        let r = ask(&mut s, json!({"jsonrpc": "2.0", "id": 3, "method": "prompts/get", "params": {"name": "nope"}}));
        assert_eq!(r["error"]["code"].as_i64(), Some(INVALID_PARAMS));
    }

    #[test]
    fn resources_read_a_glyph() {
        let mut s = server();
        tool(&mut s, "run_command", json!({"command": "glyph.newGlyph", "params": {"name": "agent"}}));
        tool(
            &mut s,
            "run_command",
            json!({"command": "path.addPath", "params": {"glyph": "agent", "nodes": [{"x": 0, "y": 0}, {"x": 100, "y": 0}, {"x": 50, "y": 80}]}}),
        );
        let r = ask(&mut s, json!({"jsonrpc": "2.0", "id": 1, "method": "resources/read", "params": {"uri": "fontcraft://glyph/agent"}}));
        let content = &r["result"]["contents"][0];
        assert_eq!(content["uri"], "fontcraft://glyph/agent");
        let body: Value = serde_json::from_str(content["text"].as_str().unwrap()).unwrap();
        assert_eq!(body["glyph"]["name"], "agent");
        assert_eq!(body["layers"][0]["shapes"].as_array().map(Vec::len), Some(1));

        let r = ask(&mut s, json!({"jsonrpc": "2.0", "id": 2, "method": "resources/read", "params": {"uri": "fontcraft://glyph/missing"}}));
        assert_eq!(r["error"]["code"].as_i64(), Some(RESOURCE_NOT_FOUND));
        let r = ask(&mut s, json!({"jsonrpc": "2.0", "id": 3, "method": "resources/read", "params": {"uri": "fontcraft://command/glyph.newGlyph"}}));
        assert!(r["result"]["contents"][0]["text"].as_str().unwrap().contains("glyph.newGlyph"));
        let r = ask(&mut s, json!({"jsonrpc": "2.0", "id": 4, "method": "resources/read", "params": {"uri": "fontcraft://document"}}));
        assert!(r["result"]["contents"][0]["text"].as_str().unwrap().contains("documents"));
        let r = ask(&mut s, json!({"jsonrpc": "2.0", "id": 5, "method": "resources/templates/list"}));
        assert_eq!(r["result"]["resourceTemplates"].as_array().map(Vec::len), Some(4));

        // A refused query refuses the resource too.
        let mut s = restricted(Policy::unrestricted().deny("glyph.*").unwrap());
        let r = ask(&mut s, json!({"jsonrpc": "2.0", "id": 6, "method": "resources/read", "params": {"uri": "fontcraft://glyph/A"}}));
        assert_eq!(r["error"]["code"].as_i64(), Some(RESOURCE_NOT_FOUND));
    }

    #[test]
    fn completion_offers_glyph_names() {
        let mut s = server();
        for name in ["agent", "agent.alt", "other"] {
            tool(&mut s, "run_command", json!({"command": "glyph.newGlyph", "params": {"name": name}}));
        }
        let complete = |s: &mut Server, reference: Value, name: &str, value: &str| {
            ask(
                s,
                json!({"jsonrpc": "2.0", "id": 1, "method": "completion/complete", "params": {"ref": reference, "argument": {"name": name, "value": value}}}),
            )
        };
        let r = complete(&mut s, json!({"type": "ref/resource", "uri": "fontcraft://glyph/{name}"}), "name", "AG");
        assert_eq!(r["result"]["completion"]["values"], json!(["agent", "agent.alt"]));
        assert_eq!(r["result"]["completion"]["hasMore"], false);
        let r = complete(&mut s, json!({"type": "ref/prompt", "name": "build_accented_glyphs"}), "glyphs", "aacute agent.");
        assert_eq!(r["result"]["completion"]["values"], json!(["aacute agent.alt"]));
        let r = complete(&mut s, json!({"type": "ref/resource", "uri": "fontcraft://command/{id}"}), "id", "glyph.newG");
        assert_eq!(r["result"]["completion"]["values"], json!(["glyph.newGlyph"]));
        let r = complete(&mut s, json!({"type": "ref/prompt", "name": "kern_pair_list"}), "pairs", "A");
        assert_eq!(r["result"]["completion"]["total"], 0);
        let r = complete(&mut s, json!({"type": "ref/resource", "uri": "fontcraft://nope/{x}"}), "x", "");
        assert_eq!(r["error"]["code"].as_i64(), Some(INVALID_PARAMS));
    }
}
