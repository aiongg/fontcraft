//! External Preview (desktop only): a page any browser on the local network can open to watch the
//! active Edit View tab. The engine half, with the setting and the device lists, is
//! `fontcraft_engine::cmd::sharing`.
//!
//! Rows: `settings.sharing`, `sharing.*`, `settings.resetTrustedDevices`.
//!
//! - The server runs while `sharing.enableExternalPreview` is on (off by default), on a free port
//!   under a random path ([`crate::preview_server`]); `sharing.status` gives the address to type
//!   on the device.
//! - A device is its IP address. A device the person has not trusted gets a page that asks it to
//!   wait, and is listed in `sharing.status` until `sharing.answerDevice` answers it. A refused
//!   device is told so while the preview runs. Trusted devices are kept until
//!   `settings.resetTrustedDevices`.
//! - A trusted device gets the tab's glyphs drawn as SVG by `preview.render` in the tab's master,
//!   inside an HTML page that reloads itself every [`REFRESH_SECONDS`] with a meta refresh. The
//!   page has no script; the browser's own zoom and scrolling move around in it.
//!
//! The UI's frame hook ([`Hook`]) calls [`ExternalPreview::sync`] once per frame, with the glyph the window shows as
//! the tab. The server threads never touch the
//! session: they read the device lists and the last drawing the app hands them, and they collect
//! the devices that asked, which the next frame passes to the session.

use std::net::IpAddr;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use fontcraft_engine::Session;
use fontcraft_engine::cmd::sharing::{self, MAX_PENDING_DEVICES, SharingState};
use fontcraft_ui::app::preview_host::Scope;
use serde_json::{Value, json};

use crate::preview_server::{Limits, Pages, Reply, Server};

/// How often a trusted device's page reloads, in seconds.
pub const REFRESH_SECONDS: u32 = 1;
/// How often a waiting device's page asks again, in seconds.
pub const WAIT_REFRESH_SECONDS: u32 = 2;
/// The shortest time between two drawings of the tab; edits in between are drawn after it.
pub const MIN_DRAW_INTERVAL: Duration = Duration::from_millis(250);
/// The drawing's heights in pixels, tried in turn until the line fits `preview.render`'s width
/// limit. The page scales the drawing to the width of the screen.
const DRAW_HEIGHTS: [f64; 6] = [400.0, 160.0, 64.0, 32.0, 16.0, 8.0];
/// Most glyphs drawn from one tab (the limit of `preview.render`).
const MAX_DRAWN_GLYPHS: usize = fontcraft_engine::cmd::preview::MAX_RENDER_GLYPHS;

/// The Edit View tab the page shows, as the app reads it from its window.
#[derive(Clone, Debug, PartialEq)]
pub struct Tab {
    /// The id of the document it shows.
    pub document: u64,
    pub title: String,
    /// The glyphs it shows, in order.
    pub glyphs: Vec<String>,
    /// The master it draws in; `None`: the first.
    pub master: Option<String>,
}

impl Tab {
    /// Keeps the first glyphs a drawing can hold.
    pub fn new(document: u64, title: String, glyphs: impl Iterator<Item = String>, master: Option<String>) -> Tab {
        Tab { document, title, glyphs: glyphs.take(MAX_DRAWN_GLYPHS).collect(), master }
    }
}

/// What the page of a trusted device shows.
#[derive(Clone, Debug, Default, PartialEq)]
struct View {
    title: String,
    /// The `<svg>` element, or `None` with `note` saying why there is none.
    svg: Option<String>,
    note: String,
}

/// What the app hands the server threads, and what they collect for it.
#[derive(Default)]
struct ViewerState {
    trusted: Vec<String>,
    pending: Vec<String>,
    refused: Vec<String>,
    /// Devices that asked and are not yet in `pending`, oldest first.
    asking: Vec<String>,
    view: Arc<View>,
}

/// External Preview's [`Pages`].
pub struct Viewer {
    state: Mutex<ViewerState>,
    /// Asks the app for a frame, so a new device is listed while the window is idle.
    wake: Arc<dyn Fn() + Send + Sync>,
}

