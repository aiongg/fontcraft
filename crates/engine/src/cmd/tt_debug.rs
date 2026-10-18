//! The TrueType debugger and delta suggestions. Rows: `hint.tt.debugStep`,
//! `hint.tt.suggestDeltas` (and the engine side of `window.ttDebugger`).
//!
//! Both build the instance as TrueType in memory through the export path, as the hinted preview
//! does (`hint_preview.rs`), and run its programs in FontCraft's own interpreter
//! (`crate::ttvm`), ported from the one the preview draws with, so the points match.
//!
//! The debugger keeps no session state: a result names the step it stopped at, and the next
//! request passes that `step` back with an action. Each request runs the font and control value
//! programs and then the glyph program from its start up to the stop, so the same request always
//! gives the same state. A step is one instruction of the glyph program or of a function it
//! calls; every run stops after `ttvm` 's instruction cap.
//!
//! Delta suggestions run each glyph's program to its end at each size and look at the hinted
//! outline against the pixel grid. Where an edge passes within `within` pixels of a pixel's
//! centre, measured along one axis, whether that pixel is on depends on a fraction of a pixel; a
//! delta on the nearest point of the edge that moves it at least `within` away from the centre
//! makes the pixel's state stable. In backward compatibility mode (the smooth intents unless the
//! font's `prep` turns it off) the interpreter ignores horizontal moves, so only vertical
//! deltas are suggested there.

use std::collections::BTreeMap;

use fontcraft_compile::{Flavour, HintingOptions, Options};
use fontcraft_model::{Font, Target as BuildTarget};
use serde_json::{Map, Value, json};
use skrifa::raw::FontRef;

use super::compile::{self, Container, Dest, Job, Pick, Report, Request, Sink};
use super::*;
use crate::EngineError;
use crate::params::Params;
use crate::ttvm::debug::{GlyphRun, Outcome, SizeContext, StepInfo, VmPoint};
use crate::ttvm::{Program, SmoothMode, Target};

/// Pixel sizes accepted.
pub const MIN_PPEM: u16 = 1;
pub const MAX_PPEM: u16 = 400;
/// Most sizes in one delta suggestion request.
pub const MAX_SUGGEST_SIZES: usize = 100;
/// Most glyphs in one delta suggestion request.
pub const MAX_SUGGEST_GLYPHS: usize = 2_000;
/// Most glyph runs (glyphs times sizes) in one delta suggestion request.
pub const MAX_SUGGEST_RUNS: usize = 20_000;
/// Instructions all the runs of one delta suggestion request may execute together; beyond it
/// the result stops early and says so.
pub const MAX_SUGGEST_INSTRUCTIONS: usize = 50_000_000;
/// Most suggestions returned.
pub const MAX_SUGGESTIONS: usize = 100_000;
/// Most breakpoints in one request.
pub const MAX_BREAKPOINTS: usize = 10_000;
/// Default and range of `within`, in pixels.
pub const DEFAULT_WITHIN: f64 = 0.1;
pub const MAX_WITHIN: f64 = 0.5;
/// Pieces each quadratic segment is cut into when measured against pixel centres.
const CURVE_PIECES: usize = 8;
/// Largest delta suggested, in pixels: what one DELTAP entry can move at the default shift.
const MAX_DELTA_PX: f64 = 1.0;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "hint.tt.debugStep",
            "Debugger Step",
            [],
            None,
            "{glyph: name, instance?: name (a static instance by name, `#n` for an unnamed one, or a master id; default: the first master), ppem: number (whole pixel size 1 to 400), intent?: 'grayscale'|'cleartype'|'directwrite' (default 'grayscale'), action?: 'start'|'into'|'over'|'out'|'continue'|'stop' (default 'start'), step?: number (the `step` of the previous result; default 0), breakpoints?: [number] (byte offsets of instructions in the glyph program; 'over', 'out' and 'continue' stop at them)} runs the glyph's TrueType program to the next stop: 'start' before the first instruction, 'into' after one instruction, 'over' at the next instruction at the same call depth or shallower, 'out' at the first one after the current function returns, 'continue' at the next breakpoint, each of them at the end of the program if nothing comes first; 'stop' ends the session and runs nothing. A step is one instruction of the glyph program or of a function it calls. Coordinates and control values are in pixels → {glyph, gid, ppem, intent, step, finished, error?, hinting, depth, next?: {program: 'glyph'|'font'|'controlValue', pc, instruction}, breakpoint?: pc, program: [{pc, instruction}], stack, graphicsState: {projectionVector, dualProjectionVector, freedomVector, rp0, rp1, rp2, zp0, zp1, zp2, loop, round, minimumDistance, controlValueCutIn, singleWidthCutIn, singleWidth, deltaBase, deltaShift, autoFlip, instructControl, backwardCompatibility}, contours, points: [{x, y, originalX, originalY, onCurve, touchedX, touchedY}], phantom: [same, 4 items], twilight: [same], storage, cvt} or {stopped: true}",
            has_doc,
            debug_step
        ),
        cmd!(
            query "hint.tt.suggestDeltas",
            "Suggest Deltas…",
            [],
            None,
            "{glyphs?: [names] (default: every exported glyph), instance?: name (as in hint.tt.debugStep), ppemRange: [number, number] (first and last whole pixel size, 1 to 400, at most 100 sizes), intent?: 'grayscale'|'cleartype'|'directwrite' (default 'grayscale'), within?: number (pixels, above 0 and at most 0.5; default 0.1)} runs each glyph's TrueType program at every size of the range and lists the points whose edge passes within `within` pixels of a pixel centre along one axis, with the delta that moves the edge at least `within` away from it; only vertical deltas where the interpreter ignores horizontal moves. Glyphs made of components are skipped → {instance, intent, within, suggestions: [{glyph, ppem, point, direction: 'x'|'y', offset (edge minus pixel centre, pixels), delta (pixels, a multiple of the delta step), pixel: [column, row]}], skipped: [{glyph, reason}], truncated?: string}",
            has_doc,
            suggest_deltas
        ),
    ]
}

