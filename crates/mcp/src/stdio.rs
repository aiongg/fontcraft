//! The stdio transport: newline-delimited JSON-RPC ([`Server::serve`]).
//!
//! A reader thread takes the lines in and the calling thread, which owns the [`Server`], answers them one at a time
//! and writes everything. So while a request runs, a cancellation of it is seen, its progress goes out, and a running
//! app is asked now and then whether a subscribed resource changed. What waits for an answer is capped in number and
//! in bytes: beyond that the reader stops reading until there is room.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::io::{self, BufRead, Read as _, Write};
use std::rc::Rc;
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use fontcraft_engine::work::Token;
use serde_json::Value;

use crate::{INVALID_REQUEST, MAX_LINE, PARSE_ERROR, Server, error, request_key};

/// Most messages that wait for an answer.
pub const MAX_QUEUED: usize = 256;
/// Most bytes of messages that wait for an answer, unless it is one message.
pub const MAX_QUEUED_BYTES: usize = 4 * MAX_LINE;
/// How often a running app is asked whether a subscribed resource changed.
const POLL: Duration = Duration::from_secs(1);

/// What the reader took in.
enum Work {
    Message(Value),
    /// The answer to a line that is no message.
    Reply(Value),
    End(io::Result<()>),
}

struct Item {
    work: Work,
    /// The request's id, when it can be cancelled.
    key: Option<String>,
    token: Arc<Token>,
    bytes: usize,
}

#[derive(Default)]
struct State {
    items: VecDeque<Item>,
    bytes: usize,
    /// The request being answered.
    running: Option<(String, Arc<Token>)>,
    closed: bool,
}

/// The messages that wait for an answer.
#[derive(Default)]
struct Queue {
    state: Mutex<State>,
    changed: Condvar,
}

impl Queue {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Adds an item once there is room; false when nobody answers any more.
    fn push(&self, item: Item) -> bool {
        let mut state = self.lock();
        while !state.closed
            && !state.items.is_empty()
            && (state.items.len() >= MAX_QUEUED || state.bytes.saturating_add(item.bytes) > MAX_QUEUED_BYTES)
        {
            state = self.changed.wait(state).unwrap_or_else(PoisonError::into_inner);
        }
        if state.closed {
            return false;
        }
        state.bytes = state.bytes.saturating_add(item.bytes);
        state.items.push_back(item);
        self.changed.notify_all();
        true
    }

    /// The next item, which is then the one being answered; `None` when none came within `wait`.
    fn pop(&self, wait: Option<Duration>) -> Option<Item> {
        let deadline = wait.map(|wait| Instant::now() + wait);
        let mut state = self.lock();
        loop {
            if let Some(item) = state.items.pop_front() {
                state.bytes = state.bytes.saturating_sub(item.bytes);
                state.running = item.key.clone().map(|key| (key, Arc::clone(&item.token)));
                self.changed.notify_all();
                return Some(item);
            }
            state = match deadline {
                None => self.changed.wait(state).unwrap_or_else(PoisonError::into_inner),
                Some(deadline) => {
                    let left = deadline.saturating_duration_since(Instant::now());
                    if left.is_zero() {
                        return None;
                    }
                    self.changed.wait_timeout(state, left).unwrap_or_else(PoisonError::into_inner).0
                }
            };
        }
    }

    fn answered(&self) {
        self.lock().running = None;
    }

    /// Cancels the request `key`, running or waiting. An unknown one is ignored, as the protocol asks.
    fn cancel(&self, key: &str) {
        let state = self.lock();
        let running = state.running.iter().filter(|(k, _)| k == key).map(|(_, token)| token);
        let waiting = state.items.iter().filter(|i| i.key.as_deref() == Some(key)).map(|i| &i.token);
        for token in running.chain(waiting) {
            token.cancel();
        }
    }
}

/// Closes the queue when the server stops answering, so the reader does not wait for room.
struct Closing(Arc<Queue>);

