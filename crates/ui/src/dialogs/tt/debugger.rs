//! The TrueType Debugger window (`window.ttDebugger`): one glyph's TrueType program run an instruction at a time at a
//! pixel size, with the interpreter's state after each step.
//!
//! | Control | Does |
//! |---|---|
//! | Glyph, PPEM, intent | what to debug; a change starts again at the first instruction |
//! | Start, Into, Over, Out, Continue, Stop | the actions of `hint.tt.debugStep` |
//! | instruction list | the glyph program; the next instruction is selected; clicking a line sets or clears a breakpoint there (●) |
//! | glyph pane | the pixel grid and the glyph's points as the program has moved them; hollow dots are where they started, touched points are filled |
//! | inspector | stack (top first), graphics state, points, twilight points, storage and control values |
//!
//! The app keeps only what to debug, the breakpoints and the last result ([`DebuggerState`]); each button posts
//! `window.ttDebugger` with its action, which runs `hint.tt.debugStep` from the step of that result, so the window,
//! the control channel and an agent see the same states.

use craft::ui::buttons::Button;
use craft::ui::contract::{Bind, Entry as Choice, Props as _, Show as _, ShowContainer as _};
use craft::ui::dialog::{Dialog, DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use craft::ui::display::{Picture, PictureSize, Shade, Text};
use craft::ui::layout_components::{Row, Stack};
use craft::ui::list::{ThumbAspect, ThumbSize};
use craft::ui::pickers::Select;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::Env;
use craft::ui::sections::Section;
use craft::ui::text::{NumberField, TextField};
use egui::{Pos2, Rect, Stroke, Ui};
use serde_json::{Map, Value, json};

use crate::app::{FontCraftApp, Msg};

/// The shell command that opens the window and runs its actions.
pub const ID: &str = "window.ttDebugger";
const DEFAULT_PPEM: u16 = 12;
/// Most breakpoints the window keeps (the command's own limit).
const MAX_BREAKPOINTS: usize = 10_000;
/// Most rows one inspector section shows; the rest are summarised.
const MAX_ROWS: usize = 2_000;
/// The rendering intents, as `hint.tt.debugStep` names them.
const INTENTS: [&str; 3] = ["grayscale", "cleartype", "directwrite"];

craft::ui::words! {
    /// The words of the TrueType Debugger.
    pub struct DebuggerWords {
        /// The window's title.
        title: "fontcraftTtDebugger.title" = "TrueType Debugger";
        /// The glyph field.
        glyph: "fontcraftTtDebugger.glyph" = "Glyph";
        /// The size field.
        ppem: "fontcraftTtDebugger.ppem" = "PPEM";
        /// The intent pop-up.
        intent: "fontcraftTtDebugger.intent" = "Rendering";
        /// The grayscale intent.
        grayscale: "fontcraftTtDebugger.grayscale" = "Grayscale";
        /// The ClearType intent.
        cleartype: "fontcraftTtDebugger.cleartype" = "GDI ClearType";
        /// The DirectWrite intent.
        directwrite: "fontcraftTtDebugger.directwrite" = "DirectWrite";
        /// The Start action.
        start: "fontcraftTtDebugger.start" = "Start";
        /// The Into action.
        into: "fontcraftTtDebugger.into" = "Into";
        /// The Over action.
        over: "fontcraftTtDebugger.over" = "Over";
        /// The Out action.
        out: "fontcraftTtDebugger.out" = "Out";
        /// The Continue action.
        go_on: "fontcraftTtDebugger.continue" = "Continue";
        /// The Stop action.
        stop: "fontcraftTtDebugger.stop" = "Stop";
        /// Shown before the first step.
        idle: "fontcraftTtDebugger.idle" = "Choose a glyph and press Start.";
        /// The program list's title.
        program: "fontcraftTtDebugger.program" = "Glyph program";
        /// An empty glyph program.
        no_program: "fontcraftTtDebugger.noProgram" = "The glyph has no instructions.";
        /// The tooltip of an instruction line.
        breakpoint_tip: "fontcraftTtDebugger.breakpointTip" = "Click to set or clear a breakpoint";
        /// The glyph pane's name.
        pane: "fontcraftTtDebugger.pane" = "Glyph pixels";
        /// The inspector sections.
        stack: "fontcraftTtDebugger.stack" = "Stack";
        graphics_state: "fontcraftTtDebugger.graphicsState" = "Graphics state";
        points: "fontcraftTtDebugger.points" = "Points";
        phantom: "fontcraftTtDebugger.phantom" = "Phantom points";
        twilight: "fontcraftTtDebugger.twilight" = "Twilight points";
        storage: "fontcraftTtDebugger.storage" = "Storage";
        cvt: "fontcraftTtDebugger.cvt" = "Control values";
        /// The summary of rows past the limit.
        more: template "fontcraftTtDebugger.more" = "… {count} more";
        /// Status lines.
        failed: template "fontcraftTtDebugger.failed" = "Step {step}: {error}";
        hinting_off: "fontcraftTtDebugger.hintingOff" = "The font's programs turn hinting off at this size; the glyph program does not run.";
        finished: template "fontcraftTtDebugger.finished" = "Finished after {step} steps";
        next: template "fontcraftTtDebugger.next" = "Step {step}, call depth {depth}: next {instruction} at {pc}{place}";
        in_program: template "fontcraftTtDebugger.inProgram" = " in the {program} program";
    }
}

/// What the window debugs and the last result; app state, changed only by `window.ttDebugger`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct DebuggerState {
    pub open: bool,
    pub glyph: Option<String>,
    /// 0 until set: the window then uses [`DEFAULT_PPEM`].
    pub ppem: u16,
    /// `grayscale`, `cleartype` or `directwrite`; empty for grayscale.
    pub intent: String,
    pub breakpoints: Vec<u64>,
    /// The last `hint.tt.debugStep` result.
    pub result: Option<Value>,
    pub error: Option<String>,
    /// Moves with every change, so the open window knows to copy the state again.
    pub generation: u64,
}

