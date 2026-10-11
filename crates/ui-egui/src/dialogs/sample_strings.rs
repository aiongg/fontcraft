//! Sample strings: the Select Sample Text window (`editView.sampleStrings`) and the commands
//! that put a stored sample string into an Edit View tab.
//!
//! The strings come from the user settings (`sampleStrings.groups`, a list of `{name, strings}`
//! that the settings store keeps between sessions); while nothing is stored, a built-in list of
//! our own writing stands in. A string is written as the settings write it: `/name ` names a
//! glyph, `/Placeholder` is a placeholder, and the two characters `\n` are a line break.
//!
//! Choosing a string replaces the whole text of the tab (`editView.setText`). Next and Previous
//! step through the strings of all groups in order and wrap around at the ends.
//!
//! | In the window | Does |
//! |---|---|
//! | click a string | puts it into the tab; the window closes unless it is pinned |
//! | Up, Down | puts the previous or next string into the tab; the window stays open |
//! | Return, Escape | closes the window |
//! | Pin | keeps the window open after a click (`sampleStrings.togglePinned`) |

use egui::{Rect, Sense, vec2};
use fontcraft_engine::Session;
use fontcraft_engine::settings::Reserved;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::app::FontcraftApp;
use crate::shell::{Args, UiCommand};

type Pending = Vec<(&'static str, Value)>;

/// Most groups read from the settings.
pub const MAX_GROUPS: usize = 256;
/// Most strings read from the settings, over all groups.
pub const MAX_STRINGS: usize = 10_000;
/// Longest sample string read from the settings, in bytes; longer ones are left out.
pub const MAX_STRING_BYTES: usize = 4096;
/// Longest group name shown, in characters.
const MAX_NAME_CHARS: usize = 128;
/// Strings the window shows before it scrolls.
const VISIBLE_ROWS: f32 = 16.0;
/// Widest the window grows, in points.
const WINDOW_WIDTH: f32 = 420.0;

pub const COMMANDS: &[UiCommand] = &[
    (
        "edit.selectSampleText",
        "Select Sample Text…",
        &["Edit"],
        "Cmd+Alt+F",
        "{tab?: id, index?: integer, text?: string} without index or text, open the sample strings window; index puts that string (counted over all groups in order, from 0) into the tab in place of its whole text, text puts the given string there; /name, /Placeholder and \\n are read as in the settings. The window closes after a choice unless it is pinned → {open, pinned, current, groups: [{name, strings}]} | the tab's text as editView.getText gives it, with index",
    ),
    (
        "edit.selectNextSampleString",
        "Select Next Sample String",
        &["Edit", "Other"],
        "",
        "{tab?: id} replace the tab's text with the sample string after the last one chosen (none chosen yet: the first); after the last string comes the first → the tab's text as editView.getText gives it, with index",
    ),
    (
        "edit.selectPreviousSampleString",
        "Select Previous Sample String",
        &["Edit", "Other"],
        "",
        "{tab?: id} replace the tab's text with the sample string before the last one chosen (none chosen yet: the last); before the first string comes the last → the tab's text as editView.getText gives it, with index",
    ),
    (
        "sampleStrings.togglePinned",
        "Pin Sample Strings Window",
        &[],
        "",
        "{on?: bool} keep the sample strings window open after a string is chosen (omitted: the other state) → {pinned}",
    ),
];

/// One group of sample strings, as the settings store it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Group {
    pub name: String,
    pub strings: Vec<String>,
}

/// The window's state.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SampleStringsState {
    pub open: bool,
    pub pinned: bool,
    /// The string chosen last, counted over all groups in order.
    pub current: Option<usize>,
    /// The window opened this frame: the list takes the keyboard.
    #[serde(skip)]
    pub fresh: bool,
    /// The strings laid out in the last frame with their rectangles, and the pin button: tests
    /// click them.
    #[serde(skip)]
    pub laid_out: Vec<(usize, Rect)>,
    #[serde(skip)]
    pub pin_button: Option<Rect>,
}

