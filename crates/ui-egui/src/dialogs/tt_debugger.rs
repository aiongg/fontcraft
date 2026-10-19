//! The TrueType Debugger window (`window.ttDebugger`): one glyph's TrueType program run an
//! instruction at a time at a pixel size, with the interpreter's state after each step.
//!
//! | Control | Does |
//! |---|---|
//! | Glyph, PPEM, intent | what to debug; a change starts again at the first instruction |
//! | Start, Into, Over, Out, Continue, Stop | the actions of `hint.tt.debugStep` |
//! | instruction list | the glyph program; the next instruction is highlighted; clicking a line sets or clears a breakpoint there (●) |
//! | glyph pane | the pixel grid and the glyph's points as the program has moved them; hollow dots are where they started, touched points are drawn in the accent colour |
//! | inspector | stack (top first), graphics state, points, twilight points, storage and control values |
//!
//! The window keeps only what to debug, the breakpoints and the last result; each button runs
//! `hint.tt.debugStep` with the step of that result, so the window, the control channel and an
//! agent see the same states.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::app::FontcraftApp;
use crate::panels::Pending;
use crate::shell::{Args, UiCommand};

const ID: &str = "window.ttDebugger";
const DEFAULT_PPEM: u16 = 12;
/// Most breakpoints the window keeps (the command's own limit).
const MAX_BREAKPOINTS: usize = 10_000;
/// Side of the glyph pane, in points.
const PANE: f32 = 260.0;
/// Width of the instruction list and of the inspector, in points.
const LIST_WIDTH: f32 = 220.0;
const INSPECTOR_WIDTH: f32 = 260.0;
/// Most rows one inspector section shows; the rest are summarised.
const MAX_ROWS: usize = 2_000;

/// The window's state, kept in `UiState::panels`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct TtDebuggerState {
    pub open: bool,
    pub glyph: Option<String>,
    /// 0 until set: the window then uses [`DEFAULT_PPEM`].
    pub ppem: u16,
    /// `grayscale`, `cleartype` or `directwrite`; empty for grayscale.
    pub intent: String,
    pub breakpoints: Vec<u64>,
    /// The last `hint.tt.debugStep` result.
    #[serde(skip)]
    pub result: Option<Value>,
    #[serde(skip)]
    pub error: Option<String>,
}

impl TtDebuggerState {
    fn ppem(&self) -> u16 {
        if self.ppem == 0 { DEFAULT_PPEM } else { self.ppem }
    }

    fn intent(&self) -> &str {
        if self.intent.is_empty() { "grayscale" } else { &self.intent }
    }
}

pub const COMMANDS: &[UiCommand] = &[(
    ID,
    "TrueType Debugger",
    &["Window"],
    "",
    "{open?: bool (default true), glyph?: name (default: the current glyph), ppem?: number (default 12), intent?: 'grayscale'|'cleartype'|'directwrite', action?: 'start'|'into'|'over'|'out'|'continue'|'stop', toggleBreakpoint?: byte offset in the glyph program} shows or hides the TrueType Debugger and runs `hint.tt.debugStep` for it: a new glyph, size or intent starts again at the first instruction, `action` steps from the state shown → {open, glyph, ppem, intent, breakpoints, state (the hint.tt.debugStep result, or null)}",
)];

