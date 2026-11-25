//! The element list: `ui.elements`, and what `ui.click {element}` and `ui.set {element, value}` act on.
//!
//! egui builds an AccessKit tree per window and pass while it is asked to (no Cargo feature: only the platform
//! adapters of eframe are behind one). [`activate`] asks for it and adds [`Tree`], an egui plugin that keeps the
//! elements of the last pass of every window, the main one and the floating ones, which are all drawn inside the main
//! window's pass. It keeps them for [`KEEP_PASSES`] passes after the last request, then drops them.
//!
//! A click or a change goes through the window's input at the element's visible place, so it does what the mouse and
//! the keyboard do there: nothing reaches an element that is disabled, hidden or scrolled out of view. A change that
//! needs to see what its first input did (the list a select opened, the value a slider was dragged to) runs again after
//! [`FRAMES`] frames, with a [`Stage`] kept on the app between the rounds.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use craft::behavior::PointerButton;
use craft::keymap::{Key, Mods};
use craft::ui::runtime::windows::{Released, WindowHost};
use egui::accesskit::{Action, Node, NodeId, Role, Toggled};
use egui::{Context, IdMap, Plugin, Pos2, Rect, Ui, Vec2, ViewportId, WidgetRects, pos2, vec2};
use serde_json::{Map, Value, json};

use super::window_input::{focus_on_press, queued, window, window_name};
use super::{Method, object};
use crate::app::FontCraftApp;

pub const METHODS: &[(&str, Method)] = &[("ui.elements", list)];

/// Passes of the main window the trees are kept for after the last request that read them.
const KEEP_PASSES: u32 = 600;
/// Frames drawn before a request is answered: the one that applies what the requests before it fired, the first one
/// with a tree, and one in which the widgets that describe themselves from the pass before have done so.
pub const FRAMES: u64 = 3;
/// Most elements kept of one window.
const MAX_WINDOW_ELEMENTS: usize = 20_000;
/// Most nodes of one window's tree walked.
const MAX_NODES: usize = 100_000;
/// Elements one reply lists, unless it asks for another number, and at most.
const DEFAULT_LIMIT: u64 = 500;
const MAX_LIMIT: u64 = 2_000;
/// Longest label or value kept, in characters; longer ones are cut.
const MAX_ELEMENT_TEXT: usize = 500;
/// Most characters `ui.set` types into a field.
const MAX_SET_TEXT: usize = 4_096;
/// Most options an error names.
const MAX_NAMED_OPTIONS: usize = 20;
/// Most drags that bring a slider to its value.
const MAX_SLIDES: u32 = 4;
/// How far from its value a slider may stay, as a share of its range.
const SLIDE_SHARE: f64 = 0.001;
/// The thumb of a slider, in points: only as exact as a press that grabs it needs.
const THUMB: f32 = 16.0;
/// How far a drag goes first, so that a correction shorter than a click's slack is a drag too.
const DRAG_START: f32 = 12.0;

/// Whether the request is answered from the trees, so it waits for frames ([`FRAMES`]).
pub fn reads(method: &str, params: &Value) -> bool {
    method == "ui.elements" || (matches!(method, "ui.click" | "ui.set") && params.get("element").is_some_and(|v| !v.is_null()))
}

/// Asks for the trees of the coming passes.
pub fn activate(ctx: &Context) {
    ctx.enable_accesskit();
    // Added once: a second one is dropped.
    ctx.add_plugin(Tree::default());
    if let Some(tree) = ctx.plugin_opt::<Tree>() {
        tree.lock().wanted = KEEP_PASSES;
    }
    ctx.request_repaint();
}

/// One element of a window, as its last pass showed it. Rectangles are in the window's points.
#[derive(Clone, Debug)]
pub struct Element {
    id: u64,
    role: Role,
    label: String,
    value: Option<String>,
    number: Option<f64>,
    /// The label of the nearest labelled element around it.
    within: Option<Arc<str>>,
    rect: Option<Rect>,
    /// The part that shows: not scrolled or clipped away.
    visible: Option<Rect>,
    enabled: bool,
    read_only: bool,
    focused: bool,
    toggled: Option<Toggled>,
    selected: Option<bool>,
    expanded: Option<bool>,
    /// The smallest and largest number, and what one key press changes it by.
    range: Option<(f64, f64)>,
    step: Option<f64>,
    /// The element says it takes typed text.
    types: bool,
}

/// How `ui.set` changes an element.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Set {
    /// `true` or `false`, with a click.
    Toggle,
    /// A text or a number, typed.
    Text,
    /// The label of an option, picked from the list that opens.
    Choice,
    /// A number, dragged to.
    Number,
}

impl Set {
    fn name(self) -> &'static str {
        match self {
            Set::Toggle => "bool",
            Set::Text => "text",
            Set::Choice => "choice",
            Set::Number => "number",
        }
    }
}

