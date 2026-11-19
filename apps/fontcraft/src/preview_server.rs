//! The web preview server (desktop only): serves pages to the local network on a free port, 4
//! connections at a time, 64 KB of request headers. Two sites use it:
//! - [`Hinting`]: the preview page and the hinted font while the preview-address dialog is open
//!   (owner decision M10 § 7, question 5). Rows: `hint.tt.showPreviewAddress`. The dialog and the
//!   page are `fontcraft_ui_egui::dialogs::tt_preview_address`; this module only serves them.
//! - External Preview (`crate::external_preview`), which answers by the peer's address.
//!
//! A small HTTP/1.1 server on `std::net`. Everything a peer sends is untrusted:
//! - Only `GET` and `HEAD` are answered, one request per connection, with no request body.
//! - The request head is capped in bytes and header lines, connections and the request rate are
//!   capped ([`Limits`]), and a slow peer runs into [`REQUEST_TIMEOUT`].
//! - Every page lives under a random path, `/<token>/…`; every other path is 404. A device that
//!   only knows the port, or a web page that points a host name at this computer, cannot read
//!   the font without the address the app shows. What lies under the path is the [`Pages`]' own.
//!
//! [`Scope::LocalNetwork`] listens on every IPv4 interface and shows the address of the interface
//! that carries the default route; [`Scope::ThisComputer`] listens on loopback only. The accept
//! loop polls a stop flag, so [`Server::stop`] closes the port within [`ACCEPT_POLL`].

use std::collections::hash_map::RandomState;
use std::fmt::Write as _;
use std::hash::{BuildHasher, Hasher};
use std::io::{self, Read, Write};
use std::net::{IpAddr, Ipv4Addr, Shutdown, SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime};

use fontcraft_ui_egui::dialogs::tt_preview_address::{FONT_FILE, PreviewHost, Scope, Site};

/// Most connections served at once; more are answered 503 and closed.
pub const MAX_CONNECTIONS: usize = 4;
/// Largest request head (request line and headers together).
pub const MAX_HEADER_BYTES: usize = 64 << 10;
/// Most header lines in one request.
pub const MAX_HEADERS: usize = 100;
/// Most requests answered per second over all connections; more get 429.
pub const MAX_REQUESTS_PER_SECOND: usize = 50;
/// How long a request may take to arrive in full, from the connection's start.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
/// How long one write of the reply may block (a font over a slow network).
const WRITE_TIMEOUT: Duration = Duration::from_secs(30);
/// How often the accept loop looks at the stop flag while no connection arrives.
const ACCEPT_POLL: Duration = Duration::from_millis(25);
/// A connection refused for being one too many gets this long to take its 503.
const BUSY_WRITE_TIMEOUT: Duration = Duration::from_secs(1);
/// After the reply, at most this much of what the peer still sends is read, for at most
/// [`LINGER_TIME`], so the reply is not lost to a reset.
const LINGER_BYTES: usize = 64 << 10;
const LINGER_TIME: Duration = Duration::from_millis(500);
/// A documentation address (TEST-NET-1): connecting a UDP socket to it picks the interface of
/// the default route without sending anything.
const ROUTE_PROBE: (Ipv4Addr, u16) = (Ipv4Addr::new(192, 0, 2, 1), 9);

/// The server's caps. [`Limits::default`] holds the named constants above; tests shrink them.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub max_connections: usize,
    pub max_header_bytes: usize,
    pub max_headers: usize,
    pub max_requests_per_second: usize,
    pub request_timeout: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            max_connections: MAX_CONNECTIONS,
            max_header_bytes: MAX_HEADER_BYTES,
            max_headers: MAX_HEADERS,
            max_requests_per_second: MAX_REQUESTS_PER_SECOND,
            request_timeout: REQUEST_TIMEOUT,
        }
    }
}

/// What a server answers under its random path.
pub trait Pages: Send + Sync + 'static {
    /// The reply to a `GET` of `/<token>/<rest>` from `peer` (`rest` is empty for the page
    /// itself). A `HEAD` gets the same reply without its body.
    fn answer(&self, rest: &str, peer: IpAddr) -> Reply;
}

