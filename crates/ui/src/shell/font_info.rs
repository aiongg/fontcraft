//! The Font Info panel's shell commands and view state: which tab shows, and the dialog a row asked for.
//!
//! | Command | Does |
//! |---|---|
//! | `font.info {tab?}` | shows the Font Info panel, on `tab` or on the tab it showed last |
//! | `font.info.selectTab {tab}` | shows one tab, showing the panel when it is hidden |
//! | `font.info.openDialog {dialog, scope, targets?, name?, index?}` | asks `dialogs` for a picker or a value dialog |
//! | `font.info.rows.select {scope, rows, mode?}` | selects rows of the shown tab |
//! | `font.info.rows.copy`, `.cut`, `.paste`, `.delete` | act on the selected rows of the shown tab; paste asks the window for the clipboard's text and pastes it into what the tab shows |
//!
//! The row selection is view state: it names rows by kind, key and place among the rows of that key, and the projection
//! marks those the shown tab has, so it stays when other masters or exports are picked.
//!
//! The dialogs module (`dialogs::field_picker`, `custom_parameter_picker`, `os2_bits`, `panose`, `feature_picker`) takes the request
//! on its next pass and opens the dialog.

use craft::ui::contract::PlainData;
use craft::ui::runtime::commands::CommandRef;
use fontcraft_engine::fontinfo::names::names_of;
use fontcraft_engine::fontinfo::{Names, Scope};
use serde_json::{Map, Value, json};

use super::commands::{Module, UiCommand};
use super::dock::PanelId;
use crate::app::clipboard::PasteKind;
use crate::app::models::font_info::RowMark;
use crate::app::{FontCraftApp, Msg};

pub const MODULE: Module = Module { commands: COMMANDS, run };

const COMMANDS: &[UiCommand] = &[
    UiCommand {
        id: "font.info",
        label: "Font Info…",
        menu: &["File"],
        shortcut: Some("Cmd+I"),
        params: "{tab?: 'font'|'masters'|'exports'|'features'|'other'|'notes'} show the Font Info panel of the current font, on `tab` or on the tab it showed last",
    },
    UiCommand {
        id: "font.info.selectTab",
        label: "Select Font Info Tab",
        menu: &[],
        shortcut: None,
        params: "{tab: 'font'|'masters'|'exports'|'features'|'other'|'notes'} show one tab of the Font Info panel, showing the panel when it is hidden",
    },
    UiCommand {
        id: "font.info.openDialog",
        label: "Open Font Info Dialog",
        menu: &[],
        shortcut: None,
        params: "{dialog: 'fieldPicker'|'parameterPicker'|'os2Bits'|'panose'|'glyphs'|'records'|'featurePicker', scope: 'font'|'master'|'instance', targets?: [index], name?: string, index?: number} open a picker of the Font Info panel, or the dialog of the parameter `name` (its `index`th row of that name)",
    },
    UiCommand {
        id: SELECT_ROWS,
        label: "Select Font Info Rows",
        menu: &[],
        shortcut: None,
        params: "{scope: 'font'|'master'|'instance', rows: [{kind: 'field'|'param', key: string, index?: integer}], mode?: 'set'|'toggle'|'add'} select rows of the Font Info tab of that scope; index: which of the rows of that key and kind, from 0. set (the default) selects only these, toggle adds or drops each, add keeps the others. An empty list selects none",
    },
    UiCommand {
        id: COPY_ROWS,
        label: "Copy Font Info Rows",
        menu: &[],
        shortcut: None,
        params: "{} copy the selected rows of the shown Font Info tab to the clipboard, as Edit › Copy does while rows are selected",
    },
    UiCommand {
        id: CUT_ROWS,
        label: "Cut Font Info Rows",
        menu: &[],
        shortcut: None,
        params: "{} copy the selected rows of the shown Font Info tab to the clipboard and remove them from every master or export the tab shows, as one undo step; built-in fields stay",
    },
    UiCommand {
        id: PASTE_ROWS,
        label: "Paste Font Info Rows",
        menu: &[],
        shortcut: None,
        params: "{} paste the rows on the clipboard into the font, masters or exports the shown Font Info tab shows, as one undo step",
    },
    UiCommand {
        id: DELETE_ROWS,
        label: "Delete Font Info Rows",
        menu: &[],
        shortcut: None,
        params: "{} remove the selected rows of the shown Font Info tab from every master or export it shows, as one undo step; built-in fields stay",
    },
];

