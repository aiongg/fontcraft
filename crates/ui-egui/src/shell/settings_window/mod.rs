//! The Settings window (`app.settings`, `window.settings`): the seven sections in the reference's
//! order, a switcher on the left and the chosen section's body on the right. A section that is not
//! built yet is listed greyed with the reason, and `app.settings` refuses to open it.
//!
//! Each section is a [`Section`] in [`SECTIONS`]; a package fills its section by giving it a
//! `body`. A body reads settings with [`value`] and changes them only by queueing `settings.set`
//! (or its own commands) in `pending`, for example through [`checkbox`] and [`text_field`]. So
//! the sections do not depend on where the settings are stored.

pub mod addons;
pub mod user;

#[cfg(test)]
mod tests;

use std::collections::BTreeMap;

use egui::Rect;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{Args, UiCommand};
use crate::app::FontcraftApp;
use crate::panels::Pending;

pub const COMMANDS: &[UiCommand] = &[(
    "app.settings",
    "Settings…",
    &["Edit"],
    "Cmd+Comma",
    "{section?: \"updates\" | \"appearance\" | \"userSettings\" | \"sampleStrings\" | \"sharing\" | \"addons\" | \"shortcuts\", open?: bool} open the Settings window (open: false closes it, writing any text still being typed), on a section or else on the section last shown; a section that is not available yet is refused → {open, section, sections: [{id, label, available, reason?}]}",
)];

/// Draws a section's body. It reads settings with [`value`] and queues `settings.set` or other
/// commands in `pending`.
pub type SectionBody = fn(&mut FontcraftApp, &mut egui::Ui, &mut Pending);

/// One section of the Settings window.
#[derive(Clone, Copy)]
pub struct Section {
    /// What `app.settings {section}` takes.
    pub id: &'static str,
    pub label: &'static str,
    /// The body; `None` while the section is not built.
    pub body: Option<SectionBody>,
    /// Why the section is greyed while `body` is `None`.
    pub unavailable: &'static str,
}

const NOT_YET: &str = "Not available yet.";

/// The section the window shows when none was chosen.
const USER_SETTINGS: Section = Section { id: "userSettings", label: "User Settings", body: Some(user::show), unavailable: "" };

/// The sections, in the reference's order.
pub const SECTIONS: &[Section] = &[
    Section { id: "updates", label: "Updates", body: None, unavailable: "FontCraft does not look for new versions by itself." },
    Section {
        id: "appearance",
        label: "Appearance",
        body: None,
        unavailable: "Not available yet. Its settings can be changed with settings.set in the Command Console.",
    },
    USER_SETTINGS,
    Section { id: "sampleStrings", label: "Sample Strings", body: None, unavailable: NOT_YET },
    Section { id: "sharing", label: "Sharing", body: None, unavailable: NOT_YET },
    Section { id: "addons", label: "Extensions", body: Some(addons::show), unavailable: "" },
    Section { id: "shortcuts", label: "Shortcuts", body: None, unavailable: NOT_YET },
];

/// The window's state, kept in `UiState::panels`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SettingsWindowState {
    #[serde(skip)]
    pub open: bool,
    /// The section last shown; the window opens on it again.
    pub section: String,
    /// Text typed into a field and not written yet, by setting id.
    #[serde(skip)]
    pub drafts: BTreeMap<String, String>,
    /// Where the last frame drew the switcher entries (`section:<id>`) and the controls (by
    /// setting id).
    #[serde(skip)]
    pub laid_out: BTreeMap<String, Rect>,
}

/// The section the window shows: the one last chosen while it is available, else User Settings.
pub fn current(st: &SettingsWindowState) -> &'static Section {
    SECTIONS.iter().find(|s| s.id == st.section && s.body.is_some()).unwrap_or(&USER_SETTINGS)
}

fn report(st: &SettingsWindowState) -> Value {
    let sections: Vec<Value> = SECTIONS
        .iter()
        .map(|s| match s.body {
            Some(_) => json!({"id": s.id, "label": s.label, "available": true}),
            None => json!({"id": s.id, "label": s.label, "available": false, "reason": s.unavailable}),
        })
        .collect();
    json!({"open": st.open, "section": current(st).id, "sections": sections})
}