/// A `ui.set` that goes on once the frames after its first input have been drawn.
#[derive(Debug)]
pub struct Stage {
    element: u64,
    value: Value,
    /// Rounds run so far, and window input steps queued in them.
    round: u32,
    queued: usize,
    next: Next,
}

#[derive(Clone, Copy, Debug)]
enum Next {
    /// A field was clicked and should have the keyboard.
    Focused,
    /// A field's text was typed; `options` showed in a list under the same label before.
    Commit { options: bool },
    /// The list of a select is open, or opening; `typed` once the option's label was typed into it.
    Pick { typed: bool },
    /// The slider was dragged from this value.
    Slide { from: f64 },
}

impl Element {
    fn set(&self) -> Option<Set> {
        if self.read_only || !self.enabled {
            return None;
        }
        match self.role {
            Role::TextInput
            | Role::MultilineTextInput
            | Role::SearchInput
            | Role::EmailInput
            | Role::NumberInput
            | Role::PasswordInput
            | Role::PhoneNumberInput
            | Role::UrlInput
            | Role::SpinButton
            | Role::EditableComboBox => Some(Set::Text),
            _ if self.types => Some(Set::Text),
            Role::ComboBox => Some(Set::Choice),
            Role::Slider => self.range.zip(self.number).map(|_| Set::Number),
            _ if self.toggled.is_some() => Some(Set::Toggle),
            _ => None,
        }
    }

    /// `{id, window, role, label, value?, in?, rect, visible, enabled, …}`; `offset` takes its rectangle to
    /// main-window points, and is `None` while the window has not reported where it is.
    fn json(&self, viewport: ViewportId, offset: Option<Vec2>) -> Value {
        let rect = self.rect.filter(|r| r.is_finite()).zip(offset).map(|(rect, offset)| {
            let rect = rect.translate(offset);
            json!([rect.min.x, rect.min.y, rect.max.x, rect.max.y])
        });
        let mut out = Map::new();
        out.insert("id".into(), json!(format!("{:016x}", self.id)));
        out.insert("window".into(), json!(window_name(viewport)));
        out.insert("role".into(), json!(role_name(self.role)));
        out.insert("label".into(), json!(self.label));
        let value = match (&self.value, self.number) {
            (Some(text), _) => Some(json!(text)),
            (None, Some(n)) => Some(json!(n)),
            (None, None) => None,
        };
        if let Some(value) = value {
            out.insert("value".into(), value);
        }
        if let Some(within) = &self.within {
            out.insert("in".into(), json!(within.as_ref()));
        }
        out.insert("rect".into(), rect.unwrap_or(Value::Null));
        out.insert("visible".into(), json!(self.visible.is_some()));
        out.insert("enabled".into(), json!(self.enabled));
        if self.focused {
            out.insert("focused".into(), json!(true));
        }
        if let Some(toggled) = self.toggled {
            let toggled = match toggled {
                Toggled::False => json!(false),
                Toggled::True => json!(true),
                Toggled::Mixed => json!("mixed"),
            };
            out.insert("toggled".into(), toggled);
        }
        for (key, state) in [("selected", self.selected), ("expanded", self.expanded)] {
            if let Some(state) = state {
                out.insert(key.into(), json!(state));
            }
        }
        if let Some(set) = self.set() {
            out.insert("set".into(), json!(set.name()));
            if let (Set::Number, Some((min, max))) = (set, self.range) {
                out.insert("min".into(), json!(min));
                out.insert("max".into(), json!(max));
                if let Some(step) = self.step {
                    out.insert("step".into(), json!(step));
                }
            }
        }
        Value::Object(out)
    }
}

/// A role's name, in camel case (`textInput`).
fn role_name(role: Role) -> String {
    let name = format!("{role:?}");
    let mut chars = name.chars();
    chars.next().map(|first| first.to_lowercase().chain(chars).collect()).unwrap_or_default()
}

/// `text` cut to [`MAX_ELEMENT_TEXT`] characters.
fn cut(text: &str) -> String {
    text.chars().take(MAX_ELEMENT_TEXT).collect()
}

/// The elements of one window.
struct Window {
    viewport: ViewportId,
    /// The main window's pass they were read in.
    pass: u64,
    elements: Vec<Element>,
    truncated: bool,
}

/// The plugin that keeps the elements of every window's last pass.
#[derive(Default)]
struct Tree {
    /// Main-window passes the trees are still kept for; 0 keeps nothing.
    wanted: u32,
    pass: u64,
    windows: Vec<Window>,
}

