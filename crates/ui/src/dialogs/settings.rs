//! The Settings window (`app.settings`): the section switcher over the shown section's body. Its state and commands
//! are `shell/settings.rs`; this module shows it while `ViewState::settings.open` and posts what the person does.
//!
//! | Section | Body |
//! |---|---|
//! | User Settings | the `user.*` switches and the crash-report contact fields (`settings.set`) |
//! | Sample Strings | the groups, plus and minus, the shown group's strings one per line, Default (`settings.*SampleString*`) |
//! | Sharing | Enable External Preview (`settings.set`), the page's address, the devices waiting with Accept and Refuse (`sharing.answerDevice`), Reset while a device is trusted (`settings.resetTrustedDevices`) |
//! | Extensions | the Extension Manager's Indexes tab (`super::extension_indexes`) and Open Extension Manager… (`window.pluginManager`) |
//!
//! A device that asks to see External Preview while the window is closed gets a notice that asks Accept or Refuse
//! ([`run`]); Later leaves it waiting in the Sharing section.

use craft::ui::buttons::Button;
use craft::ui::choice::{Checkbox, Segmented};
use craft::ui::contract::{Bind, Entry, Props as _, Show as _, ShowContainer as _};
use craft::ui::dialog::{DialogHost, DialogId, DialogResult, DialogStack, PromptChoice, StackDialog, StandardPrompt, dialog_id};
use craft::ui::display::{Shade, Text};
use craft::ui::form::Field;
use craft::ui::layout_components::{Row, Stack};
use craft::ui::patterns::form_dialogs::FormDialog;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::{Env, Label, WordSet as _};
use craft::ui::style::TypeRole;
use craft::ui::text::TextField;
use egui::Ui;
use fontcraft_engine::cmd::sharing::{self, ENABLE_SETTING};
use serde_json::{Value, json};

use super::extension_indexes;
use crate::app::{FontCraftApp, Msg};
use crate::shell::extension_manager::{self, IndexesModel};
use crate::shell::settings::{self as settings_state, Group, SECTIONS};

craft::ui::words! {
    /// The words of the Settings window.
    pub struct SettingsWords {
        title: "fontcraftSettings.title" = "Settings";
        open_manager: "fontcraftSettings.openManager" = "Open Extension Manager…";
        sections: "fontcraftSettings.sections" = "Section";
        done: "fontcraftSettings.done" = "Done";
        close: "fontcraftSettings.close" = "Close";
        keep_names: "fontcraftSettings.keepNames" = "Keep Glyph Names from Imported Files";
        keep_names_note: "fontcraftSettings.keepNamesNote" = "Not used yet: opened files keep their glyph names either way.";
        no_alignment: "fontcraftSettings.noAlignment" = "Disable Automatic Alignment for Imported Files";
        no_alignment_note: "fontcraftSettings.noAlignmentNote" = "Components of an opened font stay where the file places them.";
        no_localization: "fontcraftSettings.noLocalization" = "Disable Localization";
        no_localization_note: "fontcraftSettings.noLocalizationNote" = "Keeps the interface in English. Takes effect after FontCraft restarts.";
        versions: "fontcraftSettings.versions" = "Use Versions";
        versions_note: "fontcraftSettings.versionsNote" = "Not used yet: FontCraft writes a file only when you save it and keeps no earlier versions.";
        crash_reports: "fontcraftSettings.crashReports" = "Send Crash Reports";
        name: "fontcraftSettings.name" = "Name";
        email: "fontcraftSettings.email" = "Email";
        crash_note: "fontcraftSettings.crashNote" = "Not used yet: FontCraft sends no crash reports. The choice and the contact details are kept.";
        groups: "fontcraftSettings.groups" = "Group";
        add_group: "fontcraftSettings.addGroup" = "Add Group";
        remove_group: "fontcraftSettings.removeGroup" = "Remove Group";
        strings: "fontcraftSettings.strings" = "Sample strings, one per line";
        defaults: "fontcraftSettings.defaults" = "Default";
        no_groups: "fontcraftSettings.noGroups" = "No groups. Add one with Add Group, or restore the built-in ones with Default.";
        enable_preview: "fontcraftSettings.enablePreview" = "Enable External Preview";
        preview_note: "fontcraftSettings.previewNote" = "Lets a browser on a device in the same local network watch the glyph being edited. Each device has to be accepted here first.";
        address: "fontcraftSettings.address" = "Address to open on the device";
        not_served: "fontcraftSettings.notServed" = "The page is served while External Preview is on, in the desktop app.";
        waiting: "fontcraftSettings.waiting" = "Devices waiting for an answer";
        accept: "fontcraftSettings.accept" = "Accept";
        refuse: "fontcraftSettings.refuse" = "Refuse";
        later: "fontcraftSettings.later" = "Later";
        trusted: "fontcraftSettings.trusted" = "Trusted devices";
        reset_devices: "fontcraftSettings.resetDevices" = "Reset";
        device_asks: "fontcraftSettings.deviceAsks" = "A device asks to see External Preview:";
        device_note: "fontcraftSettings.deviceNote" = "Accept only a device you know. Later leaves it waiting in Settings › Sharing.";
    }
}

