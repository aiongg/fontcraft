//! The kerning part of the Info box, at the lower left of the canvas while Show Info is on.
//!
//! For the current glyph of an Edit View tab it shows, at each edge as drawn, the kerning
//! towards the neighbouring glyph, the lock of the current glyph in that pair and the current
//! glyph's kerning group on that side (`infoBox.kerning`). While the Text tool is the chosen tool
//! a smaller box before it shows the glyph before the current one: its name, its lock in the pair
//! and its group that faces the current glyph (`infoBox.precedingGlyph`).
//!
//! | Control | Runs |
//! |---|---|
//! | kern field, a number typed (Return or leaving the field) or stepped with Up and Down | `kerning.setPair` on the sides the locks name: a closed lock gives the glyph's group, an open lock or no group the glyph |
//! | kern field emptied | `kerning.removePair` on the pair that applies |
//! | lock, closed | `kerning.addException` for that side of the pair |
//! | lock, open | `kerning.removeException` for that side; a refusal goes to the status line |
//! | group field | `kerning.setLeftGroup`, `…RightGroup`, `…TopGroup` or `…BottomGroup` for the glyph |
//!
//! Pairs are in logical order, as the commands take them. Left to right, the left edge is the
//! pair with the glyph before; right to left, it is the pair with the glyph after, which is drawn
//! on the left; top to bottom, the first edge is the pair with the glyph above.
//!
//! The kern fields follow the Info box field rules (`widgets`) and add what kern fields need: the
//! grey placeholder `Kern` when no pair applies, Up and Down from zero in an empty field, and
//! steps of 100 with Cmd+Shift (`spacing.infoBoxArrowSteps`).

use egui::text::CCursorRange;
use egui::{Id, Key, Modifiers};
use serde_json::{Value, json};

use super::{Pending, widgets};
use crate::app::FontcraftApp;
use crate::canvas::view;
use crate::icons::Icon;
use crate::state::WritingDirection;
use crate::theme::Tokens;

/// Longest text a kern field takes.
const MAX_KERN_CHARS: usize = 32;
/// Up and Down change a kern value by 1, by 10 with Shift and by 100 with Cmd+Shift.
const STEPS: [(Modifiers, f64); 3] = [(Modifiers::COMMAND.plus(Modifiers::SHIFT), 100.0), (Modifiers::SHIFT, 10.0), (Modifiers::NONE, 1.0)];
/// The keys a focused kern field keeps for itself, as `widgets` fields do.
const FILTER: egui::EventFilter = egui::EventFilter { tab: false, horizontal_arrows: true, vertical_arrows: true, escape: true };

/// Two glyphs next to each other in the text, in logical order, with `kerning.getPair`'s reply.
struct Pair {
    left: String,
    right: String,
    reply: Value,
}

#[derive(Clone, Copy, PartialEq)]
enum Lock {
    Closed,
    Open,
    Ungrouped,
}

impl Pair {
    /// The kerning that applies, `None` when no stored pair applies.
    fn value(&self) -> Option<f64> {
        self.reply.get("step").filter(|s| !s.is_null())?;
        self.reply.get("value").and_then(Value::as_f64)
    }

    /// The lock of `side` (`"left"` or `"right"`).
    fn lock(&self, side: &str) -> Lock {
        match self.reply.get("locks").and_then(|l| l.get(side)).and_then(Value::as_str) {
            Some("closed") => Lock::Closed,
            Some("open") => Lock::Open,
            _ => Lock::Ungrouped,
        }
    }

    /// The group of `side`'s glyph that faces the pair.
    fn group(&self, side: &str) -> Option<&str> {
        self.reply.get("groups").and_then(|g| g.get(side)).and_then(Value::as_str)
    }

    fn glyph(&self, side: &str) -> &str {
        if side == "left" { &self.left } else { &self.right }
    }

    /// The side a typed value is stored on: the group behind a closed lock, else the glyph.
    fn edited_side(&self, side: &str) -> String {
        match (self.lock(side), self.group(side)) {
            (Lock::Closed, Some(group)) => format!("@{group}"),
            _ => self.glyph(side).to_string(),
        }
    }

