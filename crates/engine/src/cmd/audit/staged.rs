//! Audit runs in steps (M12.64b): `audit.start`, `audit.step` and `audit.cancel`.
//!
//! A long run on the open document is split so a window can show progress and stop it: no
//! thread is needed, so it works the same on the web. `audit.start` reads the parameters of
//! `audit.run` and takes a snapshot of the font; each `audit.step` examines the next layers
//! (`budget` of them), then, one step each, the glyph and source checks, the in-memory compile of
//! the instances, and the binary and family checks. The last step keeps the run as `audit.run`
//! does and returns its result document; until then the kept run stays as it was. `audit.cancel`
//! drops the run and returns what it found so far, marked `partial`.
//!
//! The results equal those of one `audit.run` on the snapshot: the layers are examined in the
//! same order and the findings sorted the same way at the end.

use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, Mutex};

use super::*;

/// Layers one step examines when the call names no budget.
const DEFAULT_BUDGET: i64 = 256;
/// Most layers one step examines.
const MAX_BUDGET: i64 = 100_000;
/// Steps after the layers: glyph and source checks, compiling, binary and family checks.
const LATER_STAGES: usize = 3;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "audit.start",
            "Start Audit",
            [],
            None,
            "{profile?, checks?, exclude?, groups?, scope?, glyphs?, glyphFile?: path, masters?, layers?, minSeverity?, config?, maxResults?, includePassed?, includeIgnored?, network?: as audit.run} starts a run of audit.run on the open document in steps, on the font as it is now; nothing is examined until audit.step. A run already in progress on the document is dropped. Files and families are audited with audit.run → {runId, stage: \"layers\", done: 0, total: the layers to examine plus 3}",
            has_doc,
            start
        )
        .files(const { &[FileParam::read("glyphFile")] }),
        cmd!(
            query "audit.step",
            "Continue Audit",
            [],
            None,
            "{runId: id, budget?: count = 256 (layers per step, 1 to 100,000)} examines the next `budget` layers of the run in progress, else its next later stage: the glyph and source checks, the compile of the source's instances, the binary and family checks. The last step keeps the run for audit.results and audit.fix, as audit.run does. Refused when the run is not in progress on the open document → {runId, stage: \"layers\" | \"source\" | \"compile\" | \"binary\" | \"done\", done, total, finished: bool, report?: the result document of audit.run (when finished)}",
            has_doc,
            step
        ),
        cmd!(
            query "audit.cancel",
            "Cancel Audit",
            [],
            None,
            "{runId: id} stops the run in progress and drops it. The kept run and the font do not change → {runId, cancelled: true, done, total, partial: the result document of what the run had examined, with partial: true and fixes named by checks and glyphs}",
            has_doc,
            cancel
        ),
    ]
}

/// Where a run in steps is.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Stage {
    Layers,
    Source,
    Compile,
    Binary,
    Done,
}

impl Stage {
    fn as_str(self) -> &'static str {
        match self {
            Stage::Layers => "layers",
            Stage::Source => "source",
            Stage::Compile => "compile",
            Stage::Binary => "binary",
            Stage::Done => "done",
        }
    }
}

/// A run in steps on one document.
pub struct Staged {
    id: String,
    /// The font as it was when the run started; the run examines it.
    font: Arc<Font>,
    target: Target,
    run: Run,
    values: BTreeSet<String>,
    layer_checks: Vec<&'static CheckSpec>,
    /// The layers to examine, as glyph and layer indices into `font`, found once at the start.
    layer_ids: Vec<(usize, usize)>,
    /// The font-wide facts the audit context holds, surveyed once at the start.
    context_facts: Facts,
    layers: usize,
    /// Layers examined so far.
    examined: usize,
    stage: Stage,
    ex: Examined,
    raw: Raw,
    places: HashMap<usize, Places>,
    /// What [`Staged::finish`] changes, as it was before the first call: a step or cancel
    /// whose report then fails leaves the run as it was, and the next try finishes it again.
    before_finish: Option<BeforeFinish>,
}

/// The parts of a run that finishing it changes.
struct BeforeFinish {
    truncated: usize,
    tally_notes: Vec<String>,
    target_notes: usize,
}

impl std::fmt::Debug for Staged {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Staged").field("id", &self.id).field("stage", &self.stage).field("examined", &self.examined).finish()
    }
}

/// What a run in steps lives in on its document.
pub type Slot = Option<Arc<Mutex<Staged>>>;

impl Staged {
    fn total(&self) -> usize {
        self.layers.saturating_add(LATER_STAGES)
    }

    fn done(&self) -> usize {
        let later = match self.stage {
            Stage::Layers | Stage::Source => 0,
            Stage::Compile => 1,
            Stage::Binary => 2,
            Stage::Done => 3,
        };
        self.examined.saturating_add(later)
    }

    fn facts(&self, source: bool) -> audit::run::Facts {
        audit::run::Facts { source, variable: source && self.font.masters.len() > 1, network: self.run.network, values: self.values.clone() }
    }