/// One font built in memory.
struct Built {
    bytes: Vec<u8>,
    glyph_order: Vec<String>,
    label: String,
}

/// The instance or master `instance` names, or the first master (as the hinted preview reads it).
fn job(font: &Font, p: &Params) -> Result<Job> {
    let Some(name) = p.opt_str("instance")? else {
        let m = font.default_master()?;
        return Ok(Job { label: m.name.clone(), source: "default", target: BuildTarget::Master(m.id.clone()) });
    };
    if let Some(m) = font.masters.iter().find(|m| m.id == name) {
        return Ok(Job { label: m.name.clone(), source: "master", target: BuildTarget::Master(m.id.clone()) });
    }
    let jobs = compile::jobs(font, &Pick::Named(vec![name.to_string()]), false, p)?;
    jobs.into_iter().next().ok_or_else(|| p.bad(format!("the font has no instance or master `{name}`")))
}

fn build(s: &Session, p: &Params) -> Result<Built> {
    let font = s.font()?;
    let job = job(font, p)?;
    let options = Options { hinting: HintingOptions { autohint: false, ..Default::default() }, ..Default::default() };
    let req = Request {
        flavours: vec![Flavour::TrueType],
        containers: vec![Container::Plain],
        options,
        dest: Dest::Memory,
        first_only: false,
        autohinter: crate::external_autohint::trusted(s),
    };
    let mut sink = Sink::new(&req.dest, true, s.export_outside())?;
    let mut report = Report::default();
    let doc = s.doc()?;
    compile::build(font, &doc.title, doc.path.as_deref(), s.glyph_data()?, std::slice::from_ref(&job), &req, &mut sink, &mut report)?;
    if report.fonts.is_empty() {
        return Err(compile::nothing_built(&report));
    }
    let Some(out) = report.fonts.into_iter().next() else { return Err(EngineError::Internal("the font was not built".into())) };
    let bytes = out.files.into_iter().next().and_then(|f| f.data).ok_or_else(|| EngineError::Internal("the font was not kept in memory".into()))?;
    Ok(Built { bytes, glyph_order: out.glyph_order, label: job.label })
}

fn font_ref<'a>(built: &'a Built) -> Result<FontRef<'a>> {
    FontRef::new(&built.bytes).map_err(|e| EngineError::Internal(format!("the built font cannot be read: {e}")))
}

