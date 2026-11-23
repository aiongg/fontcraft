//! The Features tab's work off the window's thread (M8.25): the check of the code as typed, and the compile after the
//! code changed.
//!
//! - **Check.** The code editor reports the code as typed (`ViewMsg::FeatureDraft`). Once the draft has stood for
//!   [`CHECK_PAUSE`] seconds, `font.features.check` runs on it through the host's jobs (a thread of its own on the
//!   desktop; at once on the web and in tests with inline jobs), with the font of the revision shown then. One check
//!   runs at a time; a check whose draft is no longer the one typed when it lands is dropped, and the newest draft is
//!   checked next. The answer goes to the view state (`FeaturesView::checked`), which the code editor underlines.
//! - **Compile.** After `font.features.setCode`, `update` or `updateEntry`, the preview fonts of the current master are
//!   built in the background once the document has stood still for [`CHECK_PAUSE`] seconds
//!   (`fontcraft_engine::feature_state::prebuild`). When the build lands on the revision it read, `font.features.compile`
//!   runs on the window's thread and takes that build instead of compiling; an edit in between starts it again.

use std::sync::{Arc, Mutex};

use fontcraft_engine::cmd::features_check::{CheckContext, check_context, check_font};
use fontcraft_engine::feature_state::SessionFeaturesExt as _;
use serde_json::{Value, json};

use crate::app::FontCraftApp;
use crate::shell::font_info_tabs::{CheckedDraft, FeatureDraft};

/// How long, in seconds, a draft or a document must stand still before it is checked or compiled.
pub const CHECK_PAUSE: f64 = 0.5;
/// How often a frame looks for finished work, in seconds.
const POLL: f64 = 0.03;

/// Where a job leaves its result.
type Landing<T> = Arc<Mutex<Option<T>>>;

/// What a compile job read: the document's id and revision, and the master.
type CompileKey = (u64, u64, String);

/// The checks and compiles under way and what waits for them.
#[derive(Default)]
pub struct FeatureWork {
    /// The draft last seen, and the time it was first seen.
    seen: Option<(FeatureDraft, f64)>,
    /// The check under way, and the draft it checks.
    check: Option<(FeatureDraft, Landing<Result<Value, String>>)>,
    /// New code waits for a compile.
    compile_wanted: bool,
    /// The document revision last seen while a compile waits, and the time it was first seen.
    still: Option<((u64, u64), f64)>,
    /// The build under way.
    compile: Option<(CompileKey, Landing<()>)>,
}

impl FeatureWork {
    /// Takes the wait out of the pauses, as if they had passed: for tests, which do not wait.
    #[cfg(test)]
    pub fn skip_pauses(&mut self) {
        if let Some((_, since)) = &mut self.seen {
            *since = f64::NEG_INFINITY;
        }
        if let Some((_, since)) = &mut self.still {
            *since = f64::NEG_INFINITY;
        }
    }
}

