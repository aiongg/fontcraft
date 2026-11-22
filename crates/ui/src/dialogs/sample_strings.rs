//! Sample strings: the Select Sample Text window (`editView.sampleStrings`) and the commands that put a stored sample
//! string into the edit view's text.
//!
//! The strings come from the user settings (`sampleStrings.groups`, a list of `{name, strings}`); while nothing is
//! stored, a built-in list of our own writing stands in. A string is written as the settings write it: `/name ` names
//! a glyph, `/Placeholder` is a placeholder, and the two characters `\n` are a line break.
//!
//! Choosing a string replaces the whole text (`editView.setText`). Next and Previous step through the strings of all
//! groups in order and wrap around at the ends.
//!
//! | In the window | Does |
//! |---|---|
//! | click a string | puts it into the text; the window closes unless it is pinned |
//! | Up, Down | puts the previous or next string into the text; the window stays open |
//! | Choose, Return | puts the selected string into the text and closes the window |
//! | Close, Escape | closes the window |
//! | Pin | keeps the window open after a click (`sampleStrings.togglePinned`) |

use std::borrow::Cow;

use craft::behavior::{ColumnKey, ColumnWidth, Selection, SelectionChange, keyed};
use craft::ui::buttons::ToggleButton;
use craft::ui::contract::{Bind, CollectionEvent, ColumnSpec, LeadSource, Props as _, SelectMode, Show as _, ShowCollection as _};
use craft::ui::dialog::{DialogHost, DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use craft::ui::display::{Shade, Text};
use craft::ui::list::ListRow;
use craft::ui::patterns::form_dialogs::FormDialog;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::Env;
use craft::ui::table::Table;
use egui::Ui;
use fontcraft_engine::Session;
use fontcraft_engine::settings::Reserved;
use serde_json::{Map, Value, json};

use crate::app::{FontCraftApp, Msg};
use crate::shell::commands::{Module, UiCommand};

craft::ui::words! {
    /// The words of the Select Sample Text window.
    pub struct SampleStringsWords {
        title: "fontcraftSampleStrings.title" = "Select Sample Text";
        choose: "fontcraftSampleStrings.choose" = "Choose";
        close: "fontcraftSampleStrings.close" = "Close";
        pin: "fontcraftSampleStrings.pin" = "Pin";
        pin_tip: "fontcraftSampleStrings.pinTip" = "Keep this window open after choosing a string";
        list: "fontcraftSampleStrings.list" = "Sample strings";
        group: "fontcraftSampleStrings.group" = "Group";
        string: "fontcraftSampleStrings.string" = "String";
        none: "fontcraftSampleStrings.none" = "No sample strings";
    }
}

/// Most groups read from or written to the settings.
pub const MAX_GROUPS: usize = 256;
/// Most strings read or written, over all groups.
pub const MAX_STRINGS: usize = 10_000;
/// Longest sample string, in bytes; longer ones are left out on reading and refused on writing.
pub const MAX_STRING_BYTES: usize = 4096;
/// Longest group name, in characters.
pub const MAX_NAME_CHARS: usize = 128;
/// Rows the list shows before it scrolls: the dialog body scrolls too, so the list must be capped.
const LIST_ROWS: u16 = 16;

const GROUP: ColumnKey = ColumnKey::new("group");
const STRING: ColumnKey = ColumnKey::new("string");

/// The window's state, part of the edit view's text state.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SampleStrings {
    pub open: bool,
    pub pinned: bool,
    /// The string chosen last, counted over all groups in order.
    pub current: Option<usize>,
}

/// One group of sample strings.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Group {
    pub name: String,
    pub strings: Vec<String>,
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

/// The groups the settings hold, within the limits; the built-in ones while the settings hold none or something
/// unreadable.
pub fn groups(session: &Session) -> Vec<Group> {
    let Ok(Some(Value::Array(stored))) = session.settings().reserved::<Value>(Reserved::SampleStrings) else { return default_groups() };
    let mut left = MAX_STRINGS;
    stored
        .iter()
        .take(MAX_GROUPS)
        .filter_map(|g| {
            let name = g.get("name").and_then(Value::as_str)?;
            let list = g.get("strings").and_then(Value::as_array)?;
            let strings: Vec<String> =
                list.iter().filter_map(Value::as_str).filter(|s| s.len() <= MAX_STRING_BYTES).take(left).map(str::to_string).collect();
            left = left.saturating_sub(strings.len());
            Some(Group { name: name.chars().take(MAX_NAME_CHARS).collect(), strings })
        })
        .collect()
}

