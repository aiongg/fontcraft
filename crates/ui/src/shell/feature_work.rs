//! The Features tab's work off the window's thread (M8.25): the check of the code as typed, and the compile after the
//! code changed.
//!
//! - **Expansion.** While the Features tab shows the expanded code of a build (`FeaturesView::expanded`),
//!   `font.features.expandCode` runs the same way (`fontcraft_engine::cmd::features_expand::expand_font`): at once for a
//!   build or setting not expanded yet, after the pause when the document changed.
//! - **Check.** The code editor reports the code as typed (`ViewMsg::FeatureDraft`). Once the draft has stood for
//!   [`CHECK_PAUSE`] seconds, `font.features.check` runs on it through the host's jobs (a thread of its own on the
//!   desktop; at once on the web and in tests with inline jobs), with the font of the revision shown then. One check
//!   runs at a time; a check whose draft is no longer the one typed when it lands is dropped, and the newest draft is
//!   checked next. The answer goes to the view state (`FeaturesView::checked`), which the code editor underlines.
//! - **Compile.** After a command that changes the feature entries, and when they changed in another way (undo, a macro),
//!   the current master is compiled in the background, and the code checked for every other master, instance and the
//!   variable font, once the document has stood still for [`CHECK_PAUSE`] seconds, and at once when the tab's Compile button
//!   asks ([`ask`]; the button is busy meanwhile). The job runs all of `font.features.compile` but the keeping of its
//!   result (`fontcraft_engine::feature_state::compile_ahead`). When it lands on the revision it read,
//!   `font.features.compile` runs on the window's thread and takes that result; an edit in between starts it again.
//! - **Instance compile.** While the Exports tab shows one instance with feature-code parameters, that instance's code
//!   is compiled the same way (`fontcraft_engine::cmd::features_check::compile_instance`): at once for an instance not
//!   compiled yet, after the pause when the document changed. A result of a revision or an instance no longer shown is
//!   dropped. The answer goes to the view state (`ExportsView::problems`), which the tab shows under the parameters.

use std::sync::{Arc, Mutex};

use craft::behavior::dock::PanelState;
use fontcraft_engine::cmd::features_check::{CheckContext, check_context, check_font, compile_instance};
use fontcraft_engine::cmd::features_expand::expand_font;
use fontcraft_engine::feature_state::{AheadCompile, SessionFeaturesExt as _, compile_ahead};
use fontcraft_engine::model::{Features, Font, Snippet};
use serde_json::{Value, json};

use crate::app::{FontCraftApp, Msg};
use crate::shell::dock::PanelId;
use crate::shell::font_info::FontInfoTab;
use crate::shell::font_info_tabs::{CheckedDraft, ExpandKey, ExpandTarget, Expansion, FeatureDraft, InstanceProblems, ParamProblem};

/// How long, in seconds, a draft or a document must stand still before it is checked or compiled.
pub const CHECK_PAUSE: f64 = 0.5;
/// How often a frame looks for finished work, in seconds.
const POLL: f64 = 0.03;

/// Where a job leaves its result.
type Landing<T> = Arc<Mutex<Option<T>>>;

/// What a compile job read: the document's id and revision, and the master.
type CompileKey = (u64, u64, String);

/// What an instance compile reads: the document's id and revision, and the instance's index.
type InstanceKey = (u64, u64, usize);

/// An instance compile under way: what it reads, the parameters it compiles, and its result to come.
type InstanceJob = (InstanceKey, Vec<ParamProblem>, Landing<Result<Value, String>>);

/// The checks and compiles under way and what waits for them.
#[derive(Default)]
pub struct FeatureWork {
    /// The draft last seen, and the time it was first seen.
    seen: Option<(FeatureDraft, f64)>,
    /// The check under way, and the draft it checks.
    check: Option<(FeatureDraft, Landing<Result<Value, String>>)>,
    /// New code waits for a compile.
    compile_wanted: bool,
    /// The active document and its feature entries as last seen.
    entries: Option<(u64, Arc<Features>)>,
    /// The document revision last seen while a compile waits, and the time it was first seen.
    still: Option<((u64, u64), f64)>,
    /// The compile under way.
    compile: Option<(CompileKey, Landing<Option<AheadCompile>>)>,
    /// The document and master the Compile button asked for, until that compile lands.
    asked: Option<(u64, String)>,
    /// The instance the Exports tab shows, and the time it was first seen at this revision.
    shown: Option<(InstanceKey, f64)>,
    /// The instance compile under way.
    instance: Option<InstanceJob>,
    /// The expansion the Features tab shows, and the time it was first seen at this revision.
    expand_seen: Option<(ExpandKey, f64)>,
    /// The expansion under way.
    expand: Option<(ExpandKey, Landing<Result<Value, String>>)>,
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
        if let Some((_, since)) = &mut self.shown {
            *since = f64::NEG_INFINITY;
        }
        if let Some((_, since)) = &mut self.expand_seen {
            *since = f64::NEG_INFINITY;
        }
    }
}