/// The intent's name and the interpreter target that imitates it (as `fontcraft_render::hinted`
/// maps its intents).
fn intent(p: &Params) -> Result<(&'static str, Target)> {
    let smooth = |mode, symmetric_rendering| Target::Smooth { mode, symmetric_rendering, preserve_linear_metrics: false };
    match p.opt_str("intent")? {
        None | Some("grayscale") => Ok(("grayscale", smooth(SmoothMode::Normal, true))),
        Some("cleartype") => Ok(("cleartype", smooth(SmoothMode::Lcd, false))),
        Some("directwrite") => Ok(("directwrite", smooth(SmoothMode::Lcd, true))),
        Some(other) => Err(p.bad(format!("`intent` must be 'grayscale', 'cleartype' or 'directwrite', got '{other}'"))),
    }
}

fn ppem_value(p: &Params, v: &Value, key: &str) -> Result<u16> {
    let n = v.as_f64().filter(|n| n.is_finite() && n.fract() == 0.0);
    n.filter(|n| (f64::from(MIN_PPEM)..=f64::from(MAX_PPEM)).contains(n))
        .map(|n| n as u16)
        .ok_or_else(|| p.bad(format!("`{key}`: {v} is not a whole pixel size from {MIN_PPEM} to {MAX_PPEM}")))
}

fn program_name(program: Program) -> &'static str {
    match program {
        Program::Font => "font",
        Program::ControlValue => "controlValue",
        Program::Glyph => "glyph",
    }
}

fn px(bits: i32) -> f64 {
    f64::from(bits) / 64.0
}

fn f2dot14(v: (i32, i32)) -> Value {
    json!([f64::from(v.0) / 16384.0, f64::from(v.1) / 16384.0])
}

fn point_json(p: &VmPoint) -> Value {
    json!({
        "x": px(p.x), "y": px(p.y), "originalX": px(p.original_x), "originalY": px(p.original_y),
        "onCurve": p.on_curve, "touchedX": p.touched_x, "touchedY": p.touched_y,
    })
}

/// What a debugger action stops at, given the step it starts from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Action {
    Start,
    Into,
    Over,
    Out,
    Continue,
}

/// The stop rule of `action` from step `from`. Returns whether to stop before the instruction
/// described by `info`, and keeps the call depth seen at `from`.
struct StopRule<'b> {
    action: Action,
    from: usize,
    breakpoints: &'b [usize],
    depth_at_from: Option<usize>,
    hit: Option<usize>,
}

impl StopRule<'_> {
    fn stop(&mut self, info: &StepInfo) -> bool {
        if info.step == self.from {
            self.depth_at_from = Some(info.depth);
        }
        let past = info.step > self.from;
        let at_breakpoint = past && info.program == Program::Glyph && self.breakpoints.contains(&info.pc);
        let stop = match self.action {
            Action::Start => true,
            Action::Into => past,
            Action::Continue => at_breakpoint,
            Action::Over => at_breakpoint || (past && self.depth_at_from.is_none_or(|d| info.depth <= d)),
            Action::Out => at_breakpoint || (past && self.depth_at_from.is_some_and(|d| info.depth < d)),
        };
        if stop && at_breakpoint {
            self.hit = Some(info.pc);
        }
        stop
    }
}