impl Plugin for Tree {
    fn debug_name(&self) -> &'static str {
        "fontcraft.elements"
    }

    fn on_end_pass(&mut self, ui: &mut Ui) {
        if self.wanted == 0 {
            return;
        }
        let ctx = ui.ctx().clone();
        if ctx.will_discard() {
            return;
        }
        let viewport = ctx.viewport_id();
        let focus = ctx.memory(|memory| memory.focused());
        let read = ctx.viewport(|state| {
            let nodes = state.this_pass.accesskit_state.as_ref()?;
            let focus = nodes.focus_override.or(focus).map(|id| id.accesskit_id());
            Some(read(&nodes.nodes, &state.this_pass.widgets, egui::accesskit_root_id().accesskit_id(), focus))
        });
        if let Some((elements, truncated)) = read {
            self.windows.retain(|w| w.viewport != viewport);
            let window = Window { viewport, pass: self.pass, elements, truncated };
            if viewport == ViewportId::ROOT { self.windows.insert(0, window) } else { self.windows.push(window) }
        }
        if viewport == ViewportId::ROOT {
            // The other windows are drawn inside the main window's pass: one not seen in it has closed.
            let pass = self.pass;
            self.windows.retain(|w| w.pass == pass);
            self.pass = self.pass.wrapping_add(1);
            self.wanted -= 1;
            if self.wanted == 0 {
                self.windows.clear();
            }
        }
    }
}

/// A node still to visit: the label around it and the rectangle that shows what is inside its parent.
struct Visit {
    node: NodeId,
    within: Option<Arc<str>>,
    clip: Option<Rect>,
}

/// The elements of a tree in reading order, and whether some were left out by the limits. Hidden nodes and what is
/// below them are left out, and so are the text runs of a field and nameless containers.
fn read(nodes: &IdMap<Node>, widgets: &WidgetRects, root: NodeId, focus: Option<NodeId>) -> (Vec<Element>, bool) {
    let by_node: HashMap<NodeId, (egui::Id, &Node)> = nodes.iter().map(|(id, node)| (id.accesskit_id(), (*id, node))).collect();
    let mut elements = Vec::new();
    let mut seen: HashSet<NodeId> = HashSet::new();
    let mut stack = vec![Visit { node: root, within: None, clip: None }];
    while let Some(visit) = stack.pop() {
        if seen.len() >= MAX_NODES || elements.len() >= MAX_WINDOW_ELEMENTS {
            return (elements, true);
        }
        if !seen.insert(visit.node) {
            continue;
        }
        let Some((id, node)) = by_node.get(&visit.node).copied() else { continue };
        if node.is_hidden() || node.role() == Role::TextRun {
            continue;
        }
        let widget = widgets.get(id);
        let bounds = bounds(node).or(widget.map(|w| w.rect));
        let shown = match widget {
            Some(widget) => widget.visible.then_some(widget.interact_rect),
            None => bounds.map(|b| visit.clip.map_or(b, |clip| b.intersect(clip))),
        };
        let label = label(node, &by_node);
        let acts = node.supports_action(Action::Click) || node.supports_action(Action::Focus);
        let container = matches!(node.role(), Role::Unknown | Role::GenericContainer);
        if visit.node != root && (!container || acts || !label.is_empty()) {
            elements.push(Element {
                id: visit.node.0,
                role: node.role(),
                label: label.clone(),
                value: node.value().map(cut),
                number: node.numeric_value().filter(|n| n.is_finite()),
                within: visit.within.clone(),
                rect: bounds,
                visible: shown.filter(|r| r.is_finite() && r.is_positive()),
                enabled: !node.is_disabled() && widget.is_none_or(|w| w.enabled),
                read_only: node.is_read_only(),
                focused: focus == Some(visit.node),
                toggled: node.toggled(),
                selected: node.is_selected(),
                expanded: node.is_expanded(),
                range: node.min_numeric_value().zip(node.max_numeric_value()).filter(|(lo, hi)| lo.is_finite() && hi.is_finite() && lo < hi),
                step: node.numeric_value_step().filter(|n| n.is_finite()),
                types: node.supports_action(Action::ReplaceSelectedText),
            });
        }
        let within = if label.is_empty() || visit.node == root { visit.within } else { Some(Arc::from(label)) };
        let clip = widget.map(|w| w.interact_rect).or(visit.clip);
        // The stack gives back the last one first.
        stack.extend(node.children().iter().rev().map(|child| Visit { node: *child, within: within.clone(), clip }));
    }
    (elements, false)
}

/// A node's bounds, in its window's points.
fn bounds(node: &Node) -> Option<Rect> {
    let b = node.bounds()?;
    Some(Rect::from_min_max(egui::pos2(b.x0 as f32, b.y0 as f32), egui::pos2(b.x1 as f32, b.y1 as f32)))
}

