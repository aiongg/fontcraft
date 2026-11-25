//! The MCP streamable HTTP transport (protocol revision 2025-06-18) on a small HTTP/1.1 server of
//! our own, built on `std::net`. See `docs/mcp.md` › HTTP transport.
//!
//! One endpoint, [`ENDPOINT`], takes `POST` (one JSON-RPC message per request) and `DELETE` (ends
//! a session). Replies are `application/json`, or `text/event-stream` when there are notifications
//! for the client (log messages, changed resources, progress) and it accepts a stream. Progress goes out while the
//! request runs: the first of it begins the reply, and the rest follows as events up to the reply itself. `GET` is
//! refused with 405: the server never starts a stream of its own.
//!
//! Threads: one accepts connections, one per connection reads and checks requests, and the thread
//! that called [`serve`] owns the [`Server`] and answers the requests one at a time, as the stdio
//! loop does. The backend therefore never needs to be `Send`. A cancellation is carried out by the
//! thread that reads it, so it reaches a request that runs or waits.
//!
//! Everything a peer sends is untrusted: request heads, bodies, connections, sessions and the
//! message rate are capped by [`Limits`], a slow peer runs into a deadline, a connection is closed
//! after [`MAX_CONNECTION_TIME`], and a request that carries a foreign `Origin` (a web page) or a
//! `Host` that is not one of the server's own (DNS rebinding) is refused before its body is
//! looked at.

use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::net::{IpAddr, Shutdown, TcpListener, TcpStream};
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError, mpsc};
use std::time::{Duration, Instant};

use fontcraft_engine::work::Token;
use serde_json::Value;

use crate::stdio::cancelled_request;
use crate::subscriptions::Client;
use crate::{INVALID_REQUEST, PARSE_ERROR, PROTOCOL_VERSIONS, Server, error, request_key};

/// Where `--transport http` listens without `--listen`: loopback only, next to the app's control
/// port.
pub const DEFAULT_LISTEN: &str = "127.0.0.1:7986";
/// The one path the server answers.
pub const ENDPOINT: &str = "/mcp";
/// Largest request body: the stdio transport's longest line.
pub const MAX_BODY: usize = crate::MAX_LINE;
/// Largest request head (request line and headers together).
pub const MAX_HEADER_BYTES: usize = 16 << 10;
/// Most header lines in one request.
pub const MAX_HEADERS: usize = 100;
/// Most connections served at once; more are answered 503 and closed.
pub const MAX_CONNECTIONS: usize = 16;
/// Most open sessions; a new one ends the session unused for longest.
pub const MAX_SESSIONS: usize = 64;
/// How long a connection may stay idle between requests, and how long one request may take to
/// arrive in full.
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(60);
/// How long one connection is kept: after the request that crosses it, the connection is closed,
/// so a peer that sends a request now and then cannot hold a slot for good.
pub const MAX_CONNECTION_TIME: Duration = Duration::from_secs(600);
/// Most messages answered per second over all connections, cancellations among them; more get 429.
pub const MAX_MESSAGES_PER_SECOND: usize = 200;
/// After an error reply the connection is closed; this much unread input is drained first so the
/// peer sees the reply instead of a reset.
const LINGER_BYTES: usize = 256 << 10;
const LINGER_TIME: Duration = Duration::from_secs(1);
/// A connection refused for being one too many gets this long to take its 503.
const BUSY_WRITE_TIMEOUT: Duration = Duration::from_secs(1);
/// Pause after a failed `accept` (out of file descriptors, for example), so the loop does not spin.
const ACCEPT_BACKOFF: Duration = Duration::from_millis(50);
/// Line endings skipped before a request line.
const MAX_BLANK_BYTES: usize = 64;
/// Longest `Content-Length` value read, in digits.
const MAX_LENGTH_DIGITS: usize = 19;
/// The error of a request its client cancelled.
const CANCELLED: i64 = -32800;

/// The transport's caps. [`Limits::default`] holds the named constants above; tests shrink them.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub max_body: usize,
    pub max_header_bytes: usize,
    pub max_headers: usize,
    pub max_connections: usize,
    pub max_sessions: usize,
    pub idle_timeout: Duration,
    pub connection_time: Duration,
    pub max_messages_per_second: usize,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_body: MAX_BODY,
            max_header_bytes: MAX_HEADER_BYTES,
            max_headers: MAX_HEADERS,
            max_connections: MAX_CONNECTIONS,
            max_sessions: MAX_SESSIONS,
            idle_timeout: IDLE_TIMEOUT,
            connection_time: MAX_CONNECTION_TIME,
            max_messages_per_second: MAX_MESSAGES_PER_SECOND,
        }
    }
}