/// Every string of every group, in order, with its group's name.
fn all_strings(groups: &[Group]) -> Vec<(&str, &str)> {
    groups.iter().flat_map(|g| g.strings.iter().map(move |s| (g.name.as_str(), s.as_str()))).collect()
}

/// A sample string as text: the two characters `\n` become a line break.
pub fn as_text(sample: &str) -> String {
    sample.replace("\\n", "\n")
}

pub const MODULE: Module = Module { commands: COMMANDS, run };

const COMMANDS: &[UiCommand] = &[
    UiCommand {
        id: "edit.selectSampleText",
        label: "Select Sample Text…",
        menu: &["Edit"],
        shortcut: Some("Cmd+Alt+F"),
        params: "{index?: integer, text?: string} without index or text, open the sample strings window; index puts that string (counted over all groups in order, from 0) into the edit view in place of its whole text, text puts the given string there; /name, /Placeholder and \\n are read as in the settings. The window closes after a choice unless it is pinned",
    },
    UiCommand {
        id: "edit.selectNextSampleString",
        label: "Select Next Sample String",
        menu: &["Edit", "Other"],
        shortcut: None,
        params: "{} replace the edit view's text with the sample string after the last one chosen (none chosen yet: the first); after the last string comes the first",
    },
    UiCommand {
        id: "edit.selectPreviousSampleString",
        label: "Select Previous Sample String",
        menu: &["Edit", "Other"],
        shortcut: None,
        params: "{} replace the edit view's text with the sample string before the last one chosen (none chosen yet: the last); before the first string comes the last",
    },
    UiCommand {
        id: "sampleStrings.togglePinned",
        label: "Pin Sample Strings Window",
        menu: &[],
        shortcut: None,
        params: "{on?: bool} keep the sample strings window open after a string is chosen (omitted: the other state)",
    },
];

fn run(app: &mut FontCraftApp, command: &CommandRef) -> Result<(), String> {
    let empty = Map::new();
    let p = command.params.as_object().unwrap_or(&empty);
    let id = command.id.as_str();
    if id == "sampleStrings.togglePinned" {
        let st = &mut app.view.text.samples;
        st.pinned = match p.get("on") {
            None | Some(Value::Null) => !st.pinned,
            Some(Value::Bool(b)) => *b,
            Some(_) => return Err("`on` must be true or false".into()),
        };
        return Ok(());
    }
    let groups = groups(&app.session);
    let strings = all_strings(&groups);
    let count = strings.len();
    let chosen = match id {
        "edit.selectSampleText" => match (p.get("index").filter(|v| !v.is_null()), p.get("text").filter(|v| !v.is_null())) {
            (Some(_), Some(_)) => return Err("give `index` or `text`, not both".into()),
            (Some(i), None) => {
                let i = i
                    .as_u64()
                    .and_then(|i| usize::try_from(i).ok())
                    .filter(|i| *i < count)
                    .ok_or_else(|| format!("`index` must be a whole number below the number of sample strings ({count})"))?;
                Some((Some(i), strings.get(i).map_or("", |s| s.1).to_string()))
            }
            (None, Some(text)) => {
                let text = text.as_str().ok_or("`text` must be a string")?;
                if text.len() > MAX_STRING_BYTES {
                    return Err(format!("`text` holds at most {MAX_STRING_BYTES} bytes"));
                }
                Some((strings.iter().position(|s| s.1 == text), text.to_string()))
            }
            (None, None) => None,
        },
        "edit.selectNextSampleString" | "edit.selectPreviousSampleString" => {
            if count == 0 {
                return Err("there are no sample strings".into());
            }
            let current = app.view.text.samples.current.filter(|c| *c < count);
            let i = match (current, id == "edit.selectNextSampleString") {
                (Some(c), true) => (c + 1) % count,
                (Some(c), false) => c.checked_sub(1).unwrap_or(count - 1),
                (None, true) => 0,
                (None, false) => count - 1,
            };
            Some((Some(i), strings.get(i).map_or("", |s| s.1).to_string()))
        }
        other => return Err(format!("unknown command `{other}`")),
    };
    let Some((at, sample)) = chosen else {
        app.view.text.samples.open = true;
        return Ok(());
    };
    crate::shell::commands::run(app, &CommandRef::with_params("editView.setText", json!({ "text": as_text(&sample) })))?;
    let st = &mut app.view.text.samples;
    st.current = at;
    if id == "edit.selectSampleText" && !st.pinned {
        st.open = false;
    }
    Ok(())
}