/// A node's label, else the labels of the nodes that label it.
fn label(node: &Node, nodes: &HashMap<NodeId, (egui::Id, &Node)>) -> String {
    /// Most labelling nodes read.
    const MAX_LABELS: usize = 8;
    if let Some(label) = node.label() {
        return cut(label);
    }
    let labels: Vec<&str> =
        node.labelled_by().iter().take(MAX_LABELS).filter_map(|id| nodes.get(id)).filter_map(|(_, n)| n.label().or(n.value())).collect();
    cut(&labels.join(" "))
}

/// Reads the windows' elements, the main window's first; an error while no pass with a tree has been drawn.
fn with_windows<R>(app: &FontCraftApp, method: &str, read: impl FnOnce(&[Window]) -> R) -> Result<R, String> {
    let ctx = app.window.as_ref().ok_or_else(|| format!("{method}: no window: the element list needs the desktop app or a test frame"))?;
    let tree = ctx.plugin_opt::<Tree>();
    let tree = tree.as_ref().map(|tree| tree.lock()).filter(|tree| !tree.windows.is_empty());
    let tree = tree.ok_or_else(|| format!("{method}: the window has not drawn since its elements were asked for; ask again after a frame"))?;
    Ok(read(&tree.windows))
}

/// What takes a point of `viewport` to main-window points.
fn offset(host: &WindowHost, viewport: ViewportId) -> Option<Vec2> {
    if viewport == ViewportId::ROOT {
        return Some(vec2(0.0, 0.0));
    }
    Some(host.facts(viewport)?.inner.min - host.facts(ViewportId::ROOT)?.inner.min)
}

/// `{prefix?, role?, window?, offset?, limit?}` → `{elements, total, truncated}`: the elements of the windows in
/// reading order, the main window's first; those whose label starts with `prefix` (in any case), with the role `role`
/// and in the window `window`. `total` counts all of them, `offset` and `limit` pick the ones listed.
fn list(app: &mut FontCraftApp, params: &Value) -> Result<Value, String> {
    const M: &str = "ui.elements";
    let params = object(M, params)?;
    let text = |key: &str| match params.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.to_lowercase())),
        Some(_) => Err(format!("{M}: `{key}` must be a string")),
    };
    let number = |key: &str, default: u64, max: u64| match params.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(v) => v.as_u64().filter(|n| *n <= max).ok_or_else(|| format!("{M}: `{key}` is a whole number from 0 to {max}")),
    };
    let (prefix, role, only) = (text("prefix")?, text("role")?, text("window")?);
    let first = number("offset", 0, u64::from(u32::MAX))?;
    let limit = number("limit", DEFAULT_LIMIT, MAX_LIMIT)?;
    let host = window(app, M)?;
    with_windows(app, M, |windows| {
        let (mut listed, mut total, mut truncated) = (Vec::new(), 0u64, false);
        for w in windows.iter().filter(|w| only.as_ref().is_none_or(|name| *name == window_name(w.viewport))) {
            truncated |= w.truncated;
            let offset = offset(&host, w.viewport);
            for element in &w.elements {
                let named = prefix.as_ref().is_none_or(|p| element.label.to_lowercase().starts_with(p.as_str()));
                if !named || role.as_ref().is_some_and(|r| role_name(element.role).to_lowercase() != *r) {
                    continue;
                }
                if total >= first && (listed.len() as u64) < limit {
                    listed.push(element.json(w.viewport, offset));
                }
                total += 1;
            }
        }
        json!({"elements": listed, "total": total, "truncated": truncated})
    })
}

/// The element a request names, and its window.
pub fn find(app: &FontCraftApp, method: &str, id: &Value) -> Result<(ViewportId, Element), String> {
    let text = id.as_str().ok_or_else(|| format!("{method}: `element` must be an element id (see ui.elements)"))?;
    let number = u64::from_str_radix(text, 16).map_err(|_| format!("{method}: `{}` is not an element id (see ui.elements)", cut(text)))?;
    let found = with_windows(app, method, |windows| {
        windows.iter().find_map(|w| w.elements.iter().find(|e| e.id == number).map(|e| (w.viewport, e.clone())))
    })?;
    found.ok_or_else(|| format!("{method}: no element `{text}` shows; list them again with ui.elements"))
}

impl Element {
    /// The middle of the part that shows, in its window's points, or why it cannot be pressed.
    pub fn press_point(&self, method: &str) -> Result<Pos2, String> {
        if !self.enabled {
            return Err(format!("{method}: `{}` is disabled", self.label));
        }
        self.visible.map(|r| r.center()).ok_or_else(|| format!("{method}: `{}` is scrolled out of view; scroll to it with ui.wheel", self.label))
    }
}

