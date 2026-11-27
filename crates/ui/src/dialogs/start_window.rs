//! The Start Window: the recent documents, or the script and glyph-set chooser that creates a document. Its state and
//! commands are `shell/start.rs`; this module shows it while `ViewState::start.open` and posts what the person does.
//!
//! | In the window | Does |
//! |---|---|
//! | Recent Documents / New Document | switches between the list and the chooser (`window.startWindow {chooser}`) |
//! | double-click or Enter on a recent document, or Open | opens it (`file.openRecent`) |
//! | Show in Finder | shows the marked document in the file manager (`file.showInFinder`) |
//! | Show window on startup | `settings.set` of `start.showAtLaunch` |
//! | a script in the list | shows its glyph sets on the right |
//! | No / Yes | leaves the script out of the new document or includes it |
//! | a glyph set's check box | takes the set out of the script's picks or adds it |
//! | Create Document | `file.newFromGlyphSets` with the included scripts and their checked sets |
//! | Close | closes the window |

use std::borrow::Cow;
use std::path::Path;

use craft::behavior::{SelectionChange, keyed};
use craft::ui::buttons::Button;
use craft::ui::choice::{Checkbox, Segmented};
use craft::ui::contract::{Bind, CollectionEvent, Entry, Props as _, Show as _, ShowCollection as _, ShowContainer as _};
use craft::ui::dialog::{DialogHost, DialogId, DialogResult, DialogStack, StackDialog, dialog_id};
use craft::ui::display::{Shade, Text};
use craft::ui::layout_components::{Row, Stack};
use craft::ui::list::{List, ListRow};
use craft::ui::patterns::form_dialogs::FormDialog;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::{Env, Label, WordSet as _};
use craft::ui::style::TypeRole;
use craft::ui::text::TextField;
use egui::Ui;
use serde_json::{Value, json};

use crate::app::{FontCraftApp, Msg};
use crate::shell::start::{self, NEW_FROM_GLYPH_SETS, SHOW_AT_LAUNCH};

craft::ui::words! {
    /// The words of the Start Window.
    pub struct StartWords {
        title: "fontcraftStart.title" = "FontCraft";
        mode: "fontcraftStart.mode" = "Start";
        recent: "fontcraftStart.recent" = "Recent Documents";
        new_document: "fontcraftStart.newDocument" = "New Document";
        open: "fontcraftStart.open" = "Open";
        close: "fontcraftStart.close" = "Close";
        show_in_finder: "fontcraftStart.showInFinder" = "Show in Finder";
        show_at_launch: "fontcraftStart.showAtLaunch" = "Show window on startup";
        version: "fontcraftStart.version" = "Version";
        no_recent: "fontcraftStart.noRecent" = "No recent documents.";
        search: "fontcraftStart.search" = "Search scripts";
        scripts: "fontcraftStart.scripts" = "Scripts";
        included: "fontcraftStart.included" = "Included";
        include: "fontcraftStart.include" = "Include";
        no: "fontcraftStart.no" = "No";
        yes: "fontcraftStart.yes" = "Yes";
        glyphs: "fontcraftStart.glyphs" = "glyphs";
        pick_script: "fontcraftStart.pickScript" = "Click a script to see its glyph sets.";
        no_catalog: "fontcraftStart.noCatalog" = "The glyph sets cannot be read.";
        create: "fontcraftStart.create" = "Create Document";
    }
}

/// Rows the lists show before they scroll.
const LIST_ROWS: u16 = 12;

/// What the window shows, copied from the app each pass.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct StartModel {
    /// The chooser shows in place of the recent documents.
    pub chooser: bool,
    pub recent: Vec<String>,
    pub selected: Option<String>,
    pub show_at_launch: bool,
    pub search: String,
    /// The scripts the search lets through: id, readable name, included.
    pub scripts: Vec<(String, String, bool)>,
    /// The script whose glyph sets show: id, readable name, included.
    pub script: Option<(String, String, bool)>,
    /// Its glyph sets: id, the label with the glyph count, checked.
    pub sets: Vec<(String, String, bool)>,
    /// The `scripts` parameter Create Document runs with; `None` while nothing is picked.
    pub create: Option<Value>,
    pub error: Option<String>,
    pub catalog_empty: bool,
}