/// After a run of `id`: code that changed is compiled in the background.
pub fn after_run(app: &mut FontCraftApp, id: &str) {
    if matches!(id, "font.features.setCode" | "font.features.update" | "font.features.updateEntry") {
        app.feature_work.compile_wanted = true;
    }
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
    let check = step_check(app, now);
    let compile = step_compile(app, now);
    match (check, compile) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

fn take<T>(landing: &Landing<T>) -> Option<T> {
    landing.lock().ok().and_then(|mut l| l.take())
}

fn step_check(app: &mut FontCraftApp, now: f64) -> Option<f64> {
    let draft = app.view.font_info.features.draft.clone();
    // Without a draft the editor shows the stored code: the last compile speaks for it, and a check could go stale.
    if draft.is_none() && app.view.font_info.features.checked.take().is_some() {
        app.view_changed();
    }
    let work = &mut app.feature_work;
    match &draft {
        None => work.seen = None,
        Some(d) if work.seen.as_ref().is_none_or(|(seen, _)| seen != d) => work.seen = Some((d.clone(), now)),
        Some(_) => {}
    }
    if let Some((asked, landing)) = &work.check
        && let Some(result) = take(landing)
    {
        let asked = asked.clone();
        work.check = None;
        if draft.as_ref() == Some(&asked) {
            let diagnostics = result.map(|r| crate::app::models::font_features::checked(&r, &asked.key)).unwrap_or_default();
            app.view.font_info.features.checked = Some(CheckedDraft { draft: asked, diagnostics });
            app.view_changed();
        }
    }
    let work = &app.feature_work;
    if work.check.is_some() {
        return Some(POLL);
    }
    let (wanted, since) = work.seen.clone()?;
    if app.view.font_info.features.checked.as_ref().is_some_and(|c| c.draft == wanted) {
        return None;
    }
    let wait = CHECK_PAUSE - (now - since);
    if wait > 0.0 {
        return Some(wait.max(POLL));
    }
    start_check(app, wanted);
    app.feature_work.check.is_some().then_some(POLL)
}

/// Starts the check of `draft` against the active document as it is now.
fn start_check(app: &mut FontCraftApp, draft: FeatureDraft) {
    let Some(doc) = app.session.active() else { return };
    let font = Arc::clone(&doc.font);
    let Ok(cx) = check_context(&app.session) else { return };
    let master = crate::shell::masters::current_master_id(&font, &app.view.layers);
    let mut params = draft.key.with(&[("code", json!(draft.code))]);
    if let (Some(m), Some(obj)) = (master, params.as_object_mut()) {
        obj.insert("master".into(), Value::String(m));
    }
    let landing: Landing<Result<Value, String>> = Arc::default();
    let out = Arc::clone(&landing);
    let wake = app.window.clone();
    let job = move || {
        let result = run_check(&font, &cx, &params);
        if let Ok(mut slot) = out.lock() {
            *slot = Some(result);
        }
        if let Some(ctx) = wake {
            ctx.request_repaint();
        }
    };
    app.feature_work.check = Some((draft, landing));
    app.host.jobs().spawn("feature code check", Box::new(job));
}

fn run_check(font: &fontcraft_engine::model::Font, cx: &CheckContext, params: &Value) -> Result<Value, String> {
    fontcraft_engine::guard::catch_panic(|| check_font(font, cx, params).map_err(|e| e.to_string()))
        .unwrap_or_else(|e| Err(format!("the check stopped: {e}")))
}

fn step_compile(app: &mut FontCraftApp, now: f64) -> Option<f64> {
    let active = app.session.active().map(|d| (d.id, d.revision));
    if let Some((key, landing)) = &app.feature_work.compile
        && take(landing).is_some()
    {
        let key = key.clone();
        app.feature_work.compile = None;
        if active == Some((key.0, key.1)) {
            // The build of this revision is kept: the compile takes it and only maps the diagnostics.
            let _ = app.session.execute("font.features.compile", &json!({ "master": key.2 }));
        } else {
            app.feature_work.compile_wanted = true;
        }
    }
    let work = &mut app.feature_work;
    if work.compile.is_some() {
        return Some(POLL);
    }
    if !work.compile_wanted {
        work.still = None;
        return None;
    }
    let Some(active) = active else {
        work.compile_wanted = false;
        return None;
    };
    if work.still.as_ref().is_none_or(|(seen, _)| *seen != active) {
        work.still = Some((active, now));
    }
    let since = work.still.as_ref().map_or(now, |(_, at)| *at);
    let wait = CHECK_PAUSE - (now - since);
    if wait > 0.0 {
        return Some(wait.max(POLL));
    }
    work.compile_wanted = false;
    work.still = None;
    start_compile(app);
    app.feature_work.compile.is_some().then_some(POLL)
}

/// Starts the background build of the active document's current master.
fn start_compile(app: &mut FontCraftApp) {
    let Some(doc) = app.session.active() else { return };
    let (id, revision, font) = (doc.id, doc.revision, Arc::clone(&doc.font));
    let Some(master) = crate::shell::masters::current_master_id(&font, &app.view.layers) else { return };
    let Ok(cx) = check_context(&app.session) else { return };
    let options = fontcraft_engine::compile::Options {
        glyph_data: fontcraft_engine::compile::DataRef(Some(cx.glyph_data)),
        feature_dir: cx.include_dir,
        include_check: fontcraft_engine::file_access::include_check(),
        ..fontcraft_engine::compile::Options::default()
    };
    let builds = app.session.preview_builds();
    let landing: Landing<()> = Arc::default();
    let out = Arc::clone(&landing);
    let wake = app.window.clone();
    let key: CompileKey = (id, revision, master);
    let job_master = key.2.clone();
    let job = move || {
        let _ =
            fontcraft_engine::guard::catch_panic(|| fontcraft_engine::feature_state::prebuild(&font, id, revision, &job_master, &options, &builds));
        if let Ok(mut slot) = out.lock() {
            *slot = Some(());
        }
        if let Some(ctx) = wake {
            ctx.request_repaint();
        }
    };
    app.feature_work.compile = Some((key, landing));
    app.host.jobs().spawn("feature code compile", Box::new(job));
}