fn debug_step(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("hint.tt.debugStep", p)?;
    let action = match p.opt_str("action")? {
        None | Some("start") => Action::Start,
        Some("into") => Action::Into,
        Some("over") => Action::Over,
        Some("out") => Action::Out,
        Some("continue") => Action::Continue,
        Some("stop") => return Ok(json!({"stopped": true})),
        Some(other) => return Err(p.bad(format!("`action` must be 'start', 'into', 'over', 'out', 'continue' or 'stop', got '{other}'"))),
    };
    let name = p.str("glyph")?;
    if s.font()?.glyph(name).is_none() {
        return Err(p.bad(format!("there is no glyph `{name}`")));
    }
    let ppem = ppem_value(&p, p.get("ppem").ok_or_else(|| p.bad("`ppem` is required"))?, "ppem")?;
    let (intent_name, target) = intent(&p)?;
    let from = p.opt_int("step", 0, i64::from(u32::MAX))?.map(|n| n as usize).unwrap_or(0);
    let mut breakpoints = Vec::new();
    if let Some(list) = p.opt_array("breakpoints")? {
        if list.len() > MAX_BREAKPOINTS {
            return Err(p.bad(format!("`breakpoints` holds {} offsets; at most {MAX_BREAKPOINTS} are allowed", list.len())));
        }
        for v in list {
            let pc = v.as_u64().filter(|n| *n <= u64::from(u32::MAX)).ok_or_else(|| p.bad(format!("`breakpoints`: {v} is not a byte offset")))?;
            breakpoints.push(pc as usize);
        }
    }
    let built = build(s, &p)?;
    let gid = built.glyph_order.iter().position(|g| g == name).ok_or_else(|| p.bad(format!("glyph `{name}` is not exported")))?;
    let font = font_ref(&built)?;
    let ctx = SizeContext::new(&font, ppem, target).map_err(|e| p.bad(e))?;
    let mut rule = StopRule { action, from, breakpoints: &breakpoints, depth_at_from: None, hit: None };
    let run = ctx.run_glyph(gid as u32, &mut |info: &StepInfo| rule.stop(info)).map_err(|e| p.bad(e))?;
    let mut out = state_json(&run);
    out.insert("glyph".into(), json!(name));
    out.insert("gid".into(), json!(gid));
    out.insert("ppem".into(), json!(ppem));
    out.insert("intent".into(), json!(intent_name));
    out.insert("instance".into(), json!(built.label));
    if let Some(pc) = rule.hit {
        out.insert("breakpoint".into(), json!(pc));
    }
    if let Some(e) = &ctx.setup_error {
        out.insert("setupError".into(), json!(e));
    }
    Ok(Value::Object(out))
}

/// The JSON of a stopped run (the fields of `hint.tt.debugStep`'s result about the state).
fn state_json(run: &GlyphRun) -> Map<String, Value> {
    let snap = &run.snapshot;
    let mut m = Map::new();
    m.insert("step".into(), json!(snap.steps));
    m.insert("finished".into(), json!(matches!(snap.outcome, Outcome::Finished)));
    if let Outcome::Failed(e) = &snap.outcome {
        m.insert("error".into(), json!(e.to_string()));
    }
    m.insert("hinting".into(), json!(run.hinting));
    m.insert("depth".into(), json!(snap.depth));
    if let Some((program, pc, text)) = &snap.next {
        m.insert("next".into(), json!({"program": program_name(*program), "pc": pc, "instruction": text}));
    }
    m.insert("program".into(), Value::Array(run.listing.iter().map(|(pc, text)| json!({"pc": pc, "instruction": text})).collect()));
    m.insert("stack".into(), json!(snap.stack));
    m.insert(
        "graphicsState".into(),
        json!({
            "projectionVector": f2dot14(snap.projection),
            "dualProjectionVector": f2dot14(snap.dual_projection),
            "freedomVector": f2dot14(snap.freedom),
            "rp0": snap.rp[0], "rp1": snap.rp[1], "rp2": snap.rp[2],
            "zp0": snap.zp[0], "zp1": snap.zp[1], "zp2": snap.zp[2],
            "loop": snap.loop_counter,
            "round": snap.round,
            "minimumDistance": px(snap.min_distance),
            "controlValueCutIn": px(snap.control_value_cutin),
            "singleWidthCutIn": px(snap.single_width_cutin),
            "singleWidth": px(snap.single_width),
            "deltaBase": snap.delta_base,
            "deltaShift": snap.delta_shift,
            "autoFlip": snap.auto_flip,
            "instructControl": snap.instruct_control,
            "backwardCompatibility": snap.backward_compatibility,
        }),
    );
    m.insert("contours".into(), json!(run.contours));
    let (outline, phantom) = snap.glyph.split_at(run.point_count.min(snap.glyph.len()));
    m.insert("points".into(), Value::Array(outline.iter().map(point_json).collect()));
    m.insert("phantom".into(), Value::Array(phantom.iter().map(point_json).collect()));
    m.insert("twilight".into(), Value::Array(snap.twilight.iter().map(point_json).collect()));
    m.insert("storage".into(), json!(snap.storage));
    m.insert("cvt".into(), Value::Array(snap.cvt.iter().map(|v| json!(px(*v))).collect()));
    m
}