/// A switch's label and note.
type SwitchWords = fn(&SettingsWords) -> (&Label, &Label);

/// The `user.*` switches, with their notes.
const USER_SWITCHES: [(&str, SwitchWords); 4] = [
    ("user.keepGlyphNamesFromImportedFiles", |w| (&w.keep_names, &w.keep_names_note)),
    ("user.disableAutomaticAlignmentForImportedFiles", |w| (&w.no_alignment, &w.no_alignment_note)),
    ("user.disableLocalization", |w| (&w.no_localization, &w.no_localization_note)),
    ("user.useVersions", |w| (&w.versions, &w.versions_note)),
];
const CRASH_REPORTS: &str = "user.sendCrashReports";
const CRASH_NAME: &str = "user.crashReportName";
const CRASH_EMAIL: &str = "user.crashReportEmail";

/// External Preview as the Sharing section shows it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SharingModel {
    pub enabled: bool,
    pub address: Option<String>,
    pub problem: Option<String>,
    pub pending: Vec<String>,
    pub trusted: Vec<String>,
}

/// What the window shows, copied from the app each pass.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SettingsModel {
    pub section: String,
    /// The `user.*` switches, by id, and the two contact fields.
    pub switches: Vec<(String, bool)>,
    pub crash_reports: bool,
    pub crash_name: String,
    pub crash_email: String,
    pub groups: Vec<Group>,
    /// The group the Sample Strings section shows.
    pub group: usize,
    pub sharing: SharingModel,
    /// The Extensions section: the Extension Manager's Indexes tab.
    pub indexes: IndexesModel,
}

impl SettingsModel {
    pub fn of(app: &FontCraftApp) -> SettingsModel {
        let settings = app.session.settings();
        let bool_of = |id: &str| settings.get_bool(id).unwrap_or(false);
        let text_of = |id: &str| settings.get(id).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default();
        let state = app.session.sharing();
        SettingsModel {
            section: settings_state::current(&app.view.settings).to_string(),
            switches: USER_SWITCHES.iter().map(|(id, _)| ((*id).to_string(), bool_of(id))).collect(),
            crash_reports: bool_of(CRASH_REPORTS),
            crash_name: text_of(CRASH_NAME),
            crash_email: text_of(CRASH_EMAIL),
            groups: settings_state::groups(&app.session),
            group: app.view.settings.group,
            sharing: SharingModel {
                enabled: sharing::enabled(settings),
                address: state.address.clone(),
                problem: state.problem.clone(),
                pending: state.pending.clone(),
                trusted: sharing::trusted_devices(settings),
            },
            indexes: extension_manager::indexes_model(app),
        }
    }
}

fn shell(id: &str, params: Value) -> Msg {
    Msg::Shell(CommandRef::with_params(id, params))
}

fn set(id: &str, value: Value) -> Msg {
    Msg::run("settings.set", json!({ "id": id, "value": value }))
}

/// The window's body: the section switcher, then the shown section. `new_index` is the Extensions section's Add field.
pub fn body(ui: &mut Ui, model: &SettingsModel, new_index: &mut String, sent: &mut Vec<Msg>) {
    let words = Env::of(ui).words::<SettingsWords>();
    let entries: Vec<Entry<String>> = SECTIONS.iter().map(|s| Entry::new(s.id.to_string(), Label::Static(s.label)).enabled(s.available)).collect();
    let mut section = model.section.clone();
    Segmented::new(words.sections.clone(), Bind::bound(&mut section), &entries).show(ui);
    if section != model.section {
        sent.push(shell("app.settings", json!({ "section": section })));
    }
    Stack::new().show(ui, |ui, _| match model.section.as_str() {
        "sampleStrings" => sample_strings(ui, model, sent, &words),
        "sharing" => sharing_section(ui, &model.sharing, sent, &words),
        "addons" => {
            extension_indexes::show(ui, &model.indexes, new_index, sent);
            if Button::new(words.open_manager.clone()).show(ui).clicked() {
                sent.push(shell("window.pluginManager", json!({})));
            }
        }
        _ => user_settings(ui, model, sent, &words),
    });
}