pub fn run(app: &mut FontcraftApp, id: &str, args: &Args) -> Result<Value, String> {
    if id != "app.settings" {
        return Err(format!("unknown command `{id}`"));
    }
    let open = args.flag("open")?.unwrap_or(true);
    let section = match args.string("section")? {
        None => None,
        Some(name) => {
            let Some(s) = SECTIONS.iter().find(|s| s.id == name) else {
                let ids: Vec<&str> = SECTIONS.iter().map(|s| s.id).collect();
                return Err(args.err(format!("there is no section {name:?}; the sections are {}", ids.join(", "))));
            };
            if s.body.is_none() {
                return Err(args.err(format!("the {} section is not available: {}", s.label, s.unavailable)));
            }
            Some(s)
        }
    };
    if !open {
        write_drafts(app);
    }
    let st = &mut app.state.panels.settings_window;
    if let Some(s) = section {
        st.section = s.id.to_string();
    }
    st.open = open;
    Ok(report(st))
}

/// Writes the text still being typed into the fields. A refused value is reported on the status
/// bar and the setting keeps its value.
fn write_drafts(app: &mut FontcraftApp) {
    let drafts = std::mem::take(&mut app.state.panels.settings_window.drafts);
    for (id, text) in drafts {
        let _ = app.run("settings.set", &json!({"id": id, "value": text}));
    }
}

/// A setting's value as `settings.get` gives it: the stored value or the default.
pub fn value(app: &FontcraftApp, id: &str) -> Value {
    app.session.settings().get(id).unwrap_or(Value::Null)
}

/// A checkbox for a true-or-false setting; a click runs `settings.set`.
pub fn checkbox(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending, id: &'static str, label: &str) -> egui::Response {
    let mut on = value(app, id).as_bool().unwrap_or(false);
    let response = ui.checkbox(&mut on, label);
    if response.changed() {
        pending.push(("settings.set", json!({"id": id, "value": on})));
    }
    app.state.panels.settings_window.laid_out.insert(id.to_string(), response.rect);
    response
}

/// A labelled one-line field for a text setting. What is typed is kept as a draft and written
/// with `settings.set` when the field loses focus (Return, a click elsewhere) or the window
/// closes; a refused value leaves the setting as it was.
pub fn text_field(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending, id: &'static str, label: &str) -> egui::Response {
    let stored = value(app, id).as_str().unwrap_or_default().to_string();
    let width = app.tokens.dialog_spacing * 32.0;
    let st = &mut app.state.panels.settings_window;
    let mut text = st.drafts.get(id).cloned().unwrap_or_else(|| stored.clone());
    let response = ui
        .horizontal(|ui| {
            ui.label(label);
            ui.add(egui::TextEdit::singleline(&mut text).id(egui::Id::new(("settings.field", id))).desired_width(width))
        })
        .inner;
    if response.changed() {
        st.drafts.insert(id.to_string(), text);
    }
    if response.lost_focus()
        && let Some(draft) = st.drafts.remove(id)
        && draft != stored
    {
        pending.push(("settings.set", json!({"id": id, "value": draft})));
    }
    st.laid_out.insert(id.to_string(), response.rect);
    response
}

/// A dim line under a control that says what it does in FontCraft.
pub fn note(app: &FontcraftApp, ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).small().color(app.tokens.text_dim));
}

/// Draws the window while it is open.
pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    if !app.state.panels.settings_window.open {
        return;
    }
    let section = current(&app.state.panels.settings_window);
    app.state.panels.settings_window.laid_out.clear();
    let tokens = app.tokens.clone();
    let mut open = true;
    egui::Window::new("Settings")
        .id(egui::Id::new("window.settings"))
        .open(&mut open)
        .collapsible(false)
        .resizable(true)
        .default_width(tokens.dialog_spacing * 90.0)
        .pivot(egui::Align2::CENTER_TOP)
        .default_pos(egui::pos2(ui.ctx().content_rect().center().x, tokens.dialog_top))
        .show(ui.ctx(), |ui| {
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(tokens.dialog_spacing * 18.0);
                    for s in SECTIONS {
                        let button = egui::Button::selectable(s.id == section.id, s.label);
                        let response = ui.add_enabled(s.body.is_some(), button).on_disabled_hover_text(s.unavailable);
                        if response.clicked() && s.id != section.id {
                            pending.push(("app.settings", json!({"section": s.id})));
                        }
                        app.state.panels.settings_window.laid_out.insert(format!("section:{}", s.id), response.rect);
                    }
                });
                ui.separator();
                ui.vertical(|ui| {
                    ui.heading(section.label);
                    ui.add_space(tokens.dialog_spacing);
                    egui::ScrollArea::vertical().id_salt(("window.settings", section.id)).show(ui, |ui| {
                        if let Some(body) = section.body {
                            body(app, ui, pending);
                        }
                    });
                });
            });
        });
    if !open {
        let st = &mut app.state.panels.settings_window;
        st.open = false;
        for (id, text) in std::mem::take(&mut st.drafts) {
            pending.push(("settings.set", json!({"id": id, "value": text})));
        }
    }
}