/// `ui.inspect`'s `editText.sampleStrings`: `{open, pinned, current, groups: [{name, strings}]}`.
pub fn reply(app: &FontCraftApp) -> Value {
    let st = &app.view.text.samples;
    let groups: Vec<Value> = groups(&app.session).into_iter().map(|g| json!({"name": g.name, "strings": g.strings})).collect();
    json!({"open": st.open, "pinned": st.pinned, "current": st.current, "groups": groups})
}

/// The window on the dialog host.
pub struct SampleStringsDialog {
    id: DialogId,
    /// Group name and string, over all groups in order.
    pub strings: Vec<(String, String)>,
    pub pinned: bool,
    pub selection: Selection<usize>,
    open: bool,
    /// What the person did since [`run_dialog`] last took it.
    picked: Vec<Msg>,
}

impl SampleStringsDialog {
    fn new(groups: &[Group], st: &SampleStrings) -> SampleStringsDialog {
        let strings = all_strings(groups).into_iter().map(|(g, s)| (g.to_string(), s.to_string())).collect();
        let selection = st.current.map_or_else(Selection::default, Selection::single);
        SampleStringsDialog { id: dialog_id("editView.sampleStrings"), strings, pinned: st.pinned, selection, open: true, picked: Vec::new() }
    }
}

fn shell(id: &str, params: Value) -> Msg {
    Msg::Shell(CommandRef::with_params(id, params))
}

impl StackDialog for SampleStringsDialog {
    /// What the person did in the pass the window closed.
    type Result = Vec<Msg>;

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<Vec<Msg>>> {
        let words = Env::of(ui).words::<SampleStringsWords>();
        let SampleStringsDialog { strings, pinned, selection, open, picked, .. } = self;
        let before = selection.items.first().copied();
        let shown = FormDialog::new(words.title.clone(), Bind::bound(selection))
            .opened(Bind::bound(open))
            .ok_label(words.choose.clone())
            .cancel_label(words.close.clone())
            .show(ui, |ui, _, selection| {
                let pin = ToggleButton::new(words.pin.clone(), Bind::given(*pinned)).tooltip(words.pin_tip.clone()).show(ui);
                if pin.response.clicked() {
                    picked.push(shell("sampleStrings.togglePinned", json!({})));
                }
                if strings.is_empty() {
                    Text::new(words.none.clone()).shade(Shade::Muted).show(ui);
                    return false;
                }
                let columns = vec![
                    ColumnSpec::new(GROUP, words.group.clone(), ColumnWidth::Content),
                    ColumnSpec::new(STRING, words.string.clone(), ColumnWidth::Content),
                ];
                let positions: Vec<usize> = (0..strings.len()).collect();
                let source = keyed(&positions, |at| *at).with_text(|at| Cow::Owned(strings.get(*at).map_or_else(String::new, |s| s.1.clone())));
                let out = Table::<usize>::new(words.list.clone(), columns)
                    .select(SelectMode::Single)
                    .selection(Bind::bound(selection))
                    .max_rows(LIST_ROWS)
                    .show_items(ui, &source, &mut |at, icx| {
                        let Some((group, string)) = strings.get(*at) else { return ListRow::new("") };
                        ListRow::new(group.clone()).cells(icx.cells()).cell(STRING, string.clone())
                    });
                let by_keys = out.events.iter().any(|e| matches!(e, CollectionEvent::LeadMoved { by: LeadSource::Keys, .. }));
                for event in &out.events {
                    if let CollectionEvent::Selection(SelectionChange::Set(keys)) = event
                        && let Some(&at) = keys.last()
                        && Some(at) != before
                    {
                        // A click chooses the string; Up and Down step to the one before or after and keep the window.
                        picked.push(match (by_keys, before) {
                            (true, Some(b)) if at < b => shell("edit.selectPreviousSampleString", json!({})),
                            (true, Some(_)) => shell("edit.selectNextSampleString", json!({})),
                            _ => shell("edit.selectSampleText", json!({ "index": at })),
                        });
                    }
                }
                !selection.items.is_empty()
            });
        if let Some(selection) = shown.accepted {
            if let Some(&at) = selection.items.first() {
                picked.push(shell("edit.selectSampleText", json!({ "index": at })));
            }
            return Some(DialogResult::Ok(std::mem::take(picked)));
        }
        shown.closed.map(|_| DialogResult::Ok(std::mem::take(picked)))
    }
}