    /// The stored pair that applies, in command spelling.
    fn applied(&self) -> Option<(String, String)> {
        let pair = self.reply.get("pair")?;
        let side = |k: &str| pair.get(k).and_then(Value::as_str).map(str::to_string);
        side("left").zip(side("right"))
    }
}

/// One edge of the current glyph as drawn.
struct Edge {
    /// Names the edge's controls: `infoBox.kern…`, `infoBox.lock…`, `infoBox.group…`.
    salt: &'static str,
    /// The pair with the neighbour on this edge; `None` at the start or end of a line.
    pair: Option<Pair>,
    /// The current glyph's side in `pair`.
    own: &'static str,
    /// The current glyph's group on this edge and the command that sets it.
    group: String,
    group_command: &'static str,
    group_hover: &'static str,
}

/// The glyph before the current one, for the box the Text tool shows.
struct Preceding {
    pair: Pair,
    group_command: &'static str,
}

struct Kerning {
    glyph: String,
    master: String,
    direction: &'static str,
    edges: [Edge; 2],
    preceding: Option<Preceding>,
}

fn read(app: &mut FontcraftApp) -> Option<Kerning> {
    let tab = app.state.tab()?;
    let font = app.session.font().ok()?;
    let glyph = tab.edited_glyph()?.to_string();
    let index = tab.edited_index()?;
    let master = view::active_master(font, tab)?.id.clone();
    let before = index.checked_sub(1).and_then(|i| tab.item_glyph(i)).map(str::to_string);
    let after = index.checked_add(1).and_then(|i| tab.item_glyph(i)).map(str::to_string);
    let groups = font.glyph(&glyph)?.kern_groups.clone();
    let writing = tab.direction;
    let text_tool = app.tools.toolbox.base_id() == "text";
    let direction = match writing {
        WritingDirection::Ltr => "ltr",
        WritingDirection::Rtl => "rtl",
        WritingDirection::Ttb => "vertical",
    };
    let mut pair = |left: &str, right: &str| -> Option<Pair> {
        let query = json!({"left": left, "right": right, "master": master, "direction": direction});
        let reply = app.session.execute("kerning.getPair", &query).ok()?;
        Some(Pair { left: left.to_string(), right: right.to_string(), reply })
    };
    let before_pair = before.as_deref().and_then(|b| pair(b, &glyph));
    let after_pair = after.as_deref().and_then(|a| pair(&glyph, a));
    let preceding_pair = if text_tool { before.as_deref().and_then(|b| pair(b, &glyph)) } else { None };
    let side = |group: &Option<String>| group.clone().unwrap_or_default();
    let (left, right, top, bottom) = (side(&groups.left), side(&groups.right), side(&groups.top), side(&groups.bottom));
    let edge = |salt, pair, own, group, group_command, group_hover| Edge { salt, pair, own, group, group_command, group_hover };
    let (edges, preceding_command) = match writing {
        WritingDirection::Ltr => (
            [
                edge("Left", before_pair, "right", left, "kerning.setLeftGroup", "Left kerning group"),
                edge("Right", after_pair, "left", right, "kerning.setRightGroup", "Right kerning group"),
            ],
            "kerning.setRightGroup",
        ),
        WritingDirection::Rtl => (
            [
                edge("Left", after_pair, "left", left, "kerning.setLeftGroup", "Left kerning group"),
                edge("Right", before_pair, "right", right, "kerning.setRightGroup", "Right kerning group"),
            ],
            "kerning.setLeftGroup",
        ),
        WritingDirection::Ttb => (
            [
                edge("Top", before_pair, "right", top, "kerning.setTopGroup", "Top kerning group"),
                edge("Bottom", after_pair, "left", bottom, "kerning.setBottomGroup", "Bottom kerning group"),
            ],
            "kerning.setBottomGroup",
        ),
    };
    let preceding = preceding_pair.map(|pair| Preceding { pair, group_command: preceding_command });
    Some(Kerning { glyph, master, direction, edges, preceding })
}

pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    let tokens = app.tokens.clone();
    let Some(k) = read(app) else { return };
    let canvas = ui.available_rect_before_wrap();
    let pos = egui::pos2(canvas.left() + tokens.info_box_margin, canvas.bottom() - tokens.info_box_margin);
    egui::Area::new(widgets::id("infoBox.kerning"))
        .order(egui::Order::Middle)
        .fixed_pos(pos)
        .pivot(egui::Align2::LEFT_BOTTOM)
        .constrain_to(canvas)
        .show(ui.ctx(), |ui| {
            ui.horizontal(|ui| {
                if let Some(p) = &k.preceding {
                    egui::Frame::popup(ui.style()).fill(tokens.panel).show(ui, |ui| preceding(ui, &tokens, &k, p, pending));
                }
                egui::Frame::popup(ui.style()).fill(tokens.panel).show(ui, |ui| contents(ui, &tokens, &k, pending));
            });
        });
}

/// The current glyph's box: kern field, lock and group at each edge, the glyph name between.
fn contents(ui: &mut egui::Ui, tokens: &Tokens, k: &Kerning, pending: &mut Pending) {
    ui.horizontal(|ui| {
        let [first, second] = &k.edges;
        kern(ui, tokens, k, first, pending);
        lock(ui, k, first.pair.as_ref(), first.own, &format!("infoBox.lock{}", first.salt), pending);
        group(
            ui,
            tokens,
            GroupField {
                glyph: &k.glyph,
                shown: &first.group,
                command: first.group_command,
                salt: format!("infoBox.group{}", first.salt),
                hover: first.group_hover,
            },
            pending,
        );
        ui.label(&k.glyph);
        group(
            ui,
            tokens,
            GroupField {
                glyph: &k.glyph,
                shown: &second.group,
                command: second.group_command,
                salt: format!("infoBox.group{}", second.salt),
                hover: second.group_hover,
            },
            pending,
        );
        lock(ui, k, second.pair.as_ref(), second.own, &format!("infoBox.lock{}", second.salt), pending);
        kern(ui, tokens, k, second, pending);
    });
}

/// The preceding glyph's box: its name, its lock in the pair and its group facing the current
/// glyph.
fn preceding(ui: &mut egui::Ui, tokens: &Tokens, k: &Kerning, p: &Preceding, pending: &mut Pending) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(&p.pair.left).color(tokens.text_dim));
        lock(ui, k, Some(&p.pair), "left", "infoBox.preceding.lock", pending);
        let shown = p.pair.group("left").unwrap_or_default().to_string();
        let field = GroupField {
            glyph: &p.pair.left,
            shown: &shown,
            command: p.group_command,
            salt: "infoBox.preceding.group".into(),
            hover: "Kerning group facing the current glyph",
        };
        group(ui, tokens, field, pending);
    });
}

fn kern(ui: &mut egui::Ui, tokens: &Tokens, k: &Kerning, edge: &Edge, pending: &mut Pending) {
    let id = widgets::id(format!("infoBox.kern{}", edge.salt));
    let Some(pair) = &edge.pair else {
        ui.add_enabled_ui(false, |ui| kern_field(ui, tokens, id, None));
        return;
    };
    let value = pair.value();
    match kern_field(ui, tokens, id, value) {
        Some(KernEdit::Set(v)) => pending.push((
            "kerning.setPair",
            json!({
                "left": pair.edited_side("left"), "right": pair.edited_side("right"), "value": v,
                "master": k.master, "direction": k.direction,
            }),
        )),
        Some(KernEdit::Clear) => {
            if let Some((left, right)) = pair.applied() {
                pending.push(("kerning.removePair", json!({"left": left, "right": right, "master": k.master, "direction": k.direction})));
            }
        }
        None => {}
    }
}

/// The lock of `side` in `pair`: closed runs `kerning.addException`, open
/// `kerning.removeException`; without a group on that side it is drawn disabled.
fn lock(ui: &mut egui::Ui, k: &Kerning, pair: Option<&Pair>, side: &str, salt: &str, pending: &mut Pending) {
    let state = pair.map_or(Lock::Ungrouped, |p| p.lock(side));
    let (icon, hover) = match state {
        Lock::Closed => (Icon::Locked, "Kerning goes to the group; click to make this glyph an exception"),
        Lock::Open => (Icon::Unlocked, "This glyph is an exception; click to kern it through its group again"),
        Lock::Ungrouped => (Icon::Locked, "No kerning group on this side"),
    };
    let enabled = state != Lock::Ungrouped;
    if widgets::icon_button(ui, widgets::id(salt), icon, hover, enabled, false)
        && let Some(p) = pair
    {
        let command = if state == Lock::Open { "kerning.removeException" } else { "kerning.addException" };
        pending.push((command, json!({"left": p.left, "right": p.right, "side": side, "master": k.master, "direction": k.direction})));
    }
}

