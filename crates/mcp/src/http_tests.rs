//! Behaviour tests of the HTTP transport: real sockets on 127.0.0.1, a headless server per test.

use super::*;
use crate::Headless;
use serde_json::json;
use std::net::SocketAddr;

fn start(limits: Limits) -> SocketAddr {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    // The server is built on its own thread: the backend need not be `Send`.
    std::thread::spawn(move || serve(Server::new(Box::new(Headless::with_document())), listener, limits));
    addr
}

struct Reply {
    status: u16,
    headers: Vec<(String, String)>,
    body: String,
}

impl Reply {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers.iter().find(|(n, _)| n.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
    }
    fn json(&self) -> Value {
        serde_json::from_str(&self.body).unwrap_or_else(|e| panic!("{e}: {}", self.body))
    }
}

fn connect(addr: SocketAddr) -> TcpStream {
    let stream = TcpStream::connect(addr).unwrap();
    stream.set_read_timeout(Some(Duration::from_secs(10))).unwrap();
    stream
}

fn try_read_reply(stream: &mut TcpStream) -> io::Result<Reply> {
    let bad = |what: &str| io::Error::new(io::ErrorKind::InvalidData, what.to_string());
    let mut head = Vec::new();
    let mut byte = [0u8];
    while !head.ends_with(b"\r\n\r\n") {
        if stream.read(&mut byte)? == 0 {
            return Err(io::Error::new(io::ErrorKind::UnexpectedEof, format!("closed before a reply: {:?}", String::from_utf8_lossy(&head))));
        }
        head.push(byte[0]);
    }
    let head = String::from_utf8(head).map_err(|_| bad("head is not UTF-8"))?;
    let mut lines = head.split("\r\n");
    let status = lines.next().and_then(|l| l.split(' ').nth(1)).and_then(|s| s.parse().ok()).ok_or_else(|| bad("status line"))?;
    let headers: Vec<(String, String)> = lines.filter_map(|l| l.split_once(':')).map(|(n, v)| (n.trim().to_string(), v.trim().to_string())).collect();
    if headers.iter().any(|(n, v)| n.eq_ignore_ascii_case("transfer-encoding") && v == "chunked") {
        let mut body = Vec::new();
        loop {
            let mut size = Vec::new();
            while !size.ends_with(b"\r\n") {
                stream.read_exact(&mut byte)?;
                size.push(byte[0]);
            }
            let size = usize::from_str_radix(String::from_utf8_lossy(&size).trim(), 16).map_err(|_| bad("chunk size"))?;
            let mut chunk = vec![0u8; size + 2];
            stream.read_exact(&mut chunk)?;
            if size == 0 {
                return Ok(Reply { status, headers, body: String::from_utf8(body).map_err(|_| bad("body is not UTF-8"))? });
            }
            body.extend_from_slice(&chunk[..size]);
        }
    }
    let length: usize = headers
        .iter()
        .find(|(n, _)| n.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.parse().ok())
        .ok_or_else(|| bad("Content-Length"))?;
    let mut body = vec![0u8; length];
    stream.read_exact(&mut body)?;
    Ok(Reply { status, headers, body: String::from_utf8(body).map_err(|_| bad("body is not UTF-8"))? })
}

fn read_reply(stream: &mut TcpStream) -> Reply {
    try_read_reply(stream).unwrap()
}

fn send(addr: SocketAddr, raw: &[u8]) -> Reply {
    let mut stream = connect(addr);
    stream.write_all(raw).unwrap();
    read_reply(&mut stream)
}

/// A POST of `body` to the endpoint; `extra` is more header lines, each ending in CRLF.
fn post_raw(addr: SocketAddr, extra: &str, body: &[u8]) -> Reply {
    let mut raw = format!(
        "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )
    .into_bytes();
    raw.extend_from_slice(body);
    send(addr, &raw)
}

fn post(addr: SocketAddr, session: Option<&str>, message: &Value) -> Reply {
    let extra = session.map(|id| format!("Mcp-Session-Id: {id}\r\n")).unwrap_or_default();
    post_raw(addr, &extra, message.to_string().as_bytes())
}

fn initialize(addr: SocketAddr) -> String {
    let r = post(addr, None, &json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18"}}));
    assert_eq!(r.status, 200, "{}", r.body);
    r.header("Mcp-Session-Id").unwrap().to_string()
}

fn ping(id: u64) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "method": "ping"})
}