/// The options that show in lists labelled `label`, which is how the list of a select or a field's suggestions is.
fn options(app: &FontCraftApp, method: &str, viewport: ViewportId, label: &str) -> Result<Vec<Element>, String> {
    with_windows(app, method, |windows| {
        let elements = windows.iter().filter(|w| w.viewport == viewport).flat_map(|w| &w.elements);
        elements.filter(|e| e.role == Role::ListBoxOption && e.within.as_deref().unwrap_or_default() == label).cloned().collect()
    })
}

/// Whether the keys pressed now go to `element`: it has the keyboard, or an option of its list stands for it.
fn has_keyboard(app: &FontCraftApp, method: &str, target: &Target) -> Result<bool, String> {
    Ok(target.element.focused || options(app, method, target.viewport, &target.element.label)?.iter().any(|o| o.focused))
}

/// Whether the keys pressed now go to something else than `element` and its open list.
fn keyboard_elsewhere(app: &FontCraftApp, method: &str, target: &Target) -> Result<bool, String> {
    let label = target.element.label.as_str();
    with_windows(app, method, |windows| {
        let mut focused = windows.iter().filter(|w| w.viewport == target.viewport).flat_map(|w| &w.elements).filter(|e| e.focused);
        focused.any(|e| e.id != target.element.id && e.label != label && e.within.as_deref().unwrap_or_default() != label)
    })
}

/// What a round of `ui.set` came to.
enum Round {
    Done(Value),
    /// The next round runs once frames have been drawn.
    More(Next),
}

/// The window and the element a round of `ui.set` acts on.
struct Target {
    host: WindowHost,
    viewport: ViewportId,
    element: Element,
}

impl Target {
    /// Clicks the middle of what shows of `element`; its window input steps.
    fn click(&self, method: &str, element: &Element) -> Result<usize, String> {
        let at = element.press_point(method)?;
        focus_on_press(&self.host, self.viewport);
        self.host.inject_click(self.viewport, at, PointerButton::Primary, 1, Mods::NONE, false);
        Ok(3)
    }

    /// Presses `key`; its window input steps.
    fn key(&self, key: Key, mods: Mods) -> usize {
        self.host.inject_key(self.viewport, key, mods, false);
        2
    }
}

/// `ui.set {element, value}`: a check box, switch or toggle takes `true` or `false` and is clicked when it is not
/// there already; a text or number field takes a text or a number: it is clicked, and once it has the keyboard its text
/// is selected and replaced, and Tab commits it, after Escape has closed the suggestions that would take the Tab; a select takes the label of an
/// option, which is picked from its list; a slider takes a number and is dragged there. The reply is
/// `{changed: false}`, or `{queued, window, changed: true}` with the `value` a slider reached.
pub fn set(app: &mut FontCraftApp, params: &Value) -> Result<Value, String> {
    const M: &str = "ui.set";
    let earlier = app.control.stage.take();
    let params = object(M, params)?.into_owned();
    if let Some(key) = params.keys().find(|k| !matches!(k.as_str(), "element" | "value")) {
        return Err(format!("{M}: `{}` does not go with `element`, which takes `value` alone", cut(key)));
    }
    let value = params.get("value").ok_or_else(|| format!("{M}: `value` is required with `element`"))?;
    let (viewport, element) = find(app, M, params.get("element").unwrap_or(&Value::Null))?;
    let id = element.id;
    let earlier = earlier.filter(|stage| stage.element == id && stage.value == *value);
    let target = Target { host: window(app, M)?, viewport, element };
    let next = earlier.as_ref().map(|stage| stage.next);
    let round = earlier.as_ref().map_or(0, |stage| stage.round);
    let before = earlier.map_or(0, |stage| stage.queued);
    let came_to = match target.element.set() {
        None if target.element.read_only => return Err(format!("{M}: `{}` is read-only", target.element.label)),
        None => {
            let role = role_name(target.element.role);
            return Err(format!("{M}: `{}` is a {role}, which takes no value; use ui.click, ui.drag or ui.key", target.element.label));
        }
        Some(Set::Toggle) => toggle(app, &target, value)?,
        Some(Set::Text) => type_text(app, &target, value, next)?,
        Some(Set::Choice) => choose(app, &target, value, next)?,
        Some(Set::Number) => slide(app, &target, value, round, next)?,
    };
    let steps = before.saturating_add(app.control.queued);
    match came_to {
        Round::Done(mut reply) => {
            if steps > 0 {
                super::set(&mut reply, "queued", json!(steps));
                super::set(&mut reply, "window", json!(window_name(viewport)));
            }
            Ok(reply)
        }
        Round::More(next) => {
            app.control.stage = Some(Stage { element: id, value: value.clone(), round: round.saturating_add(1), queued: steps, next });
            Ok(json!({"unfinished": true}))
        }
    }
}