impl Drop for Closing {
    fn drop(&mut self) {
        self.0.lock().closed = true;
        self.0.changed.notify_all();
    }
}

/// The request id a `notifications/cancelled` names.
pub(crate) fn cancelled_request(message: &Value) -> Option<String> {
    let cancels = message.get("id").is_none() && message.get("method").and_then(Value::as_str) == Some("notifications/cancelled");
    let id = message.get("params")?.get("requestId").filter(|_| cancels)?;
    (id.is_string() || id.is_number()).then(|| id.to_string())
}

/// Reads lines until the input ends or nobody answers.
fn read(mut input: impl BufRead, queue: &Queue) {
    loop {
        // Read at most one byte past the limit, so a peer that never ends its line cannot
        // fill memory; the rest of an oversized line is skipped.
        let mut line = Vec::new();
        let read = (&mut input).take(MAX_LINE as u64 + 1).read_until(b'\n', &mut line);
        let work = match read {
            Err(e) => Work::End(Err(e)),
            Ok(0) => Work::End(Ok(())),
            Ok(_) if line.len() > MAX_LINE && !line.ends_with(b"\n") => match skip_line(&mut input) {
                Err(e) => Work::End(Err(e)),
                Ok(()) => Work::Reply(error(&Value::Null, INVALID_REQUEST, format!("a request line may be at most {MAX_LINE} bytes"))),
            },
            Ok(_) => match std::str::from_utf8(&line) {
                // A line that is not UTF-8 is a parse error, not the end of the session.
                Err(_) => Work::Reply(error(&Value::Null, PARSE_ERROR, "the request is not valid UTF-8")),
                Ok(text) if text.trim().is_empty() => continue,
                Ok(text) if text.len() > MAX_LINE => {
                    Work::Reply(error(&Value::Null, INVALID_REQUEST, format!("a request line may be at most {MAX_LINE} bytes")))
                }
                Ok(text) => match serde_json::from_str::<Value>(text) {
                    Err(e) => Work::Reply(error(&Value::Null, PARSE_ERROR, format!("not valid JSON: {e}"))),
                    Ok(message) => {
                        if let Some(key) = cancelled_request(&message) {
                            queue.cancel(&key);
                            continue;
                        }
                        Work::Message(message)
                    }
                },
            },
        };
        let end = matches!(work, Work::End(_));
        let key = match &work {
            Work::Message(message) => request_key(message),
            _ => None,
        };
        if !queue.push(Item { work, key, token: Token::new(), bytes: line.len() }) || end {
            return;
        }
    }
}