/// The hinting preview's site: its page, and the font beside it at [`FONT_FILE`].
pub struct Hinting {
    site: Mutex<Arc<Site>>,
}

impl Pages for Hinting {
    fn answer(&self, rest: &str, _peer: IpAddr) -> Reply {
        let site = Arc::clone(&self.site.lock().unwrap_or_else(PoisonError::into_inner));
        if rest.is_empty() {
            Reply::page(200, site.page.clone())
        } else if rest == FONT_FILE {
            Reply { status: 200, content_type: "font/ttf", body: site.font.as_slice().into(), location: None, head_only: false }
        } else {
            Reply::text(404, "Not found.")
        }
    }
}

/// The app's [`PreviewHost`]: at most one server, replaced when the scope changes.
#[derive(Default)]
pub struct Host {
    server: Option<Server>,
}

impl PreviewHost for Host {
    fn serve(&mut self, site: Site, scope: Scope) -> Result<String, String> {
        if let Some(server) = self.server.as_ref().filter(|s| s.scope == scope) {
            server.replace(site);
            return Ok(server.address.clone());
        }
        let started = Server::start(site, scope, Limits::default())?;
        let address = started.address.clone();
        if let Some(mut old) = self.server.replace(started) {
            old.stop();
        }
        Ok(address)
    }

    fn stop(&mut self) {
        if let Some(mut server) = self.server.take() {
            server.stop();
        }
    }
}

/// One running server. Dropping it stops it.
pub struct Server<P: Pages = Hinting> {
    /// The page's address, as the app shows it.
    pub address: String,
    pub scope: Scope,
    shared: Arc<Shared<P>>,
    accept: Option<JoinHandle<()>>,
}

/// What the accepting and the connection threads share.
struct Shared<P> {
    pages: Arc<P>,
    token: String,
    limits: Limits,
    stop: AtomicBool,
    /// Connections being served.
    active: AtomicUsize,
    /// The start of the current one-second window and the requests answered in it.
    rate: Mutex<(Instant, usize)>,
}

impl Server<Hinting> {
    /// Listens on a free port for `scope` and serves `site` until stopped.
    pub fn start(site: Site, scope: Scope, limits: Limits) -> Result<Server<Hinting>, String> {
        Server::serve(Arc::new(Hinting { site: Mutex::new(Arc::new(site)) }), scope, limits)
    }

    /// Serves `site` from the next request on.
    pub fn replace(&self, site: Site) {
        *self.shared.pages.site.lock().unwrap_or_else(PoisonError::into_inner) = Arc::new(site);
    }
}

impl<P: Pages> Server<P> {
    /// Listens on a free port for `scope` and serves `pages` until stopped.
    pub fn serve(pages: Arc<P>, scope: Scope, limits: Limits) -> Result<Server<P>, String> {
        let (bind, shown) = match scope {
            Scope::ThisComputer => (Ipv4Addr::LOCALHOST, IpAddr::V4(Ipv4Addr::LOCALHOST)),
            Scope::LocalNetwork => {
                let ip = network_address().ok_or("this computer has no local network address; serve to this computer only instead")?;
                (Ipv4Addr::UNSPECIFIED, ip)
            }
        };
        let listener = TcpListener::bind((bind, 0)).map_err(|e| format!("no port could be opened: {e}"))?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        let token = token();
        let address = format!("http://{}/{token}/", SocketAddr::new(shown, port));
        let shared = Arc::new(Shared {
            pages,
            token,
            limits,
            stop: AtomicBool::new(false),
            active: AtomicUsize::new(0),
            rate: Mutex::new((Instant::now(), 0)),
        });
        let accept_shared = Arc::clone(&shared);
        let accept = std::thread::Builder::new()
            .name("preview-accept".into())
            .spawn(move || accept_loop(&listener, &accept_shared))
            .map_err(|e| format!("the server thread could not start: {e}"))?;
        Ok(Server { address, scope, shared, accept: Some(accept) })
    }

    /// What the server answers with.
    pub fn pages(&self) -> &Arc<P> {
        &self.shared.pages
    }

    /// Stops accepting connections and closes the port; requests already being read are not
    /// answered.
    pub fn stop(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        if let Some(accept) = self.accept.take() {
            let _ = accept.join();
        }
    }
}