pub fn run(app: &mut FontcraftApp, id: &str, args: &Args) -> Result<Value, String> {
    if id != ID {
        return Err(format!("unknown command `{id}`"));
    }
    if !args.flag("open")?.unwrap_or(true) {
        app.state.panels.tt_debugger.open = false;
        return Ok(reply(&app.state.panels.tt_debugger));
    }
    app.session.font().map_err(|e| args.err(e))?;
    let current = crate::panels::current_glyph(app);
    let st = &mut app.state.panels.tt_debugger;
    st.open = true;
    let mut restart = st.result.is_none();
    if let Some(glyph) = args.string("glyph")? {
        restart |= st.glyph.as_deref() != Some(glyph);
        st.glyph = Some(glyph.to_string());
    } else if st.glyph.is_none() {
        st.glyph = current;
    }
    if let Some(n) = args.number("ppem")? {
        if !(n.fract() == 0.0 && (1.0..=400.0).contains(&n)) {
            return Err(args.err(format!("`ppem` must be a whole pixel size from 1 to 400, got {n}")));
        }
        restart |= st.ppem() != n as u16;
        st.ppem = n as u16;
    }
    if let Some(intent) = args.string("intent")? {
        if !["grayscale", "cleartype", "directwrite"].contains(&intent) {
            return Err(args.err(format!("`intent` must be 'grayscale', 'cleartype' or 'directwrite', got '{intent}'")));
        }
        restart |= st.intent() != intent;
        st.intent = intent.to_string();
    }
    if let Some(pc) = args.index("toggleBreakpoint")? {
        if let Some(at) = st.breakpoints.iter().position(|b| *b == pc) {
            st.breakpoints.remove(at);
        } else if st.breakpoints.len() < MAX_BREAKPOINTS {
            st.breakpoints.push(pc);
            st.breakpoints.sort_unstable();
        }
    }
    let action = match args.string("action")? {
        Some("stop") => {
            st.result = None;
            st.error = None;
            return Ok(reply(st));
        }
        Some(a) => Some(a.to_string()),
        None if restart => Some("start".to_string()),
        None => None,
    };
    let (Some(action), Some(glyph)) = (action, st.glyph.clone()) else { return Ok(reply(st)) };
    let step = if action == "start" || restart { 0 } else { st.result.as_ref().and_then(|r| r["step"].as_u64()).unwrap_or(0) };
    let action = if restart && action != "start" { "start".to_string() } else { action };
    let params = json!({
        "glyph": glyph, "ppem": st.ppem(), "intent": st.intent(), "action": action, "step": step, "breakpoints": st.breakpoints,
    });
    match app.session.execute("hint.tt.debugStep", &params) {
        Ok(r) => {
            let st = &mut app.state.panels.tt_debugger;
            st.result = Some(r);
            st.error = None;
            Ok(reply(st))
        }
        Err(e) => {
            let st = &mut app.state.panels.tt_debugger;
            st.result = None;
            st.error = Some(e.to_string());
            Err(e.to_string())
        }
    }
}

fn reply(st: &TtDebuggerState) -> Value {
    json!({
        "open": st.open, "glyph": st.glyph, "ppem": st.ppem(), "intent": st.intent(),
        "breakpoints": st.breakpoints, "state": st.result.clone().unwrap_or(Value::Null),
    })
}

/// The edit fields' values between frames.
#[derive(Clone, Debug, Default)]
struct Fields {
    glyph: String,
    ppem: u16,
    /// The state the fields were last filled from; a change made elsewhere refills them.
    seen: (Option<String>, u16),
}

/// Draws the window while it is open.
pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    if !app.state.panels.tt_debugger.open {
        return;
    }
    let st = app.state.panels.tt_debugger.clone();
    let tokens = &app.tokens;
    let memory_id = egui::Id::new("window.ttDebugger.fields");
    let seen = (st.glyph.clone(), st.ppem());
    let mut fields = ui.ctx().data(|d| d.get_temp::<Fields>(memory_id)).filter(|f| f.seen == seen).unwrap_or_else(|| Fields {
        glyph: st.glyph.clone().unwrap_or_default(),
        ppem: st.ppem(),
        seen,
    });
    let mut open = true;
    egui::Window::new("TrueType Debugger")
        .id(egui::Id::new(ID))
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_width(LIST_WIDTH + PANE + INSPECTOR_WIDTH + 4.0 * tokens.dialog_spacing)
        .default_pos(egui::pos2(ui.ctx().content_rect().center().x, tokens.dialog_top))
        .show(ui.ctx(), |ui| {
            ui.horizontal(|ui| {
                ui.label("Glyph");
                let r = ui.add(egui::TextEdit::singleline(&mut fields.glyph).desired_width(120.0));
                if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) && !fields.glyph.trim().is_empty() {
                    pending.push((ID, json!({"glyph": fields.glyph.trim()})));
                }
                ui.label("PPEM");
                if ui.add(egui::DragValue::new(&mut fields.ppem).range(1..=400)).changed() {
                    pending.push((ID, json!({"ppem": fields.ppem})));
                }
                let mut intent = st.intent().to_string();
                egui::ComboBox::from_id_salt("window.ttDebugger.intent").selected_text(intent_label(&intent)).show_ui(ui, |ui| {
                    for key in ["grayscale", "cleartype", "directwrite"] {
                        ui.selectable_value(&mut intent, key.to_string(), intent_label(key));
                    }
                });
                if intent != st.intent() {
                    pending.push((ID, json!({"intent": intent})));
                }
            });
            ui.horizontal(|ui| {
                let running = st.result.as_ref().is_some_and(|r| r["finished"] != true && r.get("error").is_none());
                for (label, action, needs_run) in [
                    ("Start", "start", false),
                    ("Into", "into", true),
                    ("Over", "over", true),
                    ("Out", "out", true),
                    ("Continue", "continue", true),
                    ("Stop", "stop", false),
                ] {
                    if ui.add_enabled(!needs_run || running, egui::Button::new(label)).clicked() {
                        pending.push((ID, json!({"action": action})));
                    }
                }
            });
            if let Some(e) = &st.error {
                ui.colored_label(tokens.error_text, e);
            }
            let Some(r) = &st.result else {
                ui.colored_label(tokens.text_dim, "Choose a glyph and press Start.");
                return;
            };
            ui.label(status_line(r));
            ui.separator();
            ui.horizontal_top(|ui| {
                instruction_list(ui, tokens, r, &st.breakpoints, pending);
                glyph_pane(ui, tokens, r);
                inspector(ui, r);
            });
        });
    ui.ctx().data_mut(|d| d.insert_temp(memory_id, fields));
    if !open {
        app.state.panels.tt_debugger.open = false;
    }
}