/// Serves MCP over HTTP on `listener` until the process ends. `server` stays on the calling
/// thread. Returns early only when the accepting thread cannot be started.
pub fn serve(server: Server, listener: TcpListener, limits: Limits) -> io::Result<()> {
    let local = listener.local_addr()?;
    let shared = Arc::new(Shared {
        limits,
        loopback: local.ip().is_loopback(),
        active: AtomicUsize::new(0),
        running: Mutex::default(),
        rate: Mutex::new(Rate { start: Instant::now(), count: 0 }),
        parsing: Mutex::default(),
    });
    let (jobs, inbox) = mpsc::channel::<Job>();
    std::thread::Builder::new().name("mcp-http-accept".into()).spawn(move || accept_loop(&listener, &shared, &jobs))?;
    let mut dispatcher = Dispatcher::new(server, limits);
    for job in inbox {
        let response = if job.token.is_cancelled() { cancelled(&job.request) } else { dispatcher.answer(&job) };
        // The connection may have gone while the request ran; nothing waits for the reply then.
        let _ = job.reply.send(Part::Reply(response));
    }
    Ok(())
}

/// What the connection threads share.
struct Shared {
    limits: Limits,
    /// The listener is on a loopback address, so `Host` must name one too ([`host_allowed`]).
    loopback: bool,
    /// Connections being served.
    active: AtomicUsize,
    /// What cancels the requests that run or wait, by session and request id: at most one for each connection.
    running: Mutex<HashMap<RequestKey, Arc<Token>>>,
    rate: Mutex<Rate>,
    /// Held while a connection's thread reads what a body asks for: one body at a time is in memory as values there.
    parsing: Mutex<()>,
}

/// The messages that came in the second from `start`.
struct Rate {
    start: Instant,
    count: usize,
}

/// A session id and a request's id in it.
type RequestKey = (String, String);

impl Shared {
    /// Runs `f` while `token` cancels the request `key`. A second request under the same key is not cancelled by it.
    fn cancellable<T>(&self, key: Option<RequestKey>, token: &Arc<Token>, f: impl FnOnce() -> T) -> T {
        let held = key.filter(|key| {
            let mut running = self.running.lock().unwrap_or_else(PoisonError::into_inner);
            let free = !running.contains_key(key);
            if free {
                running.insert(key.clone(), Arc::clone(token));
            }
            free
        });
        let result = f();
        if let Some(key) = held {
            self.running.lock().unwrap_or_else(PoisonError::into_inner).remove(&key);
        }
        result
    }

    /// Counts a message; the refusal when more came this second than are answered.
    fn admit(&self) -> Result<(), Response> {
        let now = Instant::now();
        let mut rate = self.rate.lock().unwrap_or_else(PoisonError::into_inner);
        if now.saturating_duration_since(rate.start) >= Duration::from_secs(1) {
            *rate = Rate { start: now, count: 0 };
        }
        rate.count = rate.count.saturating_add(1);
        if rate.count <= self.limits.max_messages_per_second {
            return Ok(());
        }
        let mut response = Response::refuse(429, format!("at most {} messages per second are answered", self.limits.max_messages_per_second));
        response.headers.push(("Retry-After", "1".into()));
        Err(response)
    }

    /// What `request` is to the connection's thread: the id of the request it cancels, or the id it can be cancelled by.
    fn asks(&self, request: &Request) -> (Option<String>, Option<String>) {
        let _one = self.parsing.lock().unwrap_or_else(PoisonError::into_inner);
        let message = message_of(request);
        (message.as_ref().and_then(cancelled_request), message.as_ref().and_then(request_key))
    }

    fn cancel(&self, key: &RequestKey) {
        if let Some(token) = self.running.lock().unwrap_or_else(PoisonError::into_inner).get(key) {
            token.cancel();
        }
    }
}

/// One checked request, handed to the thread that owns the [`Server`].
struct Job {
    request: Request,
    /// Cancels the request.
    token: Arc<Token>,
    reply: mpsc::Sender<Part>,
}

/// What the thread that owns the [`Server`] sends back for a request: notifications while it runs, then its reply.
enum Part {
    Event(String),
    Reply(Response),
}