// ── Delta suggestions ───────────────────────────────────────────────────────────────────────────

/// One suggestion: the closest approach of an edge to a pixel centre near one point.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Suggestion {
    pub point: usize,
    /// 0 for x (a horizontal delta), 1 for y.
    pub axis: u8,
    /// Edge position minus the pixel centre, in pixels.
    pub offset: f64,
    /// The suggested move, in pixels.
    pub delta: f64,
    pub pixel: (i64, i64),
}

/// A point of the hinted outline in pixels, with its index in the glyph, or `None` for an
/// on-curve point implied between two off-curve ones.
#[derive(Clone, Copy, Debug)]
struct OutlinePoint {
    x: f64,
    y: f64,
    on: bool,
    index: usize,
}

/// The pieces of one contour as straight lines, each with the glyph points that define the
/// segment it comes from.
fn contour_pieces(points: &[OutlinePoint], out: &mut Vec<([f64; 4], Vec<usize>)>) {
    let n = points.len();
    if n < 2 {
        return;
    }
    // Start at an on-curve point; with none, at the midpoint of the first two.
    let start = points.iter().position(|p| p.on);
    let at = |i: usize| points.get(i % n).copied();
    let mid = |a: OutlinePoint, b: OutlinePoint| ((a.x + b.x) / 2.0, (a.y + b.y) / 2.0);
    let (mut cur, first_i) = match start {
        Some(i) => match at(i) {
            Some(p) => ((p.x, p.y), i),
            None => return,
        },
        None => match (at(0), at(1)) {
            (Some(a), Some(b)) => (mid(a, b), 0),
            _ => return,
        },
    };
    let mut prev_on_index = if start.is_some() { Some(first_i) } else { None };
    let mut control: Option<OutlinePoint> = None;
    for k in 1..=n {
        let Some(p) = at(first_i + k) else { continue };
        match (p.on, control) {
            (true, None) => {
                let mut owners: Vec<usize> = prev_on_index.into_iter().collect();
                owners.push(p.index);
                out.push(([cur.0, cur.1, p.x, p.y], owners));
                cur = (p.x, p.y);
                prev_on_index = Some(p.index);
            }
            (true, Some(c)) => {
                let mut owners: Vec<usize> = prev_on_index.into_iter().collect();
                owners.extend([c.index, p.index]);
                quad_pieces(cur, (c.x, c.y), (p.x, p.y), &owners, out);
                cur = (p.x, p.y);
                prev_on_index = Some(p.index);
                control = None;
            }
            (false, None) => control = Some(p),
            (false, Some(c)) => {
                let m = mid(c, p);
                let mut owners: Vec<usize> = prev_on_index.into_iter().collect();
                owners.push(c.index);
                quad_pieces(cur, (c.x, c.y), m, &owners, out);
                cur = m;
                prev_on_index = None;
                control = Some(p);
            }
        }
    }
    // Close back to the start when the contour began between two off-curve points.
    if let (Some(c), None) = (control, start)
        && let (Some(a), Some(b)) = (at(0), at(1))
    {
        quad_pieces(cur, (c.x, c.y), mid(a, b), &[c.index], out);
    }
}

fn quad_pieces(a: (f64, f64), c: (f64, f64), b: (f64, f64), owners: &[usize], out: &mut Vec<([f64; 4], Vec<usize>)>) {
    let at = |t: f64| {
        let u = 1.0 - t;
        (u * u * a.0 + 2.0 * u * t * c.0 + t * t * b.0, u * u * a.1 + 2.0 * u * t * c.1 + t * t * b.1)
    };
    let mut prev = a;
    for k in 1..=CURVE_PIECES {
        let next = at(k as f64 / CURVE_PIECES as f64);
        out.push(([prev.0, prev.1, next.0, next.1], owners.to_vec()));
        prev = next;
    }
}