impl<P: Pages> Drop for Server<P> {
    fn drop(&mut self) {
        self.stop();
    }
}

/// The address of the interface that carries the default route, if it has one.
fn network_address() -> Option<IpAddr> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;
    socket.connect(ROUTE_PROBE).ok()?;
    let ip = socket.local_addr().ok()?.ip();
    (!ip.is_unspecified() && !ip.is_loopback()).then_some(ip)
}

/// 128 random bits in hex. `RandomState` draws its keys from the system's random source once
/// per thread and changes them for every new state, so the hashes cannot be predicted.
fn token() -> String {
    let nanos = SystemTime::now().duration_since(SystemTime::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
    let mut out = String::with_capacity(32);
    for part in 0..2u64 {
        let mut h = RandomState::new().build_hasher();
        h.write_u64(part);
        h.write_u128(nanos);
        let _ = write!(out, "{:016x}", h.finish());
    }
    out
}

fn accept_loop<P: Pages>(listener: &TcpListener, shared: &Arc<Shared<P>>) {
    while !shared.stop.load(Ordering::SeqCst) {
        let (stream, peer) = match listener.accept() {
            Ok((stream, peer)) => (stream, peer.ip()),
            Err(_) => {
                std::thread::sleep(ACCEPT_POLL);
                continue;
            }
        };
        // Accepted sockets inherit the listener's non-blocking mode on some platforms.
        if stream.set_nonblocking(false).is_err() {
            continue;
        }
        if shared.active.load(Ordering::SeqCst) >= shared.limits.max_connections {
            refuse_busy(&stream, shared.limits.max_connections);
            continue;
        }
        shared.active.fetch_add(1, Ordering::SeqCst);
        let conn_shared = Arc::clone(shared);
        let spawned = std::thread::Builder::new().name("preview-conn".into()).spawn(move || {
            serve_connection(&stream, peer, &conn_shared);
            conn_shared.active.fetch_sub(1, Ordering::SeqCst);
        });
        // The stream went into the closure and is closed with it.
        if spawned.is_err() {
            shared.active.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

fn refuse_busy(stream: &TcpStream, max: usize) {
    let _ = stream.set_write_timeout(Some(BUSY_WRITE_TIMEOUT));
    let reply = Reply::text(503, format!("The preview serves at most {max} connections at a time; try again."));
    let _ = write_reply(stream, &reply);
    let _ = stream.shutdown(Shutdown::Both);
}

fn serve_connection<P: Pages>(stream: &TcpStream, peer: IpAddr, shared: &Shared<P>) {
    if stream.set_write_timeout(Some(WRITE_TIMEOUT)).is_err() {
        return;
    }
    let _ = stream.set_nodelay(true);
    let deadline = Instant::now() + shared.limits.request_timeout;
    let reply = match read_head(stream, deadline, &shared.limits) {
        Ok(None) => return,
        Ok(Some(head)) => answer(&head, peer, shared),
        Err(reply) => reply,
    };
    if shared.stop.load(Ordering::SeqCst) {
        return;
    }
    if write_reply(stream, &reply).is_ok() {
        linger(stream);
    }
}

// ---------------------------------------------------------------------------------------------
// Reading a request

/// What a request asks for.
struct Head {
    head_only: bool,
    /// The path, without the query.
    path: String,
}

/// Reads the request head: the request line and the headers, up to the blank line. `Ok(None)`:
/// the peer left or sent nothing in time.
fn read_head(mut stream: &TcpStream, deadline: Instant, limits: &Limits) -> Result<Option<Head>, Reply> {
    let mut buf: Vec<u8> = Vec::new();
    let mut chunk = [0u8; 4096];
    let end = loop {
        if let Some(end) = head_end(&buf) {
            break end;
        }
        if buf.len() >= limits.max_header_bytes {
            return Err(Reply::text(431, format!("The request line and headers may be at most {} bytes.", limits.max_header_bytes)));
        }
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() || stream.set_read_timeout(Some(remaining)).is_err() {
            return if buf.is_empty() { Ok(None) } else { Err(Reply::text(408, "The request did not arrive in time.")) };
        }
        let want = limits.max_header_bytes.saturating_sub(buf.len()).min(chunk.len());
        let Some(slot) = chunk.get_mut(..want) else { return Ok(None) };
        match stream.read(slot) {
            Ok(0) => return Ok(None),
            Ok(n) => buf.extend_from_slice(slot.get(..n).unwrap_or_default()),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) if matches!(e.kind(), io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut) => {
                return if buf.is_empty() { Ok(None) } else { Err(Reply::text(408, "The request did not arrive in time.")) };
            }
            Err(_) => return Ok(None),
        }
    };
    parse_head(buf.get(..end).unwrap_or_default(), limits).map(Some)
}

/// Where the head ends (the index of its blank line), if it has.
fn head_end(buf: &[u8]) -> Option<usize> {
    let crlf = buf.windows(4).position(|w| w == b"\r\n\r\n");
    let lf = buf.windows(2).position(|w| w == b"\n\n");
    match (crlf, lf) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

fn bad(message: &str) -> Reply {
    Reply::text(400, message)
}

fn is_token_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&b)
}

fn parse_head(head: &[u8], limits: &Limits) -> Result<Head, Reply> {
    let text = std::str::from_utf8(head).map_err(|_| bad("The request is not text."))?;
    let mut lines = text.split('\n').map(|l| l.strip_suffix('\r').unwrap_or(l)).skip_while(|l| l.is_empty());
    let request_line = lines.next().ok_or_else(|| bad("The request is empty."))?;
    let mut parts = request_line.split(' ');
    let (Some(method), Some(target), Some(version), None) = (parts.next(), parts.next(), parts.next(), parts.next()) else {
        return Err(bad("A request line is METHOD TARGET HTTP/1.1."));
    };
    if method.is_empty() || !method.bytes().all(is_token_char) || !target.starts_with('/') {
        return Err(bad("A request line is METHOD TARGET HTTP/1.1."));
    }
    if !matches!(version, "HTTP/1.1" | "HTTP/1.0") {
        return Err(if version.starts_with("HTTP/") {
            Reply::text(505, "This server speaks HTTP/1.1.")
        } else {
            bad("A request line is METHOD TARGET HTTP/1.1.")
        });
    }
    let mut count = 0usize;
    for line in lines {
        count += 1;
        if count > limits.max_headers {
            return Err(Reply::text(431, format!("A request may have at most {} headers.", limits.max_headers)));
        }
        let Some((name, value)) = line.split_once(':') else { return Err(bad("A header line is Name: value.")) };
        if name.is_empty() || !name.bytes().all(is_token_char) {
            return Err(bad("A header name is a token followed directly by a colon."));
        }
        let value = value.trim_matches([' ', '\t']);
        let has_body = (name.eq_ignore_ascii_case("content-length") && value != "0") || name.eq_ignore_ascii_case("transfer-encoding");
        if has_body {
            return Err(Reply::text(413, "This server takes requests without a body."));
        }
    }
    let head_only = match method {
        "GET" => false,
        "HEAD" => true,
        _ => return Err(Reply::text(405, "Only GET and HEAD are answered.")),
    };
    let path = target.split(['?', '#']).next().unwrap_or_default().to_string();
    Ok(Head { head_only, path })
}

// ---------------------------------------------------------------------------------------------
// Answering

/// A reply to one request.
pub struct Reply {
    status: u16,
    content_type: &'static str,
    body: Arc<[u8]>,
    /// The location of a redirect.
    location: Option<String>,
    head_only: bool,
}

impl Reply {
    /// Plain text.
    pub fn text(status: u16, message: impl Into<String>) -> Self {
        Reply { status, content_type: "text/plain; charset=utf-8", body: message.into().into_bytes().into(), location: None, head_only: false }
    }

    /// An HTML page.
    pub fn page(status: u16, html: String) -> Self {
        Reply { status, content_type: "text/html; charset=utf-8", body: html.into_bytes().into(), location: None, head_only: false }
    }
}

/// Counts a request against the rate cap: `false` when this second's allowance is spent.
fn take_rate<P>(shared: &Shared<P>) -> bool {
    let mut rate = shared.rate.lock().unwrap_or_else(PoisonError::into_inner);
    let now = Instant::now();
    if now.saturating_duration_since(rate.0) >= Duration::from_secs(1) {
        *rate = (now, 0);
    }
    if rate.1 >= shared.limits.max_requests_per_second {
        return false;
    }
    rate.1 += 1;
    true
}

fn answer<P: Pages>(head: &Head, peer: IpAddr, shared: &Shared<P>) -> Reply {
    if !take_rate(shared) {
        return Reply::text(429, "Too many requests; try again in a moment.");
    }
    let root = format!("/{}", shared.token);
    let mut reply = if head.path == root {
        Reply { status: 301, location: Some(format!("{root}/")), ..Reply::text(301, "Moved to the page's address with a final slash.") }
    } else if let Some(rest) = head.path.strip_prefix(root.as_str()).and_then(|r| r.strip_prefix('/')) {
        shared.pages.answer(rest, peer)
    } else {
        Reply::text(404, "Not found.")
    };
    reply.head_only = head.head_only;
    reply
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        301 => "Moved Permanently",
        400 => "Bad Request",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        413 => "Content Too Large",
        429 => "Too Many Requests",
        431 => "Request Header Fields Too Large",
        503 => "Service Unavailable",
        505 => "HTTP Version Not Supported",
        _ => "Error",
    }
}

/// Writes `reply`, its body unless it answers `HEAD`, and closes the writing side.
fn write_reply(mut stream: &TcpStream, reply: &Reply) -> io::Result<()> {
    let mut head = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nReferrer-Policy: no-referrer\r\nContent-Security-Policy: default-src 'none'; font-src 'self'; style-src 'unsafe-inline'\r\nConnection: close\r\n",
        reply.status,
        reason(reply.status),
        reply.content_type,
        reply.body.len()
    );
    if reply.status == 405 {
        head.push_str("Allow: GET, HEAD\r\n");
    }
    if let Some(location) = &reply.location {
        let _ = write!(head, "Location: {location}\r\n");
    }
    head.push_str("\r\n");
    stream.write_all(head.as_bytes())?;
    if !reply.head_only {
        stream.write_all(&reply.body)?;
    }
    stream.flush()?;
    stream.shutdown(Shutdown::Write)
}