/// Discards input up to and including the next newline (or the end).
fn skip_line(input: &mut impl BufRead) -> io::Result<()> {
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

impl Server {
    /// Serves until `input` ends. Only protocol messages go to `output`. When `output` fails, the thread that reads
    /// `input` ends at its next line.
    pub fn serve(&mut self, input: impl BufRead + Send + 'static, output: impl Write + 'static) -> io::Result<()> {
        let queue = Arc::new(Queue::default());
        let _closing = Closing(Arc::clone(&queue));
        let reading = Arc::clone(&queue);
        std::thread::Builder::new().name("mcp-stdio-read".into()).spawn(move || read(input, &reading))?;
        let output = Rc::new(RefCell::new(output));
        let live = Rc::clone(&output);
        self.notify = Some(Rc::new(move |notification: &Value| {
            // A failed write shows at the next reply.
            if let Ok(mut output) = live.try_borrow_mut() {
                let _ = writeln!(output, "{notification}").and_then(|()| output.flush());
            }
        }));
        let served = self.answer_all(&queue, &output);
        self.notify = None;
        served
    }

    fn answer_all(&mut self, queue: &Queue, output: &RefCell<impl Write>) -> io::Result<()> {
        loop {
            // Only a running app changes the font between requests.
            let poll = self.backend.has_app() && !self.client.subscriptions.is_empty();
            let reply = match queue.pop(poll.then_some(POLL)) {
                None => {
                    self.poll_updates();
                    None
                }
                Some(Item { work: Work::End(end), .. }) => return end,
                Some(Item { work: Work::Reply(reply), .. }) => Some(reply),
                Some(Item { token, .. }) if token.is_cancelled() => None,
                Some(Item { work: Work::Message(message), token, .. }) => {
                    self.token = Some(token);
                    self.handle(&message)
                }
            };
            queue.answered();
            let notifications = self.take_notifications();
            if notifications.is_empty() && reply.is_none() {
                continue;
            }
            let mut output = output.try_borrow_mut().map_err(|_| io::Error::other("the output is in use"))?;
            for notification in notifications {
                writeln!(output, "{notification}")?;
            }
            if let Some(reply) = reply {
                writeln!(output, "{reply}")?;
            }
            output.flush()?;
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::io::BufReader;
    use std::sync::mpsc;

    use fontcraft_engine::work;
    use serde_json::json;

    use super::*;
    use crate::{Backend, Headless};

    /// Lines as the client sends them, until it hangs up.
    struct Lines(mpsc::Receiver<String>, Vec<u8>);

    impl io::Read for Lines {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            if self.1.is_empty()
                && let Ok(line) = self.0.recv_timeout(Duration::from_secs(20))
            {
                self.1 = line.into_bytes();
            }
            let n = self.1.len().min(buffer.len());
            buffer[..n].copy_from_slice(&self.1[..n]);
            self.1.drain(..n);
            Ok(n)
        }
    }

    /// What the server wrote; `react` sees each line as it is written.
    struct Client {
        line: Vec<u8>,
        lines: Rc<RefCell<Vec<Value>>>,
        react: Box<dyn FnMut(&Value)>,
    }

    impl Write for Client {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            for byte in bytes {
                if *byte == b'\n' {
                    let value: Value = serde_json::from_slice(&std::mem::take(&mut self.line)).unwrap();
                    (self.react)(&value);
                    self.lines.borrow_mut().push(value);
                } else {
                    self.line.push(*byte);
                }
            }
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    /// Serves `first` and whatever `react` sends in return, and gives what the server wrote.
    fn converse(backend: Box<dyn Backend>, first: &[Value], mut react: impl FnMut(&Value, &mpsc::Sender<String>) + 'static) -> Vec<Value> {
        let (send, receive) = mpsc::channel();
        for message in first {
            send.send(format!("{message}\n")).unwrap();
        }
        let lines: Rc<RefCell<Vec<Value>>> = Rc::default();
        let mut send = Some(send);
        let react = Box::new(move |line: &Value| {
            if let Some(sender) = &send {
                react(line, sender);
            }
            // The client hangs up after the reply to request 99.
            if line["id"] == 99 {
                send = None;
            }
        });
        let client = Client { line: Vec::new(), lines: Rc::clone(&lines), react };
        Server::new(backend).serve(BufReader::new(Lines(receive, Vec::new())), client).unwrap();
        lines.take()
    }

    /// A backend whose command works until it is cancelled.
    pub(crate) struct Slow;

    impl Backend for Slow {
        fn execute(&mut self, _: &str, _: &Value) -> Result<Value, String> {
            work::plan(1_000);
            for _ in 0..1_000 {
                work::advance(1, "working");
                if work::cancelled() {
                    return Err("cancelled".into());
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            Ok(json!("done"))
        }
        fn commands(&mut self, _: Option<&str>) -> Result<Value, String> {
            Ok(json!({"commands": []}))
        }
        fn inspect(&mut self) -> Result<Value, String> {
            Ok(json!({}))
        }
        fn describe(&self) -> String {
            "slow".into()
        }
    }

    fn call(id: u64, command: &str, params: Value, meta: Value) -> Value {
        json!({"jsonrpc": "2.0", "id": id, "method": "tools/call",
            "params": {"name": "run_command", "arguments": {"command": command, "params": params}, "_meta": meta}})
    }

    /// Progress arrives while a call runs; the client cancels it, and it and a waiting call it also cancels get no
    /// reply, while the one after them does.
    #[test]
    fn progress_arrives_and_a_cancelled_request_gets_no_reply() {
        let first = [
            call(1, "slow", json!({}), json!({"progressToken": "p"})),
            call(2, "slow", json!({}), json!({})),
            json!({"jsonrpc": "2.0", "id": 99, "method": "ping"}),
        ];
        let mut cancelled = false;
        let lines = converse(Box::new(Slow), &first, move |line, send| {
            if line["method"] == "notifications/progress" && !std::mem::replace(&mut cancelled, true) {
                for id in [2, 1, 77] {
                    let cancel = json!({"jsonrpc": "2.0", "method": "notifications/cancelled", "params": {"requestId": id}});
                    send.send(format!("{cancel}\n")).unwrap();
                }
            }
        });
        let progress: Vec<&Value> = lines.iter().filter(|l| l["method"] == "notifications/progress").collect();
        assert!(!progress.is_empty());
        assert_eq!(progress[0]["params"]["progressToken"], "p");
        assert_eq!(progress[0]["params"]["total"], 1_000);
        assert!(progress.windows(2).all(|w| w[0]["params"]["progress"].as_u64() < w[1]["params"]["progress"].as_u64()));
        let replies: Vec<&Value> = lines.iter().filter(|l| l.get("id").is_some()).collect();
        assert_eq!(replies.len(), 1, "{lines:?}");
        assert_eq!(replies[0]["id"], 99);
    }

    /// A subscribed resource is announced once per change, and not after its subscription ends.
    #[test]
    fn a_change_is_announced_once_to_its_subscriber() {
        let ask = |id: u64, method: &str, uri: &str| json!({"jsonrpc": "2.0", "id": id, "method": method, "params": {"uri": uri}});
        let first = [
            json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}),
            ask(2, "resources/subscribe", "fontcraft://document"),
            ask(3, "resources/subscribe", "fontcraft://glyph/A"),
            ask(4, "resources/subscribe", "fontcraft://glyph/no-such-glyph"),
            ask(5, "resources/subscribe", "fontcraft://command/edit.undo"),
            call(6, "glyph.newGlyph", json!({"name": "agent"}), json!({})),
            call(7, "path.addRectangle", json!({"glyph": "A", "x": 0, "y": 0, "width": 10, "height": 10}), json!({})),
            json!({"jsonrpc": "2.0", "id": 8, "method": "ping"}),
            ask(9, "resources/unsubscribe", "fontcraft://glyph/A"),
            call(10, "edit.undo", json!({}), json!({})),
            json!({"jsonrpc": "2.0", "id": 99, "method": "ping"}),
        ];
        let lines = converse(Box::new(Headless::with_document()), &first, |_, _| {});
        assert_eq!(lines[0]["result"]["capabilities"]["resources"]["subscribe"], true);
        let at = |id: u64| lines.iter().position(|l| l["id"] == id).unwrap();
        assert!(lines[at(4)].get("error").is_some() && lines[at(5)].get("error").is_some(), "{lines:?}");
        let updates = |from: usize, to: usize| -> Vec<&str> {
            let update = |l: &'_ &Value| l["method"] == "notifications/resources/updated";
            lines[from..to].iter().filter(update).map(|l| l["params"]["uri"].as_str().unwrap()).collect()
        };
        assert_eq!(updates(0, at(6)), ["fontcraft://document"], "before the reply of the new glyph");
        assert_eq!(updates(at(6), at(7)), ["fontcraft://document", "fontcraft://glyph/A"]);
        assert_eq!(updates(at(7), at(9)), Vec::<&str>::new());
        assert_eq!(updates(at(9), lines.len()), ["fontcraft://document"]);
    }
}