fn new_glyph(id: u64, name: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "method": "tools/call", "params": {"name": "run_command", "arguments": {"command": "glyph.newGlyph", "params": {"name": name}}}})
}

#[test]
fn an_agent_initializes_and_calls_a_tool_over_http() {
    let addr = start(Limits::default());
    let r = post(addr, None, &json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": "2025-06-18"}}));
    assert_eq!(r.status, 200, "{}", r.body);
    assert_eq!(r.header("Content-Type"), Some("application/json"));
    assert_eq!(r.json()["result"]["protocolVersion"], "2025-06-18");
    let session = r.header("Mcp-Session-Id").unwrap().to_string();
    assert_eq!(session.len(), 32);
    assert!(session.bytes().all(|b| b.is_ascii_hexdigit()));

    let r = post(addr, Some(&session), &json!({"jsonrpc": "2.0", "method": "notifications/initialized"}));
    assert_eq!((r.status, r.body.as_str()), (202, ""));

    let r =
        post_raw(addr, &format!("Mcp-Session-Id: {session}\r\nMCP-Protocol-Version: 2025-06-18\r\n"), new_glyph(2, "agent").to_string().as_bytes());
    assert_eq!(r.status, 200, "{}", r.body);
    let reply = r.json();
    let result = &reply["result"];
    assert_eq!(result["isError"], false, "{result}");
    assert!(result["content"][0]["text"].as_str().unwrap().contains("agent"));

    // Log notifications come in an event stream, before the reply.
    let r = post(addr, Some(&session), &json!({"jsonrpc": "2.0", "id": 3, "method": "logging/setLevel", "params": {"level": "info"}}));
    assert_eq!(r.status, 200, "{}", r.body);
    let r = post(addr, Some(&session), &new_glyph(4, "agent2"));
    assert_eq!((r.status, r.header("Content-Type")), (200, Some("text/event-stream")), "{}", r.body);
    let note = r.body.find("notifications/message").unwrap();
    let reply = r.body.find("\"id\":4").unwrap();
    assert!(r.body.starts_with("event: message\ndata: ") && note < reply, "{}", r.body);

    // A second session has its own log level, and works on the same document.
    let other = initialize(addr);
    assert_ne!(other, session);
    let r = post(addr, Some(&other), &new_glyph(5, "agent"));
    assert_eq!(r.header("Content-Type"), Some("application/json"));
    assert_eq!(r.json()["result"]["isError"], true, "{}", r.body);

    // Two requests on one kept-alive connection.
    let mut stream = connect(addr);
    for id in [6, 7] {
        let body = ping(id).to_string();
        let raw = format!(
            "POST /mcp HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nMcp-Session-Id: {session}\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        );
        stream.write_all(raw.as_bytes()).unwrap();
        let r = read_reply(&mut stream);
        assert_eq!((r.status, r.json()["id"].as_u64()), (200, Some(id)));
    }

    // Sessions are required, must exist, and speak a known revision.
    assert_eq!(post(addr, None, &ping(8)).status, 400);
    assert_eq!(post(addr, Some("0123"), &ping(8)).status, 404);
    let r = post_raw(addr, &format!("Mcp-Session-Id: {session}\r\nMCP-Protocol-Version: 1999-01-01\r\n"), ping(8).to_string().as_bytes());
    assert_eq!(r.status, 400);

    let r = send(addr, b"GET /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nAccept: text/event-stream\r\n\r\n");
    assert_eq!((r.status, r.header("Allow")), (405, Some("POST, DELETE")));

    let r = send(addr, format!("DELETE /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nMcp-Session-Id: {session}\r\nConnection: close\r\n\r\n").as_bytes());
    assert_eq!(r.status, 200);
    assert_eq!(post(addr, Some(&session), &ping(9)).status, 404, "an ended session is gone");
    assert_eq!(post(addr, Some(&other), &ping(10)).status, 200);
}

#[test]
fn malformed_and_oversized_requests_get_errors_and_the_server_keeps_serving() {
    let addr = start(Limits::default());
    let ok_headers = "Host: 127.0.0.1\r\nContent-Type: application/json\r\n";
    let cases: Vec<(String, u16)> = vec![
        ("GARBAGE\r\n\r\n".into(), 400),
        ("POST /mcp HTTP/2.0\r\n\r\n".into(), 505),
        ("POST mcp HTTP/1.1\r\n\r\n".into(), 400),
        ("POST /mcp HTTP/1.1\r\nHost 127.0.0.1\r\n\r\n".into(), 400),
        ("POST /mcp HTTP/1.1\r\nHost : 127.0.0.1\r\n\r\n".into(), 400),
        ("POST /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\n folded\r\n\r\n".into(), 400),
        ("POST /mcp HTTP/1.1\r\nContent-Length: 2\r\n\r\n{}".into(), 400),
        (format!("POST /mcp HTTP/1.1\r\nX-Pad: {}\r\n\r\n", "a".repeat(MAX_HEADER_BYTES)), 431),
        (format!("POST /mcp HTTP/1.1\r\n{}\r\n", "X-A: 1\r\n".repeat(MAX_HEADERS + 1)), 431),
        (format!("POST /mcp HTTP/1.1\r\n{ok_headers}\r\n"), 411),
        (format!("POST /mcp HTTP/1.1\r\n{ok_headers}Transfer-Encoding: chunked\r\n\r\n"), 501),
        (format!("POST /mcp HTTP/1.1\r\n{ok_headers}Content-Length: {}\r\n\r\n", MAX_BODY + 1), 413),
        (format!("POST /mcp HTTP/1.1\r\n{ok_headers}Content-Length: 99999999999999999999999\r\n\r\n"), 400),
        (format!("POST /mcp HTTP/1.1\r\n{ok_headers}Content-Length: 2\r\nContent-Length: 3\r\n\r\n{{}}"), 400),
        (format!("POST /other HTTP/1.1\r\n{ok_headers}Content-Length: 2\r\n\r\n{{}}"), 404),
        ("POST /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: text/plain\r\nContent-Length: 2\r\n\r\n{}".into(), 415),
        (format!("POST /mcp HTTP/1.1\r\n{ok_headers}Accept: text/html\r\nContent-Length: 2\r\n\r\n{{}}"), 406),
        ("POST /mcp HTTP/1.1\r\nHost: evil.example\r\nContent-Type: application/json\r\nContent-Length: 2\r\n\r\n{}".into(), 403),
        (format!("POST /mcp HTTP/1.1\r\n{ok_headers}Origin: http://evil.example\r\nContent-Length: 2\r\n\r\n{{}}"), 403),
        (format!("PUT /mcp HTTP/1.1\r\n{ok_headers}Content-Length: 2\r\n\r\n{{}}"), 405),
    ];
    for (raw, status) in &cases {
        let r = send(addr, raw.as_bytes());
        assert_eq!(r.status, *status, "{raw:.200}: {}", r.body);
        assert_eq!(r.json()["error"]["code"], INVALID_REQUEST, "{raw:.200}");
    }

    // Bodies that are not one JSON-RPC message.
    let code = |body: &[u8]| {
        let r = post_raw(addr, "", body);
        assert_eq!(r.status, 400, "{}", r.body);
        r.json()["error"]["code"].as_i64()
    };
    assert_eq!(code(b"{not json"), Some(PARSE_ERROR));
    assert_eq!(code(b"\xff\xfe"), Some(PARSE_ERROR));
    let batch = json!([{"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}]).to_string();
    assert_eq!(code(batch.as_bytes()), Some(INVALID_REQUEST));

    // The server's own origin passes; then the server still serves.
    let body = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}).to_string();
    let raw = format!(
        "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nOrigin: http://127.0.0.1:{port}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len(),
        port = addr.port()
    );
    assert_eq!(send(addr, raw.as_bytes()).status, 200);
    let session = initialize(addr);
    assert_eq!(post(addr, Some(&session), &ping(2)).status, 200);
}

#[test]
fn a_slow_client_times_out_while_others_are_served() {
    let addr = start(Limits { idle_timeout: Duration::from_millis(300), ..Limits::default() });

    // Half a request head, then nothing.
    let mut slow_head = connect(addr);
    slow_head.write_all(b"POST /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\n").unwrap();
    // A body that stops short of its length.
    let mut slow_body = connect(addr);
    slow_body
        .write_all(b"POST /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: 100\r\n\r\n{\"jsonrpc\"")
        .unwrap();
    // A connection that never sends a byte.
    let mut idle = connect(addr);

    let session = initialize(addr);
    assert_eq!(read_reply(&mut slow_head).status, 408);
    assert_eq!(read_reply(&mut slow_body).status, 408);
    let mut rest = [0u8; 16];
    assert_eq!(idle.read(&mut rest).unwrap(), 0, "an idle connection is closed without a reply");
    assert_eq!(post(addr, Some(&session), &ping(2)).status, 200);
}

#[test]
fn connections_sessions_and_messages_are_capped() {
    // Connections: the third concurrent one is turned away, and served once the others leave.
    let addr = start(Limits { max_connections: 2, idle_timeout: Duration::from_secs(5), ..Limits::default() });
    let held: Vec<TcpStream> = (0..2).map(|_| connect(addr)).collect();
    let mut third = connect(addr);
    assert_eq!(read_reply(&mut third).status, 503);
    drop(held);
    let served = (0..100).any(|_| {
        let body = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}).to_string();
        let mut stream = connect(addr);
        let raw = format!(
            "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        // A refused connection may also be reset before its reply is read: try again.
        let ok = stream.write_all(raw.as_bytes()).is_ok() && try_read_reply(&mut stream).is_ok_and(|r| r.status == 200);
        if !ok {
            std::thread::sleep(Duration::from_millis(50));
        }
        ok
    });
    assert!(served, "the freed connection slots are used again");

    // Sessions: a new one ends the one unused for longest.
    let addr = start(Limits { max_sessions: 2, ..Limits::default() });
    let (first, second) = (initialize(addr), initialize(addr));
    assert_eq!(post(addr, Some(&first), &ping(2)).status, 200);
    let third = initialize(addr);
    assert_eq!(post(addr, Some(&second), &ping(3)).status, 404);
    assert_eq!(post(addr, Some(&first), &ping(4)).status, 200);
    assert_eq!(post(addr, Some(&third), &ping(5)).status, 200);

    // Messages per second.
    let addr = start(Limits { max_messages_per_second: 5, ..Limits::default() });
    let session = initialize(addr);
    let replies: Vec<Reply> = (0..20).map(|id| post(addr, Some(&session), &ping(id))).collect();
    let refused: Vec<&Reply> = replies.iter().filter(|r| r.status == 429).collect();
    assert!(!refused.is_empty(), "twenty messages at once exceed five per second");
    assert!(refused.iter().all(|r| r.header("Retry-After") == Some("1")));
    // Cancellations count too.
    let addr = start(Limits { max_messages_per_second: 5, ..Limits::default() });
    let cancel = json!({"jsonrpc": "2.0", "method": "notifications/cancelled", "params": {"requestId": "none"}}).to_string();
    let replies: Vec<Reply> = (0..20).map(|_| post_raw(addr, "", cancel.as_bytes())).collect();
    assert!(replies.iter().any(|r| r.status == 202) && replies.iter().any(|r| r.status == 429));
}

#[test]
fn only_the_server_s_own_hosts_pass_on_any_listener() {
    // Loopback listener: loopback names only.
    assert!(host_allowed("127.0.0.1:7986", true) && host_allowed("localhost", true) && host_allowed("[::1]:7986", true));
    assert!(!host_allowed("192.168.1.5:7986", true));
    // Listening on every address: IP addresses pass, a name that may have been rebound does not.
    assert!(host_allowed("192.168.1.5:7986", false) && host_allowed("[fe80::1]:7986", false) && host_allowed("localhost", false));
    assert!(!host_allowed("evil.example:7986", false) && !host_allowed("", false));
}

#[test]
fn a_connection_is_closed_after_its_time() {
    let addr = start(Limits { connection_time: Duration::ZERO, ..Limits::default() });
    let body = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}).to_string();
    let mut stream = connect(addr);
    let raw = format!("POST /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{body}", body.len());
    stream.write_all(raw.as_bytes()).unwrap();
    assert_eq!(read_reply(&mut stream).status, 200);
    let mut rest = [0u8; 16];
    assert_eq!(stream.read(&mut rest).unwrap(), 0, "the connection is closed after the reply");
}