impl DebuggerState {
    pub fn ppem(&self) -> u16 {
        if self.ppem == 0 { DEFAULT_PPEM } else { self.ppem }
    }

    pub fn intent(&self) -> &str {
        if self.intent.is_empty() { "grayscale" } else { &self.intent }
    }

    fn running(&self) -> bool {
        self.result.as_ref().is_some_and(|r| r.get("finished") != Some(&Value::Bool(true)) && r.get("error").is_none())
    }
}

/// Runs `window.ttDebugger` with `params`: opens or closes the window, changes what it debugs and steps.
pub fn command(app: &mut FontCraftApp, params: &Value, current: Option<String>) -> Result<(), String> {
    let st = &mut app.tt.debugger;
    st.generation = st.generation.wrapping_add(1);
    if params.get("open").and_then(Value::as_bool) == Some(false) {
        st.open = false;
        return Ok(());
    }
    app.session.font().map_err(|e| e.to_string())?;
    let st = &mut app.tt.debugger;
    st.open = true;
    let mut restart = st.result.is_none();
    if let Some(glyph) = params.get("glyph").and_then(Value::as_str) {
        restart |= st.glyph.as_deref() != Some(glyph);
        st.glyph = Some(glyph.to_string());
    } else if st.glyph.is_none() {
        st.glyph = current;
    }
    if let Some(n) = params.get("ppem").and_then(Value::as_f64) {
        if !(n.fract() == 0.0 && (1.0..=400.0).contains(&n)) {
            return Err(format!("`ppem` must be a whole pixel size from 1 to 400, got {n}"));
        }
        restart |= f64::from(st.ppem()) != n;
        st.ppem = n as u16;
    }
    if let Some(intent) = params.get("intent").and_then(Value::as_str) {
        if !INTENTS.contains(&intent) {
            return Err(format!("`intent` must be 'grayscale', 'cleartype' or 'directwrite', got '{intent}'"));
        }
        restart |= st.intent() != intent;
        st.intent = intent.to_string();
    }
    if let Some(pc) = params.get("toggleBreakpoint").and_then(Value::as_u64) {
        if let Some(at) = st.breakpoints.iter().position(|b| *b == pc) {
            st.breakpoints.remove(at);
        } else if st.breakpoints.len() < MAX_BREAKPOINTS {
            st.breakpoints.push(pc);
            st.breakpoints.sort_unstable();
        }
    }
    let action = match params.get("action").and_then(Value::as_str) {
        Some("stop") => {
            st.result = None;
            st.error = None;
            return Ok(());
        }
        Some(a) => Some(a.to_string()),
        None if restart => Some("start".to_string()),
        None => None,
    };
    let (Some(action), Some(glyph)) = (action, st.glyph.clone()) else { return Ok(()) };
    let step = if action == "start" || restart { 0 } else { st.result.as_ref().and_then(|r| r.get("step")).and_then(Value::as_u64).unwrap_or(0) };
    let action = if restart { "start".to_string() } else { action };
    let params = json!({
        "glyph": glyph, "ppem": st.ppem(), "intent": st.intent(), "action": action, "step": step, "breakpoints": st.breakpoints,
    });
    let reply = app.session.query("hint.tt.debugStep", &params);
    let st = &mut app.tt.debugger;
    match reply {
        Ok(r) => {
            st.result = Some(r);
            st.error = None;
            Ok(())
        }
        Err(e) => {
            st.result = None;
            st.error = Some(e.to_string());
            Err(e.to_string())
        }
    }
}

