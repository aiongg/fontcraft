//! Where the tools run: in this process, or in a running app.

use fontcraft_engine::{Session, work};
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
    /// Starts the keys this connection's requests are cancelled by.
    key: String,
    /// Set after any transport error: a late reply could otherwise be read as the answer to the
    /// next request.
    broken: bool,
}

#[cfg(not(target_arch = "wasm32"))]
impl Remote {
    pub fn connect(addr: &str) -> std::io::Result<Self> {
        let stream = std::net::TcpStream::connect(addr)?;
        stream.set_read_timeout(Some(TICK))?;
        Ok(Self { reader: std::io::BufReader::new(stream), addr: addr.to_string(), next_id: 0, key: connection_key(), broken: false })
    }

    fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        if self.broken {
            return Err(format!("the connection to the app at {} was lost; start the MCP server again", self.addr));
        }
        if work::cancelled() {
            return Err("cancelled".into());
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
        use std::io::Write as _;
        self.next_id += 1;
        let id = self.next_id;
        let key = format!("{}.{id}", self.key);
        let request = json!({"id": id, "method": method, "params": params, "cancelKey": key, "progress": true});
        let addr = self.addr.clone();
        writeln!(self.reader.get_mut(), "{request}").map_err(|e| Transport(format!("lost the connection to the app at {addr}: {e}")))?;
        let mut cancelled = false;
        loop {
            let line = self.read_line(|| {
                if work::cancelled() && !std::mem::replace(&mut cancelled, true) {
                    cancel(&addr, &key);
                }
            })?;
            let reply: Value =
                serde_json::from_slice(&line).map_err(|e| Transport(format!("the app at {addr} sent a reply that is not JSON: {e}")))?;
            // A reply without our id is the server refusing the connection or the request line.
            match reply.get("id").and_then(Value::as_u64) {
                Some(got) if got == id => {}
                Some(got) => return Err(Transport(format!("the app at {addr} answered request {got} when {id} was asked"))),
                None => {
                    let why = reply.get("error").and_then(Value::as_str).unwrap_or("no reason given");
                    return Err(Transport(format!("the app at {addr} refused the request: {why}")));
                }
            }
            if let (None, Some(progress)) = (reply.get("ok"), reply.get("progress")) {
                let number = |key: &str| progress.get(key).and_then(Value::as_u64).unwrap_or(0);
                work::relay(number("done"), number("total"), progress.get("message").and_then(Value::as_str).unwrap_or(""));
                continue;
            }
            if reply.get("ok").and_then(Value::as_bool) == Some(true) {
                // The app did it: a cancel came too late.
                if work::cancelled() && !READS.contains(&method) {
                    work::keep();
                }
                return Ok(reply.get("result").cloned().unwrap_or(Value::Null));
            }
            return Err(Refused(reply.get("error").and_then(Value::as_str).unwrap_or("the app reported an error without a message").to_string()));
        }
    }

    /// The next line, at most [`MAX_REPLY`] bytes; `tick` runs about every [`TICK`] until it is there.
    fn read_line(&mut self, mut tick: impl FnMut()) -> Result<Vec<u8>, CallError> {
        use std::io::{BufRead as _, ErrorKind, Read as _};
        let addr = self.addr.clone();
        let started = std::time::Instant::now();
        let mut line = Vec::new();
        loop {
            tick();
            let left = MAX_REPLY.saturating_sub(line.len() as u64);
            // What a timeout cuts off stays in `line`.
            match (&mut self.reader).take(left).read_until(b'\n', &mut line) {
                Ok(_) if line.ends_with(b"\n") => return Ok(line),
                Ok(0) if line.is_empty() => return Err(Transport(format!("the app at {addr} closed the connection"))),
                Ok(_) => {
                    return Err(Transport(format!("the app at {addr} sent a reply that was cut off or is larger than {MAX_REPLY} bytes")));
                }
                Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                    if started.elapsed() >= REPLY_TIMEOUT {
                        return Err(Transport(format!("the app at {addr} did not answer within {} seconds", REPLY_TIMEOUT.as_secs())));
                    }
                }
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                Err(e) => return Err(Transport(format!("lost the connection to the app at {addr}: {e}"))),
            }
        }
    }
}

/// The methods a request makes before the one it is for.
#[cfg(not(target_arch = "wasm32"))]
const READS: &[&str] = &["engine.commands", "document.inspect", "ui.dialog.inspect", "ui.elements"];
/// Most bytes of one reply, so a wrong peer cannot fill memory.
#[cfg(not(target_arch = "wasm32"))]
const MAX_REPLY: u64 = 64 << 20;
/// How long the app may be silent while a request runs.
#[cfg(not(target_arch = "wasm32"))]
const REPLY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(120);
/// How often a waiting call looks whether it was cancelled.
#[cfg(not(target_arch = "wasm32"))]
const TICK: std::time::Duration = std::time::Duration::from_millis(100);

/// Asks the app to cancel the request `key`, on a connection of its own: the request's is busy. Whether that worked
/// shows in the request's reply.
#[cfg(not(target_arch = "wasm32"))]
fn cancel(addr: &str, key: &str) {
    use std::io::{Read as _, Write as _};
    const WAIT: std::time::Duration = std::time::Duration::from_secs(2);
    let Ok(mut stream) = std::net::TcpStream::connect(addr) else { return };
    if stream.set_read_timeout(Some(WAIT)).is_err() || stream.set_write_timeout(Some(WAIT)).is_err() {
        return;
    }
    let request = json!({"id": 1, "method": "control.cancel", "params": {"key": key}});
    if writeln!(stream, "{request}").is_ok() {
        // Wait for the reply, so closing the connection does not overtake the request.
        let _ = stream.read(&mut [0u8; 256]);
    }
}

/// A key no other connection has. Unguessable, but not a credential: the control port has no authentication.
#[cfg(not(target_arch = "wasm32"))]
fn connection_key() -> String {
    use std::hash::{BuildHasher as _, Hasher as _};
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u32(std::process::id());
    format!("mcp-{:016x}", hasher.finish())
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