/// A cancellation on another connection stops a running request, in its own session only.
#[test]
fn a_running_request_is_cancelled_from_another_connection() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || serve(Server::new(Box::new(crate::stdio::tests::Slow)), listener, Limits::default()));
    let start = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}).to_string();
    let session = || post_raw(addr, "", start.as_bytes()).header("mcp-session-id").unwrap().to_string();
    let (session, other) = (session(), session());
    let header = format!("Mcp-Session-Id: {session}\r\n");
    let call = json!({"jsonrpc": "2.0", "id": "long", "method": "tools/call", "params": {"name": "run_command", "arguments": {"command": "slow"}}})
        .to_string();
    let running = {
        let header = header.clone();
        std::thread::spawn(move || post_raw(addr, &header, call.as_bytes()))
    };
    let cancel = json!({"jsonrpc": "2.0", "method": "notifications/cancelled", "params": {"requestId": "long"}}).to_string();
    std::thread::sleep(Duration::from_millis(300));
    assert_eq!(post_raw(addr, &format!("Mcp-Session-Id: {other}\r\n"), cancel.as_bytes()).status, 202);
    assert!(!running.is_finished(), "another session cannot cancel it");
    assert_eq!(post_raw(addr, &header, cancel.as_bytes()).status, 202);
    let reply = running.join().unwrap();
    assert_eq!((reply.status, reply.json()["error"]["code"].as_i64()), (200, Some(CANCELLED)));
}

