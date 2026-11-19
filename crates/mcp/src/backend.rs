//! Where the tools run: in this process, or in a running app.

use fontcraft_engine::Session;
use serde_json::{Value, json};

/// The app's default control port. The sibling Crafting Apps default to 7979; FontCraft uses its
/// own so agents driving two apps at once do not reach the wrong one.
pub const DEFAULT_ADDR: &str = "127.0.0.1:7985";

pub trait Backend {
    /// Runs a command; `Err` is the message shown to the agent.
    fn execute(&mut self, command: &str, params: &Value) -> Result<Value, String>;
    /// The command catalogue, optionally filtered.
    fn commands(&mut self, pattern: Option<&str>) -> Result<Value, String>;
    fn inspect(&mut self) -> Result<Value, String>;
    /// One phrase for `initialize`: which backend this is.
    fn describe(&self) -> String;
    /// Whether a running app answers [`Backend::control`]: the window tools are offered only then.
    fn has_app(&self) -> bool {
        false
    }
    /// Calls a control method of the running app (`docs/control-protocol.md`), such as
    /// `ui.screenshot`. Without an app it fails with [`NEEDS_APP`].
    fn control(&mut self, method: &str, _params: &Value) -> Result<Value, String> {
        Err(format!("`{method}` {NEEDS_APP}"))
    }
}

/// Why a window tool fails without the app; it follows the tool's or method's name.
pub const NEEDS_APP: &str =
    "needs the running app: start it with `fontcraft --control 7985` and attach with `fontcraft-cli mcp --connect 127.0.0.1:7985`";

/// An engine session inside this process: everything except UI-only commands.
pub struct Headless(pub Session);

impl Headless {
    pub fn new() -> Self {
        Self(Session::new())
    }
    pub fn with_document() -> Self {
        Self(Session::with_document())
    }
}

impl Default for Headless {
    fn default() -> Self {
        Self::new()
    }
}

impl Backend for Headless {
    fn execute(&mut self, command: &str, params: &Value) -> Result<Value, String> {
        self.0.execute(command, params).map_err(|e| match e {
            // Commands that only change the window live in the app, not in the engine.
            fontcraft_engine::EngineError::UnknownCommand(_) => {
                format!(
                    "{e} (if it is a window or view command, it needs the running app: start the app with --control and attach with `mcp --connect`)"
                )
            }
            other => other.to_string(),
        })
    }
    fn commands(&mut self, pattern: Option<&str>) -> Result<Value, String> {
        self.execute("commands.list", &json!({"pattern": pattern}))
    }
    fn inspect(&mut self) -> Result<Value, String> {
        Ok(self.0.inspect())
    }
    fn describe(&self) -> String {
        "headless session".into()
    }
}

/// A running app, over its control port (`docs/control-protocol.md`).
#[cfg(not(target_arch = "wasm32"))]
pub struct Remote {
    reader: std::io::BufReader<std::net::TcpStream>,
    addr: String,
    next_id: u64,
    /// Set after any transport error: a late reply could otherwise be read as the answer to the
    /// next request.
    broken: bool,
}

#[cfg(not(target_arch = "wasm32"))]
impl Remote {
    pub fn connect(addr: &str) -> std::io::Result<Self> {
        let stream = std::net::TcpStream::connect(addr)?;
        stream.set_read_timeout(Some(std::time::Duration::from_secs(120)))?;
        Ok(Self { reader: std::io::BufReader::new(stream), addr: addr.to_string(), next_id: 0, broken: false })
    }

    fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        if self.broken {
            return Err(format!("the connection to the app at {} was lost; start the MCP server again", self.addr));
        }
        let result = self.exchange(method, params);
        if let Err(Transport(_)) = &result {
            self.broken = true;
        }
        result.map_err(|e| match e {
            Transport(message) | Refused(message) => message,
        })
    }

    fn exchange(&mut self, method: &str, params: Value) -> Result<Value, CallError> {
        use std::io::{BufRead as _, Read as _, Write as _};
        self.next_id += 1;
        let id = self.next_id;
        let request = json!({"id": id, "method": method, "params": params});
        let addr = self.addr.clone();
        let io = |e: std::io::Error| Transport(format!("lost the connection to the app at {addr}: {e}"));
        writeln!(self.reader.get_mut(), "{request}").map_err(io)?;
        let mut line = String::new();
        // Cap the read so a wrong peer cannot fill memory.
        const MAX_REPLY: u64 = 64 << 20;
        let read = (&mut self.reader).take(MAX_REPLY).read_line(&mut line).map_err(io)?;
        if read == 0 {
            return Err(Transport(format!("the app at {addr} closed the connection")));
        }
        if !line.ends_with('\n') {
            return Err(Transport(format!("the app at {addr} sent a reply that was cut off or is larger than {MAX_REPLY} bytes")));
        }
        let reply: Value = serde_json::from_str(&line).map_err(|e| Transport(format!("the app at {addr} sent a reply that is not JSON: {e}")))?;
        // A reply without our id is the server refusing the connection or the request line.
        match reply.get("id").and_then(Value::as_u64) {
            Some(got) if got == id => {}
            Some(got) => return Err(Transport(format!("the app at {addr} answered request {got} when {id} was asked"))),
            None => {
                let why = reply.get("error").and_then(Value::as_str).unwrap_or("no reason given");
                return Err(Transport(format!("the app at {addr} refused the request: {why}")));
            }
        }
        if reply.get("ok").and_then(Value::as_bool) == Some(true) {
            Ok(reply.get("result").cloned().unwrap_or(Value::Null))
        } else {
            Err(Refused(reply.get("error").and_then(Value::as_str).unwrap_or("the app reported an error without a message").to_string()))
        }
    }
}

/// Why a call failed: the connection can no longer be trusted, or the app said no.
#[cfg(not(target_arch = "wasm32"))]
enum CallError {
    Transport(String),
    Refused(String),
}
#[cfg(not(target_arch = "wasm32"))]
use CallError::{Refused, Transport};

#[cfg(not(target_arch = "wasm32"))]
impl Backend for Remote {
    fn execute(&mut self, command: &str, params: &Value) -> Result<Value, String> {
        self.call("engine.execute", json!({"command": command, "params": params}))
    }
    fn commands(&mut self, pattern: Option<&str>) -> Result<Value, String> {
        let mut all = self.call("engine.commands", json!({}))?;
        if let (Some(pattern), Some(list)) = (pattern.map(str::to_lowercase), all.get_mut("commands").and_then(Value::as_array_mut)) {
            let has = |c: &Value, key: &str| c.get(key).and_then(Value::as_str).is_some_and(|s| s.to_lowercase().contains(&pattern));
            list.retain(|c| has(c, "id") || has(c, "label"));
        }
        Ok(all)
    }
    fn inspect(&mut self) -> Result<Value, String> {
        self.call("document.inspect", json!({}))
    }
    fn describe(&self) -> String {
        format!("attached to the running app at {}", self.addr)
    }
    fn has_app(&self) -> bool {
        true
    }
    fn control(&mut self, method: &str, params: &Value) -> Result<Value, String> {
        self.call(method, params.clone())
    }
}

/// The browser cannot open TCP connections; the type exists so code that names it still builds.
#[cfg(target_arch = "wasm32")]
pub struct Remote;