/// The shell commands of the selected rows.
pub const SELECT_ROWS: &str = "font.info.rows.select";
pub const COPY_ROWS: &str = "font.info.rows.copy";
pub const CUT_ROWS: &str = "font.info.rows.cut";
pub const PASTE_ROWS: &str = "font.info.rows.paste";
pub const DELETE_ROWS: &str = "font.info.rows.delete";

/// Most rows one selection names.
const MAX_SELECTED_ROWS: usize = 512;

/// One row of a tab: a General field or a parameter, its key and its place among the rows of that key and kind.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RowKey {
    pub param: bool,
    pub key: String,
    pub index: usize,
}

impl RowKey {
    /// `{kind, key, index}`, as the row commands take it.
    pub fn params(&self) -> Value {
        json!({ "kind": if self.param { "param" } else { "field" }, "key": self.key, "index": self.index })
    }

    fn from_json(v: &Value) -> Option<RowKey> {
        let param = match v.get("kind")?.as_str()? {
            "param" => true,
            "field" => false,
            _ => return None,
        };
        let index = match v.get("index") {
            None => 0,
            Some(n) => usize::try_from(n.as_u64()?).ok()?,
        };
        Some(RowKey { param, key: v.get("key")?.as_str()?.to_string(), index })
    }
}

/// The selected rows, the one selected last at the end, and the scope they were selected in.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RowsView {
    pub scope: Option<Scope>,
    pub rows: Vec<RowKey>,
}

/// The command that selects `rows` of `scope`; `mode` is set, toggle or add.
pub fn select_rows(scope: Scope, rows: &[RowKey], mode: &str) -> CommandRef {
    let rows: Vec<Value> = rows.iter().map(RowKey::params).collect();
    CommandRef::with_params(SELECT_ROWS, json!({ "scope": scope.name(), "rows": rows, "mode": mode }))
}

/// The tabs of the Font Info panel, in order.
#[derive(PlainData, Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum FontInfoTab {
    #[default]
    Font,
    Masters,
    Exports,
    Features,
    Other,
    Notes,
}

impl FontInfoTab {
    pub const ALL: [FontInfoTab; 6] =
        [FontInfoTab::Font, FontInfoTab::Masters, FontInfoTab::Exports, FontInfoTab::Features, FontInfoTab::Other, FontInfoTab::Notes];

    /// The `tab` parameter of `font.info.selectTab`.
    pub fn name(self) -> &'static str {
        match self {
            FontInfoTab::Font => "font",
            FontInfoTab::Masters => "masters",
            FontInfoTab::Exports => "exports",
            FontInfoTab::Features => "features",
            FontInfoTab::Other => "other",
            FontInfoTab::Notes => "notes",
        }
    }

    pub fn from_name(name: &str) -> Option<FontInfoTab> {
        FontInfoTab::ALL.into_iter().find(|t| t.name() == name)
    }
}

/// The font, or the masters or exports a tab's rows act on. The rows show the first one's values.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Target {
    pub scope: Scope,
    /// Indices into the font's masters or instances; empty for the font.
    pub indices: Vec<usize>,
}

impl Target {
    pub fn font() -> Target {
        Target { scope: Scope::Font, indices: Vec::new() }
    }

    pub fn below_font(&self) -> bool {
        self.scope != Scope::Font
    }

    /// The master or instance whose values the rows show.
    pub fn first(&self) -> usize {
        self.indices.first().copied().unwrap_or(0)
    }

    /// `scope` and, below the font, every target as `targets`.
    pub fn params(&self) -> Map<String, Value> {
        let mut p = Map::new();
        p.insert("scope".into(), json!(self.scope.name()));
        if self.below_font() {
            p.insert("targets".into(), json!(self.indices));
        }
        p
    }

    /// The target `params` names (`scope`, `targets`), as [`Target::params`] writes it.
    fn read(params: &Value) -> Result<Target, String> {
        let scope = params.get("scope").and_then(Value::as_str).unwrap_or("font");
        let scope = Scope::from_name(scope).ok_or_else(|| format!("unknown scope `{scope}`"))?;
        let indices = params
            .get("targets")
            .and_then(Value::as_array)
            .map(|list| list.iter().take(MAX_TARGETS).filter_map(Value::as_u64).filter_map(|i| usize::try_from(i).ok()).collect())
            .unwrap_or_default();
        Ok(Target { scope, indices })
    }
}

/// Most targets one request names.
const MAX_TARGETS: usize = 4096;