/// Progress is streamed while the request runs, and a request without a progress token gets a plain reply.
#[test]
fn progress_is_streamed_before_the_reply() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    std::thread::spawn(move || serve(Server::new(Box::new(crate::stdio::tests::Slow)), listener, Limits::default()));
    let start = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}).to_string();
    let session = post_raw(addr, "", start.as_bytes()).header("mcp-session-id").unwrap().to_string();
    let header = format!("Mcp-Session-Id: {session}\r\n");
    let call = json!({"jsonrpc": "2.0", "id": "long", "method": "tools/call", "params": {
        "name": "run_command", "arguments": {"command": "slow"}, "_meta": {"progressToken": "p"},
    }})
    .to_string();
    let raw = format!(
        "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\n{header}Content-Length: {}\r\n\r\n{call}",
        call.len()
    );
    let mut stream = connect(addr);
    stream.write_all(raw.as_bytes()).unwrap();
    // The first bytes of the reply arrive while the command runs.
    assert_eq!(stream.peek(&mut [0u8; 1]).unwrap(), 1);
    let cancel = json!({"jsonrpc": "2.0", "method": "notifications/cancelled", "params": {"requestId": "long"}}).to_string();
    assert_eq!(post_raw(addr, &header, cancel.as_bytes()).status, 202);
    let reply = read_reply(&mut stream);
    assert_eq!((reply.status, reply.header("Content-Type")), (200, Some("text/event-stream")));
    let messages: Vec<Value> = reply.body.lines().filter_map(|l| l.strip_prefix("data: ")).map(|l| serde_json::from_str(l).unwrap()).collect();
    let (last, progress) = messages.split_last().unwrap();
    assert!(!progress.is_empty() && progress.iter().all(|m| m["method"] == "notifications/progress" && m["params"]["progressToken"] == "p"));
    assert_eq!((&last["id"], last["error"]["code"].as_i64()), (&json!("long"), Some(CANCELLED)));

    // The connection goes on after the stream.
    let ping = ping(9).to_string();
    let raw =
        format!("POST /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/json\r\n{header}Content-Length: {}\r\n\r\n{ping}", ping.len());
    stream.write_all(raw.as_bytes()).unwrap();
    assert_eq!(read_reply(&mut stream).json()["id"], 9);
}