/// The built-in groups, used while the settings hold none.
pub fn default_groups() -> Vec<Group> {
    let group = |name: &str, strings: &[&str]| Group { name: name.into(), strings: strings.iter().map(|s| (*s).to_string()).collect() };
    vec![
        group(
            "Spacing",
            &[
                "nnnnnnnn",
                "oooooooo",
                "nnonnoonno",
                "nn/Placeholder nn oo/Placeholder oo",
                "HHOHHOOHHO",
                "HH/Placeholder HH OO/Placeholder OO",
                "HnHoHnHo\\nnHnoHonHo",
            ],
        ),
        group("Figures", &["0123456789", "H0H1H2H3H4H5H6H7H8H9H", "n0n1n2n3n4n5n6n7n8n9n"]),
        group(
            "Latin",
            &[
                "Hamburgefonstiv\\nHAMBURGEFONSTIV",
                "Five quacking zephyrs jolt my wax bed.",
                "Jackdaws love my big sphinx of quartz.",
                "Pack my box with five dozen liquor jugs.",
            ],
        ),
        group("Greek", &["Ξεσκεπάζω την ψυχοφθόρα βδελυγμία.", "ΞΕΣΚΕΠΑΖΩ ΤΗΝ ΨΥΧΟΦΘΟΡΑ ΒΔΕΛΥΓΜΙΑ"]),
        group("Cyrillic", &["Съешь же ещё этих мягких французских булок, да выпей чаю.", "СЪЕШЬ ЖЕ ЕЩЁ ЭТИХ МЯГКИХ ФРАНЦУЗСКИХ БУЛОК"]),
    ]
}

/// The groups the settings hold, within the limits; the built-in ones while the settings hold
/// none or something unreadable.
pub fn groups(session: &Session) -> Vec<Group> {
    let Ok(Some(stored)) = session.settings().reserved::<Vec<Group>>(Reserved::SampleStrings) else { return default_groups() };
    let mut left = MAX_STRINGS;
    stored
        .into_iter()
        .take(MAX_GROUPS)
        .map(|g| {
            let strings: Vec<String> = g.strings.into_iter().filter(|s| s.len() <= MAX_STRING_BYTES).take(left).collect();
            left = left.saturating_sub(strings.len());
            Group { name: g.name.chars().take(MAX_NAME_CHARS).collect(), strings }
        })
        .collect()
}

/// Every string of every group, in order.
fn all_strings(groups: &[Group]) -> Vec<&str> {
    groups.iter().flat_map(|g| g.strings.iter().map(String::as_str)).collect()
}

/// A sample string as tab text: the two characters `\n` become a line break.
pub fn as_text(sample: &str) -> String {
    sample.replace("\\n", "\n")
}

pub fn run(app: &mut FontcraftApp, id: &str, args: &Args) -> Result<Value, String> {
    if id == "sampleStrings.togglePinned" {
        let st = &mut app.state.panels.sample_strings;
        st.pinned = args.toggle(st.pinned)?;
        return Ok(json!({ "pinned": st.pinned }));
    }
    let index = crate::shell::view::target_tab(app, args)?;
    let tab = app.state.tabs.get(index).map(|t| t.id).ok_or_else(|| args.err("no such tab"))?;
    let groups = groups(&app.session);
    let strings = all_strings(&groups);
    let count = strings.len();
    let chosen = match id {
        "edit.selectSampleText" => match (args.index("index")?, args.string("text")?) {
            (Some(_), Some(_)) => return Err(args.err("give `index` or `text`, not both")),
            (Some(i), None) => {
                let i = usize::try_from(i)
                    .ok()
                    .filter(|i| *i < count)
                    .ok_or_else(|| args.err(format!("`index` must be below the number of sample strings ({count})")))?;
                Some((Some(i), strings.get(i).copied().unwrap_or_default().to_string()))
            }
            (None, Some(text)) => {
                if text.len() > MAX_STRING_BYTES {
                    return Err(args.err(format!("`text` holds at most {MAX_STRING_BYTES} bytes")));
                }
                Some((strings.iter().position(|s| *s == text), text.to_string()))
            }
            (None, None) => None,
        },
        "edit.selectNextSampleString" | "edit.selectPreviousSampleString" => {
            if count == 0 {
                return Err(args.err("there are no sample strings"));
            }
            let current = app.state.panels.sample_strings.current.filter(|c| *c < count);
            let i = match (current, id == "edit.selectNextSampleString") {
                (Some(c), true) => (c + 1) % count,
                (Some(c), false) => c.checked_sub(1).unwrap_or(count - 1),
                (None, true) => 0,
                (None, false) => count - 1,
            };
            Some((Some(i), strings.get(i).copied().unwrap_or_default().to_string()))
        }
        other => return Err(format!("unknown command `{other}`")),
    };
    let Some((at, sample)) = chosen else {
        let st = &mut app.state.panels.sample_strings;
        if !st.open {
            st.fresh = true;
        }
        st.open = true;
        return Ok(json!({ "open": true, "pinned": st.pinned, "current": st.current, "groups": groups }));
    };
    let mut reply = app.run("editView.setText", &json!({ "tab": tab, "text": as_text(&sample) }))?;
    let st = &mut app.state.panels.sample_strings;
    st.current = at;
    if id == "edit.selectSampleText" && !st.pinned {
        st.open = false;
    }
    if let Some(map) = reply.as_object_mut() {
        map.insert("index".into(), json!(at));
    }
    Ok(reply)
}