impl StartModel {
    /// The model of the window over the app's state.
    pub fn of(app: &FontCraftApp) -> StartModel {
        let st = &app.view.start;
        let recent = start::recent_documents(&app.session);
        let needle = st.search.trim().to_lowercase();
        let scripts = st
            .picks
            .iter()
            .map(|p| (p.script.clone(), start::script_label(st, &p.script), p.included))
            .filter(|(_, label, _)| needle.is_empty() || label.to_lowercase().contains(&needle))
            .collect();
        let pick = st.script.as_deref().and_then(|s| st.picks.iter().find(|p| p.script == s));
        let words = StartWords::english();
        let sets = pick.map_or_else(Vec::new, |p| {
            start::sets(st, &p.script)
                .into_iter()
                .map(|g| {
                    let label = format!("{} ({} {})", g.name, g.glyphs, words.glyphs.as_str());
                    let on = p.glyph_sets.contains(&g.id);
                    (g.id, label, on)
                })
                .collect()
        });
        let any = st.picks.iter().any(|p| p.included && !p.glyph_sets.is_empty());
        StartModel {
            chooser: st.chooser || recent.is_empty(),
            recent,
            selected: st.selected_recent.clone(),
            show_at_launch: app.session.settings().get_bool(SHOW_AT_LAUNCH).unwrap_or(true),
            search: st.search.clone(),
            scripts,
            script: pick.map(|p| (p.script.clone(), start::script_label(st, &p.script), p.included)),
            sets,
            create: any.then(|| start::picks_param(&st.picks)),
            error: st.error.clone(),
            catalog_empty: st.catalog.is_empty(),
        }
    }
}

fn shell(id: &str, params: Value) -> Msg {
    Msg::Shell(CommandRef::with_params(id, params))
}

/// The key a selection change ends on.
fn last_key<K>(change: &SelectionChange<K>) -> Option<&K> {
    match change {
        SelectionChange::Set(keys) => keys.last(),
        SelectionChange::Toggle(key) => Some(key),
        _ => None,
    }
}

/// A recent document's file name, for its row.
fn file_name(path: &str) -> String {
    Path::new(path).file_name().map_or_else(|| path.to_string(), |n| n.to_string_lossy().into_owned())
}

/// The window's body: the mode switch, then the recent documents or the chooser.
pub fn body(ui: &mut Ui, model: &StartModel, sent: &mut Vec<Msg>) {
    let words = Env::of(ui).words::<StartWords>();
    if !model.recent.is_empty() {
        let modes = [Entry::new(false, words.recent.clone()), Entry::new(true, words.new_document.clone())];
        let mut chooser = model.chooser;
        Segmented::new(words.mode.clone(), Bind::bound(&mut chooser), &modes).show(ui);
        if chooser != model.chooser {
            sent.push(shell("window.startWindow", json!({ "chooser": chooser })));
        }
    }
    if model.chooser {
        chooser_body(ui, model, sent, &words);
    } else {
        recent_body(ui, model, sent, &words);
    }
}

fn recent_body(ui: &mut Ui, model: &StartModel, sent: &mut Vec<Msg>, words: &StartWords) {
    let source = keyed(&model.recent, String::clone).with_text(|p| Cow::Owned(file_name(p)));
    let out =
        List::<String>::new(words.recent.clone()).max_rows(LIST_ROWS).current(model.selected.clone()).show_items(ui, &source, &mut |path, _| {
            let folder = Path::new(path).parent().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default();
            ListRow::new(file_name(path)).trailing(folder)
        });
    for event in &out.events {
        match event {
            CollectionEvent::Activate(path) => sent.push(shell("file.openRecent", json!({ "path": path }))),
            CollectionEvent::Selection { change, .. } => {
                if let Some(path) = last_key(change) {
                    sent.push(shell("window.startWindow", json!({ "selected": path })));
                }
            }
            _ => {}
        }
    }
    let _ = Row::new().fill_width().show(ui, |ui, _| {
        if Button::new(words.show_in_finder.clone()).enabled(model.selected.is_some()).show(ui).clicked()
            && let Some(path) = &model.selected
        {
            sent.push(shell("file.showInFinder", json!({ "path": path })));
        }
        if let Some(on) = Checkbox::new(Bind::given(model.show_at_launch)).label(words.show_at_launch.clone()).show(ui).events.committed() {
            sent.push(Msg::run("settings.set", json!({ "id": SHOW_AT_LAUNCH, "value": on })));
        }
    });
    Text::new(format!("{} {}", words.version.as_str(), crate::app::APP.version)).shade(Shade::Muted).show(ui);
}