/// The message of a `POST`, when its body is one.
fn message_of(request: &Request) -> Option<Value> {
    if request.method == "POST" { serde_json::from_slice(&request.body).ok() } else { None }
}

/// The answer to a request its client cancelled: HTTP needs one.
fn cancelled(request: &Request) -> Response {
    let id = message_of(request).and_then(|m| m.get("id").cloned()).unwrap_or(Value::Null);
    Response::rpc(200, &error(&id, CANCELLED, "the request was cancelled"))
}

fn accept_loop(listener: &TcpListener, shared: &Arc<Shared>, jobs: &mpsc::Sender<Job>) {
    loop {
        let stream = match listener.accept() {
            Ok((stream, _)) => stream,
            Err(_) => {
                std::thread::sleep(ACCEPT_BACKOFF);
                continue;
            }
        };
        if shared.active.load(Ordering::SeqCst) >= shared.limits.max_connections {
            refuse_busy(stream, shared.limits.max_connections);
            continue;
        }
        shared.active.fetch_add(1, Ordering::SeqCst);
        let (conn_shared, conn_jobs) = (Arc::clone(shared), jobs.clone());
        let spawned = std::thread::Builder::new().name("mcp-http-conn".into()).spawn(move || {
            serve_connection(stream, &conn_shared, &conn_jobs);
            conn_shared.active.fetch_sub(1, Ordering::SeqCst);
        });
        // The stream went into the closure and is dropped (closed) with it.
        if spawned.is_err() {
            shared.active.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

fn refuse_busy(stream: TcpStream, max: usize) {
    let _ = stream.set_write_timeout(Some(BUSY_WRITE_TIMEOUT));
    let response = Response::refuse(503, format!("the server is serving its maximum of {max} connections; try again"));
    let _ = write_response(&stream, &response, true);
    let _ = stream.shutdown(Shutdown::Both);
}

fn serve_connection(stream: TcpStream, shared: &Shared, jobs: &mpsc::Sender<Job>) {
    let limits = &shared.limits;
    if stream.set_write_timeout(Some(limits.idle_timeout)).is_err() {
        return;
    }
    let _ = stream.set_nodelay(true);
    let opened = Instant::now();
    let mut reader = BufReader::new(stream);
    loop {
        let request = match read_request(&mut reader, limits) {
            Ok(request) => request,
            Err(ReadError::Silent) => return,
            Err(ReadError::Reply(response)) => {
                let _ = write_response(reader.get_ref(), &response, true);
                linger(&mut reader);
                return;
            }
        };
        let (close, chunked) = (request.close, request.chunked);
        let mut stream = EventStream { to: reader.get_ref(), chunked, state: Streaming::Not };
        let response = match check(&request, shared).and_then(|()| shared.admit()) {
            Err(refusal) => refusal,
            Ok(()) => {
                let session = request.header("mcp-session-id").unwrap_or_default().to_string();
                let (cancels, key) = shared.asks(&request);
                if let Some(id) = cancels {
                    shared.cancel(&(session, id));
                    Response::empty(202)
                } else {
                    let key = key.map(|id| (session, id));
                    let token = Token::new();
                    shared.cancellable(key, &token, || {
                        let (reply, answer) = mpsc::channel();
                        if jobs.send(Job { request, token: Arc::clone(&token), reply }).is_err() {
                            Response::refuse(503, "the server is shutting down")
                        } else {
                            loop {
                                match answer.recv() {
                                    Ok(Part::Event(message)) => stream.event(&message),
                                    Ok(Part::Reply(response)) => break response,
                                    Err(_) => break Response::refuse(500, "the request was lost inside the server"),
                                }
                            }
                        }
                    })
                }
            }
        };
        let close = close || response.close || opened.elapsed() >= limits.connection_time;
        let (close, written) = match stream.state {
            Streaming::Not => (close, write_response(reader.get_ref(), &response, close).is_ok()),
            // The end of the connection ends a stream that is not chunked.
            _ => (close || !chunked, stream.finish(&response)),
        };
        if !written {
            return;
        }
        if close {
            linger(&mut reader);
            return;
        }
    }
}

/// Ends a connection we close: stop writing, then read a bounded amount of what the peer still
/// sends, so its pending bytes do not turn our reply into a reset.
fn linger(reader: &mut BufReader<TcpStream>) {
    let _ = reader.get_ref().shutdown(Shutdown::Write);
    let deadline = Instant::now() + LINGER_TIME;
    let mut drained = 0usize;
    let mut scratch = [0u8; 8192];
    while drained < LINGER_BYTES {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() || reader.get_ref().set_read_timeout(Some(remaining)).is_err() {
            return;
        }
        match reader.read(&mut scratch) {
            Ok(0) | Err(_) => return,
            Ok(n) => drained = drained.saturating_add(n),
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Reading a request

struct Request {
    method: String,
    /// The path without the query.
    path: String,
    /// Names lower-cased, values trimmed.
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    /// The peer asked to close after this request (or speaks HTTP/1.0).
    close: bool,
    /// The peer reads chunked bodies (it speaks HTTP/1.1).
    chunked: bool,
}

impl Request {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
    }
}

enum ReadError {
    /// The peer left, or stayed idle between requests: close without a reply.
    Silent,
    /// Reply, then close: the rest of the request was not read.
    Reply(Response),
}

fn is_timeout(e: &io::Error) -> bool {
    matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut)
}

/// Fills the buffer, waiting at most until `deadline`. `Ok(None)`: the deadline passed.
fn fill_until(reader: &mut BufReader<TcpStream>, deadline: Instant) -> Result<Option<&[u8]>, ReadError> {
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Ok(None);
        }
        reader.get_ref().set_read_timeout(Some(remaining)).map_err(|_| ReadError::Silent)?;
        match reader.fill_buf() {
            Ok(_) => break,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) if is_timeout(&e) => return Ok(None),
            Err(_) => return Err(ReadError::Silent),
        }
    }
    // Borrowing the buffer once more ends the loop's borrow; the bytes are already there.
    reader.fill_buf().map(Some).map_err(|_| ReadError::Silent)
}

fn timed_out() -> ReadError {
    ReadError::Reply(Response::refuse(408, "the request did not arrive in time"))
}

/// Reads one line of the request head, without its line ending, taking at most `budget` bytes.
fn read_line(reader: &mut BufReader<TcpStream>, deadline: Instant, budget: &mut usize, limits: &Limits) -> Result<Vec<u8>, ReadError> {
    let mut line = Vec::new();
    loop {
        let Some(buffer) = fill_until(reader, deadline)? else {
            return Err(timed_out());
        };
        if buffer.is_empty() {
            return Err(ReadError::Silent);
        }
        let (take, done) = match buffer.iter().position(|b| *b == b'\n') {
            Some(i) => (i.saturating_add(1), true),
            None => (buffer.len(), false),
        };
        if take > *budget {
            return Err(ReadError::Reply(Response::refuse(
                431,
                format!("the request line and headers may be at most {} bytes", limits.max_header_bytes),
            )));
        }
        *budget -= take;
        line.extend_from_slice(buffer.get(..take).unwrap_or_default());
        reader.consume(take);
        if done {
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            return Ok(line);
        }
    }
}

fn bad_request(message: impl Into<String>) -> ReadError {
    ReadError::Reply(Response::refuse(400, message))
}

fn is_token_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b)
}