fn intent_label(key: &str) -> &'static str {
    match key {
        "cleartype" => "GDI ClearType",
        "directwrite" => "DirectWrite",
        _ => "Grayscale",
    }
}

fn status_line(r: &Value) -> String {
    let step = r["step"].as_u64().unwrap_or(0);
    if let Some(e) = r["error"].as_str() {
        return format!("Step {step}: {e}");
    }
    if r["hinting"] == false {
        return "The font's programs turn hinting off at this size; the glyph program does not run.".into();
    }
    if r["finished"] == true {
        return format!("Finished after {step} steps");
    }
    let next = &r["next"];
    let place = match next["program"].as_str() {
        Some("glyph") | None => String::new(),
        Some(p) => format!(" in the {p} program"),
    };
    format!("Step {step}, call depth {}: next {} at {}{place}", r["depth"], next["instruction"].as_str().unwrap_or(""), next["pc"])
}

fn instruction_list(ui: &mut egui::Ui, tokens: &crate::theme::Tokens, r: &Value, breakpoints: &[u64], pending: &mut Pending) {
    let next = r.get("next").filter(|n| n.get("program").and_then(Value::as_str) == Some("glyph"));
    let current = next.and_then(|n| n.get("pc")).and_then(Value::as_u64);
    ui.vertical(|ui| {
        ui.set_width(LIST_WIDTH);
        ui.strong("Glyph program");
        egui::ScrollArea::vertical().id_salt("window.ttDebugger.list").max_height(PANE).show(ui, |ui| {
            let Some(list) = r["program"].as_array() else { return };
            if list.is_empty() {
                ui.colored_label(tokens.text_dim, "The glyph has no instructions.");
            }
            for ins in list {
                let Some(pc) = ins["pc"].as_u64() else { continue };
                let mark = if breakpoints.contains(&pc) { "●" } else { " " };
                let text = egui::RichText::new(format!("{mark} {pc:>5}  {}", ins["instruction"].as_str().unwrap_or(""))).monospace();
                let hover = match ins["gloss"].as_str().filter(|g| !g.is_empty()) {
                    Some(gloss) => format!("{gloss}\nClick to set or clear a breakpoint"),
                    None => "Click to set or clear a breakpoint".to_string(),
                };
                let row = ui.selectable_label(current == Some(pc), text).on_hover_text(hover);
                if current == Some(pc) {
                    row.scroll_to_me(None);
                }
                if row.clicked() {
                    pending.push((ID, json!({"toggleBreakpoint": pc})));
                }
            }
        });
    });
}