fn chooser_body(ui: &mut Ui, model: &StartModel, sent: &mut Vec<Msg>, words: &StartWords) {
    if model.catalog_empty {
        Text::new(words.no_catalog.clone()).show(ui);
        return;
    }
    if let Some(text) =
        TextField::new(words.search.clone(), Bind::given(model.search.clone())).placeholder(words.search.clone()).show(ui).events.committed()
    {
        sent.push(shell("window.startWindow", json!({ "search": text })));
    }
    let _ = Row::new().fill_width().show(ui, |ui, _| {
        Stack::new().show(ui, |ui, _| {
            let ids: Vec<String> = model.scripts.iter().map(|(id, _, _)| id.clone()).collect();
            let source = keyed(&ids, String::clone)
                .with_text(|id| Cow::Owned(model.scripts.iter().find(|(s, _, _)| s == id).map(|(_, l, _)| l.clone()).unwrap_or_default()));
            let current = model.script.as_ref().map(|(id, _, _)| id.clone());
            let out = List::<String>::new(words.scripts.clone()).max_rows(LIST_ROWS).current(current).show_items(ui, &source, &mut |id, _| {
                let Some((_, label, included)) = model.scripts.iter().find(|(s, _, _)| s == id) else { return ListRow::new(Label::Empty) };
                let row = ListRow::new(label.clone());
                if *included { row.trailing(words.included.clone()) } else { row }
            });
            for event in &out.events {
                let picked = match event {
                    CollectionEvent::Activate(id) => Some(id.clone()),
                    CollectionEvent::Selection { change, .. } => last_key(change).cloned(),
                    _ => None,
                };
                if let Some(id) = picked {
                    sent.push(shell("window.startWindow", json!({ "script": id })));
                }
            }
        });
        Stack::new().show(ui, |ui, _| {
            let Some((script, label, included)) = &model.script else {
                Text::new(words.pick_script.clone()).shade(Shade::Muted).show(ui);
                return;
            };
            Text::new(label.clone()).role(TypeRole::Heading).show(ui);
            let choices = [Entry::new(false, words.no.clone()), Entry::new(true, words.yes.clone())];
            let mut on = *included;
            Segmented::new(words.include.clone(), Bind::bound(&mut on), &choices).show(ui);
            if on != *included {
                sent.push(shell("window.startWindow", json!({ "script": script, "include": on })));
            }
            for (set, label, checked) in &model.sets {
                if let Some(on) = Checkbox::new(Bind::given(*checked)).label(label.clone()).enabled(*included).show(ui).events.committed() {
                    sent.push(shell("window.startWindow", json!({ "script": script, "glyphSet": set, "on": on })));
                }
            }
        });
    });
    if let Some(error) = &model.error {
        Text::new(error.clone()).shade(Shade::Muted).show(ui);
    }
}

/// The Start Window on the dialog host.
pub struct StartDialog {
    id: DialogId,
    pub model: StartModel,
    /// What the person did this pass, for the module to post.
    pub outbox: Vec<Msg>,
    /// The app closed the window: the next pass ends the dialog.
    pub close: bool,
    open: bool,
}

impl StartDialog {
    pub fn new(model: StartModel) -> StartDialog {
        StartDialog { id: dialog_id("fontcraft.start"), model, outbox: Vec::new(), close: false, open: true }
    }
}

impl StackDialog for StartDialog {
    type Result = ();

    fn id(&self) -> DialogId {
        self.id
    }

