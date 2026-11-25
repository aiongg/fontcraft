//! The control channel's transport and its UI-thread side.
//!
//! A request can carry a `cancelKey`, by which `control.cancel` on another connection cancels it, and `progress: true`,
//! for which progress lines go out before its reply. `control.cancel` is answered by the connection's own thread, so
//! it works while the UI thread runs the request.
//!
//! Transport: JSON lines on a loopback TCP port. Connections are read on their own threads; each request becomes a
//! `craft::automation` [`ControlRequest`] with the reply deadline, and the UI thread is woken. Only requests are
//! read: a line must be a JSON object with a string `method`, or the connection gets one error reply and is closed,
//! so the body of an HTTP request a web page sends to the port never runs. There is no authentication
//! (`docs/control-protocol.md`): the listener is on only when the app is started with `--control`.
//!
//! UI thread: [`ControlChannel`] hands over the requests between frames ([`logic`]); window input replies wait until
//! the passes have carried the input, screenshots go through the channel's capture queue, and `ui.wait` replies are
//! parked until their frames have passed or the window is idle ([`after_ui`]). A request that reads the windows'
//! elements is parked until its frames have been drawn, and runs before the next one ([`logic`]).

use std::cell::Cell;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use craft::automation::control::{ControlChannel, ControlRequest, REPLY_DEADLINE, Reply, expired_error};
use craft::automation::screenshot::settle_ms;
use craft::ui::runtime::motion;
use craft::ui::runtime::windows::WindowHost;
use egui::ViewportId;
use fontcraft_engine::work::{self, Token};
use serde_json::{Value, json};

use super::wait::{self, Wait};
use super::{Answer, Input, Work, answer, elements, reply, screenshot};
use crate::app::FontCraftApp;

/// Longest request line accepted.
const MAX_LINE: u64 = 4 << 20;
/// Most connections served at once.
const MAX_CONNECTIONS: usize = 16;
/// A connection that sends nothing for this long is closed, so idle clients cannot hold every slot.
const IDLE_TIMEOUT: Duration = Duration::from_secs(300);
/// Most requests answered before one frame, so a busy client cannot keep the window from painting.
const FRAME_REQUESTS: usize = 64;
/// Longest `cancelKey`, in bytes.
const MAX_CANCEL_KEY: usize = 128;
/// Most progress lines sent for one request, and the least time between two of them.
const MAX_PROGRESS: u32 = 1_000;
const PROGRESS_INTERVAL: Duration = Duration::from_millis(100);
/// Longest progress message, in characters.
const MAX_PROGRESS_MESSAGE: usize = 200;
/// How long a request that reads the windows' elements waits for frames, which a hidden window does not draw.
const ELEMENTS_TIMEOUT: Duration = Duration::from_secs(10);
/// The input cancelled error, for replies whose input the window host withdrew.
const INPUT_CANCELLED: &str = "input cancelled: the window dropped the queued input";

/// What cancels the requests of the connections, by `cancelKey`: at most one for each connection.
type Running = Arc<Mutex<HashMap<String, Arc<Token>>>>;

/// A request on its way to the UI thread.
struct Incoming {
    request: ControlRequest,
    token: Arc<Token>,
    progress: bool,
}

impl Incoming {
    /// What cancels the request's command and takes its progress.
    fn work(&self) -> Work {
        let report: Option<work::Report> = self.progress.then(|| {
            let to = self.request.reply.clone();
            let (last, sent) = (Cell::new(None::<Instant>), Cell::new(0u32));
            let report: work::Report = Rc::new(move |done: u64, total: u64, message: &str| {
                let now = Instant::now();
                let due = last.get().is_none_or(|last| now.saturating_duration_since(last) >= PROGRESS_INTERVAL);
                if !due || sent.get() >= MAX_PROGRESS {
                    return;
                }
                last.set(Some(now));
                sent.set(sent.get().saturating_add(1));
                let message: String = message.chars().take(MAX_PROGRESS_MESSAGE).collect();
                let _ = to.send(json!({"progress": {"done": done, "total": total, "message": message}}));
            });
            report
        });
        Work { token: Arc::clone(&self.token), report }
    }
}