impl Viewer {
    fn new(wake: Arc<dyn Fn() + Send + Sync>) -> Viewer {
        Viewer { state: Mutex::new(ViewerState::default()), wake }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ViewerState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

impl Pages for Viewer {
    fn answer(&self, rest: &str, peer: IpAddr) -> Reply {
        if !rest.is_empty() {
            return Reply::text(404, "Not found.");
        }
        let device = peer.to_canonical().to_string();
        let mut state = self.lock();
        if state.trusted.contains(&device) {
            let view = Arc::clone(&state.view);
            drop(state);
            return Reply::page(200, preview_page(&view));
        }
        if state.refused.contains(&device) {
            return Reply::page(
                403,
                page(
                    "External Preview",
                    None,
                    "<p>The computer refused this device. It can ask again once External Preview is turned off and on.</p>",
                ),
            );
        }
        let new = !state.pending.contains(&device) && !state.asking.contains(&device);
        if new && state.asking.len() < MAX_PENDING_DEVICES {
            state.asking.push(device.clone());
            drop(state);
            (self.wake)();
        }
        let body = format!(
            "<p>Waiting for the computer to confirm this device ({}). Accept it there; this page then shows the active Edit View.</p>",
            escape(&device)
        );
        Reply::page(200, page("External Preview", Some(WAIT_REFRESH_SECONDS), &body))
    }
}

/// The trusted device's page.
fn preview_page(view: &View) -> String {
    let body = match &view.svg {
        Some(svg) => format!("<main>{svg}</main>"),
        None => format!("<p>{}</p>", escape(&view.note)),
    };
    page(&view.title, Some(REFRESH_SECONDS), &body)
}

/// A page with no script; `refresh` reloads it after that many seconds.
fn page(title: &str, refresh: Option<u32>, body: &str) -> String {
    let refresh = refresh.map(|s| format!("<meta http-equiv=\"refresh\" content=\"{s}\">")).unwrap_or_default();
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">{refresh}<title>{}</title><style>body{{margin:0;background:#fff;color:#000;font:16px system-ui,sans-serif}}main{{padding:16px}}svg{{display:block;width:100%;height:auto}}p{{margin:16px}}</style></head><body>{body}</body></html>",
        escape(title)
    )
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            other => out.push(other),
        }
    }
    out
}

/// What a drawing was made from: the tab and its document's revision.
#[derive(Clone, Debug, PartialEq)]
struct Drawn {
    tab: Option<Tab>,
    revision: u64,
}

/// The app's External Preview: the server while the setting is on, and the hand-over between the
/// server threads and the session.
pub struct ExternalPreview {
    scope: Scope,
    wake: Arc<dyn Fn() + Send + Sync>,
    server: Option<Server<Viewer>>,
    /// The settings revision a start failed at: no new try until the settings change.
    failed_at: Option<u64>,
    /// The device lists last handed to the server: trusted, pending, refused.
    devices: (Vec<String>, Vec<String>, Vec<String>),
    /// What the page shows was drawn from, and when.
    drawn: Option<(Drawn, Instant)>,
}

impl ExternalPreview {
    /// `scope`: the local network in the app, this computer in tests. `wake` asks for a frame.
    pub fn new(scope: Scope, wake: Arc<dyn Fn() + Send + Sync>) -> ExternalPreview {
        ExternalPreview { scope, wake, server: None, failed_at: None, devices: Default::default(), drawn: None }
    }