/// Reads a bounded amount of what the peer still sends after the reply, so its pending bytes do
/// not turn the reply into a reset.
fn linger(mut stream: &TcpStream) {
    let deadline = Instant::now() + LINGER_TIME;
    let mut drained = 0usize;
    let mut scratch = [0u8; 4096];
    while drained < LINGER_BYTES {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() || stream.set_read_timeout(Some(remaining)).is_err() {
            return;
        }
        match stream.read(&mut scratch) {
            Ok(0) | Err(_) => return,
            Ok(n) => drained = drained.saturating_add(n),
        }
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;

    fn site() -> Site {
        Site { page: "<!doctype html><title>preview</title>".into(), font: vec![0, 1, 0, 0, 7, 7] }
    }

    /// The path and the socket address of `address` (`http://ip:port/token/`).
    fn split(address: &str) -> (SocketAddr, String) {
        let rest = address.strip_prefix("http://").unwrap();
        let (host, path) = rest.split_once('/').unwrap();
        (host.parse().unwrap(), format!("/{path}"))
    }

    /// Sends `request` as is and returns the status code and the body.
    fn send(at: SocketAddr, request: &[u8]) -> (u16, Vec<u8>) {
        let mut stream = TcpStream::connect(at).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        stream.write_all(request).unwrap();
        let mut out = Vec::new();
        stream.read_to_end(&mut out).unwrap();
        let end = out.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
        let status = std::str::from_utf8(&out[9..12]).unwrap().parse().unwrap();
        (status, out[end + 4..].to_vec())
    }

    fn get(at: SocketAddr, path: &str) -> (u16, Vec<u8>) {
        send(at, format!("GET {path} HTTP/1.1\r\nHost: {at}\r\n\r\n").as_bytes())
    }

    #[test]
    fn show_preview_address_server_serves_the_page_and_font_until_stopped() {
        let mut server = Server::start(site(), Scope::ThisComputer, Limits::default()).unwrap();
        let (at, path) = split(&server.address);
        assert!(at.ip().is_loopback(), "{}", server.address);
        assert_eq!(path.len(), 34, "a 32-digit token between slashes: {path}");

        // Another thread stands in for the other device's browser.
        let page = std::thread::spawn(move || get(at, &path)).join().unwrap();
        assert_eq!(page, (200, site().page.into_bytes()));
        let (_, path) = split(&server.address);
        assert_eq!(get(at, &format!("{path}{FONT_FILE}")), (200, site().font));
        assert_eq!(get(at, path.strip_suffix('/').unwrap()).0, 301, "the page's address without its final slash redirects");
        assert_eq!(get(at, "/").0, 404, "nothing is served outside the token");
        assert_eq!(get(at, &format!("/{FONT_FILE}")).0, 404);
        assert_eq!(get(at, "/0123456789abcdef0123456789abcdef/").0, 404);
        assert_eq!(send(at, format!("HEAD {path} HTTP/1.1\r\nHost: x\r\n\r\n").as_bytes()), (200, Vec::new()));

        // A newer font is served at the same address.
        server.replace(Site { page: "new".into(), font: vec![1] });
        assert_eq!(get(at, &path), (200, b"new".to_vec()));

        server.stop();
        assert!(TcpStream::connect_timeout(&at, Duration::from_secs(2)).is_err(), "the port is closed once stopped");
    }

    #[test]
    fn show_preview_address_server_refuses_what_it_does_not_serve() {
        let limits = Limits { max_header_bytes: 1024, max_headers: 4, ..Limits::default() };
        let server = Server::start(site(), Scope::ThisComputer, limits).unwrap();
        let (at, path) = split(&server.address);
        assert_eq!(send(at, format!("POST {path} HTTP/1.1\r\nHost: x\r\nContent-Length: 2\r\n\r\nab").as_bytes()).0, 413);
        assert_eq!(send(at, format!("DELETE {path} HTTP/1.1\r\nHost: x\r\n\r\n").as_bytes()).0, 405);
        assert_eq!(send(at, format!("GET {path} HTTP/2.0\r\n\r\n").as_bytes()).0, 505);
        assert_eq!(send(at, b"nonsense\r\n\r\n").0, 400);
        let many: String = (0..5).map(|i| format!("X-{i}: y\r\n")).collect();
        assert_eq!(send(at, format!("GET {path} HTTP/1.1\r\n{many}\r\n").as_bytes()).0, 431);
        let long = "a".repeat(2048);
        assert_eq!(send(at, format!("GET {path} HTTP/1.1\r\nX: {long}\r\n\r\n").as_bytes()).0, 431);
    }

    #[test]
    fn show_preview_address_server_caps_the_request_rate_and_times_out_slow_peers() {
        let limits = Limits { max_requests_per_second: 2, request_timeout: Duration::from_millis(300), ..Limits::default() };
        let server = Server::start(site(), Scope::ThisComputer, limits).unwrap();
        let (at, path) = split(&server.address);
        let codes: Vec<u16> = (0..4).map(|_| get(at, &path).0).collect();
        assert!(codes.contains(&429), "{codes:?}");

        let mut slow = TcpStream::connect(at).unwrap();
        slow.write_all(b"GET / HTTP/1.1\r\n").unwrap();
        slow.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut out = Vec::new();
        slow.read_to_end(&mut out).unwrap();
        assert!(out.starts_with(b"HTTP/1.1 408"), "{}", String::from_utf8_lossy(&out));
    }

    #[test]
    fn show_preview_address_host_keeps_the_address_for_the_same_scope() {
        let mut host = Host::default();
        let first = host.serve(site(), Scope::ThisComputer).unwrap();
        let again = host.serve(Site { page: "v2".into(), font: vec![2] }, Scope::ThisComputer).unwrap();
        assert_eq!(first, again);
        let (at, path) = split(&again);
        assert_eq!(get(at, &path), (200, b"v2".to_vec()));
        host.stop();
        assert!(TcpStream::connect_timeout(&at, Duration::from_secs(2)).is_err());
    }
}