fn toggle(app: &mut FontCraftApp, target: &Target, value: &Value) -> Result<Round, String> {
    const M: &str = "ui.set";
    let (element, label) = (&target.element, &target.element.label);
    let want = value.as_bool().ok_or_else(|| format!("{M}: `{label}` takes true or false"))?;
    match element.toggled {
        Some(Toggled::Mixed) | None => return Err(format!("{M}: `{label}` is mixed; click it with ui.click and look again")),
        Some(now) if (now == Toggled::True) == want => return Ok(Round::Done(json!({"changed": false}))),
        Some(_) if !want && matches!(element.role, Role::RadioButton | Role::MenuItemRadio) => {
            return Err(format!("{M}: `{label}` is turned off by choosing another option"));
        }
        Some(_) => {}
    }
    let steps = target.click(M, element)?;
    queued(app, steps, target.viewport);
    Ok(Round::Done(json!({"changed": true})))
}

/// The text a value stands for, for an element that takes one.
fn text_of(label: &str, value: &Value) -> Result<String, String> {
    let text = match value {
        Value::String(text) => text.clone(),
        Value::Number(n) => n.to_string(),
        _ => return Err(format!("ui.set: `{label}` takes a text or a number")),
    };
    if text.chars().nth(MAX_SET_TEXT).is_some() {
        return Err(format!("ui.set: a value holds at most {MAX_SET_TEXT} characters"));
    }
    Ok(text)
}

fn type_text(app: &mut FontCraftApp, target: &Target, value: &Value, next: Option<Next>) -> Result<Round, String> {
    const M: &str = "ui.set";
    let (element, label) = (&target.element, &target.element.label);
    let showing = !options(app, M, target.viewport, label)?.is_empty();
    if let Some(Next::Commit { options }) = next {
        // A field that lost the keyboard has what was typed; the keys would go to something else.
        if !has_keyboard(app, M, target)? {
            return Ok(Round::Done(json!({"changed": true})));
        }
        // Escape closes the suggestions and nothing else while they show; without them it would restore the text.
        let mut steps = if showing && !options { target.key(Key::Escape, Mods::NONE) } else { 0 };
        steps += target.key(Key::Tab, Mods::NONE);
        queued(app, steps, target.viewport);
        return Ok(Round::Done(json!({"changed": true})));
    }
    let text = text_of(label, value)?;
    let lines = element.role == Role::MultilineTextInput;
    if text.chars().any(|c| c.is_control() && !(lines && c == '\n')) {
        return Err(format!("{M}: `{label}` takes no control characters"));
    }
    if !matches!(next, Some(Next::Focused)) {
        let steps = target.click(M, element)?;
        queued(app, steps, target.viewport);
        return Ok(Round::More(Next::Focused));
    }
    if !has_keyboard(app, M, target)? {
        return Err(format!("{M}: `{label}` did not take the keyboard when it was clicked; something covers it"));
    }
    let mut command = Mods::NONE;
    command.cmd = true;
    let mut steps = target.key(Key::A, command);
    if text.is_empty() {
        steps += target.key(Key::Backspace, Mods::NONE);
    } else {
        target.host.inject_text(target.viewport, &text);
        steps += 1;
    }
    queued(app, steps, target.viewport);
    Ok(Round::More(Next::Commit { options: showing }))
}

fn choose(app: &mut FontCraftApp, target: &Target, value: &Value, next: Option<Next>) -> Result<Round, String> {
    const M: &str = "ui.set";
    let (element, label) = (&target.element, &target.element.label);
    let want = cut(text_of(label, value)?.trim());
    if want.is_empty() || want.chars().any(char::is_control) {
        return Err(format!("{M}: `{label}` takes the label of one of its options"));
    }
    let open = element.expanded == Some(true);
    let typed = match next {
        Some(Next::Pick { typed }) => typed,
        _ if !open && element.value.as_deref() == Some(want.as_str()) => return Ok(Round::Done(json!({"changed": false}))),
        _ if !open => {
            let steps = target.click(M, element)?;
            queued(app, steps, target.viewport);
            return Ok(Round::More(Next::Pick { typed: false }));
        }
        _ => false,
    };
    if !open {
        return Err(format!("{M}: the list of `{label}` did not open"));
    }
    let options = options(app, M, target.viewport, label)?;
    let folded = want.to_lowercase();
    let mut alike = options.iter().filter(|o| o.label.to_lowercase() == folded);
    let found = options.iter().find(|o| o.label == want).or_else(|| alike.next().filter(|_| alike.next().is_none()));
    if let Some(option) = found.filter(|o| o.enabled && o.visible.is_some()) {
        let steps = target.click(M, option)?;
        queued(app, steps, target.viewport);
        return Ok(Round::Done(json!({"changed": true})));
    }
    if found.is_none_or(|o| o.enabled) && !typed {
        if keyboard_elsewhere(app, M, target)? {
            return Err(format!("{M}: the list of `{label}` does not have the keyboard, so `{want}` was not typed into it"));
        }
        // Typing moves the list to the options that start with the text, and narrows one with a filter field.
        target.host.inject_text(target.viewport, &want);
        queued(app, 1, target.viewport);
        return Ok(Round::More(Next::Pick { typed: true }));
    }
    // Escape puts back the value the list opened with.
    let steps = target.key(Key::Escape, Mods::NONE);
    queued(app, steps, target.viewport);
    if found.is_some() {
        return Err(format!("{M}: `{want}` cannot be chosen in `{label}`"));
    }
    let names: Vec<&str> = options.iter().take(MAX_NAMED_OPTIONS).map(|o| o.label.as_str()).collect();
    Err(format!("{M}: `{label}` shows no option `{want}`; it shows {}", names.join(", ")))
}

