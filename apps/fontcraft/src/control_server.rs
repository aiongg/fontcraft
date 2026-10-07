//! The control channel's transport: JSON lines on a loopback TCP port.
//!
//! Connections are read on their own threads; each request is handed to the UI thread, which
//! answers it between frames ([`Inbox::drain`]) through `fontcraft_ui_egui::control::handle`.
//!
//! Only requests are read. A line must be a JSON object with a string `method`; anything else
//! gets one error reply and the connection is closed, so the body of an HTTP request sent to this
//! port by a web page can never run as a command.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Ipv4Addr, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use fontcraft_ui_egui::{FontcraftApp, control};
use serde_json::{Value, json};

/// Longest request line accepted.
const MAX_LINE: u64 = 4 << 20;
/// Most connections served at once.
const MAX_CONNECTIONS: usize = 16;
/// A connection that sends nothing for this long is closed, so idle clients cannot hold every slot.
const IDLE_TIMEOUT: Duration = Duration::from_secs(300);
/// How long a connection waits for the UI thread to answer before telling its client.
const REPLY_TIMEOUT: Duration = Duration::from_secs(120);
/// Most requests answered in one frame, and the most time spent on them, so a busy client cannot
/// keep the window from painting.
const FRAME_REQUESTS: usize = 64;
const FRAME_BUDGET: Duration = Duration::from_millis(8);

type Request = (Value, Sender<Value>);

/// The UI thread's end: requests waiting to be answered.
pub struct Inbox {
    requests: Receiver<Request>,
    ctx: eframe::egui::Context,
}

impl Inbox {
    /// Answers waiting requests, up to this frame's budget; the rest wait for the next frame,
    /// which is requested at once.
    pub fn drain(&self, app: &mut FontcraftApp) {
        let start = Instant::now();
        for _ in 0..FRAME_REQUESTS {
            let Ok((request, reply)) = self.requests.try_recv() else { return };
            // The client may have gone away; that is its business.
            let _ = reply.send(control::handle(app, &request));
            if start.elapsed() >= FRAME_BUDGET {
                break;
            }
        }
        self.ctx.request_repaint();
    }
}

/// Starts listening on `127.0.0.1:port`. `ctx` is woken when a request arrives.
pub fn start(port: u16, ctx: eframe::egui::Context) -> std::io::Result<Inbox> {
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, port))?;
    let (to_ui, inbox) = channel::<Request>();
    let open = Arc::new(AtomicUsize::new(0));
    let wake = ctx.clone();
    std::thread::Builder::new().name("control-accept".into()).spawn(move || {
        for stream in listener.incoming() {
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
            let (to_ui, ctx, slot) = (to_ui.clone(), wake.clone(), open.clone());
            let spawned = std::thread::Builder::new().name("control-connection".into()).spawn(move || {
                let _ = serve(stream, &to_ui, &ctx);
                slot.fetch_sub(1, Ordering::SeqCst);
            });
            if spawned.is_err() {
                // The closure never ran, so give the slot back here.
                open.fetch_sub(1, Ordering::SeqCst);
                log::error!("control channel: cannot start a thread for a connection");
            }
        }
    })?;
    Ok(Inbox { requests: inbox, ctx })
}

fn refuse(mut stream: TcpStream, why: &str) -> std::io::Result<()> {
    writeln!(stream, "{}", json!({"ok": false, "error": why}))
}

fn serve(stream: TcpStream, to_ui: &Sender<Request>, ctx: &eframe::egui::Context) -> std::io::Result<()> {
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
        let (reply_to, reply) = channel();
        if to_ui.send((request, reply_to)).is_err() {
            return refuse(writer, "the app is shutting down; closing the connection");
        }
        ctx.request_repaint();
        match reply.recv_timeout(REPLY_TIMEOUT) {
            Ok(value) => writeln!(writer, "{value}")?,
            // The reply to this request may still come; the connection is closed so it cannot be
            // taken for the answer to a later one.
            Err(_) => return refuse(writer, "the app did not answer in time (it may be busy or shutting down); closing the connection"),
        }
    }
}