/// Opens the window when the text state asks for it, posts what the person did, and closes it when a choice closed
/// the window.
pub fn run_dialog(host: &mut DialogHost, app: &mut FontCraftApp) {
    let st = app.view.text.samples.clone();
    match host.get_mut::<SampleStringsDialog>() {
        None if st.open => {
            let groups = groups(&app.session);
            host.open(SampleStringsDialog::new(&groups, &st));
        }
        Some(dialog) => {
            app.sent.append(&mut dialog.picked);
            dialog.pinned = st.pinned;
            if !st.open {
                dialog.open = false;
            }
        }
        None => {}
    }
    if let Some(result) = host.take::<SampleStringsDialog>() {
        app.view.text.samples.open = false;
        if let DialogResult::Ok(mut msgs) = result {
            app.sent.append(&mut msgs);
        }
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]
mod tests {
    use std::sync::Arc;

    use craft::host::Host;

    use super::*;
    use crate::app::{APP, ViewMsg};
    use crate::dialogs::find_glyphs::FindGlyphsDialog;

    fn app() -> FontCraftApp {
        let mut app = FontCraftApp::with_edit_tab(Session::with_document(), Arc::new(Host::builder(APP).build()));
        app.sent.push(Msg::View(ViewMsg::SelectGlyph("A".into())));
        app.drain();
        app
    }

    fn shell_ok(app: &mut FontCraftApp, id: &str, params: Value) {
        app.status.clear();
        app.sent.push(shell(id, params));
        app.drain();
        assert!(app.status.is_empty(), "{id}: {}", app.status);
    }

    fn text(app: &FontCraftApp) -> Value {
        crate::canvas::text::reply(app)["text"].clone()
    }

    #[test]
    fn sample_strings_replace_the_text_step_and_wrap_and_the_window_follows_the_state() {
        let mut app = app();
        let count = all_strings(&default_groups()).len();
        let mut host = DialogHost::new();

        shell_ok(&mut app, "edit.selectSampleText", json!({}));
        assert!(app.view.text.samples.open);
        run_dialog(&mut host, &mut app);
        assert!(host.is_open::<SampleStringsDialog>(), "the window opens");
        assert_eq!(host.get::<SampleStringsDialog>().unwrap().strings.len(), count);

        shell_ok(&mut app, "edit.selectSampleText", json!({"index": 1}));
        assert_eq!(text(&app), json!("oooooooo"));
        assert_eq!(app.view.text.samples.current, Some(1));
        assert!(!app.view.text.samples.open, "a choice closes the window");

        shell_ok(&mut app, "edit.selectPreviousSampleString", json!({}));
        shell_ok(&mut app, "edit.selectPreviousSampleString", json!({}));
        assert_eq!(app.view.text.samples.current, Some(count - 1), "before the first comes the last");
        shell_ok(&mut app, "edit.selectNextSampleString", json!({}));
        assert_eq!(app.view.text.samples.current, Some(0), "after the last comes the first");

        shell_ok(&mut app, "sampleStrings.togglePinned", json!({}));
        shell_ok(&mut app, "edit.selectSampleText", json!({}));
        shell_ok(&mut app, "edit.selectSampleText", json!({"index": 6}));
        assert!(app.view.text.samples.open, "a pinned window stays open");
        let lines = text(&app);
        assert!(lines.as_str().unwrap().contains('\n'), "\\n in a sample string is a line break: {lines}");

        app.status.clear();
        app.sent.push(shell("edit.selectSampleText", json!({"index": count})));
        app.drain();
        assert!(!app.status.is_empty(), "an index past the strings is refused");
        assert!(!app.session.doc().unwrap().modified(), "the text is view state");
    }

    #[test]
    fn find_glyphs_searches_and_inserts_the_selected_results() {
        let mut app = app();
        shell_ok(&mut app, "editView.setText", json!({"glyphs": ["A"]}));
        shell_ok(&mut app, "edit.find", json!({"query": "V"}));
        let st = app.view.text.find.clone();
        assert!(st.open && st.results.iter().any(|r| r.name == "V"), "{st:?}");
        assert_eq!(st.selected.len(), 1, "a new search selects its first result");
        let mut host = DialogHost::new();
        crate::dialogs::find_glyphs::run_dialog(&mut host, &mut app);
        assert!(host.is_open::<FindGlyphsDialog>());

        app.status.clear();
        app.sent.push(shell("edit.find", json!({"select": ["nope"]})));
        app.drain();
        assert!(!app.status.is_empty(), "a glyph that is not a result is refused");

        shell_ok(&mut app, "edit.find", json!({"select": ["V"], "insert": true}));
        assert_eq!(crate::canvas::text::reply(&app)["items"], json!(["A", "V"]));
        assert!(!app.view.text.find.open, "inserting closes the pop-up");
    }
}