/// A point of the result in pixels: (x, y, original x, original y, on curve, touched).
fn point_of(p: &Value) -> Option<(f32, f32, f32, f32, bool, bool)> {
    let f = |k: &str| p[k].as_f64().filter(|v| v.is_finite()).map(|v| v as f32);
    Some((f("x")?, f("y")?, f("originalX")?, f("originalY")?, p["onCurve"] == true, p["touchedX"] == true || p["touchedY"] == true))
}

fn glyph_pane(ui: &mut egui::Ui, tokens: &crate::theme::Tokens, r: &Value) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(PANE, PANE), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, tokens.canvas);
    let pts: Vec<_> = r["points"].as_array().map(|a| a.iter().filter_map(point_of).collect()).unwrap_or_default();
    if pts.is_empty() {
        return;
    }
    // Fit the current and original positions, with a pixel of margin.
    let (mut lo_x, mut lo_y, mut hi_x, mut hi_y) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for p in &pts {
        for (x, y) in [(p.0, p.1), (p.2, p.3)] {
            lo_x = lo_x.min(x);
            lo_y = lo_y.min(y);
            hi_x = hi_x.max(x);
            hi_y = hi_y.max(y);
        }
    }
    let (lo_x, lo_y) = (lo_x.floor() - 1.0, lo_y.floor() - 1.0);
    let (hi_x, hi_y) = (hi_x.ceil() + 1.0, hi_y.ceil() + 1.0);
    let span = (hi_x - lo_x).max(hi_y - lo_y).max(1.0);
    let scale = PANE / span;
    let to_screen = |x: f32, y: f32| egui::pos2(rect.left() + (x - lo_x) * scale, rect.bottom() - (y - lo_y) * scale);
    // The pixel grid, when its cells are big enough to see.
    if scale >= 4.0 {
        let grid = egui::Stroke::new(1.0, tokens.metric_line);
        let mut x = lo_x;
        while x <= lo_x + span {
            painter.line_segment([to_screen(x, lo_y), to_screen(x, lo_y + span)], grid);
            x += 1.0;
        }
        let mut y = lo_y;
        while y <= lo_y + span {
            painter.line_segment([to_screen(lo_x, y), to_screen(lo_x + span, y)], grid);
            y += 1.0;
        }
    }
    // Contours through their points, in point order.
    let stroke = egui::Stroke::new(tokens.outline_width, tokens.outline);
    let mut first = 0usize;
    for end in r["contours"].as_array().into_iter().flatten().filter_map(Value::as_u64) {
        let end = end as usize;
        let Some(contour) = pts.get(first..=end) else { break };
        let line: Vec<egui::Pos2> = contour.iter().map(|p| to_screen(p.0, p.1)).collect();
        painter.add(egui::Shape::closed_line(line, stroke));
        first = end + 1;
    }
    for p in &pts {
        painter.circle_stroke(to_screen(p.2, p.3), tokens.handle_radius, egui::Stroke::new(1.0, tokens.text_dim));
        let colour = if p.5 {
            tokens.accent
        } else if p.4 {
            tokens.node_corner
        } else {
            tokens.node_handle
        };
        painter.circle_filled(to_screen(p.0, p.1), tokens.handle_radius, colour);
    }
}

fn rows(ui: &mut egui::Ui, title: &str, items: Vec<String>) {
    egui::CollapsingHeader::new(format!("{title} ({})", items.len())).id_salt(("window.ttDebugger.section", title)).show(ui, |ui| {
        for line in items.iter().take(MAX_ROWS) {
            ui.monospace(line);
        }
        if items.len() > MAX_ROWS {
            ui.label(format!("… {} more", items.len() - MAX_ROWS));
        }
    });
}

fn point_rows(r: &Value, key: &str) -> Vec<String> {
    r[key]
        .as_array()
        .map(|a| {
            a.iter()
                .enumerate()
                .map(|(i, p)| {
                    let touched = match (p["touchedX"] == true, p["touchedY"] == true) {
                        (true, true) => "xy",
                        (true, false) => "x",
                        (false, true) => "y",
                        (false, false) => "",
                    };
                    format!("{i:>4}  {:>9.3} {:>9.3}  {touched}", p["x"].as_f64().unwrap_or(0.0), p["y"].as_f64().unwrap_or(0.0))
                })
                .collect()
        })
        .unwrap_or_default()
}