/// The message a window control posts.
fn post(params: Value) -> Msg {
    Msg::Shell(CommandRef::with_params(ID, params))
}

/// The open window on the window's dialog host: a copy of [`DebuggerState`], refreshed by the dialog module when its
/// generation moves.
pub struct DebuggerDialog {
    pub state: DebuggerState,
    pub outbox: Vec<Msg>,
    /// Set from outside (`window.ttDebugger {open: false}`): the window closes in its next pass.
    pub closing: bool,
    open: bool,
}

impl DebuggerDialog {
    pub fn new(state: DebuggerState) -> Self {
        DebuggerDialog { state, outbox: Vec::new(), closing: false, open: true }
    }
}

/// The id of a control: an action (`start`, `into`, …), `glyph`, `ppem`, `intent`, or `pc<n>` for an instruction line.
pub fn control_id(part: &str) -> egui::Id {
    egui::Id::unique((ID, part))
}

/// Shows the window's body over `st`; what the person did goes to `sent`.
pub fn view(ui: &mut Ui, st: &DebuggerState, sent: &mut Vec<Msg>) {
    let words = Env::of(ui).words::<DebuggerWords>();
    Stack::new().show(ui, |ui, _| {
        Row::new().show(ui, |ui, _| {
            let glyph = TextField::new(words.glyph.clone(), Bind::given(st.glyph.clone().unwrap_or_default())).id(control_id("glyph")).show(ui);
            if let Some(name) = glyph.events.committed().map(|g| g.trim().to_string()).filter(|g| !g.is_empty()) {
                sent.push(post(json!({"glyph": name})));
            }
            let ppem = NumberField::new(words.ppem.clone(), Bind::given(Some(f64::from(st.ppem())))).id(control_id("ppem")).show(ui);
            if let Some(Some(n)) = ppem.edit.events.committed() {
                sent.push(post(json!({"ppem": n})));
            }
            let labels = [&words.grayscale, &words.cleartype, &words.directwrite];
            let choices: Vec<Choice<String>> = INTENTS.iter().zip(labels).map(|(key, label)| Choice::new(key.to_string(), label.clone())).collect();
            let intent = Select::new(words.intent.clone(), Bind::given(st.intent().to_string()), &choices).id(control_id("intent")).show(ui);
            if let Some(key) = intent.events.committed() {
                sent.push(post(json!({"intent": key})));
            }
        });
        Row::new().show(ui, |ui, _| {
            let running = st.running();
            let actions = [
                (&words.start, "start", false),
                (&words.into, "into", true),
                (&words.over, "over", true),
                (&words.out, "out", true),
                (&words.go_on, "continue", true),
                (&words.stop, "stop", false),
            ];
            for (label, action, needs_run) in actions {
                if Button::new(label.clone()).id(control_id(action)).enabled(!needs_run || running).show(ui).clicked() {
                    sent.push(post(json!({"action": action})));
                }
            }
        });
        if let Some(error) = &st.error {
            Text::new(error.clone()).show(ui);
        }
        let Some(r) = &st.result else {
            Text::new(words.idle.clone()).shade(Shade::Muted).show(ui);
            return;
        };
        Text::new(status_line(&words, r)).show(ui);
        Row::new().show(ui, |ui, _| {
            instruction_list(ui, &words, r, &st.breakpoints, sent);
            glyph_pane(ui, &words, r);
            inspector(ui, &words, r);
        });
    });
}