/// The listener and the requests and replies waiting on the UI thread.
pub struct Server {
    /// The bound address.
    pub addr: SocketAddr,
    requests: Receiver<Incoming>,
    /// Settles replies and takes screenshots; the requests come through `requests`.
    channel: ControlChannel,
    /// Replies waiting for their window input to run: where each goes and what it says.
    after_input: Vec<(Sender<Value>, Value)>,
    /// `ui.wait` replies waiting for frames or for the window to go idle.
    waits: Vec<Waiting>,
    /// Requests that read the windows' elements, waiting for frames, oldest first: when each arrived and the frames
    /// drawn since.
    elements: Vec<(Incoming, Instant, u64)>,
    next_token: u64,
    stop: Arc<AtomicBool>,
}

/// A parked `ui.wait`: where the reply goes, what it waits for, when it arrived and the frames drawn since.
struct Waiting {
    reply: Sender<Value>,
    wait: Wait,
    since: Instant,
    frames: u64,
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        // The accept loop blocks in `accept`: a connection wakes it to see the flag.
        let _ = TcpStream::connect_timeout(&self.addr, Duration::from_millis(500));
    }
}

impl Server {
    /// Serves `listener`; `ctx` is woken when a request arrives.
    pub fn start(listener: TcpListener, ctx: egui::Context) -> std::io::Result<Server> {
        let addr = listener.local_addr()?;
        let (to_ui, requests) = channel::<Incoming>();
        let running = Running::default();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = Arc::clone(&stop);
        let open = Arc::new(AtomicUsize::new(0));
        std::thread::Builder::new().name("control-accept".into()).spawn(move || {
            for stream in listener.incoming() {
                if stopped.load(Ordering::Acquire) {
                    break;
                }
                let stream = match stream {
                    Ok(stream) => stream,
                    Err(e) => {
                        // Out of file descriptors and the like: wait instead of spinning.
                        log::error!("control channel: cannot accept a connection: {e}");
                        std::thread::sleep(Duration::from_millis(200));
                        continue;
                    }
                };
                if open.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS {
                    open.fetch_sub(1, Ordering::SeqCst);
                    let _ = refuse(stream, "too many connections; closing the connection");
                    continue;
                }
                let (to_ui, ctx, slot, running) = (to_ui.clone(), ctx.clone(), Arc::clone(&open), Arc::clone(&running));
                let spawned = std::thread::Builder::new().name("control-connection".into()).spawn(move || {
                    let _ = serve(stream, &to_ui, &ctx, &running);
                    slot.fetch_sub(1, Ordering::SeqCst);
                });
                if spawned.is_err() {
                    // The closure never ran, so give the slot back here.
                    open.fetch_sub(1, Ordering::SeqCst);
                    log::error!("control channel: cannot start a thread for a connection");
                }
            }
        })?;
        Ok(Server {
            addr,
            requests,
            channel: ControlChannel::new(channel().1),
            after_input: Vec::new(),
            waits: Vec::new(),
            elements: Vec::new(),
            next_token: 0,
            stop,
        })
    }

    /// Whether replies wait on frames.
    pub fn waiting(&self) -> bool {
        !self.after_input.is_empty() || self.channel.waiting() > 0 || !self.waits.is_empty() || !self.elements.is_empty()
    }

    /// Whether nothing but `ui.wait` replies waits on frames.
    fn others_idle(&self) -> bool {
        self.after_input.is_empty() && self.channel.waiting() == 0 && self.elements.is_empty()
    }
}

fn refuse(mut stream: TcpStream, why: &str) -> std::io::Result<()> {
    writeln!(stream, "{}", json!({"id": null, "ok": false, "error": why}))
}