fn inspector(ui: &mut egui::Ui, r: &Value) {
    ui.vertical(|ui| {
        ui.set_width(INSPECTOR_WIDTH);
        egui::ScrollArea::vertical().id_salt("window.ttDebugger.inspector").max_height(PANE).show(ui, |ui| {
            let numbers = |key: &str| -> Vec<String> {
                r[key].as_array().map(|a| a.iter().enumerate().map(|(i, v)| format!("{i:>5}  {v}")).collect()).unwrap_or_default()
            };
            let mut stack: Vec<String> = numbers("stack");
            stack.reverse();
            rows(ui, "Stack", stack);
            let gs: Vec<String> = r["graphicsState"].as_object().map(Map::iter).into_iter().flatten().map(|(k, v)| format!("{k}: {v}")).collect();
            rows(ui, "Graphics state", gs);
            rows(ui, "Points", point_rows(r, "points"));
            rows(ui, "Phantom points", point_rows(r, "phantom"));
            rows(ui, "Twilight points", point_rows(r, "twilight"));
            rows(ui, "Storage", numbers("storage"));
            rows(ui, "Control values", numbers("cvt"));
        });
    });
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

    use fontcraft_engine::Session;
    use fontcraft_engine::model::{Path, Shape};

    use super::*;

    fn app() -> FontcraftApp {
        let mut session = Session::with_document();
        session
            .edit("setup", |font| {
                let mut g = font.glyph_mut("I")?;
                g.layer_mut("m01").unwrap().shapes.push(Shape::Path(Path::rect(100.0, 150.0, 200.0, 650.0)));
                Ok(())
            })
            .unwrap();
        let mut app = FontcraftApp::new(session);
        app.run("hint.tt.snapPoint", &json!({"glyph": "I", "nodes": [[0, 0]], "zone": "none"})).unwrap();
        app
    }

    fn frame(app: &mut FontcraftApp, ctx: &egui::Context) {
        let input =
            egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1280.0, 800.0))), ..Default::default() };
        ctx.run_ui(input, |ui| app.ui(ui)).textures_delta.clear();
    }

    #[test]
    fn window_tt_debugger_steps_a_glyph_and_keeps_breakpoints() {
        let mut app = app();
        let r = app.run(ID, &json!({"glyph": "I", "ppem": 10})).unwrap();
        assert_eq!((&r["open"], &r["glyph"], &r["ppem"]), (&json!(true), &json!("I"), &json!(10)));
        assert_eq!(r["state"]["step"], 0, "a new glyph starts at the first instruction");
        let first_pc = r["state"]["program"][0]["pc"].as_u64().unwrap();
        let ctx = egui::Context::default();
        frame(&mut app, &ctx);

        let r = app.run(ID, &json!({"action": "into"})).unwrap();
        assert_eq!(r["state"]["step"], 1);
        let r = app.run(ID, &json!({"toggleBreakpoint": first_pc})).unwrap();
        assert_eq!(r["breakpoints"], json!([first_pc]));
        assert_eq!(r["state"]["step"], 1, "a breakpoint change does not step");
        let r = app.run(ID, &json!({"action": "continue"})).unwrap();
        assert_eq!(r["state"]["finished"], true);
        let ys: Vec<f64> = r["state"]["points"].as_array().unwrap().iter().map(|p| p["y"].as_f64().unwrap()).collect();
        assert!(ys.iter().all(|y| *y == 2.0 || *y == 7.0), "{ys:?}");
        frame(&mut app, &ctx);

        // Another size starts again; Stop clears the state; closing keeps the settings.
        let r = app.run(ID, &json!({"ppem": 11, "action": "into"})).unwrap();
        assert_eq!(r["state"]["step"], 0);
        let r = app.run(ID, &json!({"action": "stop"})).unwrap();
        assert_eq!(r["state"], Value::Null);
        frame(&mut app, &ctx);
        let r = app.run(ID, &json!({"open": false})).unwrap();
        assert_eq!((&r["open"], &r["ppem"], &r["breakpoints"]), (&json!(false), &json!(11), &json!([first_pc])));
        assert!(app.run(ID, &json!({"ppem": 0})).is_err());
        assert!(app.run(ID, &json!({"glyph": "nope"})).is_err());
    }
}