/// The suggestions for one glyph run: every crossing of a pixel row or column centre line by
/// the outline whose distance from the nearest pixel centre is under `within`.
fn suggestions_for(run: &GlyphRun, within: f64) -> Vec<Suggestion> {
    let snap = &run.snapshot;
    let n = run.point_count.min(snap.glyph.len());
    // The rasteriser draws the glyph with its left phantom point at the origin.
    let shift = snap.glyph.get(n).map(|p0| if snap.backward_compatibility { p0.original_x } else { p0.x }).map(px).unwrap_or(0.0);
    let pts: Vec<OutlinePoint> =
        snap.glyph.iter().take(n).enumerate().map(|(i, p)| OutlinePoint { x: px(p.x) - shift, y: px(p.y), on: p.on_curve, index: i }).collect();
    let mut pieces = Vec::new();
    let mut first = 0usize;
    for &end in &run.contours {
        let end = usize::from(end);
        if end < first || end >= pts.len() {
            break;
        }
        if let Some(contour) = pts.get(first..=end) {
            contour_pieces(contour, &mut pieces);
        }
        first = end + 1;
    }
    let step = 1.0 / f64::from(1u32 << u32::from(snap.delta_shift.min(6)));
    let axes: &[u8] = if snap.backward_compatibility { &[1] } else { &[0, 1] };
    let mut best: BTreeMap<(usize, u8), Suggestion> = BTreeMap::new();
    for (seg, owners) in &pieces {
        let [ax, ay, bx, by] = *seg;
        for &axis in axes {
            // For y deltas, walk the column centres the piece spans and measure its y there.
            let (a_run, a_val, b_run, b_val) = if axis == 1 { (ax, ay, bx, by) } else { (ay, ax, by, bx) };
            let (lo, hi) = if a_run <= b_run { (a_run, b_run) } else { (b_run, a_run) };
            if !(lo.is_finite() && hi.is_finite()) || hi - lo <= 0.0 {
                continue;
            }
            let mut centre = (lo - 0.5).ceil() + 0.5;
            while centre < hi {
                let t = (centre - a_run) / (b_run - a_run);
                let value = a_val + t * (b_val - a_val);
                let nearest = (value - 0.5).round() + 0.5;
                let offset = value - nearest;
                if offset.abs() < within {
                    let owner = owners.iter().copied().min_by(|i, j| {
                        let d =
                            |k: usize| pts.get(k).map(|q| if axis == 1 { (q.x - centre).abs() } else { (q.y - centre).abs() }).unwrap_or(f64::MAX);
                        d(*i).total_cmp(&d(*j)).then(i.cmp(j))
                    });
                    if let Some(point) = owner {
                        let need = within - offset.abs();
                        let steps = (need / step).ceil().max(1.0);
                        let size = (steps * step).min(MAX_DELTA_PX);
                        let delta = if offset < 0.0 { -size } else { size };
                        let pixel =
                            if axis == 1 { ((centre - 0.5) as i64, (nearest - 0.5) as i64) } else { ((nearest - 0.5) as i64, (centre - 0.5) as i64) };
                        let s = Suggestion { point, axis, offset, delta, pixel };
                        best.entry((point, axis))
                            .and_modify(|b| {
                                if offset.abs() < b.offset.abs() {
                                    *b = s
                                }
                            })
                            .or_insert(s);
                    }
                }
                centre += 1.0;
            }
        }
    }
    best.into_values().collect()
}