fn read_request(reader: &mut BufReader<TcpStream>, limits: &Limits) -> Result<Request, ReadError> {
    // Between requests the connection may idle; a few blank lines before a request are skipped.
    let idle_deadline = Instant::now() + limits.idle_timeout;
    let mut blank = 0usize;
    loop {
        let Some(buffer) = fill_until(reader, idle_deadline)? else {
            return Err(ReadError::Silent);
        };
        let first = buffer.first().copied();
        match first {
            None => return Err(ReadError::Silent),
            Some(b'\r' | b'\n') if blank < MAX_BLANK_BYTES => {
                reader.consume(1);
                blank += 1;
            }
            Some(b'\r' | b'\n') => return Err(bad_request("too many blank lines before the request")),
            Some(_) => break,
        }
    }
    // From its first byte, the whole request must arrive within the timeout.
    let deadline = Instant::now() + limits.idle_timeout;
    let mut budget = limits.max_header_bytes;

    let line = read_line(reader, deadline, &mut budget, limits)?;
    let line = String::from_utf8(line).map_err(|_| bad_request("the request line is not text"))?;
    let mut parts = line.split(' ');
    let (Some(method), Some(target), Some(version), None) = (parts.next(), parts.next(), parts.next(), parts.next()) else {
        return Err(bad_request("a request line is METHOD TARGET HTTP/1.1"));
    };
    if method.is_empty() || !method.bytes().all(is_token_char) || !target.starts_with('/') {
        return Err(bad_request("a request line is METHOD TARGET HTTP/1.1"));
    }
    let http10 = match version {
        "HTTP/1.1" => false,
        "HTTP/1.0" => true,
        v if v.starts_with("HTTP/") => {
            return Err(ReadError::Reply(Response::refuse(505, "this server speaks HTTP/1.1")));
        }
        _ => return Err(bad_request("a request line is METHOD TARGET HTTP/1.1")),
    };

    let mut headers: Vec<(String, String)> = Vec::new();
    loop {
        let line = read_line(reader, deadline, &mut budget, limits)?;
        if line.is_empty() {
            break;
        }
        if headers.len() >= limits.max_headers {
            return Err(ReadError::Reply(Response::refuse(431, format!("a request may have at most {} headers", limits.max_headers))));
        }
        let Some(colon) = line.iter().position(|b| *b == b':') else {
            return Err(bad_request("a header line is Name: value"));
        };
        let (name, value) = (line.get(..colon).unwrap_or_default(), line.get(colon.saturating_add(1)..).unwrap_or_default());
        // Folded lines and whitespace before the colon are refused (RFC 9112 § 5).
        if name.is_empty() || !name.iter().all(|b| is_token_char(*b)) {
            return Err(bad_request("a header name is a token followed directly by a colon"));
        }
        if !value.iter().all(|b| *b == b'\t' || (b' '..=b'~').contains(b)) {
            return Err(bad_request("header values must be printable ASCII"));
        }
        let name = String::from_utf8_lossy(name).to_ascii_lowercase();
        let value = String::from_utf8_lossy(value).trim_matches([' ', '\t']).to_string();
        headers.push((name, value));
    }

    let has_token = |name: &str, token: &str| {
        headers.iter().filter(|(n, _)| n == name).flat_map(|(_, v)| v.split(',')).any(|t| t.trim().eq_ignore_ascii_case(token))
    };
    let close = if http10 { !has_token("connection", "keep-alive") } else { has_token("connection", "close") };
    if !http10 && !headers.iter().any(|(n, _)| n == "host") {
        return Err(bad_request("an HTTP/1.1 request needs a Host header"));
    }

    if headers.iter().any(|(n, _)| n == "transfer-encoding") {
        return Err(ReadError::Reply(Response::refuse(501, "send the body with Content-Length; transfer codings are not supported")));
    }
    let mut length: Option<usize> = None;
    for (_, value) in headers.iter().filter(|(n, _)| n == "content-length") {
        if value.is_empty() || value.len() > MAX_LENGTH_DIGITS || !value.bytes().all(|b| b.is_ascii_digit()) {
            return Err(bad_request("Content-Length must be a decimal number"));
        }
        let n: usize = value.parse().map_err(|_| bad_request("Content-Length must be a decimal number"))?;
        if length.is_some_and(|l| l != n) {
            return Err(bad_request("the request has two different Content-Length headers"));
        }
        length = Some(n);
    }
    let length = match length {
        Some(n) => n,
        None if method == "POST" => {
            return Err(ReadError::Reply(Response::refuse(411, "a POST request needs a Content-Length header")));
        }
        None => 0,
    };
    if length > limits.max_body {
        return Err(ReadError::Reply(Response::refuse(413, format!("a request body may be at most {} bytes", limits.max_body))));
    }

    let wants_continue = headers.iter().any(|(n, v)| n == "expect" && v.eq_ignore_ascii_case("100-continue"));
    if wants_continue && length > 0 {
        let mut stream: &TcpStream = reader.get_ref();
        stream.write_all(b"HTTP/1.1 100 Continue\r\n\r\n").map_err(|_| ReadError::Silent)?;
    }
    let mut body = vec![0u8; length];
    let mut filled = 0usize;
    while filled < length {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(timed_out());
        }
        reader.get_ref().set_read_timeout(Some(remaining)).map_err(|_| ReadError::Silent)?;
        let Some(slot) = body.get_mut(filled..) else { break };
        match reader.read(slot) {
            Ok(0) => return Err(ReadError::Silent),
            Ok(n) => filled = filled.saturating_add(n),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) if is_timeout(&e) => return Err(timed_out()),
            Err(_) => return Err(ReadError::Silent),
        }
    }

    let path = target.split_once('?').map_or(target, |(p, _)| p).to_string();
    Ok(Request { method: method.to_string(), path, headers, body, close, chunked: !http10 })
}