fn note(ui: &mut Ui, text: &Label) {
    Text::new(text.clone()).role(TypeRole::Caption).shade(Shade::Muted).show(ui);
}

fn user_settings(ui: &mut Ui, model: &SettingsModel, sent: &mut Vec<Msg>, words: &SettingsWords) {
    for ((id, label_of), (_, on)) in USER_SWITCHES.iter().zip(&model.switches) {
        let (label, text) = label_of(words);
        if let Some(next) = Checkbox::new(Bind::given(*on)).label(label.clone()).show(ui).events.committed() {
            sent.push(set(id, json!(next)));
        }
        note(ui, text);
    }
    if let Some(next) = Checkbox::new(Bind::given(model.crash_reports)).label(words.crash_reports.clone()).show(ui).events.committed() {
        sent.push(set(CRASH_REPORTS, json!(next)));
    }
    for (id, label, value) in [(CRASH_NAME, &words.name, &model.crash_name), (CRASH_EMAIL, &words.email, &model.crash_email)] {
        Field::new(label.clone()).show(ui, |ui, _| {
            if let Some(text) = TextField::new(label.clone(), Bind::given(value.clone())).show(ui).events.committed() {
                sent.push(set(id, json!(text)));
            }
        });
    }
    note(ui, &words.crash_note);
}

fn sample_strings(ui: &mut Ui, model: &SettingsModel, sent: &mut Vec<Msg>, words: &SettingsWords) {
    let _ = Row::new().fill_width().show(ui, |ui, _| {
        if Button::new(words.add_group.clone()).show(ui).clicked() {
            sent.push(shell("settings.addSampleStringGroup", json!({})));
        }
        let can_remove = model.group < model.groups.len();
        if Button::new(words.remove_group.clone()).enabled(can_remove).show(ui).clicked() && can_remove {
            sent.push(shell("settings.removeSampleStringGroup", json!({ "group": model.group })));
        }
        if Button::new(words.defaults.clone()).show(ui).clicked() {
            sent.push(shell("settings.resetSampleStrings", json!({})));
        }
    });
    if model.groups.is_empty() {
        note(ui, &words.no_groups);
        return;
    }
    let entries: Vec<Entry<usize>> = model.groups.iter().enumerate().map(|(i, g)| Entry::new(i, g.name.clone())).collect();
    let mut group = model.group.min(model.groups.len().saturating_sub(1));
    let shown = group;
    Segmented::new(words.groups.clone(), Bind::bound(&mut group), &entries).show(ui);
    if group != shown {
        sent.push(shell("app.settings", json!({ "group": group })));
    }
    let Some(g) = model.groups.get(shown) else { return };
    let text = g.strings.join("\n");
    let lines = g.strings.len().clamp(4, 16);
    if let Some(typed) = TextField::new(words.strings.clone(), Bind::given(text)).lines(lines).show(ui).events.committed() {
        let strings: Vec<&str> = typed.lines().filter(|l| !l.is_empty()).collect();
        sent.push(shell("settings.setSampleStrings", json!({ "group": shown, "strings": strings })));
    }
}

fn sharing_section(ui: &mut Ui, model: &SharingModel, sent: &mut Vec<Msg>, words: &SettingsWords) {
    if let Some(on) = Checkbox::new(Bind::given(model.enabled)).label(words.enable_preview.clone()).show(ui).events.committed() {
        sent.push(set(ENABLE_SETTING, json!(on)));
    }
    note(ui, &words.preview_note);
    if model.enabled {
        Field::new(words.address.clone()).show(ui, |ui, _| match &model.address {
            Some(address) => {
                Text::new(address.clone()).show(ui);
            }
            None => note(ui, &words.not_served),
        });
    }
    if let Some(problem) = &model.problem {
        Text::new(problem.clone()).shade(Shade::Muted).show(ui);
    }
    if !model.pending.is_empty() {
        Text::new(words.waiting.clone()).role(TypeRole::Heading).show(ui);
        for device in &model.pending {
            let _ = Row::new().show(ui, |ui, _| {
                Text::new(device.clone()).show(ui);
                if Button::new(words.accept.clone()).show(ui).clicked() {
                    sent.push(Msg::run("sharing.answerDevice", json!({ "device": device, "trust": true })));
                }
                if Button::new(words.refuse.clone()).show(ui).clicked() {
                    sent.push(Msg::run("sharing.answerDevice", json!({ "device": device, "trust": false })));
                }
            });
        }
    }
    if !model.trusted.is_empty() {
        let _ = Row::new().show(ui, |ui, _| {
            Text::new(format!("{}: {}", words.trusted.as_str(), model.trusted.join(", "))).show(ui);
            if Button::new(words.reset_devices.clone()).show(ui).clicked() {
                sent.push(Msg::run("settings.resetTrustedDevices", json!({})));
            }
        });
    }
}