/// A session hears of a subscribed resource another session changed with its next reply.
#[test]
fn a_session_hears_of_a_change_by_another_with_its_next_reply() {
    let addr = start(Limits::default());
    let initialize = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}).to_string();
    let open = || post_raw(addr, "", initialize.as_bytes()).header("mcp-session-id").unwrap().to_string();
    let (one, two) = (open(), open());
    let post = |session: &str, message: Value| post_raw(addr, &format!("Mcp-Session-Id: {session}\r\n"), message.to_string().as_bytes());
    post(&one, json!({"jsonrpc": "2.0", "id": 2, "method": "resources/subscribe", "params": {"uri": "fontcraft://glyph/A"}}));
    let draw = json!({"command": "path.addRectangle", "params": {"glyph": "A", "x": 0, "y": 0, "width": 10, "height": 10}});
    let changed = post(&two, json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "run_command", "arguments": draw}}));
    assert_eq!(changed.json()["result"]["isError"], false, "the other session has no subscription: {}", changed.body);
    let ping = json!({"jsonrpc": "2.0", "id": 3, "method": "ping"});
    let told = post(&one, ping.clone());
    assert!(told.body.contains("notifications/resources/updated") && told.body.contains("fontcraft://glyph/A"), "{}", told.body);
    assert_eq!(post(&one, ping).json()["result"], json!({}), "once");
}