/// The commands of the feature entries that change no code.
const KEEP_CODE: [&str; 3] = ["font.features.compile", "font.features.setNotes", "font.features.showSnippetFolder"];

/// After a run of `id`: code that changed is compiled in the background.
pub fn after_run(app: &mut FontCraftApp, id: &str) {
    let changes = id.starts_with("font.features.") && !KEEP_CODE.contains(&id) && fontcraft_engine::find_command(id).is_some_and(|c| !c.query);
    if changes {
        app.feature_work.compile_wanted = true;
    }
}

/// Whether the two sets of entries hold the same code under the same names.
fn same_code(a: &Features, b: &Features) -> bool {
    let same = |a: &[Snippet], b: &[Snippet]| {
        let one = |e: &Snippet| (e.name.clone(), e.disabled, e.labels.clone());
        a.len() == b.len() && a.iter().zip(b).all(|(a, b)| a.code == b.code && one(a) == one(b))
    };
    same(&a.prefixes, &b.prefixes) && same(&a.classes, &b.classes) && same(&a.features, &b.features)
}

/// Feature entries of the document that changed since the last look are compiled in the background, however they
/// changed: an undo, a macro or an agent's command. A document looked at for the first time is left as it is.
fn watch_entries(app: &mut FontCraftApp) {
    let now = app.session.active().map(|d| (d.id, Arc::clone(&d.font.features)));
    let work = &mut app.feature_work;
    let changed = match (&work.entries, &now) {
        (Some((seen, before)), Some((id, entries))) if seen == id => !Arc::ptr_eq(before, entries) && !same_code(before, entries),
        _ => false,
    };
    if changed {
        work.compile_wanted = true;
    }
    work.entries = now;
}

/// Compiles the active document's current master in the background now: the Features tab's Compile button. A compile
/// of the same revision and master under way is the one asked for.
pub fn ask(app: &mut FontCraftApp) -> Result<(), String> {
    let doc = app.session.active().ok_or("no document is open")?;
    let id = doc.id;
    let master = crate::shell::masters::current_master_id(&doc.font, &app.view.layers).ok_or("the font has no master")?;
    check_context(&app.session).map_err(|e| e.to_string())?;
    app.feature_work.asked = Some((id, master));
    if app.feature_work.compile.is_none() {
        app.feature_work.compile_wanted = false;
        app.feature_work.still = None;
        start_compile(app);
    }
    show_busy(app);
    Ok(())
}