fn status_line(words: &DebuggerWords, r: &Value) -> String {
    let step = r.get("step").and_then(Value::as_u64).unwrap_or(0).to_string();
    if let Some(e) = r.get("error").and_then(Value::as_str) {
        return words.failed.fill(&[("step", step.as_str()), ("error", e)]).to_string();
    }
    if r.get("hinting") == Some(&Value::Bool(false)) {
        return words.hinting_off.as_str().to_string();
    }
    if r.get("finished") == Some(&Value::Bool(true)) {
        return words.finished.fill(&[("step", step.as_str())]).to_string();
    }
    let next = r.get("next").cloned().unwrap_or(Value::Null);
    let place = match next.get("program").and_then(Value::as_str) {
        Some("glyph") | None => String::new(),
        Some(p) => words.in_program.fill(&[("program", p)]).to_string(),
    };
    let depth = r.get("depth").map(Value::to_string).unwrap_or_default();
    let pc = next.get("pc").map(Value::to_string).unwrap_or_default();
    let instruction = next.get("instruction").and_then(Value::as_str).unwrap_or_default();
    words
        .next
        .fill(&[("step", step.as_str()), ("depth", depth.as_str()), ("instruction", instruction), ("pc", pc.as_str()), ("place", place.as_str())])
        .to_string()
}

fn instruction_list(ui: &mut Ui, words: &DebuggerWords, r: &Value, breakpoints: &[u64], sent: &mut Vec<Msg>) {
    let next = r.get("next").filter(|n| n.get("program").and_then(Value::as_str) == Some("glyph"));
    let current = next.and_then(|n| n.get("pc")).and_then(Value::as_u64);
    Section::new(words.program.clone()).show(ui, |ui, _| {
        Stack::new().scroll(craft::ui::contract::Scroll::vertical()).show(ui, |ui, _| {
            let list = r.get("program").and_then(Value::as_array).cloned().unwrap_or_default();
            if list.is_empty() {
                Text::new(words.no_program.clone()).shade(Shade::Muted).show(ui);
            }
            for ins in &list {
                let Some(pc) = ins.get("pc").and_then(Value::as_u64) else { continue };
                let mark = if breakpoints.contains(&pc) { "●" } else { " " };
                let text = format!("{mark} {pc:>5}  {}", ins.get("instruction").and_then(Value::as_str).unwrap_or(""));
                let tip = match ins.get("gloss").and_then(Value::as_str).filter(|g| !g.is_empty()) {
                    Some(gloss) => format!("{gloss}\n{}", words.breakpoint_tip.as_str()),
                    None => words.breakpoint_tip.as_str().to_string(),
                };
                let line = Button::new(text).id(control_id(&format!("pc{pc}"))).tooltip(tip).selected(current == Some(pc));
                if line.show(ui).clicked() {
                    sent.push(post(json!({"toggleBreakpoint": pc})));
                }
            }
        });
    });
}

