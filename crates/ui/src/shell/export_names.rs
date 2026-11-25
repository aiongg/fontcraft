//! What the exports write, for the Exports tab (M7.32): `font.instance.names` run off the window's thread while the tab
//! shows.
//!
//! The query runs through the host's jobs on the font of the revision shown then: at once for a document whose names are
//! not known yet, after [`CHECK_PAUSE`] once the document changed. A result of a revision no longer shown is dropped and
//! the font is asked again. The answer goes to the view state (`ExportsView::names`); until the next one lands the tab
//! keeps showing the last one of the document.

use std::sync::{Arc, Mutex};

use fontcraft_engine::cmd::instance_names::names_of;
use serde_json::{Value, json};

use super::feature_work::{CHECK_PAUSE, exports_tab_shows};
use crate::app::FontCraftApp;
use crate::shell::font_info_tabs::ExportedNames;

/// How often a frame looks for finished work, in seconds.
const POLL: f64 = 0.03;

/// The document's id and revision.
type Key = (u64, u64);

/// The query under way and what waits for it.
#[derive(Default)]
pub struct ExportNamesWork {
    /// The revision last seen, and the time it was first seen.
    seen: Option<(Key, f64)>,
    job: Option<(Key, Arc<Mutex<Option<Value>>>)>,
}

/// One frame's look at the work: collects what finished, starts what is due, and asks for a frame when work waits.
pub fn logic(app: &mut FontCraftApp, ctx: &egui::Context) {
    let now = ctx.input(|i| i.time);
    if let Some(wait) = step(app, now) {
        ctx.request_repaint_after(std::time::Duration::from_secs_f64(wait.max(0.0)));
    }
}

/// [`logic`] at time `now`, in seconds; returns how many seconds until a frame should look again.
pub fn step(app: &mut FontCraftApp, now: f64) -> Option<f64> {
    let wanted = if exports_tab_shows(app) { app.session.active().map(|d| (d.id, d.revision)) } else { None };
    if let Some((key, landing)) = &app.export_names.job
        && let Some(reply) = landing.lock().ok().and_then(|mut l| l.take())
    {
        let key = *key;
        app.export_names.job = None;
        if wanted == Some(key) {
            app.view.font_info.exports.names = Some(ExportedNames { doc: key.0, revision: key.1, reply: Arc::new(reply) });
            app.view_changed();
        }
    }
    let Some(wanted) = wanted else {
        app.export_names.seen = None;
        return None;
    };
    if app.export_names.job.is_some() {
        return Some(POLL);
    }
    let known = app.view.font_info.exports.names.as_ref().map(|n| (n.doc, n.revision));
    if known == Some(wanted) {
        return None;
    }
    let since = match app.export_names.seen {
        Some((seen, since)) if seen == wanted => since,
        _ => {
            app.export_names.seen = Some((wanted, now));
            now
        }
    };
    let wait = CHECK_PAUSE - (now - since);
    if known.is_some_and(|(doc, _)| doc == wanted.0) && wait > 0.0 {
        return Some(wait.max(POLL));
    }
    start(app, wanted);
    app.export_names.job.is_some().then_some(POLL)
}

fn start(app: &mut FontCraftApp, key: Key) {
    let Some(doc) = app.session.active() else { return };
    let (font, title) = (Arc::clone(&doc.font), doc.title.clone());
    let landing: Arc<Mutex<Option<Value>>> = Arc::default();
    let out = Arc::clone(&landing);
    let wake = app.window.clone();
    let job = move || {
        let reply = fontcraft_engine::guard::catch_panic(|| names_of(&font, &title, &json!({})).map_err(|e| e.to_string()))
            .unwrap_or_else(Err)
            .unwrap_or_else(|error| json!({ "error": error }));
        if let Ok(mut slot) = out.lock() {
            *slot = Some(reply);
        }
        if let Some(ctx) = wake {
            ctx.request_repaint();
        }
    };
    app.export_names.job = Some((key, landing));
    app.host.jobs().spawn("exported names", Box::new(job));
}