/// The Settings window on the dialog host.
pub struct SettingsDialog {
    id: DialogId,
    pub model: SettingsModel,
    pub outbox: Vec<Msg>,
    /// The app closed the window: the next pass ends the dialog.
    pub close: bool,
    open: bool,
    /// The Extensions section's Add field.
    new_index: String,
}

impl SettingsDialog {
    pub fn new(model: SettingsModel) -> SettingsDialog {
        SettingsDialog { id: dialog_id("fontcraft.settings"), model, outbox: Vec::new(), close: false, open: true, new_index: String::new() }
    }
}

impl StackDialog for SettingsDialog {
    type Result = ();

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<()>> {
        if self.close {
            return Some(DialogResult::Cancel);
        }
        let words = Env::of(ui).words::<SettingsWords>();
        let SettingsDialog { model, outbox, open, new_index, .. } = self;
        let mut value = false;
        let shown = FormDialog::new(words.title.clone(), Bind::bound(&mut value))
            .opened(Bind::bound(open))
            .ok_label(words.done.clone())
            .cancel_label(words.close.clone())
            .enter_confirms(false)
            .show(ui, |ui, _, _| {
                body(ui, model, new_index, outbox);
                true
            });
        shown.closed.map(|_| DialogResult::Cancel)
    }
}

/// The notice for a device that asks to see External Preview.
struct DeviceNotice(String);

/// Opens the window while the app asks for it, refreshes it, posts what the person did and closes it when the app
/// closed it; asks about each new External Preview device while the window is closed.
pub fn run(host: &mut DialogHost, app: &mut FontCraftApp) {
    if host.take::<SettingsDialog>().is_some() && app.view.settings.open {
        app.sent.push(shell("app.settings", json!({ "open": false })));
    } else if !app.view.settings.open {
        if let Some(dialog) = host.get_mut::<SettingsDialog>() {
            dialog.close = true;
        }
    } else {
        let model = SettingsModel::of(app);
        match host.get_mut::<SettingsDialog>() {
            Some(dialog) => {
                app.sent.append(&mut dialog.outbox);
                dialog.model = model;
            }
            None => {
                host.open(SettingsDialog::new(model));
            }
        }
    }
    device_notice(host, app);
}