/// The Compile button is busy while the compile it asked for is under way.
fn show_busy(app: &mut FontCraftApp) {
    let busy = app.feature_work.asked.is_some();
    if app.view.font_info.features.compiling != busy {
        app.view.font_info.features.compiling = busy;
        app.view_changed();
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
    watch_entries(app);
    let waits = [step_check(app, now), step_compile(app, now), step_instance(app, now), step_expand(app, now)];
    show_busy(app);
    waits.into_iter().flatten().reduce(f64::min)
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
    if app.feature_work.asked.as_ref().is_some_and(|(id, _)| active.map(|a| a.0) != Some(*id)) {
        app.feature_work.asked = None;
    }
    if let Some((key, landing)) = &app.feature_work.compile
        && let Some(result) = take(landing)
    {
        let key = key.clone();
        app.feature_work.compile = None;
        if active == Some((key.0, key.1)) {
            land_compile(app, key, result);
        } else {
            app.feature_work.compile_wanted = true;
        }
        if app.feature_work.asked.is_some() && !app.feature_work.compile_wanted {
            // The compile that landed was of another master.
            start_compile(app);
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

/// Keeps the compile that landed on the revision it read: `font.features.compile` takes it and only maps the
/// diagnostics. A compile that failed has no result; the command then says why.
fn land_compile(app: &mut FontCraftApp, (doc, _, master): CompileKey, result: Option<AheadCompile>) {
    let params = match result {
        Some(compile) => {
            app.session.keep_ahead_compile(compile);
            json!({ "master": master, "targets": "all" })
        }
        // The command fails as the job did, before it checks the other targets here on the window's thread.
        None => json!({ "master": master }),
    };
    if app.feature_work.asked.as_ref().is_some_and(|(d, m)| *d == doc && *m == master) {
        app.feature_work.asked = None;
        if let Err(error) = app.apply_msg(Msg::run("font.features.compile", params)) {
            app.status = error;
        }
    } else {
        let _ = app.session.execute("font.features.compile", &params);
    }
}

/// Starts the background compile of the active document: of the master the Compile button asked for, else of the
/// current one. Nothing waits for a compile that cannot start.
fn start_compile(app: &mut FontCraftApp) {
    let asked = app.feature_work.asked.take();
    let Some(doc) = app.session.active() else { return };
    let (id, revision, font) = (doc.id, doc.revision, Arc::clone(&doc.font));
    let asked_master = asked.filter(|(d, _)| *d == id).map(|(_, m)| m);
    let Some(master) = asked_master.clone().or_else(|| crate::shell::masters::current_master_id(&font, &app.view.layers)) else { return };
    let Ok(cx) = check_context(&app.session) else { return };
    app.feature_work.asked = asked_master.map(|m| (id, m));
    let previous = app.session.feature_state_of(id);
    let builds = app.session.preview_builds();
    let landing: Landing<Option<AheadCompile>> = Arc::default();
    let out = Arc::clone(&landing);
    let wake = app.window.clone();
    let key: CompileKey = (id, revision, master);
    let job_master = key.2.clone();
    let job = move || {
        let result = fontcraft_engine::guard::catch_panic(|| compile_ahead(&font, id, revision, &job_master, &cx, previous, &builds, true));
        if let Ok(mut slot) = out.lock() {
            *slot = Some(result.ok().flatten());
        }
        if let Some(ctx) = wake {
            ctx.request_repaint();
        }
    };
    app.feature_work.compile = Some((key, landing));
    app.host.jobs().spawn("feature code compile", Box::new(job));
}

/// The build the expanded-code view shows: the one picked, else the current master, also when the one picked is gone.
pub fn expand_target(font: &Font, layers: &crate::shell::masters::LayersView, picked: Option<&ExpandTarget>) -> Option<ExpandTarget> {
    let there = picked.filter(|t| match t {
        ExpandTarget::Master(id) => font.masters.iter().any(|m| m.id == *id),
        ExpandTarget::Instance(at) => font.instances.get(*at).is_some_and(|i| i.kind.is_none()),
        ExpandTarget::Variable => true,
    });
    there.cloned().or_else(|| crate::shell::masters::current_master_id(font, layers).map(ExpandTarget::Master))
}

/// What the Features tab's expanded-code view needs, while it shows.
fn wanted_expansion(app: &FontCraftApp) -> Option<ExpandKey> {
    if app.view.font_info.tab != FontInfoTab::Features || app.view.workspace.layout.panel_state(&PanelId::FontInfo) == PanelState::Hidden {
        return None;
    }
    let shown = app.view.font_info.features.expanded.as_ref()?;
    let doc = app.session.active()?;
    Some((doc.id, doc.revision, expand_target(&doc.font, &app.view.layers, shown.target.as_ref())?, shown.generated))
}

fn step_expand(app: &mut FontCraftApp, now: f64) -> Option<f64> {
    let wanted = wanted_expansion(app);
    if let Some((key, landing)) = &app.feature_work.expand
        && let Some(result) = take(landing)
    {
        let key = key.clone();
        app.feature_work.expand = None;
        if wanted.as_ref() == Some(&key) {
            app.view.font_info.features.expansion = Some(expansion(key, result));
            app.view_changed();
        }
    }
    let Some(wanted) = wanted else {
        app.feature_work.expand_seen = None;
        if app.view.font_info.features.expansion.take().is_some() {
            app.view_changed();
        }
        return None;
    };
    if app.feature_work.expand.is_some() {
        return Some(POLL);
    }
    let known = app.view.font_info.features.expansion.as_ref().map(|e| e.key.clone());
    if known.as_ref() == Some(&wanted) {
        return None;
    }
    let since = match &app.feature_work.expand_seen {
        Some((seen, since)) if *seen == wanted => *since,
        _ => {
            app.feature_work.expand_seen = Some((wanted.clone(), now));
            now
        }
    };
    // The build shown before waits until the document stands still.
    let wait = CHECK_PAUSE - (now - since);
    if known.is_some_and(|(doc, _, target, generated)| (doc, &target, generated) == (wanted.0, &wanted.2, wanted.3)) && wait > 0.0 {
        return Some(wait.max(POLL));
    }
    start_expand(app, wanted);
    app.feature_work.expand.is_some().then_some(POLL)
}

/// The expansion that `result`, `font.features.expandCode`'s answer, is.
fn expansion(key: ExpandKey, result: Result<Value, String>) -> Expansion {
    match result {
        Ok(reply) => Expansion {
            key,
            code: Arc::new(reply.get("code").and_then(Value::as_str).unwrap_or_default().to_string()),
            refused: crate::app::models::font_features::strings(reply.get("refused")),
            problem: None,
        },
        Err(problem) => Expansion { key, code: Arc::default(), refused: Vec::new(), problem: Some(problem) },
    }
}

/// Starts the expansion of `key`.
fn start_expand(app: &mut FontCraftApp, key: ExpandKey) {
    let Some(doc) = app.session.active() else { return };
    let font = Arc::clone(&doc.font);
    let Ok(cx) = check_context(&app.session) else { return };
    let landing: Landing<Result<Value, String>> = Arc::default();
    let out = Arc::clone(&landing);
    let wake = app.window.clone();
    let raw = json!({ "target": key.2.param(), "includeImplicit": key.3 });
    let job = move || {
        let result = fontcraft_engine::guard::catch_panic(|| expand_font(&font, &cx, &raw).map_err(|e| e.to_string()))
            .unwrap_or_else(|e| Err(format!("the expansion stopped: {e}")));
        if let Ok(mut slot) = out.lock() {
            *slot = Some(result);
        }
        if let Some(ctx) = wake {
            ctx.request_repaint();
        }
    };
    app.feature_work.expand = Some((key, landing));
    app.host.jobs().spawn("feature code expansion", Box::new(job));
}

/// The one instance the Exports tab shows while Font Info shows that tab.
fn shown_instance(app: &FontCraftApp) -> Option<InstanceKey> {
    if app.view.font_info.tab != FontInfoTab::Exports || app.view.workspace.layout.panel_state(&PanelId::FontInfo) == PanelState::Hidden {
        return None;
    }
    let doc = app.session.active()?;
    let selected = crate::app::models::font_exports::shown_selection(&app.view.font_info.exports.selected, doc.font.instances.len());
    match selected.as_slice() {
        [one] => Some((doc.id, doc.revision, *one)),
        _ => None,
    }
}

fn step_instance(app: &mut FontCraftApp, now: f64) -> Option<f64> {
    let wanted = shown_instance(app);
    if let Some((key, _, landing)) = &app.feature_work.instance
        && let Some(result) = take(landing)
    {
        let key = *key;
        let job = app.feature_work.instance.take();
        if let Some((_, mut params, _)) = job
            && wanted == Some(key)
        {
            if let Ok(reply) = result {
                crate::app::models::font_exports::place_problems(&mut params, &reply);
            }
            keep_problems(app, key, params);
        }
    }
    let Some(wanted) = wanted else {
        app.feature_work.shown = None;
        return None;
    };
    if app.feature_work.instance.is_some() {
        return Some(POLL);
    }
    let known = app.view.font_info.exports.problems.as_ref().map(|p| (p.doc, p.revision, p.instance));
    if known == Some(wanted) {
        return None;
    }
    let since = match app.feature_work.shown {
        Some((seen, since)) if seen == wanted => since,
        _ => {
            app.feature_work.shown = Some((wanted, now));
            now
        }
    };
    // An instance compiled before waits until the document stands still.
    let wait = CHECK_PAUSE - (now - since);
    if known.is_some_and(|(doc, _, instance)| (doc, instance) == (wanted.0, wanted.2)) && wait > 0.0 {
        return Some(wait.max(POLL));
    }
    start_instance(app, wanted);
    app.feature_work.instance.is_some().then_some(POLL)
}

/// The Exports tab shows `params`, what the compile of `key` found.
fn keep_problems(app: &mut FontCraftApp, (doc, revision, instance): InstanceKey, params: Vec<ParamProblem>) {
    let found = Some(InstanceProblems { doc, revision, instance, params });
    let before = app.view.font_info.exports.problems.as_ref().map(|p| &p.params);
    let changed = before != found.as_ref().map(|p| &p.params);
    app.view.font_info.exports.problems = found;
    if changed {
        app.view_changed();
    }
}

/// Starts the instance compile of `key`, when the instance has a feature-code parameter switched on.
fn start_instance(app: &mut FontCraftApp, key: InstanceKey) {
    let Some(doc) = app.session.active() else { return };
    let font = Arc::clone(&doc.font);
    let params = crate::app::models::font_exports::code_params(&font, key.2);
    if params.is_empty() {
        keep_problems(app, key, params);
        return;
    }
    let Ok(cx) = check_context(&app.session) else { return };
    let landing: Landing<Result<Value, String>> = Arc::default();
    let out = Arc::clone(&landing);
    let wake = app.window.clone();
    let instance = key.2;
    let job = move || {
        let raw = json!({ "instance": instance });
        let result = fontcraft_engine::guard::catch_panic(|| compile_instance(&font, &cx, &raw).map_err(|e| e.to_string()));
        if let Ok(mut slot) = out.lock() {
            *slot = Some(result.unwrap_or_else(Err));
        }
        if let Some(ctx) = wake {
            ctx.request_repaint();
        }
    };
    app.feature_work.instance = Some((key, params, landing));
    app.host.jobs().spawn("instance feature code compile", Box::new(job));
}