    /// Does the next piece of work.
    fn advance(&mut self, budget: usize) {
        match self.stage {
            Stage::Layers => {
                let end = self.examined.saturating_add(budget).min(self.layers);
                let glyphs = self.font.glyphs.as_arcs();
                let chosen: Vec<(usize, &Arc<Glyph>, usize, &Layer)> = self
                    .layer_ids
                    .get(self.examined..end)
                    .unwrap_or_default()
                    .iter()
                    .filter_map(|&(gi, li)| {
                        let g = glyphs.get(gi)?;
                        Some((gi, g, li, g.layers.get(li)?))
                    })
                    .collect();
                let ctx = context(&self.font, self.context_facts);
                examine_layers(&ctx, 0, &self.run, &self.layer_checks, &chosen, &mut self.ex, &mut self.raw, &mut self.places);
                self.examined = end;
                if end >= self.layers {
                    self.stage = Stage::Source;
                }
            }
            Stage::Source => {
                let facts = self.facts(true);
                examine_rest(&self.font, 0, &self.run, &facts, &mut self.ex, &mut self.raw);
                self.stage = Stage::Compile;
            }
            Stage::Compile => {
                compile_sources(&mut self.target, &self.run, &mut self.ex);
                self.stage = Stage::Binary;
            }
            Stage::Binary => {
                let base = self.facts(false);
                let ids = self.target.ids();
                examine_binaries(&self.target.bins, &ids, &self.target.settings.values, &self.run, &base, &mut self.ex, &mut self.raw);
                self.stage = Stage::Done;
            }
            Stage::Done => {}
        }
    }

    /// Sorts and caps what the run found, as the end of `audit.run` does. The findings stay in
    /// the run, so finishing again gives the same result.
    fn finish(&mut self) {
        match &self.before_finish {
            Some(b) => {
                self.ex.tally.truncated = b.truncated;
                self.ex.tally.notes = b.tally_notes.clone();
                self.target.notes.truncate(b.target_notes);
            }
            None => {
                self.before_finish = Some(BeforeFinish {
                    truncated: self.ex.tally.truncated,
                    tally_notes: self.ex.tally.notes.clone(),
                    target_notes: self.target.notes.len(),
                });
            }
        }
        let ignore = self.target.settings.ignore_index();
        finish(self.raw.clone(), &self.run, &self.places, &ignore, true, &mut self.ex);
        self.target.notes.append(&mut self.ex.tally.notes);
    }

    fn progress(&self) -> Value {
        json!({"runId": self.id, "stage": self.stage.as_str(), "done": self.done(), "total": self.total()})
    }
}

fn start(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("audit.start", v)?;
    document_target(&p)?;
    let doc = s.doc()?;
    let target = document_target_of(doc);
    let font = doc.font.clone();
    let run = read_run(&p, s, &font, &target.settings)?;
    let values: BTreeSet<String> = target.settings.values.keys().cloned().collect();
    let mut staged = Staged {
        id: next_run_id(),
        font: font.clone(),
        target,
        run,
        values,
        layer_checks: Vec::new(),
        layer_ids: Vec::new(),
        context_facts: (None, None, false),
        layers: 0,
        examined: 0,
        stage: Stage::Layers,
        ex: Examined::default(),
        raw: Raw::new(),
        places: HashMap::new(),
        before_finish: None,
    };
    let facts = staged.facts(true);
    staged.layer_checks = start_font(&font, 0, &staged.run, &facts, &mut staged.ex, &mut staged.places);
    staged.layer_ids = layers_of(&font, &staged.run).into_iter().map(|(gi, _, li, _)| (gi, li)).collect();
    staged.layers = staged.layer_ids.len();
    let ctx = AuditContext::new(&font);
    staged.context_facts = (ctx.grid_step, ctx.main_curve, ctx.variable);
    if staged.layers == 0 {
        staged.stage = Stage::Source;
    }
    let out = staged.progress();
    s.doc_mut()?.audit_staged = Some(Arc::new(Mutex::new(staged)));
    Ok(out)
}

/// The run `runId` of the open document.
fn in_progress(p: &Params, s: &Session) -> Result<Arc<Mutex<Staged>>> {
    let id = p.str("runId")?;
    let slot = s.doc()?.audit_staged.clone();
    let staged = slot.ok_or_else(|| p.bad(format!("no audit run `{id}` is in progress on this document")))?;
    let same = staged.lock().map(|g| g.id == id).unwrap_or(false);
    if !same {
        return Err(p.bad(format!("no audit run `{id}` is in progress on this document")));
    }
    Ok(staged)
}

fn step(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("audit.step", v)?;
    let budget = p.opt_int("budget", 1, MAX_BUDGET)?.unwrap_or(DEFAULT_BUDGET);
    let budget = usize::try_from(budget).unwrap_or(1);
    let staged = in_progress(&p, s)?;
    let mut st = staged.lock().map_err(|_| EngineError::Internal("the audit run in progress failed earlier; start it again".into()))?;
    st.advance(budget);
    if st.stage != Stage::Done {
        let mut out = st.progress();
        if let Some(o) = out.as_object_mut() {
            o.insert("finished".into(), json!(false));
        }
        return Ok(out);
    }
    st.finish();
    let (id, font) = (st.id.clone(), st.font.clone());
    let report = keep_run(s, &st.target, &st.run, &st.ex, font, id)?;
    let mut out = st.progress();
    drop(st);
    s.doc_mut()?.audit_staged = None;
    if let Some(o) = out.as_object_mut() {
        o.insert("finished".into(), json!(true));
        o.insert("report".into(), report);
    }
    Ok(out)
}

fn cancel(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("audit.cancel", v)?;
    let staged = in_progress(&p, s)?;
    let mut st = staged.lock().map_err(|_| EngineError::Internal("the audit run in progress failed earlier".into()))?;
    let (done, total) = (st.done(), st.total());
    st.finish();
    let mut partial = document(&st.target, &st.run, &st.ex, FixParams::BySelection, false)?;
    if let Some(o) = partial.as_object_mut() {
        o.insert("runId".into(), json!(st.id));
        o.insert("partial".into(), json!(true));
    }
    let id = st.id.clone();
    drop(st);
    s.doc_mut()?.audit_staged = None;
    Ok(json!({"runId": id, "cancelled": true, "done": done, "total": total, "partial": partial}))
}