/// A point of the result in pixels: (x, y, original x, original y, touched).
fn point_of(p: &Value) -> Option<(f32, f32, f32, f32, bool)> {
    let f = |k: &str| p.get(k).and_then(Value::as_f64).filter(|v| v.is_finite()).map(|v| v as f32);
    let flag = |k: &str| p.get(k) == Some(&Value::Bool(true));
    Some((f("x")?, f("y")?, f("originalX")?, f("originalY")?, flag("touchedX") || flag("touchedY")))
}

fn glyph_pane(ui: &mut Ui, words: &DebuggerWords, r: &Value) {
    let pts: Vec<_> = r.get("points").and_then(Value::as_array).map(|a| a.iter().filter_map(point_of).collect()).unwrap_or_default();
    let contours: Vec<usize> =
        r.get("contours").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_u64).map(|e| e as usize).collect()).unwrap_or_default();
    Picture::painter(words.pane.clone(), ThumbAspect::SQUARE, move |cx, rect| paint_pane(cx.painter.clone(), cx.ink(), rect, &pts, &contours))
        .extent(PictureSize::Thumb(ThumbSize::Large))
        .show(ui);
}

/// Paints the pixel grid, the contours as the program moved them, where each point started (hollow) and each point
/// (filled when touched) in `rect`.
fn paint_pane(painter: egui::Painter, ink: egui::Color32, rect: Rect, pts: &[(f32, f32, f32, f32, bool)], contours: &[usize]) {
    if pts.is_empty() {
        return;
    }
    let (mut lo_x, mut lo_y, mut hi_x, mut hi_y) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for p in pts {
        for (x, y) in [(p.0, p.1), (p.2, p.3)] {
            (lo_x, lo_y, hi_x, hi_y) = (lo_x.min(x), lo_y.min(y), hi_x.max(x), hi_y.max(y));
        }
    }
    let (lo_x, lo_y) = (lo_x.floor() - 1.0, lo_y.floor() - 1.0);
    let span = ((hi_x.ceil() + 1.0) - lo_x).max((hi_y.ceil() + 1.0) - lo_y).max(1.0);
    let scale = rect.width().min(rect.height()) / span;
    let to_screen = |x: f32, y: f32| Pos2::new(rect.left() + (x - lo_x) * scale, rect.bottom() - (y - lo_y) * scale);
    let hairline = 1.0 / painter.pixels_per_point().max(0.01);
    let faint = ink.gamma_multiply(0.25);
    // A grid line per pixel, while pixels are large enough to tell apart.
    if scale >= 4.0 {
        let lines = span as usize;
        for i in 0..=lines {
            let v = i as f32;
            painter.line_segment([to_screen(lo_x + v, lo_y), to_screen(lo_x + v, lo_y + span)], Stroke::new(hairline, faint));
            painter.line_segment([to_screen(lo_x, lo_y + v), to_screen(lo_x + span, lo_y + v)], Stroke::new(hairline, faint));
        }
    }
    let mut first = 0usize;
    for end in contours {
        let Some(contour) = pts.get(first..=*end) else { break };
        let line: Vec<Pos2> = contour.iter().map(|p| to_screen(p.0, p.1)).collect();
        painter.add(egui::Shape::closed_line(line, Stroke::new(hairline * 2.0, ink)));
        first = end.saturating_add(1);
    }
    let radius = (scale * 0.15).clamp(hairline * 2.0, hairline * 6.0);
    for p in pts {
        painter.circle_stroke(to_screen(p.2, p.3), radius, Stroke::new(hairline, faint));
        if p.4 {
            painter.circle_filled(to_screen(p.0, p.1), radius, ink);
        } else {
            painter.circle_stroke(to_screen(p.0, p.1), radius, Stroke::new(hairline, ink));
        }
    }
}