// ---------------------------------------------------------------------------------------------
// Checks made on the connection thread, before the request reaches the server

/// The host part of an authority (`host`, `host:port`, `[v6]:port`).
fn host_of(authority: &str) -> &str {
    if let Some(rest) = authority.strip_prefix('[') {
        return rest.split_once(']').map_or(rest, |(host, _)| host);
    }
    authority.split_once(':').map_or(authority, |(host, _)| host)
}

fn is_loopback_name(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost") || host.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

/// May a request carry `Host: host`? On a loopback listener only loopback names. On any other
/// listener also an IP address, but never a DNS name: a web page whose own name was made to
/// resolve to this machine (DNS rebinding) sends that name, and the server cannot tell which
/// names really are its own.
fn host_allowed(host: &str, loopback: bool) -> bool {
    let host = host_of(host);
    is_loopback_name(host) || (!loopback && host.parse::<IpAddr>().is_ok())
}

/// The media type of a header value, without parameters, lower-cased.
fn media_type(value: &str) -> String {
    value.split(';').next().unwrap_or("").trim().to_ascii_lowercase()
}

fn check(request: &Request, shared: &Shared) -> Result<(), Response> {
    if request.path != ENDPOINT {
        return Err(Response::refuse(404, format!("the MCP endpoint is {ENDPOINT}")));
    }
    let host = request.header("host").unwrap_or("");
    // A page whose name resolves to this machine (DNS rebinding) still sends its own name as Host.
    if !host_allowed(host, shared.loopback) {
        let to = if shared.loopback { "localhost" } else { "localhost or an IP address" };
        return Err(Response::refuse(403, format!("this server answers only requests addressed to {to}, not to a host name")));
    }
    // Browsers send Origin; MCP clients outside a browser do not. Only the server's own origin
    // passes, so a web page on another site cannot drive the editor.
    if let Some(origin) = request.header("origin") {
        let same = origin.len() > 7
            && origin.get(..7).is_some_and(|s| s.eq_ignore_ascii_case("http://"))
            && origin.get(7..).is_some_and(|authority| authority.eq_ignore_ascii_case(host));
        if !same {
            return Err(Response::refuse(403, "requests from web pages on other origins are refused"));
        }
    }
    match request.method.as_str() {
        "POST" => {}
        "DELETE" => return Ok(()),
        _ => {
            let mut response = Response::refuse(405, "the endpoint takes POST (a JSON-RPC message) and DELETE (end the session)");
            response.headers.push(("Allow", "POST, DELETE".into()));
            return Err(response);
        }
    }
    if media_type(request.header("content-type").unwrap_or("")) != "application/json" {
        return Err(Response::refuse(415, "the body must be Content-Type: application/json"));
    }
    if let Some(accept) = request.header("accept") {
        let ok = accept
            .split(',')
            .map(media_type)
            .any(|t| matches!(t.as_str(), "application/json" | "text/event-stream" | "application/*" | "text/*" | "*/*"));
        if !ok {
            return Err(Response::refuse(406, "the server replies with application/json or text/event-stream"));
        }
    }
    Ok(())
}

fn accepts_stream(request: &Request) -> bool {
    request.header("accept").is_some_and(|a| a.split(',').map(media_type).any(|t| t == "text/event-stream"))
}

// ---------------------------------------------------------------------------------------------
// Answering, on the thread that owns the server

struct Session {
    /// The session's log level and subscriptions.
    client: Client,
    /// The dispatcher's tick at its last request; the smallest is ended first when sessions run out.
    last_used: u64,
}

struct Dispatcher {
    server: Server,
    limits: Limits,
    sessions: HashMap<String, Session>,
    tick: u64,
}

impl Dispatcher {
    fn new(server: Server, limits: Limits) -> Self {
        Self { server, limits, sessions: HashMap::new(), tick: 0 }
    }

    fn answer(&mut self, job: &Job) -> Response {
        let request = &job.request;
        self.tick = self.tick.saturating_add(1);
        if request.method == "DELETE" {
            return self.end_session(request);
        }
        self.post(request, &job.token, &job.reply)
    }

    fn end_session(&mut self, request: &Request) -> Response {
        let Some(id) = request.header("mcp-session-id") else {
            return Response::refuse(400, "DELETE needs the Mcp-Session-Id header of the session to end");
        };
        match self.sessions.remove(id) {
            Some(_) => Response::empty(200),
            None => Response::refuse(404, "no such session"),
        }
    }

    fn post(&mut self, request: &Request, token: &Arc<Token>, events: &mpsc::Sender<Part>) -> Response {
        let Ok(text) = std::str::from_utf8(&request.body) else {
            return Response::rpc(400, &error(&Value::Null, PARSE_ERROR, "the request is not valid UTF-8"));
        };
        let message: Value = match serde_json::from_str(text) {
            Ok(message) => message,
            Err(e) => return Response::rpc(400, &error(&Value::Null, PARSE_ERROR, format!("not valid JSON: {e}"))),
        };
        if !message.is_object() {
            return Response::rpc(400, &error(&Value::Null, INVALID_REQUEST, "a message is one JSON object (batches are not supported)"));
        }
        let initialize = message.get("method").and_then(Value::as_str) == Some("initialize") && message.get("id").is_some();
        let session = if initialize {
            None
        } else {
            let Some(id) = request.header("mcp-session-id") else {
                return Response::refuse(400, "send `initialize` first, then the Mcp-Session-Id header its reply carries");
            };
            if !self.sessions.contains_key(id) {
                return Response::refuse(404, "no such session (it ended or was replaced): send `initialize` again");
            }
            if let Some(version) = request.header("mcp-protocol-version")
                && !PROTOCOL_VERSIONS.contains(&version)
            {
                return Response::refuse(400, format!("MCP-Protocol-Version `{version}` is not one this server speaks"));
            }
            Some(id.to_string())
        };

        // The log level and the subscriptions are per session; the server holds the current request's. Handling
        // the request also finds what other sessions changed since.
        self.server.client = session.as_ref().and_then(|id| self.sessions.get_mut(id)).map(|s| std::mem::take(&mut s.client)).unwrap_or_default();
        self.server.token = Some(Arc::clone(token));
        let streams = accepts_stream(request);
        self.server.notify = (streams && !initialize).then(|| {
            let events = events.clone();
            let notify: Rc<dyn Fn(&Value)> = Rc::new(move |notification: &Value| {
                let _ = events.send(Part::Event(notification.to_string()));
            });
            notify
        });
        let reply = self.server.handle(&message);
        self.server.notify = None;
        let notifications = self.server.take_notifications();
        let client = std::mem::take(&mut self.server.client);
        if let Some(s) = session.as_ref().and_then(|id| self.sessions.get_mut(id)) {
            s.client = client;
            s.last_used = self.tick;
        }

        let Some(reply) = reply else {
            return if message.get("id").is_some() && token.is_cancelled() { cancelled(request) } else { Response::empty(202) };
        };
        // An error that names no request (a malformed message) is the client's fault.
        if reply.get("error").is_some() && reply.get("id").is_none_or(Value::is_null) {
            return Response::rpc(400, &reply);
        }
        let mut response = if !notifications.is_empty() && streams {
            let reply_line = reply.to_string();
            let mut body = String::new();
            for line in notifications.iter().map(String::as_str).chain(std::iter::once(reply_line.as_str())) {
                body.push_str(&event(line));
            }
            Response { status: 200, headers: vec![("Content-Type", EVENT_STREAM.into())], body: body.into_bytes(), close: false }
        } else {
            Response::rpc(200, &reply)
        };
        if initialize && reply.get("result").is_some() {
            response.headers.push(("Mcp-Session-Id", self.open_session()));
        }
        response
    }

    fn open_session(&mut self) -> String {
        while self.sessions.len() >= self.limits.max_sessions.max(1) {
            let Some(oldest) = self.sessions.iter().min_by_key(|(_, s)| s.last_used).map(|(id, _)| id.clone()) else {
                break;
            };
            self.sessions.remove(&oldest);
        }
        let mut id = session_id(self.tick);
        // Two equal ids would need a hash collision; take another rather than share a session.
        while self.sessions.contains_key(&id) {
            self.tick = self.tick.saturating_add(1);
            id = session_id(self.tick);
        }
        self.sessions.insert(id.clone(), Session { client: Client::default(), last_used: self.tick });
        id
    }
}

/// 128 bits from the standard library's randomly keyed hasher, as hex. Unguessable, but not a
/// credential: the transport has no authentication, and anyone who can reach the port can start a
/// session of their own.
fn session_id(salt: u64) -> String {
    use std::hash::{BuildHasher, Hasher};
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let mut id = String::with_capacity(32);
    for half in 0..2u64 {
        let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
        hasher.write_u64(salt);
        hasher.write_u64(half);
        hasher.write_u128(nanos);
        id.push_str(&format!("{:016x}", hasher.finish()));
    }
    id
}

// ---------------------------------------------------------------------------------------------
// Writing a response

struct Response {
    status: u16,
    headers: Vec<(&'static str, String)>,
    body: Vec<u8>,
    /// Close the connection after this response.
    close: bool,
}

impl Response {
    fn empty(status: u16) -> Self {
        Self { status, headers: Vec::new(), body: Vec::new(), close: false }
    }

    fn rpc(status: u16, message: &Value) -> Self {
        Self { status, headers: vec![("Content-Type", "application/json".into())], body: message.to_string().into_bytes(), close: false }
    }

    /// An HTTP error. The body is a JSON-RPC error without an id, as the transport allows, so a
    /// client shows the reason. The statuses that leave the request unread close the connection.
    fn refuse(status: u16, message: impl Into<String>) -> Self {
        let mut response = Self::rpc(status, &error(&Value::Null, INVALID_REQUEST, message));
        response.close = matches!(status, 408 | 411 | 413 | 431 | 501 | 503 | 505);
        response
    }
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        202 => "Accepted",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        406 => "Not Acceptable",
        408 => "Request Timeout",
        411 => "Length Required",
        413 => "Content Too Large",
        415 => "Unsupported Media Type",
        429 => "Too Many Requests",
        431 => "Request Header Fields Too Large",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        503 => "Service Unavailable",
        505 => "HTTP Version Not Supported",
        _ => "",
    }
}

const EVENT_STREAM: &str = "text/event-stream";

/// A server-sent event of one message, which is one line.
fn event(message: &str) -> String {
    format!("event: message\ndata: {message}\n\n")
}

/// Where a streamed reply stands.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Streaming {
    /// Nothing was sent: the reply can be an ordinary one.
    Not,
    Open,
    /// The peer has gone, or takes nothing: the rest is dropped. The request goes on, as the protocol asks.
    Lost,
}