/// Reads one connection's requests, one line each, and writes each reply with the request's `id`.
fn serve(stream: TcpStream, to_ui: &Sender<Incoming>, ctx: &egui::Context, running: &Running) -> std::io::Result<()> {
    stream.set_read_timeout(Some(IDLE_TIMEOUT))?;
    let mut writer = stream.try_clone()?;
    let mut reader = BufReader::new(stream);
    loop {
        let mut line = Vec::new();
        let read = (&mut reader).take(MAX_LINE + 1).read_until(b'\n', &mut line)?;
        if read == 0 {
            return Ok(());
        }
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let request = match std::str::from_utf8(&line).ok().and_then(|text| serde_json::from_str::<Value>(text).ok()) {
            Some(v) if line.len() as u64 <= MAX_LINE && v.get("method").is_some_and(Value::is_string) => v,
            _ => {
                let why = "a request is one line holding a JSON object with a string `method`, at most 4 MiB; closing the connection";
                return refuse(writer, why);
            }
        };
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        let method = request.get("method").and_then(Value::as_str).unwrap_or_default();
        let params = request.get("params").cloned().unwrap_or(Value::Null);
        if method == "control.cancel" {
            writeln!(writer, "{}", super::with_id(reply(cancel(running, &params)), &id))?;
            continue;
        }
        let token = Token::new();
        let _registered = match request.get("cancelKey") {
            None | Some(Value::Null) => None,
            Some(key) => match Registered::new(running, key, &token) {
                Ok(registered) => Some(registered),
                Err(e) => {
                    writeln!(writer, "{}", super::with_id(reply(Err(e)), &id))?;
                    continue;
                }
            },
        };
        let progress = request.get("progress").and_then(Value::as_bool).unwrap_or(false);
        let (request, replies) = ControlRequest::new(method, params);
        let request = request.with_deadline();
        let dispatch = request.dispatch.clone();
        if to_ui.send(Incoming { request, token, progress }).is_err() {
            return refuse(writer, "the app is shutting down; closing the connection");
        }
        ctx.request_repaint();
        let answer = loop {
            match replies.recv_timeout(REPLY_DEADLINE) {
                Ok(line) if line.get("ok").is_none() && line.get("progress").is_some() => writeln!(writer, "{}", super::with_id(line, &id))?,
                Ok(answer) => break answer,
                Err(_) => break json!({"ok": false, "error": dispatch.give_up()}),
            }
        };
        writeln!(writer, "{}", super::with_id(answer, &id))?;
    }
}

/// A request that `control.cancel` finds by its key, until it is dropped.
struct Registered(Running, String);

impl Registered {
    fn new(running: &Running, key: &Value, token: &Arc<Token>) -> Result<Registered, String> {
        let key = key.as_str().filter(|key| !key.is_empty() && key.len() <= MAX_CANCEL_KEY);
        let key = key.ok_or_else(|| format!("`cancelKey` is a string of at most {MAX_CANCEL_KEY} bytes"))?;
        let mut tokens = running.lock().unwrap_or_else(PoisonError::into_inner);
        if tokens.contains_key(key) {
            return Err("another running request has this `cancelKey`".into());
        }
        tokens.insert(key.to_string(), Arc::clone(token));
        Ok(Registered(Arc::clone(running), key.to_string()))
    }
}

impl Drop for Registered {
    fn drop(&mut self) {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).remove(&self.1);
    }
}

/// `{key}` → `{cancelled}`: cancels the request running or waiting under the key; false when there is none or it can no
/// longer be cancelled.
fn cancel(running: &Running, params: &Value) -> Result<Value, String> {
    let key = params.get("key").and_then(Value::as_str).ok_or("control.cancel: `key` (the `cancelKey` of a request) is required")?;
    let token = running.lock().unwrap_or_else(PoisonError::into_inner).get(key).cloned();
    Ok(json!({"cancelled": token.is_some_and(|token| token.cancel())}))
}

/// The next request that is still wanted; a request that waited past its deadline, or whose connection gave it up, is
/// answered and skipped.
fn next_request(server: &Server) -> Option<Incoming> {
    while let Ok(incoming) = server.requests.try_recv() {
        let request = &incoming.request;
        if request.deadline.is_some_and(|deadline| motion::now() >= deadline) || !request.dispatch.start() {
            let _ = request.reply.send(json!({"ok": false, "error": expired_error()}));
            continue;
        }
        return Some(incoming);
    }
    None
}

