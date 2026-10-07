//! FontCraft MCP server (L6): the [Model Context Protocol](https://modelcontextprotocol.io) over
//! stdio, newline-delimited JSON-RPC 2.0. See `docs/mcp.md`.
//!
//! Two backends answer the tools:
//! - [`Headless`]: an engine session inside this process.
//! - [`Remote`]: a running FontCraft app, reached over its control port (`fontcraft --control`),
//!   so the agent's changes appear in the window.
//!
//! The server reads one line and writes one reply. It never panics on input: malformed JSON, wrong
//! types and unknown methods get JSON-RPC errors, and a bug inside a tool becomes an error result.
#![forbid(unsafe_code)]

mod backend;

use std::io::{BufRead, Write};

pub use backend::{Backend, DEFAULT_ADDR, Headless, Remote};
use fontcraft_engine::guard;
use serde_json::{Value, json};

/// Protocol revisions this server speaks, newest first. `2025-03-26` is left out on purpose: that
/// revision requires JSON-RPC batches, which this server does not accept.
pub const PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2024-11-05"];
/// Longest request line accepted.
pub const MAX_LINE: usize = 4 << 20;

const PARSE_ERROR: i64 = -32700;
const INVALID_REQUEST: i64 = -32600;
const METHOD_NOT_FOUND: i64 = -32601;
const INVALID_PARAMS: i64 = -32602;

pub struct Server {
    backend: Box<dyn Backend>,
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

impl Server {
    pub fn new(backend: Box<dyn Backend>) -> Self {
        Self { backend }
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
            Err(panic) => error(&id, -32603, format!("internal error (please report it): {panic}")),
        })
    }

    fn dispatch(&mut self, method: &str, params: &Value) -> Result<Value, (i64, String)> {
        match method {
            "initialize" => {
                let asked = params.get("protocolVersion").and_then(Value::as_str).unwrap_or("");
                let version = PROTOCOL_VERSIONS.iter().find(|v| **v == asked).or(PROTOCOL_VERSIONS.first()).copied().unwrap_or("2025-06-18");
                Ok(json!({
                    "protocolVersion": version,
                    "capabilities": {"tools": {}},
                    "serverInfo": {"name": "fontcraft", "version": env!("CARGO_PKG_VERSION")},
                    "instructions": format!("FontCraft font editor ({}). Every action is a command: call list_commands, then run_command.", self.backend.describe()),
                }))
            }
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({"tools": tools()})),
            "tools/call" => {
                let name = params.get("name").and_then(Value::as_str).ok_or((INVALID_PARAMS, "`name` (a tool name) is required".to_string()))?;
                let args = params.get("arguments").cloned().unwrap_or(Value::Null);
                let result = match name {
                    "list_commands" => self.backend.commands(args.get("pattern").and_then(Value::as_str)),
                    "run_command" => match args.get("command").and_then(Value::as_str) {
                        Some(command) => self.backend.execute(command, args.get("params").unwrap_or(&Value::Null)),
                        None => Err("`command` (a command id) is required".to_string()),
                    },
                    "inspect_document" => self.backend.inspect(),
                    other => return Err((INVALID_PARAMS, format!("unknown tool `{other}`"))),
                };
                // A failed tool is a result the agent reads, not a protocol error.
                Ok(match result {
                    Ok(value) => json!({"content": [{"type": "text", "text": value.to_string()}], "isError": false}),
                    Err(message) => json!({"content": [{"type": "text", "text": message}], "isError": true}),
                })
            }
            other => Err((METHOD_NOT_FOUND, format!("method `{other}` is not supported"))),
        }
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
            if let Some(reply) = reply {
                writeln!(output, "{reply}")?;
                output.flush()?;
            }
        }
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
}