/// A dialog a row of the panel asked for.
#[derive(Clone, Debug, PartialEq)]
pub enum InfoDialog {
    FieldPicker(Target),
    ParameterPicker(Target),
    /// The OS/2 bit dialog of `name` (fsType, codePageRanges, unicodeRanges), its `index`th row of that name.
    Bits {
        target: Target,
        name: String,
        index: usize,
    },
    /// The PANOSE dialog of the `index`th `panose` row.
    Panose {
        target: Target,
        index: usize,
    },
    /// The glyph picker of the `index`th row of the parameter `name`, which names glyphs.
    Glyphs {
        target: Target,
        name: String,
        index: usize,
    },
    /// The row table of the `index`th row of `name` (gasp Table, meta Table, Name Table Entry).
    Records {
        target: Target,
        name: String,
        index: usize,
    },
    /// Add Feature, from the plus beside the Features heading.
    FeaturePicker,
}

/// The panel's view state.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FontInfoView {
    pub tab: FontInfoTab,
    /// The dialog asked for and not yet opened.
    pub request: Option<InfoDialog>,
    pub features: super::font_info_tabs::FeaturesView,
    pub exports: super::font_info_tabs::ExportsView,
    pub masters: super::font_info_masters::MastersTabView,
    pub rows: RowsView,
}

/// The `font.info.openDialog` parameters that ask for `dialog`.
pub fn open_params(dialog: &InfoDialog) -> Value {
    let (name, target, extra) = match dialog {
        InfoDialog::FieldPicker(t) => ("fieldPicker", t, None),
        InfoDialog::ParameterPicker(t) => ("parameterPicker", t, None),
        InfoDialog::Bits { target, name, index } => ("os2Bits", target, Some((name.as_str(), *index))),
        InfoDialog::Panose { target, index } => ("panose", target, Some(("panose", *index))),
        InfoDialog::Glyphs { target, name, index } => ("glyphs", target, Some((name.as_str(), *index))),
        InfoDialog::Records { target, name, index } => ("records", target, Some((name.as_str(), *index))),
        InfoDialog::FeaturePicker => return json!({ "dialog": "featurePicker" }),
    };
    let mut p = target.params();
    p.insert("dialog".into(), json!(name));
    if let Some((parameter, index)) = extra {
        p.insert("name".into(), json!(parameter));
        p.insert("index".into(), json!(index));
    }
    Value::Object(p)
}

/// The place in a list a run left entries at: what it added or moved, from its reply. `None` for a command that adds
/// and moves none; empty for one that removes entries.
enum Placed {
    Masters(Vec<usize>),
    Exports(Vec<usize>),
}

fn placed(id: &str, reply: &Value) -> Option<Placed> {
    let index = |v: &Value| v.as_u64().and_then(|i| usize::try_from(i).ok());
    let one = |v: Option<&Value>| v.and_then(|v| v.get("index")).and_then(index).map(|i| vec![i]);
    let list = |key: &str| -> Option<Vec<usize>> {
        let entries = reply.get(key)?.as_array()?;
        Some(entries.iter().take(MAX_TARGETS).filter_map(|e| e.get("index").and_then(index)).collect())
    };
    let moved = || -> Option<Vec<usize>> { Some(reply.get("indices")?.as_array()?.iter().take(MAX_TARGETS).filter_map(index).collect()) };
    Some(match id {
        "font.master.add" | "font.instance.toMaster" => Placed::Masters(one(reply.get("master"))?),
        "font.master.duplicate" | "font.master.addFromOtherFont" => Placed::Masters(list("masters")?),
        "font.master.reorder" => Placed::Masters(moved()?),
        "font.master.remove" => Placed::Masters(Vec::new()),
        "font.instance.add" | "font.instance.addVariableFontSetting" => Placed::Exports(one(Some(reply))?),
        "font.instance.addForEachMaster" => Placed::Exports(list("instances")?),
        "font.instance.reorder" => Placed::Exports(moved()?),
        "font.instance.remove" => Placed::Exports(Vec::new()),
        _ => return None,
    })
}

/// After a run of `id` the person asked for: the masters or exports it added or moved become the selection of their
/// tab's list, so what is typed next goes to them. A removal leaves nothing selected, which shows the first entry.
pub fn after_run(app: &mut FontCraftApp, id: &str, reply: &Value) {
    match placed(id, reply) {
        Some(Placed::Masters(mut selected)) => {
            selected.sort_unstable();
            selected.dedup();
            app.view.font_info.masters.selected = selected;
        }
        Some(Placed::Exports(mut selected)) => {
            selected.sort_unstable();
            selected.dedup();
            app.view.font_info.exports.selected = selected;
        }
        None => {}
    }
}