/// Answers the requests that arrived, and the captures that finished, before the frame.
pub fn logic(app: &mut FontCraftApp, ctx: &egui::Context) {
    let Some(mut server) = app.control.server.take() else { return };
    let host = WindowHost::of(ctx);
    server.channel.captures.note_windows(&host, None);
    let finished = server.channel.captures.drain(motion::now(), None);
    screenshot::finish(finished, pixels_per_point(&host));
    answer_elements(app, &mut server);
    let mut answered = 0;
    while answered < FRAME_REQUESTS
        && let Some(incoming) = next_request(&server)
    {
        answered += 1;
        let request = &incoming.request;
        if incoming.token.is_cancelled() {
            settle(&mut server, incoming.request, Err("cancelled".into()), 0);
            continue;
        }
        app.control.work = Some(incoming.work());
        let answered_now = answer(app, &request.method, &request.params);
        app.control.work = None;
        match answered_now {
            Answer::Reply(result) => {
                settle(&mut server, incoming.request, result, app.control.queued);
            }
            Answer::Elements => server.elements.push((incoming, motion::now(), 0)),
            Answer::Screenshot(path) => {
                server.next_token = server.next_token.wrapping_add(1);
                let settle = settle_ms(ctx.global_style().animation_time);
                server.channel.settle(incoming.request, Reply::screenshot(server.next_token, path, ViewportId::ROOT), None, settle);
            }
            Answer::Wait(wait) => {
                server.waits.push(Waiting { reply: request.reply.clone(), wait, since: motion::now(), frames: 0 });
            }
        }
    }
    if answered == FRAME_REQUESTS || server.waiting() {
        ctx.request_repaint();
    }
    app.control.server = Some(server);
}

/// Sends a request's reply: now, or once the window input it queued has run.
fn settle(server: &mut Server, request: ControlRequest, result: Result<Value, String>, queued: usize) {
    match result {
        Ok(value) if queued > 0 => server.after_input.push((request.reply.clone(), reply(Ok(value)))),
        result => server.channel.settle(request, Reply::from_result(result), None, 0.0),
    }
}

/// Runs the requests that read the windows' elements and whose frames have been drawn. One at a time: the input a
/// click or a change queues runs before the next one looks for its element.
fn answer_elements(app: &mut FontCraftApp, server: &mut Server) {
    while let Some((_, since, frames)) = server.elements.first() {
        if *frames < elements::FRAMES && motion::now().saturating_duration_since(*since) < ELEMENTS_TIMEOUT {
            return;
        }
        if !server.after_input.is_empty() {
            return;
        }
        let drawn = *frames >= elements::FRAMES;
        let (incoming, ..) = server.elements.remove(0);
        let request = &incoming.request;
        app.control.queued = 0;
        let result = if drawn {
            super::run(app, &request.method, &request.params)
        } else {
            Err(format!("{}: the window draws no frames (it is hidden or minimized)", request.method))
        };
        settle(server, incoming.request, result, app.control.queued);
    }
}

/// The end of a UI pass: screenshots due are issued, and replies waiting on window input go out once it has run.
pub fn after_ui(app: &mut FontCraftApp, ctx: &egui::Context) {
    let input_idle = app.control.input == Input::Idle;
    let Some(server) = app.control.server.as_mut() else { return };
    let host = WindowHost::of(ctx);
    server.channel.captures.note_pass();
    let finished = server.channel.captures.issue(&host, motion::now(), None);
    screenshot::finish(finished, pixels_per_point(&host));
    if !server.after_input.is_empty() {
        if host.take_injected_withdrawn() {
            for (to, _) in server.after_input.drain(..) {
                let _ = to.send(json!({"ok": false, "error": INPUT_CANCELLED}));
            }
        } else if host.injected_idle_at_pass_end() {
            for (to, value) in server.after_input.drain(..) {
                let _ = to.send(value);
            }
        }
    }
    for (_, _, frames) in &mut server.elements {
        *frames = frames.saturating_add(1);
    }
    if !server.waits.is_empty() {
        let idle = input_idle && host.injected_idle() && server.others_idle();
        let now = motion::now();
        server.waits.retain_mut(|waiting| {
            waiting.frames = waiting.frames.saturating_add(1);
            let elapsed = now.saturating_duration_since(waiting.since);
            let settled = waiting.wait.settled(waiting.frames, idle);
            if settled || elapsed >= waiting.wait.timeout {
                let _ = waiting.reply.send(reply(Ok(wait::result(waiting.frames, idle, !settled, elapsed))));
                return false;
            }
            true
        });
    }
    if server.waiting() {
        ctx.request_repaint();
    }
}

fn pixels_per_point(host: &WindowHost) -> f32 {
    host.screenshot_facts(ViewportId::ROOT).map_or(1.0, |facts| facts.pixels_per_point)
}
