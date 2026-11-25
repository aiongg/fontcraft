//! The element list: `ui.elements`, and what `ui.click {element}` and `ui.set {element, value}` act on.
//!
//! egui builds an AccessKit tree per window and pass while it is asked to (no Cargo feature: only the platform
//! adapters of eframe are behind one). [`activate`] asks for it and adds [`Tree`], an egui plugin that keeps the
//! elements of the last pass of every window, the main one and the floating ones, which are all drawn inside the main
//! window's pass. It keeps them for [`KEEP_PASSES`] passes after the last request, then drops them.
//!
//! A click or a change goes through the window's input at the element's visible place, so it does what the mouse and
//! the keyboard do there: nothing reaches an element that is disabled, hidden or scrolled out of view.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use craft::behavior::PointerButton;
use craft::keymap::{Key, Mods};
use craft::ui::runtime::windows::WindowHost;
use egui::accesskit::{Action, Node, NodeId, Role, Toggled};
use egui::{Context, IdMap, Plugin, Pos2, Rect, Ui, Vec2, ViewportId, WidgetRects, vec2};
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
            out.insert("set".into(), json!(if set == Set::Toggle { "bool" } else { "text" }));
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

/// `ui.set {element, value}`: a check box, switch or toggle takes `true` or `false` and is clicked when it is not
/// there already; a text or number field takes a text or a number: it is clicked, its text selected and replaced, and
/// Tab commits it. The reply is `{changed: false}` or the window input's `{queued, window}` with `changed: true`.
pub fn set(app: &mut FontCraftApp, params: &Value) -> Result<Value, String> {
    const M: &str = "ui.set";
    let params = object(M, params)?.into_owned();
    if let Some(key) = params.keys().find(|k| !matches!(k.as_str(), "element" | "value")) {
        return Err(format!("{M}: `{}` does not go with `element`, which takes `value` alone", cut(key)));
    }
    let value = params.get("value").ok_or_else(|| format!("{M}: `value` is required with `element`"))?;
    let (viewport, element) = find(app, M, params.get("element").unwrap_or(&Value::Null))?;
    let at = element.press_point(M)?;
    let label = &element.label;
    let host = window(app, M)?;
    let steps = match element.set() {
        None if element.read_only => return Err(format!("{M}: `{label}` is read-only")),
        None => {
            let role = role_name(element.role);
            return Err(format!("{M}: `{label}` is a {role}, which takes no value; use ui.click, ui.drag or ui.key"));
        }
        Some(Set::Toggle) => {
            let want = value.as_bool().ok_or_else(|| format!("{M}: `{label}` takes true or false"))?;
            match element.toggled {
                Some(Toggled::Mixed) | None => return Err(format!("{M}: `{label}` is mixed; click it with ui.click and look again")),
                Some(now) if (now == Toggled::True) == want => return Ok(json!({"changed": false})),
                Some(_) if !want && matches!(element.role, Role::RadioButton | Role::MenuItemRadio) => {
                    return Err(format!("{M}: `{label}` is turned off by choosing another option"));
                }
                Some(_) => {}
            }
            focus_on_press(&host, viewport);
            host.inject_click(viewport, at, PointerButton::Primary, 1, Mods::NONE, false);
            3
        }
        Some(Set::Text) => {
            let text = match value {
                Value::String(text) => text.clone(),
                Value::Number(n) => n.to_string(),
                _ => return Err(format!("{M}: `{label}` takes a text or a number")),
            };
            if text.chars().nth(MAX_SET_TEXT).is_some() {
                return Err(format!("{M}: a value holds at most {MAX_SET_TEXT} characters"));
            }
            let lines = element.role == Role::MultilineTextInput;
            if text.chars().any(|c| c.is_control() && !(lines && c == '\n')) {
                return Err(format!("{M}: `{label}` takes no control characters"));
            }
            focus_on_press(&host, viewport);
            host.inject_click(viewport, at, PointerButton::Primary, 1, Mods::NONE, false);
            let mut command = Mods::NONE;
            command.cmd = true;
            host.inject_key(viewport, Key::A, command, false);
            if text.is_empty() {
                host.inject_key(viewport, Key::Backspace, Mods::NONE, false);
            } else {
                host.inject_text(viewport, &text);
            }
            host.inject_key(viewport, Key::Tab, Mods::NONE, false);
            // The click's three steps, two for each key and one for a text.
            if text.is_empty() { 9 } else { 8 }
        }
    };
    let mut reply = queued(app, steps, viewport);
    super::set(&mut reply, "changed", json!(true));
    Ok(reply)
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
}