fn run(app: &mut FontCraftApp, command: &CommandRef) -> Result<(), String> {
    app.session.font().map_err(|e| e.to_string())?;
    let params = &command.params;
    match command.id.as_str() {
        "font.info" | "font.info.selectTab" => {
            let tab = match params.get("tab").and_then(Value::as_str) {
                Some(name) => Some(
                    FontInfoTab::from_name(name)
                        .ok_or_else(|| format!("unknown tab `{name}`: use font, masters, exports, features, other or notes"))?,
                ),
                None if command.id == "font.info.selectTab" => return Err("`tab` is required".into()),
                None => None,
            };
            if let Some(tab) = tab {
                app.view.font_info.tab = tab;
            }
            crate::shell::dock::show_window(&mut app.view.workspace, PanelId::FontInfo, true, app.view.canvas.rect);
            Ok(())
        }
        "font.info.openDialog" => {
            let target = Target::read(params)?;
            let index = params.get("index").and_then(Value::as_u64).and_then(|i| usize::try_from(i).ok()).unwrap_or(0);
            let name = params.get("name").and_then(Value::as_str).unwrap_or_default().to_string();
            let request = match params.get("dialog").and_then(Value::as_str).unwrap_or_default() {
                "fieldPicker" => InfoDialog::FieldPicker(target),
                "parameterPicker" => InfoDialog::ParameterPicker(target),
                "os2Bits" if crate::dialogs::os2_bits::BitSet::of(&name).is_some() => InfoDialog::Bits { target, name, index },
                "os2Bits" => return Err(format!("`{name}` has no bit dialog: use fsType, codePageRanges or unicodeRanges")),
                "panose" => InfoDialog::Panose { target, index },
                "glyphs" if names_of(&name) == Names::Glyph => InfoDialog::Glyphs { target, name, index },
                "glyphs" => return Err(format!("`{name}` is no parameter that names glyphs")),
                "records" if crate::dialogs::record_table::RecordKind::of(&name).is_some() => InfoDialog::Records { target, name, index },
                "records" => return Err(format!("`{name}` has no row table: use gasp Table, meta Table or Name Table Entry")),
                "featurePicker" => InfoDialog::FeaturePicker,
                other => {
                    return Err(format!(
                        "unknown dialog `{other}`: use fieldPicker, parameterPicker, os2Bits, panose, glyphs, records or featurePicker"
                    ));
                }
            };
            app.view.font_info.request = Some(request);
            Ok(())
        }
        SELECT_ROWS => {
            let scope = params.get("scope").and_then(Value::as_str).ok_or("`scope` is required")?;
            let scope = Scope::from_name(scope).ok_or_else(|| format!("unknown scope `{scope}`"))?;
            let list = params.get("rows").and_then(Value::as_array).ok_or("`rows` is required")?;
            let read = |v: &Value| RowKey::from_json(v).ok_or("each row needs `kind` (field or param) and `key`");
            let given: Vec<RowKey> = list.iter().take(MAX_SELECTED_ROWS).map(read).collect::<Result<_, _>>()?;
            let mode = params.get("mode").and_then(Value::as_str).unwrap_or("set");
            if !["set", "toggle", "add"].contains(&mode) {
                return Err(format!("unknown mode `{mode}`: use set, toggle or add"));
            }
            let view = &mut app.view.font_info.rows;
            let mut rows = if mode == "set" || view.scope != Some(scope) { Vec::new() } else { std::mem::take(&mut view.rows) };
            for key in given {
                let held = rows.iter().position(|r| *r == key);
                if let Some(at) = held {
                    rows.remove(at);
                }
                if mode != "toggle" || held.is_none() {
                    rows.push(key);
                }
            }
            rows.truncate(MAX_SELECTED_ROWS);
            *view = RowsView { scope: Some(scope), rows };
            Ok(())
        }
        COPY_ROWS | CUT_ROWS | DELETE_ROWS => {
            let (target, rows) = app.models.font_info.shown_rows().ok_or("the shown tab has no rows")?;
            let mut p = target.params();
            let keys: Vec<Value> = rows.iter().filter(|r| r.mark != RowMark::None).map(|r| r.key().params()).collect();
            if keys.is_empty() {
                return Err("no row is selected".into());
            }
            p.insert("rows".into(), json!(keys));
            let p = Value::Object(p);
            if command.id != DELETE_ROWS {
                app.sent.push(Msg::run("font.info.copyRows", p.clone()));
            }
            if command.id != COPY_ROWS {
                app.sent.push(Msg::run("font.info.removeRows", p));
            }
            Ok(())
        }
        PASTE_ROWS => {
            app.clipboard.request = Some(PasteKind::InfoRows);
            Ok(())
        }
        other => Err(format!("no Font Info command `{other}`")),
    }
}