fn slide(app: &mut FontCraftApp, target: &Target, value: &Value, round: u32, next: Option<Next>) -> Result<Round, String> {
    const M: &str = "ui.set";
    let (element, label) = (&target.element, &target.element.label);
    let want = value.as_f64().filter(|n| n.is_finite()).ok_or_else(|| format!("{M}: `{label}` takes a number"))?;
    let (Some((min, max)), Some(now)) = (element.range, element.number) else {
        return Err(format!("{M}: `{label}` says no value to set"));
    };
    if want < min || want > max {
        return Err(format!("{M}: `{label}` takes a number from {min} to {max}"));
    }
    let span = max - min;
    let reached = (want - now).abs() <= span * SLIDE_SHARE;
    // A slider that stops at its own values stays where the last drag left it.
    let stuck = matches!(next, Some(Next::Slide { from }) if from == now);
    if reached || stuck || round >= MAX_SLIDES {
        return Ok(Round::Done(json!({"changed": next.is_some(), "value": now})));
    }
    let at = element.press_point(M)?;
    let rect = element.rect.filter(|r| r.is_finite() && r.width() > 0.0).ok_or_else(|| format!("{M}: `{label}` has no place in the window"))?;
    let thumb = THUMB.min(rect.width() / 2.0);
    let x = |v: f64| rect.left() + thumb / 2.0 + ((v - min) / span).clamp(0.0, 1.0) as f32 * (rect.width() - thumb);
    let (from, to) = (pos2(x(now), at.y), pos2(x(want), at.y));
    if element.visible.is_none_or(|r| !r.contains(from)) {
        return Err(format!("{M}: the thumb of `{label}` is scrolled out of view"));
    }
    let aside = if from.x + DRAG_START <= rect.right() { DRAG_START } else { -DRAG_START };
    focus_on_press(&target.host, target.viewport);
    target.host.inject_click(target.viewport, from, PointerButton::Primary, 1, Mods::NONE, true);
    for place in [from + vec2(aside, 0.0), to] {
        target.host.inject_move(target.viewport, place, Mods::NONE, 0.0);
    }
    target.host.inject_release(target.viewport, Some(to), Released::Button(PointerButton::Primary));
    queued(app, 5, target.viewport);
    Ok(Round::More(Next::Slide { from: now }))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::testing::TestApp;

    fn named(t: &mut TestApp, prefix: &str) -> Vec<Value> {
        let list = t.ok("ui.elements", json!({"prefix": prefix}));
        list["elements"].as_array().cloned().unwrap_or_default()
    }

    /// An agent finds a panel's field by its label, sets it, and reads the new value back.
    #[test]
    fn a_panel_field_is_found_set_and_read_back() {
        let mut t = TestApp::new();
        t.ok("engine.execute", json!({"command": "font.info", "params": {}}));
        t.frames(3);
        let field = |t: &mut TestApp| {
            let all = named(t, "family name");
            all.iter().find(|e| e["role"] == "textInput").cloned().unwrap_or_else(|| panic!("no Family Name field: {all:?}"))
        };
        let family = field(&mut t);
        assert_eq!(family["set"], "text");
        assert_eq!(family["visible"], true, "{family}");
        let reply = t.ok("ui.set", json!({"element": family["id"], "value": "Listed Sans"}));
        assert_eq!(reply["changed"], true);
        t.frames(3);
        assert_eq!(t.app.session.font().unwrap().family_name, "Listed Sans");
        assert_eq!(field(&mut t)["value"], "Listed Sans");

        // A tab is clicked by its element.
        let notes = named(&mut t, "notes");
        let tab = notes.iter().find(|e| e["visible"] == true && e["enabled"] == true).cloned().unwrap_or_else(|| panic!("no Notes tab: {notes:?}"));
        t.ok("ui.click", json!({"element": tab["id"]}));
        t.frames(3);
        assert_eq!(t.app.view.font_info.tab, crate::shell::font_info::FontInfoTab::Notes);

        // Limits and refusals.
        let page = t.ok("ui.elements", json!({"limit": 2, "offset": 1}));
        assert_eq!(page["elements"].as_array().map(Vec::len), Some(2));
        assert!(page["total"].as_u64() > Some(2));
        for (method, bad) in [
            ("ui.elements", json!({"limit": MAX_LIMIT + 1})),
            ("ui.elements", json!({"prefix": 3})),
            ("ui.click", json!({"element": "nope"})),
            ("ui.click", json!({"element": "00000000000000ff"})),
            ("ui.click", json!({"element": tab["id"], "x": 1, "y": 1})),
            ("ui.set", json!({"element": tab["id"], "value": true, "layers": true})),
            ("ui.set", json!({"element": family["id"]})),
        ] {
            assert!(!t.error(method, bad).is_empty());
        }
    }

    /// A click that does not give the field the keyboard types nothing: the keys would go to what took it.
    #[test]
    fn a_field_that_did_not_get_the_keyboard_is_not_typed_into() {
        let mut t = TestApp::new();
        t.ok("engine.execute", json!({"command": "font.info", "params": {}}));
        t.frames(3);
        let before = t.app.session.font().unwrap().clone();
        let fields = t.ok("ui.elements", json!({"role": "textInput"}));
        let fields = fields["elements"].as_array().cloned().unwrap_or_default();
        let fields: Vec<&Value> = fields.iter().filter(|e| e["set"] == "text" && e["visible"] == true && e["label"] != "").collect();
        let (Some(field), Some(other)) = (fields.first(), fields.get(1)) else { panic!("two fields: {fields:?}") };
        for value in ["", "Typed elsewhere"] {
            let params = json!({"element": field["id"], "value": value});
            for _ in 0..crate::control::frames_before(&t.app, "ui.set", &params) {
                t.frame();
            }
            crate::control::handle(&mut t.app, &json!({"id": 1, "method": "ui.set", "params": params}));
            t.frames(6);
            // Something else takes the keyboard before the next round.
            let stage = t.app.control.stage.take();
            assert!(stage.is_some());
            t.ok("ui.click", json!({"element": other["id"]}));
            t.app.control.stage = stage;
            assert!(t.error("ui.set", params).contains("did not take the keyboard"));
            t.frames(3);
            assert!(t.app.session.font().unwrap().same_as(&before));
        }
    }

    /// A select takes the label of an option, a slider a number, and a field keeps the text it was given while its
    /// suggestions show.
    #[test]
    fn a_select_a_slider_and_a_field_with_suggestions_are_set() {
        let mut t = TestApp::new();
        let one = |t: &mut TestApp, prefix: &str, role: &str| {
            let all = named(t, prefix);
            all.iter().find(|e| e["role"] == role && e["visible"] == true).cloned().unwrap_or_else(|| panic!("no {role} {prefix}: {all:?}"))
        };
        let size = one(&mut t, "cell size", "slider");
        assert_eq!((&size["set"], &size["min"]), (&json!("number"), &json!(crate::app::font_view::MIN_CELL_SIZE)));
        for want in [200.0, 203.0] {
            let reply = t.ok("ui.set", json!({"element": size["id"], "value": want}));
            t.frames(3);
            let now = one(&mut t, "cell size", "slider")["value"].as_f64().unwrap_or_default();
            assert!((now - want).abs() < 1.0, "{want}: {now}, {reply}");
        }
        assert!(t.error("ui.set", json!({"element": size["id"], "value": 1.0e9})).contains("from"));

        t.ok("engine.execute", json!({"command": "font.info", "params": {}}));
        t.ok("engine.execute", json!({"command": "font.info.selectTab", "params": {"tab": "other"}}));
        t.frames(3);
        let kind = one(&mut t, "font type", "comboBox");
        assert_eq!(kind["set"], "choice");
        assert!(t.error("ui.set", json!({"element": kind["id"], "value": "Sideways"})).contains("Variable"));
        t.frames(3);
        assert_eq!(t.ok("ui.set", json!({"element": kind["id"], "value": "Variable"}))["changed"], true);
        t.frames(3);
        assert_eq!(one(&mut t, "font type", "comboBox")["value"], "Variable");
        assert_eq!(t.ok("ui.set", json!({"element": kind["id"], "value": "Variable"}))["changed"], false);
        t.ok("engine.execute", json!({"command": "font.info", "params": {}}));

        t.ok("ui.set", json!({"commandConsole": true}));
        t.frames(3);
        let line = one(&mut t, "command", "textInput");
        t.ok("ui.set", json!({"element": line["id"], "value": "glyph.new"}));
        t.frames(3);
        assert_eq!(one(&mut t, "command", "textInput")["value"], "glyph.new", "the Tab took no suggestion");
    }
}