    /// Once per frame: starts or stops the server with the setting, lists the devices that asked
    /// in the session, hands the server the device lists, and draws the tab `tab` reads when a
    /// trusted device may see it and it changed. Returns how long until the next frame is
    /// needed, when a drawing waits for [`MIN_DRAW_INTERVAL`].
    pub fn sync(&mut self, session: &mut Session, tab: impl FnOnce() -> Option<Tab>) -> Option<Duration> {
        if !sharing::enabled(session.settings()) {
            if let Some(mut server) = self.server.take() {
                server.stop();
            }
            self.failed_at = None;
            self.devices = Default::default();
            self.drawn = None;
            if *session.sharing() != SharingState::default() {
                *session.sharing_mut() = SharingState::default();
            }
            return None;
        }
        if self.server.is_none() {
            let revision = session.settings().revision();
            if self.failed_at == Some(revision) {
                return None;
            }
            match Server::serve(Arc::new(Viewer::new(Arc::clone(&self.wake))), self.scope, Limits::default()) {
                Ok(server) => {
                    let state = session.sharing_mut();
                    state.address = Some(server.address.clone());
                    state.problem = None;
                    self.failed_at = None;
                    self.server = Some(server);
                }
                Err(problem) => {
                    let state = session.sharing_mut();
                    state.address = None;
                    state.problem = Some(problem);
                    self.failed_at = Some(revision);
                    return None;
                }
            }
        }
        let viewer = Arc::clone(self.server.as_ref()?.pages());

        let trusted = sharing::trusted_devices(session.settings());
        let asked = std::mem::take(&mut viewer.lock().asking);
        for device in &asked {
            session.sharing_mut().ask(device, &trusted);
        }
        let devices = (trusted, session.sharing().pending.clone(), session.sharing().refused.clone());
        if devices != self.devices {
            let mut state = viewer.lock();
            state.trusted.clone_from(&devices.0);
            state.pending.clone_from(&devices.1);
            state.refused.clone_from(&devices.2);
            drop(state);
            self.devices = devices;
        }

        // Nothing is drawn while no device may see it.
        if self.devices.0.is_empty() {
            self.drawn = None;
            return None;
        }
        let tab = tab();
        let revision = tab.as_ref().and_then(|t| session.documents().iter().find(|d| d.id == t.document)).map_or(0, |d| d.revision);
        let now = Drawn { tab, revision };
        if let Some((before, at)) = &self.drawn {
            if *before == now {
                return None;
            }
            let since = at.elapsed();
            if since < MIN_DRAW_INTERVAL {
                return Some(MIN_DRAW_INTERVAL.saturating_sub(since));
            }
        }
        let view = draw(session, now.tab.as_ref());
        viewer.lock().view = Arc::new(view);
        self.drawn = Some((now, Instant::now()));
        None
    }
}

/// External Preview as the UI's frame hook: the page shows the active document's shown glyph in its first master.
pub struct Hook(pub ExternalPreview);

impl fontcraft_ui::app::preview_host::FrameHook for Hook {
    fn frame(&mut self, app: &mut fontcraft_ui::FontCraftApp) -> Option<Duration> {
        let glyph = app.models.glyph_metrics.glyph.clone();
        let tab = app.session.active().map(|d| Tab::new(d.id, d.title.clone(), glyph.into_iter(), None));
        self.0.sync(&mut app.session, || tab)
    }
}