fn inspector(ui: &mut Ui, words: &DebuggerWords, r: &Value) {
    let numbers = |key: &str| -> Vec<String> {
        r.get(key).and_then(Value::as_array).map(|a| a.iter().enumerate().map(|(i, v)| format!("{i:>5}  {v}")).collect()).unwrap_or_default()
    };
    let mut stack = numbers("stack");
    stack.reverse();
    let gs: Vec<String> =
        r.get("graphicsState").and_then(Value::as_object).map(Map::iter).into_iter().flatten().map(|(k, v)| format!("{k}: {v}")).collect();
    Stack::new().scroll(craft::ui::contract::Scroll::vertical()).show(ui, |ui, _| {
        rows(ui, words, &words.stack, &stack);
        rows(ui, words, &words.graphics_state, &gs);
        rows(ui, words, &words.points, &point_rows(r, "points"));
        rows(ui, words, &words.phantom, &point_rows(r, "phantom"));
        rows(ui, words, &words.twilight, &point_rows(r, "twilight"));
        rows(ui, words, &words.storage, &numbers("storage"));
        rows(ui, words, &words.cvt, &numbers("cvt"));
    });
}

fn rows(ui: &mut Ui, words: &DebuggerWords, title: &craft::ui::runtime::env::Label, items: &[String]) {
    Section::new(format!("{} ({})", title.as_str(), items.len())).show(ui, |ui, _| {
        for line in items.iter().take(MAX_ROWS) {
            Text::new(line.clone()).show(ui);
        }
        if items.len() > MAX_ROWS {
            let more = items.len().saturating_sub(MAX_ROWS).to_string();
            Text::new(words.more.fill(&[("count", more.as_str())])).shade(Shade::Muted).show(ui);
        }
    });
}

fn point_rows(r: &Value, key: &str) -> Vec<String> {
    let flag = |p: &Value, k: &str| p.get(k) == Some(&Value::Bool(true));
    let number = |p: &Value, k: &str| p.get(k).and_then(Value::as_f64).unwrap_or(0.0);
    r.get(key)
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .enumerate()
                .map(|(i, p)| {
                    let touched = match (flag(p, "touchedX"), flag(p, "touchedY")) {
                        (true, true) => "xy",
                        (true, false) => "x",
                        (false, true) => "y",
                        (false, false) => "",
                    };
                    format!("{i:>4}  {:>9.3} {:>9.3}  {touched}", number(p, "x"), number(p, "y"))
                })
                .collect()
        })
        .unwrap_or_default()
}

impl StackDialog for DebuggerDialog {
    type Result = ();

    fn id(&self) -> DialogId {
        dialog_id("fontcraftTtDebugger")
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<()>> {
        if self.closing {
            return Some(DialogResult::Cancel);
        }
        let words = Env::of(ui).words::<DebuggerWords>();
        let DebuggerDialog { state, outbox, open, .. } = self;
        let shown = Dialog::new(words.title.clone()).opened(Bind::bound(open)).modeless().resizable(true).show(ui, |ui, _| view(ui, state, outbox));
        shown.output.closed.map(|_| DialogResult::Ok(()))
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use craft::ui::testing::Fixture;

    use super::*;

    /// The step buttons wait for a running program; Start and a click on an instruction line post their actions.
    #[test]
    fn buttons_and_lines_post_window_tt_debugger() {
        let st = DebuggerState {
            open: true,
            glyph: Some("I".into()),
            result: Some(json!({"step": 1, "depth": 0, "next": {"program": "glyph", "pc": 2, "instruction": "MDAP[1]"},
                "program": [{"pc": 0, "instruction": "PUSHB[ ] 0"}, {"pc": 2, "instruction": "MDAP[1]"}], "points": []})),
            ..DebuggerState::default()
        };
        let posted = Rc::new(RefCell::new(Vec::new()));
        let seen = Rc::clone(&posted);
        let mut f = Fixture::new(move |ui| {
            let mut sent = Vec::new();
            view(ui, &st, &mut sent);
            seen.borrow_mut().extend(sent);
        });
        f.settle();
        for part in ["into", "pc2"] {
            let target = f.node_of(control_id(part)).expect(part);
            f.click(&target).settle();
        }
        assert_eq!(*posted.borrow(), vec![post(json!({"action": "into"})), post(json!({"toggleBreakpoint": 2}))]);
    }
}
