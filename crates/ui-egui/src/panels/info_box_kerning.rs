//! The kerning part of the Info box glyph box (`info_box_metrics`), while Show Info is on.
//!
//! For the current glyph of an Edit View tab it gives, for each edge as drawn, the kerning
//! towards the neighbouring glyph, the lock of the current glyph in that pair and the current
//! glyph's kerning group on that side (`infoBox.kerning`); the glyph box draws them at its outer
//! edges. While the Text tool is the chosen tool a smaller box beside the glyph box shows the
//! glyph before the current one: its name, its lock in the pair and its group that faces the
//! current glyph (`infoBox.precedingGlyph`).
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
//! The kern fields follow the Info box field rules (`widgets`), show the grey placeholder `Kern`
//! when no pair applies, and step from zero in an empty field.

use egui::Id;
use serde_json::{Value, json};

use super::{Pending, widgets};
use crate::app::FontcraftApp;
use crate::canvas::view;
use crate::icons::Icon;
use crate::state::WritingDirection;
use crate::theme::Tokens;

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
pub(super) struct Preceding {
    pair: Pair,
    group_command: &'static str,
}

pub(super) struct Kerning {
    glyph: String,
    master: String,
    direction: &'static str,
    edges: [Edge; 2],
    pub(super) preceding: Option<Preceding>,
}

pub(super) fn read(app: &mut FontcraftApp) -> Option<Kerning> {
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

/// The kern field at edge `index` (0: left or top as drawn, 1: right or bottom).
pub(super) fn edge_kern(ui: &mut egui::Ui, tokens: &Tokens, k: &Kerning, index: usize, pending: &mut Pending) {
    if let Some(edge) = k.edges.get(index) {
        kern(ui, tokens, k, edge, pending);
    }
}

/// The group field and the lock at edge `index`, the group outermost.
pub(super) fn edge_group(ui: &mut egui::Ui, tokens: &Tokens, k: &Kerning, index: usize, pending: &mut Pending) {
    let Some(edge) = k.edges.get(index) else { return };
    let field = GroupField {
        glyph: &k.glyph,
        shown: &edge.group,
        command: edge.group_command,
        salt: format!("infoBox.group{}", edge.salt),
        hover: edge.group_hover,
    };
    let lock_salt = format!("infoBox.lock{}", edge.salt);
    if index == 0 {
        group(ui, tokens, field, pending);
        lock(ui, k, edge.pair.as_ref(), edge.own, &lock_salt, pending);
    } else {
        lock(ui, k, edge.pair.as_ref(), edge.own, &lock_salt, pending);
        group(ui, tokens, field, pending);
    }
}

/// Draws the preceding glyph's box beside the glyph box `beside`: on its left, or on its right
/// when the text runs right to left.
pub(super) fn show_preceding(ctx: &egui::Context, canvas: egui::Rect, tokens: &Tokens, k: &Kerning, beside: egui::Rect, pending: &mut Pending) {
    let Some(p) = &k.preceding else { return };
    let gap = tokens.info_box_glyph_gap;
    let (pos, pivot) = if k.direction == "rtl" {
        (beside.right_top() + egui::vec2(gap, 0.0), egui::Align2::LEFT_TOP)
    } else {
        (beside.left_top() - egui::vec2(gap, 0.0), egui::Align2::RIGHT_TOP)
    };
    egui::Area::new(widgets::id("infoBox.preceding")).order(egui::Order::Middle).fixed_pos(pos).pivot(pivot).constrain_to(canvas).show(ctx, |ui| {
        egui::Frame::popup(ui.style()).fill(tokens.panel).show(ui, |ui| preceding(ui, tokens, k, p, pending));
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

fn parse(text: &str) -> Option<f64> {
    text.trim().parse::<f64>().ok().filter(|n| n.is_finite())
}

/// A kern field showing `value`, or the grey placeholder `Kern` when no pair applies. Returns
/// what to commit: a number, or `Clear` when a field that showed a value is emptied.
fn kern_field(ui: &mut egui::Ui, tokens: &Tokens, id: Id, value: Option<f64>) -> Option<KernEdit> {
    let shown = value.map(widgets::format_number).unwrap_or_default();
    let step = |text: &str, delta: f64| Some(widgets::format_number(parse(text).or(value).unwrap_or(0.0) + delta));
    let look = widgets::Look { color: None, placeholder: Some(egui::RichText::new("Kern").color(tokens.text_dim)) };
    let committed = widgets::line(ui, id, &shown, tokens.field_width, look, &step).0?;
    let trimmed = committed.trim();
    if trimmed.is_empty() {
        return value.is_some().then_some(KernEdit::Clear);
    }
    parse(trimmed).map(KernEdit::Set)
}