/// Draws `tab` with `preview.render`, or says why it cannot be drawn.
fn draw(session: &mut Session, tab: Option<&Tab>) -> View {
    let say = |title: &str, why: &str| View { title: title.to_string(), svg: None, note: why.to_string() };
    let Some(tab) = tab else { return say("External Preview", "The computer shows no Edit View.") };
    if session.active().map(|d| d.id) != Some(tab.document) {
        return say(&tab.title, "The Edit View's font is not the active document.");
    }
    if tab.glyphs.is_empty() {
        return say(&tab.title, "The Edit View shows no glyphs.");
    }
    // A master the font no longer has is drawn in the first master, as the tab draws it.
    let masters: Vec<Option<&String>> = if tab.master.is_some() { vec![tab.master.as_ref(), None] } else { vec![None] };
    let mut problem = String::new();
    for master in &masters {
        for size in DRAW_HEIGHTS {
            let params = json!({"glyphs": tab.glyphs, "master": master, "format": "svg", "size": size});
            match session.query("preview.render", &params) {
                Ok(reply) => {
                    return match reply.get("svg").and_then(Value::as_str) {
                        Some(svg) => View { title: tab.title.clone(), svg: Some(svg.to_string()), note: String::new() },
                        None => say(&tab.title, "The Edit View could not be drawn."),
                    };
                }
                Err(e) => problem = e.to_string(),
            }
        }
    }
    say(&tab.title, &format!("The Edit View could not be drawn: {problem}"))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing, clippy::string_slice)]

    use std::io::{Read, Write};
    use std::net::{SocketAddr, TcpStream};

    use super::*;

    /// Fetches `address` (`http://ip:port/token/`) and returns the status code and the body.
    fn fetch(address: &str) -> (u16, String) {
        let rest = address.strip_prefix("http://").unwrap();
        let (host, path) = rest.split_once('/').unwrap();
        let at: SocketAddr = host.parse().unwrap();
        let mut stream = TcpStream::connect(at).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        stream.write_all(format!("GET /{path} HTTP/1.1\r\nHost: {at}\r\n\r\n").as_bytes()).unwrap();
        let mut out = Vec::new();
        stream.read_to_end(&mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        let (head, body) = text.split_once("\r\n\r\n").unwrap();
        (head[9..12].parse().unwrap(), body.to_string())
    }

    fn tab(session: &Session) -> Option<Tab> {
        let document = session.active()?.id;
        Some(Tab::new(document, "A".into(), ["A".to_string()].into_iter(), None))
    }

    #[test]
    fn external_preview_waits_for_confirmation_shows_the_tab_and_forgets_on_reset() {
        let mut session = Session::with_document();
        session.execute("glyph.newGlyph", &json!({"name": "A"})).unwrap();
        let woken = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = Arc::clone(&woken);
        let mut preview = ExternalPreview::new(
            Scope::ThisComputer,
            Arc::new(move || {
                counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            }),
        );

        // Off by default: nothing listens.
        preview.sync(&mut session, || None);
        assert_eq!(session.sharing().address, None);

        session.execute("settings.set", &json!({"id": "sharing.enableExternalPreview", "value": true})).unwrap();
        preview.sync(&mut session, || None);
        let address = session.sharing().address.clone().expect("the server runs once the setting is on");

        // A new device waits and is listed once the app looks.
        let (status, body) = fetch(&address);
        assert_eq!(status, 200);
        assert!(body.contains("Waiting") && !body.contains("<svg"), "{body}");
        assert!(body.contains("http-equiv=\"refresh\""), "the waiting page asks again by itself");
        assert_eq!(woken.load(std::sync::atomic::Ordering::SeqCst), 1, "a new device asks the app for a frame");
        let t = tab(&session);
        preview.sync(&mut session, || t);
        assert_eq!(session.sharing().pending, ["127.0.0.1"]);

        // Once trusted, it gets the tab as SVG, with no script.
        session.execute("sharing.answerDevice", &json!({"device": "127.0.0.1", "trust": true})).unwrap();
        let t = tab(&session);
        preview.sync(&mut session, || t);
        let (status, body) = fetch(&address);
        assert_eq!(status, 200);
        assert!(body.contains("<svg"), "{body}");
        assert!(!body.contains("<script"), "{body}");

        // Reset: the device must be confirmed again.
        session.execute("settings.resetTrustedDevices", &json!({})).unwrap();
        preview.sync(&mut session, || None);
        let (_, body) = fetch(&address);
        assert!(body.contains("Waiting") && !body.contains("<svg"), "{body}");

        // A refused device is told so.
        preview.sync(&mut session, || None);
        session.execute("sharing.answerDevice", &json!({"device": "127.0.0.1", "trust": false})).unwrap();
        preview.sync(&mut session, || None);
        assert_eq!(fetch(&address).0, 403);

        // Off again: the port closes and the session forgets the address and the lists.
        session.execute("settings.set", &json!({"id": "sharing.enableExternalPreview", "value": false})).unwrap();
        preview.sync(&mut session, || None);
        assert_eq!(*session.sharing(), SharingState::default());
        let at: SocketAddr = address.strip_prefix("http://").unwrap().split_once('/').unwrap().0.parse().unwrap();
        assert!(TcpStream::connect_timeout(&at, Duration::from_secs(2)).is_err());
    }
}