/// The reply of a request that sends events while it runs.
struct EventStream<'a> {
    to: &'a TcpStream,
    chunked: bool,
    state: Streaming,
}

impl EventStream<'_> {
    /// Sends one message, after the head of the reply when it is the first.
    fn event(&mut self, message: &str) {
        let head = match self.state {
            Streaming::Lost => return,
            Streaming::Open => "",
            Streaming::Not if self.chunked => {
                "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\nCache-Control: no-store\r\n\r\n"
            }
            Streaming::Not => "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n",
        };
        let sent = self.write(head.as_bytes()).and_then(|()| self.chunk(event(message).as_bytes()));
        self.state = if sent.is_ok() { Streaming::Open } else { Streaming::Lost };
    }

    /// Sends the reply as the last events. False when the peer did not take it.
    fn finish(&mut self, response: &Response) -> bool {
        if self.state == Streaming::Lost {
            return false;
        }
        let events = response.headers.iter().any(|(name, value)| *name == "Content-Type" && value == EVENT_STREAM);
        let sent = if events {
            self.chunk(&response.body)
        } else if response.body.is_empty() {
            Ok(())
        } else {
            self.chunk(event(&String::from_utf8_lossy(&response.body)).as_bytes())
        };
        sent.and_then(|()| if self.chunked { self.write(b"0\r\n\r\n") } else { Ok(()) }).is_ok()
    }

    fn chunk(&mut self, bytes: &[u8]) -> io::Result<()> {
        if !self.chunked {
            return self.write(bytes);
        }
        // An empty chunk would end the body.
        if bytes.is_empty() {
            return Ok(());
        }
        self.write(format!("{:x}\r\n", bytes.len()).as_bytes())?;
        self.write(bytes)?;
        self.write(b"\r\n")
    }

    fn write(&mut self, bytes: &[u8]) -> io::Result<()> {
        let mut to = self.to;
        to.write_all(bytes)?;
        to.flush()
    }
}

fn write_response(mut stream: &TcpStream, response: &Response, close: bool) -> io::Result<()> {
    let mut head =
        format!("HTTP/1.1 {} {}\r\nContent-Length: {}\r\nCache-Control: no-store\r\n", response.status, reason(response.status), response.body.len());
    for (name, value) in &response.headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    if close {
        head.push_str("Connection: close\r\n");
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes())?;
    stream.write_all(&response.body)?;
    stream.flush()
}

#[cfg(test)]
#[path = "http_tests.rs"]
mod tests;