/// Draws the window while it is open and an Edit View tab shows. Choosing a string queues its
/// command in `pending`.
pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    let Some(tab) = app.state.tab().map(|t| t.id) else { return };
    if !app.state.panels.sample_strings.open {
        return;
    }
    let groups = groups(&app.session);
    let FontcraftApp { state, tokens, .. } = app;
    let st = &mut state.panels.sample_strings;
    let list_id = egui::Id::new("editView.sampleStrings.list");
    let mut laid_out = Vec::new();
    let mut pin_button = None;
    let mut open = true;
    let mut close = false;
    egui::Window::new("Select Sample Text")
        .id(egui::Id::new("editView.sampleStrings"))
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_width(WINDOW_WIDTH)
        .pivot(egui::Align2::CENTER_TOP)
        .default_pos(egui::pos2(ui.ctx().content_rect().center().x, tokens.dialog_top))
        .show(ui.ctx(), |ui| {
            let pin = ui.selectable_label(st.pinned, "Pin").on_hover_text("Keep this window open after choosing a string");
            pin_button = Some(pin.rect);
            if pin.clicked() {
                pending.push(("sampleStrings.togglePinned", json!({})));
            }
            ui.add_space(tokens.dialog_spacing);
            // The list holds the keyboard while the window is open, so Up and Down reach it
            // and not the text cursor.
            let list = ui.interact(ui.max_rect(), list_id, Sense::focusable_noninteractive());
            if st.fresh {
                list.request_focus();
                st.fresh = false;
            }
            let filter = egui::EventFilter { vertical_arrows: true, escape: true, ..Default::default() };
            ui.memory_mut(|m| m.set_focus_lock_filter(list_id, filter));
            if list.has_focus() || list.lost_focus() {
                let (up, down, done) = ui.input(|i| {
                    (
                        i.key_pressed(egui::Key::ArrowUp),
                        i.key_pressed(egui::Key::ArrowDown),
                        i.key_pressed(egui::Key::Enter) || i.key_pressed(egui::Key::Escape),
                    )
                });
                if up {
                    pending.push(("edit.selectPreviousSampleString", json!({ "tab": tab })));
                }
                if down {
                    pending.push(("edit.selectNextSampleString", json!({ "tab": tab })));
                }
                close |= done;
            }
            let height = tokens.list_row_height;
            egui::ScrollArea::vertical().id_salt("sampleStrings.v").max_height(height * VISIBLE_ROWS).show(ui, |ui| {
                let mut at = 0usize;
                if groups.iter().all(|g| g.strings.is_empty()) {
                    ui.colored_label(tokens.text_dim, "No sample strings");
                }
                for g in &groups {
                    ui.label(egui::RichText::new(&g.name).strong());
                    for s in &g.strings {
                        let shown = st.current == Some(at);
                        let response = ui.add_sized(vec2(ui.available_width(), height), egui::Button::selectable(shown, s.as_str()));
                        laid_out.push((at, response.rect));
                        if response.clicked() {
                            pending.push(("edit.selectSampleText", json!({ "tab": tab, "index": at })));
                            list.request_focus();
                        }
                        at += 1;
                    }
                }
            });
        });
    if !open || close {
        st.open = false;
    }
    st.laid_out = laid_out;
    st.pin_button = pin_button;
}