fn suggest_deltas(s: &mut Session, p: &Value) -> Result<Value> {
    let p = Params::new("hint.tt.suggestDeltas", p)?;
    let range = p.opt_array("ppemRange")?.ok_or_else(|| p.bad("`ppemRange` is required"))?;
    let (Some(first), Some(last), 2) = (range.first(), range.get(1), range.len()) else {
        return Err(p.bad("`ppemRange` must hold two sizes: the first and the last"));
    };
    let (first, last) = (ppem_value(&p, first, "ppemRange")?, ppem_value(&p, last, "ppemRange")?);
    if first > last {
        return Err(p.bad(format!("`ppemRange`: the first size ({first}) is larger than the last ({last})")));
    }
    let sizes: Vec<u16> = (first..=last).collect();
    if sizes.len() > MAX_SUGGEST_SIZES {
        return Err(p.bad(format!("`ppemRange` spans {} sizes; at most {MAX_SUGGEST_SIZES} are allowed", sizes.len())));
    }
    let within = p.opt_f64("within")?.unwrap_or(DEFAULT_WITHIN);
    if !(within > 0.0 && within <= MAX_WITHIN) {
        return Err(p.bad(format!("`within` must be above 0 and at most {MAX_WITHIN} pixels, got {within}")));
    }
    let (intent_name, target) = intent(&p)?;
    let names = p.opt_glyphs("glyphs", s.font()?)?;
    let built = build(s, &p)?;
    let glyphs: Vec<(String, usize)> = match names {
        Some(names) => names
            .into_iter()
            .map(|name| match built.glyph_order.iter().position(|g| *g == name) {
                Some(gid) => Ok((name, gid)),
                None => Err(p.bad(format!("`glyphs`: glyph `{name}` is not exported"))),
            })
            .collect::<Result<_>>()?,
        None => built.glyph_order.iter().cloned().enumerate().map(|(gid, name)| (name, gid)).collect(),
    };
    if glyphs.len() > MAX_SUGGEST_GLYPHS {
        return Err(
            p.bad(format!("{} glyphs were asked for; at most {MAX_SUGGEST_GLYPHS} can be checked at once (name them in `glyphs`)", glyphs.len()))
        );
    }
    if glyphs.len().saturating_mul(sizes.len()) > MAX_SUGGEST_RUNS {
        return Err(p.bad(format!(
            "{} glyphs at {} sizes is more than {MAX_SUGGEST_RUNS} runs; narrow `glyphs` or `ppemRange`",
            glyphs.len(),
            sizes.len()
        )));
    }
    let font = font_ref(&built)?;
    let mut suggestions = Vec::new();
    let mut skipped: BTreeMap<String, String> = BTreeMap::new();
    let mut executed = 0usize;
    let mut truncated: Option<String> = None;
    'sizes: for &ppem in &sizes {
        let ctx = SizeContext::new(&font, ppem, target).map_err(|e| p.bad(e))?;
        for (name, gid) in &glyphs {
            if skipped.contains_key(name) {
                continue;
            }
            if executed >= MAX_SUGGEST_INSTRUCTIONS || suggestions.len() >= MAX_SUGGESTIONS {
                truncated = Some(format!("stopped at {ppem} PPM: the request reached its limit of work; narrow `glyphs` or `ppemRange`"));
                break 'sizes;
            }
            let run = match ctx.run_glyph(*gid as u32, &mut |_: &StepInfo| false) {
                Ok(run) => run,
                Err(why) => {
                    skipped.insert(name.clone(), why);
                    continue;
                }
            };
            executed = executed.saturating_add(run.snapshot.steps);
            if let Outcome::Failed(e) = &run.snapshot.outcome {
                skipped.insert(name.clone(), format!("its program fails at {ppem} PPM ({e})"));
                continue;
            }
            for sug in suggestions_for(&run, within) {
                suggestions.push(json!({
                    "glyph": name, "ppem": ppem, "point": sug.point,
                    "direction": if sug.axis == 0 { "x" } else { "y" },
                    "offset": sug.offset, "delta": sug.delta, "pixel": [sug.pixel.0, sug.pixel.1],
                }));
            }
        }
    }
    let mut out = Map::new();
    out.insert("instance".into(), json!(built.label));
    out.insert("intent".into(), json!(intent_name));
    out.insert("within".into(), json!(within));
    out.insert("suggestions".into(), Value::Array(suggestions));
    out.insert("skipped".into(), Value::Array(skipped.into_iter().map(|(glyph, reason)| json!({"glyph": glyph, "reason": reason})).collect()));
    if let Some(t) = truncated {
        out.insert("truncated".into(), json!(t));
    }
    Ok(Value::Object(out))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;

    fn rule(action: Action, from: usize, bps: &[usize]) -> StopRule<'_> {
        StopRule { action, from, breakpoints: bps, depth_at_from: None, hit: None }
    }

    fn info(step: usize, pc: usize, depth: usize) -> StepInfo {
        StepInfo { step, program: if depth == 0 { Program::Glyph } else { Program::Font }, pc, depth }
    }

    #[test]
    fn stop_rules_follow_call_depth_and_breakpoints() {
        // Steps 0..5: glyph pc 0, call into a function (depth 1) for steps 1..3, back at step 4.
        let trace = [info(0, 0, 0), info(1, 10, 1), info(2, 11, 1), info(3, 12, 1), info(4, 2, 0), info(5, 4, 0)];
        let first_stop = |mut r: StopRule<'_>| trace.iter().find(|i| r.stop(i)).map(|i| i.step);
        assert_eq!(first_stop(rule(Action::Start, 0, &[])), Some(0));
        assert_eq!(first_stop(rule(Action::Into, 0, &[])), Some(1));
        assert_eq!(first_stop(rule(Action::Over, 0, &[])), Some(4), "over a call");
        assert_eq!(first_stop(rule(Action::Out, 2, &[])), Some(4), "out of the function");
        assert_eq!(first_stop(rule(Action::Out, 0, &[])), None, "out at depth 0 runs to the end");
        assert_eq!(first_stop(rule(Action::Continue, 0, &[4])), Some(5), "a glyph breakpoint");
        assert_eq!(first_stop(rule(Action::Continue, 0, &[11])), None, "breakpoints are glyph offsets");
        assert_eq!(first_stop(rule(Action::Over, 0, &[2])), Some(4));
    }

    fn run_with(points: Vec<VmPoint>, contours: Vec<u16>, back_compat: bool) -> GlyphRun {
        let n = points.len();
        let mut glyph = points;
        glyph.extend([VmPoint::default(); 4]);
        let snapshot = crate::ttvm::debug::Snapshot {
            outcome: Outcome::Finished,
            steps: 0,
            next: None,
            depth: 0,
            stack: vec![],
            projection: (0, 16384),
            dual_projection: (0, 16384),
            freedom: (0, 16384),
            rp: [0; 3],
            zp: [1; 3],
            loop_counter: 1,
            round: "grid",
            min_distance: 64,
            control_value_cutin: 68,
            single_width_cutin: 0,
            single_width: 0,
            delta_base: 9,
            delta_shift: 3,
            auto_flip: true,
            instruct_control: 0,
            backward_compatibility: back_compat,
            glyph,
            twilight: vec![],
            storage: vec![],
            cvt: vec![],
        };
        GlyphRun { snapshot, point_count: n, contours, listing: vec![], hinting: true }
    }

    fn on(x: f64, y: f64) -> VmPoint {
        let b = |v: f64| (v * 64.0) as i32;
        VmPoint { x: b(x), y: b(y), original_x: b(x), original_y: b(y), on_curve: true, touched_x: false, touched_y: false }
    }

    #[test]
    fn an_edge_through_pixel_centres_gets_a_delta_away_from_them() {
        // A box from (1, 1.5) to (2, 6.5) px: its horizontal edges run through pixel centres.
        let run = run_with(vec![on(1.0, 1.5), on(2.0, 1.5), on(2.0, 6.5), on(1.0, 6.5)], vec![3], true);
        let found = suggestions_for(&run, 0.1);
        assert!(found.iter().all(|s| s.axis == 1), "backward compatibility: vertical deltas only");
        let bottom: Vec<_> = found.iter().filter(|s| s.point < 2).collect();
        let top: Vec<_> = found.iter().filter(|s| s.point >= 2).collect();
        assert_eq!((bottom.len(), top.len()), (1, 1), "{found:?}");
        assert_eq!(bottom[0].offset, 0.0);
        assert_eq!(bottom[0].delta, 0.125, "one delta step of 1/8 pixel moves it 0.1 px away");
        assert_eq!(bottom[0].pixel, (1, 1));

        // Off the centres by more than `within`: nothing to suggest.
        let run = run_with(vec![on(1.0, 1.8), on(2.0, 1.8), on(2.0, 6.8), on(1.0, 6.8)], vec![3], true);
        assert!(suggestions_for(&run, 0.1).is_empty());

        // Without backward compatibility the vertical edges at x = 1.45 count too.
        let run = run_with(vec![on(1.45, 1.0), on(3.0, 1.0), on(3.0, 3.0), on(1.45, 3.0)], vec![3], false);
        let found = suggestions_for(&run, 0.1);
        let x: Vec<_> = found.iter().filter(|s| s.axis == 0).collect();
        assert!(!x.is_empty() && x.iter().all(|s| s.point == 0 || s.point == 3), "{found:?}");
        assert!(x.iter().all(|s| s.delta == -0.125), "pushed left, away from the centre at 1.5");
    }
}