/// A kerning group field of `glyph`, running `command` with the text committed.
struct GroupField<'a> {
    glyph: &'a str,
    shown: &'a str,
    command: &'static str,
    salt: String,
    hover: &'a str,
}

fn group(ui: &mut egui::Ui, tokens: &Tokens, field: GroupField<'_>, pending: &mut Pending) {
    let response = ui.scope(|ui| widgets::text(ui, widgets::id(&field.salt), field.shown, tokens.field_width));
    if let Some(text) = response.inner {
        pending.push((field.command, json!({"glyphs": [field.glyph], "group": text.trim()})));
    }
    response.response.on_hover_text(field.hover);
}

/// What the user did to a kern field.
#[derive(Clone, Copy, Debug, PartialEq)]
enum KernEdit {
    Set(f64),
    Clear,
}

/// A kern field being edited: its text, and the pass that last drew it focused.
#[derive(Clone)]
struct Editing {
    text: String,
    pass: u64,
}

fn parse(text: &str) -> Option<f64> {
    text.trim().parse::<f64>().ok().filter(|n| n.is_finite())
}

/// The arrow keys of this frame as a change of the value; consumed so the field keeps them.
fn arrow_delta(input: &mut egui::InputState) -> f64 {
    let mut delta = 0.0;
    for (key, sign) in [(Key::ArrowUp, 1.0), (Key::ArrowDown, -1.0)] {
        // The most modifiers first: a pattern also matches presses with Shift held.
        for (modifiers, step) in STEPS {
            delta += sign * step * input.count_and_consume_key(modifiers, key) as f64;
        }
    }
    delta
}

/// A kern field showing `value`, or the grey placeholder `Kern` when no pair applies. Returns
/// what to commit: a number, or `Clear` when a field that showed a value is emptied.
fn kern_field(ui: &mut egui::Ui, tokens: &Tokens, id: Id, value: Option<f64>) -> Option<KernEdit> {
    let ctx = ui.ctx().clone();
    let pass = ctx.cumulative_pass_nr();
    let shown = value.map(widgets::format_number).unwrap_or_default();
    // An edit left behind by a field that stopped being drawn is dropped, not committed later.
    let editing = ctx.data(|d| d.get_temp::<Editing>(id)).filter(|e| pass.saturating_sub(e.pass) <= 1);
    let focused = ctx.memory(|m| m.has_focus(id));
    let mut text = editing.as_ref().map_or_else(|| shown.clone(), |e| e.text.clone());
    let mut commit = None;
    let mut cancelled = false;
    if focused && editing.is_some() {
        let (escape, delta) = ui.input_mut(|i| (i.consume_key(Modifiers::NONE, Key::Escape), arrow_delta(i)));
        if escape {
            cancelled = true;
            text.clone_from(&shown);
            ctx.memory_mut(|m| m.surrender_focus(id));
        } else if delta != 0.0 {
            let next = widgets::format_number(parse(&text).or(value).unwrap_or(0.0) + delta);
            text.clone_from(&next);
            commit = Some(next);
        }
    }
    let hint = egui::RichText::new("Kern").color(tokens.text_dim);
    let mut output = egui::TextEdit::singleline(&mut text)
        .id(id)
        .desired_width(tokens.field_width)
        .char_limit(MAX_KERN_CHARS)
        .event_filter(FILTER)
        .hint_text(hint)
        .show(ui);
    if output.response.has_focus() {
        if editing.is_none() {
            output.state.cursor.set_char_range(Some(CCursorRange::select_all(&output.galley)));
            output.state.store(&ctx, id);
        }
        ctx.data_mut(|d| d.insert_temp(id, Editing { text, pass }));
    } else if editing.is_some() {
        ctx.data_mut(|d| d.remove::<Editing>(id));
        if !cancelled && text != shown {
            commit = Some(text);
        }
    }
    let committed = commit?;
    let trimmed = committed.trim();
    if trimmed.is_empty() {
        return value.is_some().then_some(KernEdit::Clear);
    }
    parse(trimmed).map(KernEdit::Set)
}