    fn show(&mut self, ui: &mut Ui, _: &DialogStack) -> Option<DialogResult<()>> {
        if self.close {
            return Some(DialogResult::Cancel);
        }
        let words = Env::of(ui).words::<StartWords>();
        let (ok, enabled) = if self.model.chooser {
            (words.create.clone(), self.model.create.is_some())
        } else {
            (words.open.clone(), self.model.selected.is_some())
        };
        let StartDialog { model, outbox, open, .. } = self;
        let mut value = false;
        let shown = FormDialog::new(words.title.clone(), Bind::bound(&mut value))
            .opened(Bind::bound(open))
            .ok_label(ok)
            .ok_enabled(enabled)
            .cancel_label(words.close.clone())
            .accept_stays_open(true)
            .show(ui, |ui, _, _| {
                body(ui, model, outbox);
                true
            });
        if shown.accepted.is_some() || shown.picked == Some(0) {
            match (&model.create, &model.selected) {
                (Some(scripts), _) if model.chooser => outbox.push(Msg::run(NEW_FROM_GLYPH_SETS, json!({ "scripts": scripts }))),
                (_, Some(path)) if !model.chooser => outbox.push(shell("file.openRecent", json!({ "path": path }))),
                _ => {}
            }
        }
        shown.closed.filter(|_| shown.accepted.is_none()).map(|_| DialogResult::Cancel)
    }
}

/// Opens the window while the app asks for it, refreshes what it shows, posts what the person did, and closes it
/// when the app closed it.
pub fn run(host: &mut DialogHost, app: &mut FontCraftApp) {
    if host.take::<StartDialog>().is_some() && app.view.start.open {
        app.sent.push(shell("window.startWindow", json!({ "open": false })));
        return;
    }
    if !app.view.start.open {
        if let Some(dialog) = host.get_mut::<StartDialog>() {
            dialog.close = true;
        }
        return;
    }
    let model = StartModel::of(app);
    match host.get_mut::<StartDialog>() {
        Some(dialog) => {
            app.sent.append(&mut dialog.outbox);
            dialog.model = model;
        }
        None => {
            host.open(StartDialog::new(model));
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use craft::ui::testing::Fixture;
    use serde_json::json;

    use super::{StartModel, body, shell};
    use crate::app::Msg;

    /// In the chooser, Yes includes the shown script and a glyph set's check box adds the set to its picks.
    #[test]
    fn the_chooser_posts_include_and_glyph_set_picks() {
        let model = StartModel {
            chooser: true,
            scripts: vec![("latin".into(), "Latin".into(), false)],
            script: Some(("latin".into(), "Latin".into(), false)),
            sets: vec![("core".into(), "Core (100 glyphs)".into(), false)],
            ..Default::default()
        };
        let posted = Rc::new(RefCell::new(Vec::new()));
        let seen = Rc::clone(&posted);
        let shown = Rc::new(RefCell::new(model));
        let current = Rc::clone(&shown);
        let mut fixture = Fixture::new(move |ui| {
            let mut sent = Vec::new();
            body(ui, &current.borrow(), &mut sent);
            seen.borrow_mut().extend(sent);
        });
        fixture.settle();
        let yes = fixture.by_label("Yes").expect("the Yes segment");
        fixture.click(&yes).settle();
        assert_eq!(*posted.borrow(), vec![shell("window.startWindow", json!({ "script": "latin", "include": true }))]);

        posted.borrow_mut().clear();
        {
            let mut m = shown.borrow_mut();
            m.script = Some(("latin".into(), "Latin".into(), true));
        }
        fixture.settle();
        let set = fixture.by_label("Core (100 glyphs)").expect("the glyph set's check box");
        fixture.click(&set).settle();
        assert_eq!(*posted.borrow(), vec![shell("window.startWindow", json!({ "script": "latin", "glyphSet": "core", "on": true }))]);
    }

    /// Activating a recent document opens it.
    #[test]
    fn a_recent_document_opens_on_activate() {
        let model = StartModel { recent: vec!["/fonts/A.glyphs".into()], ..Default::default() };
        let posted = Rc::new(RefCell::new(Vec::<Msg>::new()));
        let seen = Rc::clone(&posted);
        let mut fixture = Fixture::new(move |ui| {
            let mut sent = Vec::new();
            body(ui, &model, &mut sent);
            seen.borrow_mut().extend(sent);
        });
        fixture.settle();
        let row = fixture.by_label("A.glyphs").expect("the recent document's row");
        fixture.click(&row).key(egui::Key::Enter, egui::Modifiers::NONE).settle();
        assert!(posted.borrow().contains(&shell("file.openRecent", json!({ "path": "/fonts/A.glyphs" }))), "{:?}", posted.borrow());
    }
}