fn device_notice(host: &mut DialogHost, app: &mut FontCraftApp) {
    if let Some(answer) = host.take::<StandardPrompt>().and_then(DialogResult::ok) {
        let mut answer = answer;
        if let Some(DeviceNotice(device)) = answer.take_tag::<DeviceNotice>() {
            match answer.choice {
                PromptChoice::Proceed => app.sent.push(Msg::run("sharing.answerDevice", json!({ "device": device, "trust": true }))),
                PromptChoice::Alternate => app.sent.push(Msg::run("sharing.answerDevice", json!({ "device": device, "trust": false }))),
                _ => {}
            }
        }
    }
    let pending = &app.session.sharing().pending;
    let st = &mut app.view.settings;
    st.noticed.retain(|d| pending.contains(d));
    if st.open || host.is_open::<StandardPrompt>() {
        return;
    }
    let Some(device) = pending.iter().find(|d| !st.noticed.contains(d)).cloned() else { return };
    st.noticed.push(device.clone());
    let words = SettingsWords::english();
    let prompt = StandardPrompt::custom(format!("{} {device}", words.device_asks.as_str()))
        .paragraph(words.device_note.clone())
        .button(PromptChoice::Proceed, words.accept.clone())
        .button(PromptChoice::Alternate, words.refuse.clone())
        .button(PromptChoice::Cancel, words.later.clone())
        .tag(DeviceNotice(device));
    host.open(prompt);
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use craft::ui::testing::Fixture;
    use serde_json::json;

    use super::{SettingsModel, SharingModel, body};
    use crate::app::Msg;

    /// Lines typed into the Sample Strings field replace the shown group's strings, and the Select Sample Text window
    /// then lists them; the buttons post their commands, and Remove Group is greyed with no group.
    #[test]
    fn a_sample_strings_edit_reaches_the_select_sample_text_list() {
        use std::sync::Arc;

        use craft::host::Host;
        use craft::ui::dialog::DialogHost;
        use craft::ui::runtime::commands::CommandRef;
        use egui::{Key, Modifiers};
        use fontcraft_engine::Session;

        use crate::app::{APP, FontCraftApp};
        use crate::dialogs::sample_strings::{SampleStringsDialog, run_dialog};

        let mut app = FontCraftApp::with_edit_tab(Session::with_document(), Arc::new(Host::builder(APP).build()));
        app.run_now(Msg::Shell(CommandRef::with_params("app.settings", json!({"section": "sampleStrings", "group": 1}))), false).unwrap();
        let model = SettingsModel::of(&app);
        assert_eq!(model.group, 1);
        let posted = Rc::new(RefCell::new(Vec::<Msg>::new()));
        let seen = Rc::clone(&posted);
        let mut fixture = Fixture::new(move |ui| {
            let mut sent = Vec::new();
            body(ui, &model, &mut String::new(), &mut sent);
            seen.borrow_mut().extend(sent);
        });
        fixture.settle();
        let field = fixture.by_label("Sample strings, one per line").expect("the strings field");
        fixture.click(&field).key(Key::A, Modifiers::COMMAND).type_text("H0H");
        fixture.key(Key::Enter, Modifiers::NONE).key(Key::Enter, Modifiers::NONE).type_text("/zero /one ");
        // Clicking Add Group takes the focus from the field, which commits the typed lines first.
        let add = fixture.by_label("Add Group").expect("Add Group");
        fixture.click(&add).settle();
        let reset = fixture.by_label("Default").expect("Default");
        fixture.click(&reset).settle();
        let shell = |id: &str, params: serde_json::Value| Msg::Shell(CommandRef::with_params(id, params));
        let edit = shell("settings.setSampleStrings", json!({"group": 1, "strings": ["H0H", "/zero /one "]}));
        assert_eq!(
            *posted.borrow(),
            vec![edit.clone(), shell("settings.addSampleStringGroup", json!({})), shell("settings.resetSampleStrings", json!({}))],
            "one string per line, the empty line left out"
        );

        app.run_now(edit, false).unwrap();
        app.run_now(shell("edit.selectSampleText", json!({})), false).unwrap();
        let mut host = DialogHost::new();
        run_dialog(&mut host, &mut app);
        let listed = host.get::<SampleStringsDialog>().expect("the Select Sample Text window").strings.clone();
        let figures: Vec<&str> = listed.iter().filter(|(g, _)| g == "Figures").map(|(_, s)| s.as_str()).collect();
        assert_eq!(figures, vec!["H0H", "/zero /one "]);

        let empty = SettingsModel { section: "sampleStrings".into(), ..Default::default() };
        let clicked = Rc::new(RefCell::new(Vec::<Msg>::new()));
        let seen = Rc::clone(&clicked);
        let mut fixture = Fixture::new(move |ui| {
            let mut sent = Vec::new();
            body(ui, &empty, &mut String::new(), &mut sent);
            seen.borrow_mut().extend(sent);
        });
        fixture.settle();
        let remove = fixture.by_label("Remove Group").expect("Remove Group");
        fixture.click(&remove).settle();
        assert!(clicked.borrow().is_empty(), "Remove Group does nothing with no group");
        assert!(fixture.texts().iter().any(|t| t.text.starts_with("No groups.")), "the empty note shows");
    }

    /// In the Sharing section, Accept answers the waiting device and the switch writes the setting.
    #[test]
    fn sharing_accepts_a_waiting_device_and_switches_the_preview() {
        let model = SettingsModel {
            section: "sharing".into(),
            sharing: SharingModel { enabled: true, pending: vec!["192.0.2.7".into()], ..Default::default() },
            ..Default::default()
        };
        let posted = Rc::new(RefCell::new(Vec::<Msg>::new()));
        let seen = Rc::clone(&posted);
        let mut fixture = Fixture::new(move |ui| {
            let mut sent = Vec::new();
            body(ui, &model, &mut String::new(), &mut sent);
            seen.borrow_mut().extend(sent);
        });
        fixture.settle();
        let accept = fixture.by_label("Accept").expect("the Accept button");
        fixture.click(&accept).settle();
        assert_eq!(*posted.borrow(), vec![Msg::run("sharing.answerDevice", json!({ "device": "192.0.2.7", "trust": true }))]);
        posted.borrow_mut().clear();
        let switch = fixture.by_label("Enable External Preview").expect("the switch");
        fixture.click(&switch).settle();
        assert_eq!(*posted.borrow(), vec![Msg::run("settings.set", json!({ "id": "sharing.enableExternalPreview", "value": false }))]);
    }
}
