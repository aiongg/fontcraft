//! The command catalog, the menu bar and the enabled state of every command.
//!
//! Engine commands come from `fontcraft_engine::command_specs()`: label, menu path, default shortcut and `enabled`.
//! Shell commands come from `shell::commands::MODULES`, tools from `fontcraft_tools::TOOLS`, the shortcuts that run a
//! command with a number from `shell::numbered_keys`. All of them reach the framework as one `AppCatalog`, so keys,
//! menu rows and the tool rail fire them the same way.
//!
//! A menu row is greyed out when its engine command's `enabled` refuses or the selection gives it nothing to act on,
//! and an on/off switch's row shows a check mark while the switch is on (`menu_states`).

use craft::menu::{ItemKind, MenuBar, Row};
use craft::ui::runtime::commands::{AppCatalog, Binding, CommandDef, CommandInput, CommandKind, CommandRef, CommandState, Scope};
use fontcraft_engine::Session;
use fontcraft_engine::cmd::CommandSpec;
use serde_json::Value;

use crate::shell::commands::{self as shell_commands, UiCommand};

/// The menus of the menu bar, in order. A row whose path starts elsewhere is not shown.
pub const MENU_ORDER: &[&str] = &["File", "Edit", "Font", "Glyph", "Path", "Filter", "View", "Script", "Window", "Help"];

/// The prefix of a tool's command id: `tool.select` makes the Select tool current.
pub const TOOL_PREFIX: &str = "tool.";

/// The Select tool's command, which also makes current the pointer tool its `tool` parameter names. It is a tool
/// definition in the catalog and a shell command for the command lists; a `tool` that names no tool reaches the shell
/// command, which refuses it.
pub const SELECT_TOOL: &str = "tool.select";

/// Whether the shortcut of the command `id` works while Font View shows (`font_view`) with the tool `tool` current: its
/// engine command's `when` (`CommandSpec::when`) names where. `fontView` works only in Font View; `editView`,
/// `textMode` and `tool:<id>` (that tool current) only in the edit view. A command with no `when`, or one that names a
/// panel or a dialog, works everywhere: the panel or dialog that has the keyboard keeps its own keys.
pub fn shortcut_works(id: &str, font_view: bool, tool: &str) -> bool {
    let when = fontcraft_engine::cmd::command_specs().iter().find(|spec| spec.id == id).and_then(|spec| spec.when);
    match when {
        Some("fontView") => font_view,
        Some("editView" | "textMode") => !font_view,
        Some(when) => when.strip_prefix("tool:").is_none_or(|own| !font_view && own == tool),
        None => true,
    }
}

/// What a fired command is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Engine,
    Shell,
    Tool(&'static str),
}

/// The catalog the framework reads every frame, and the menu bar.
pub struct Catalog {
    /// `None` when the definitions were refused (two with one id); the message is in `error`.
    app: Option<AppCatalog>,
    /// The catalog with each command's enabled state, rebuilt when the engine moves.
    input: CommandInput,
    /// The menu bar with each row's enabled state.
    pub menu: MenuBar,
    /// Why the catalog could not be built.
    pub error: Option<String>,
    generation: u64,
    /// Each engine command's enabled state, in `command_specs()` order, as last asked.
    enabled: Vec<bool>,
    /// The rows that come from the extensions (`menus_dynamic`), after the catalog's own, and what they were made for.
    dynamic: Vec<Row>,
    dynamic_key: Option<u64>,
    /// The Window menu's panel rows (`shell::window::refresh_menu`), after the dynamic rows, and what they were made for.
    panels: Vec<Row>,
    panels_key: Option<u64>,
    /// The Window menu's document rows (`document_menu`), after the panel rows, and what they were made for.
    documents: Vec<Row>,
    documents_key: Option<u64>,
    /// The rows of File › Open Recent (`document_menu`), after File › Open…, and what they were made for.
    recent: Vec<Row>,
    recent_key: Option<u64>,
    /// Each switch row's state (`menu_states::SWITCHES`), as last read.
    checks: Vec<(&'static str, bool)>,
    /// The engine rows the selection greys out, with why, and what they were worked out for.
    greyed: Vec<(&'static str, String)>,
    greyed_key: Option<u64>,
    /// The commands that open a dialog when fired without parameters (`dialogs::for_command`), asked once.
    dialogs: Vec<&'static str>,
}

impl Catalog {
    /// The catalog of every engine command, shell command and tool, with every command enabled.
    pub fn build() -> Catalog {
        let generation = 1;
        let (app, error) = match AppCatalog::new(definitions(), generation) {
            Ok(app) => (Some(app), None),
            Err(e) => (None, Some(format!("command catalog: {e:?}"))),
        };
        let input = app.clone().map_or_else(CommandInput::new, |app| CommandInput::new().catalog(app));
        Catalog {
            app,
            input,
            menu: menu(|_| true, &MenuStates::default(), &[], &[]),
            error,
            generation,
            enabled: Vec::new(),
            dynamic: Vec::new(),
            dynamic_key: None,
            panels: Vec::new(),
            panels_key: None,
            documents: Vec::new(),
            documents_key: None,
            recent: Vec::new(),
            recent_key: None,
            checks: Vec::new(),
            greyed: Vec::new(),
            greyed_key: None,
            dialogs: crate::dialogs::commands_with_dialogs(),
        }
    }

    /// The commands of this frame: the catalog and each command's state.
    pub fn input(&self) -> CommandInput {
        self.input.clone()
    }

    /// Asks every engine command's `enabled` rule again; called when the engine moved.
    pub fn refresh(&mut self, session: &Session) {
        let specs = fontcraft_engine::cmd::command_specs();
        let enabled: Vec<bool> = specs.iter().map(|spec| (spec.enabled)(session).is_ok()).collect();
        let mut input = self.app.clone().map_or_else(CommandInput::new, |app| CommandInput::new().catalog(app));
        for (spec, on) in specs.iter().zip(&enabled) {
            input = input.state(CommandRef::new(spec.id), CommandState::new().enabled(*on));
        }
        self.input = input;
        self.enabled = enabled;
        self.rebuild_menu();
    }

    /// Puts the extensions' rows at the end of their menus. `key` names what they show; the rows are made and the
    /// menu bar rebuilt only when it changed.
    pub fn set_dynamic(&mut self, key: u64, rows: impl FnOnce() -> Vec<Row>) {
        if self.dynamic_key == Some(key) {
            return;
        }
        self.dynamic_key = Some(key);
        self.dynamic = rows();
        self.rebuild_menu();
    }

    /// Puts the Window menu's panel rows after the dynamic rows; rebuilt only when `key` changed.
    pub fn set_panel_rows(&mut self, key: u64, rows: impl FnOnce() -> Vec<Row>) {
        if self.panels_key == Some(key) {
            return;
        }
        self.panels_key = Some(key);
        self.panels = rows();
        self.rebuild_menu();
    }

    /// Puts the open documents' rows at the end of the Window menu; rebuilt only when `key` changed.
    pub fn set_document_rows(&mut self, key: u64, rows: impl FnOnce() -> Vec<Row>) {
        if self.documents_key == Some(key) {
            return;
        }
        self.documents_key = Some(key);
        self.documents = rows();
        self.rebuild_menu();
    }

    /// Sets the rows of File › Open Recent; rebuilt only when `key` changed.
    pub fn set_recent_rows(&mut self, key: u64, rows: impl FnOnce() -> Vec<Row>) {
        if self.recent_key == Some(key) {
            return;
        }
        self.recent_key = Some(key);
        self.recent = rows();
        self.rebuild_menu();
    }

    /// Sets the switch rows' check marks; the menu bar is rebuilt only when one changed.
    pub fn set_checks(&mut self, checks: Vec<(&'static str, bool)>) {
        if self.checks == checks {
            return;
        }
        self.checks = checks;
        self.rebuild_menu();
    }

    /// Whether the command `id` opens a dialog when fired without parameters.
    pub fn has_dialog(&self, id: &str) -> bool {
        self.dialogs.contains(&id)
    }

    /// Whether the rows the selection greys out were last worked out for `key`.
    pub fn greyed_for(&self, key: u64) -> bool {
        self.greyed_key == Some(key)
    }

    /// Sets the engine rows the selection greys out, with why, worked out for `key`; the menu bar is rebuilt only when
    /// they changed.
    pub fn set_greyed(&mut self, key: u64, greyed: Vec<(&'static str, String)>) {
        self.greyed_key = Some(key);
        if self.greyed == greyed {
            return;
        }
        self.greyed = greyed;
        self.rebuild_menu();
    }

    fn rebuild_menu(&mut self) {
        let enabled = &self.enabled;
        let extra: Vec<Row> = self.dynamic.iter().chain(&self.panels).chain(&self.documents).cloned().collect();
        let states = MenuStates { checks: &self.checks, greyed: &self.greyed };
        self.menu = menu(|at| enabled.get(at).copied().unwrap_or(true), &states, &self.recent, &extra);
    }

    /// The catalog's generation: it changes only when the definitions do.
    pub fn generation(&self) -> u64 {
        self.generation
    }
}

/// What the command `id` is.
pub fn kind(id: &str) -> Kind {
    if let Some(tool) = id.strip_prefix(TOOL_PREFIX).and_then(|name| fontcraft_tools::tool::TOOLS.iter().find(|t| t.id == name)) {
        return Kind::Tool(tool.id);
    }
    if shell_commands::find(id).is_some() { Kind::Shell } else { Kind::Engine }
}

/// The tool the tool command `command` makes current: its own tool `own`, or for [`SELECT_TOOL`] the pointer tool its
/// `tool` parameter names. Err says why `tool` names none.
pub fn chosen_tool(own: &'static str, command: &CommandRef) -> Result<&'static str, String> {
    if command.id != SELECT_TOOL {
        return Ok(own);
    }
    match command.params.get("tool") {
        None | Some(Value::Null) => Ok(own),
        Some(Value::String(id)) => fontcraft_tools::tool::TOOLS.iter().find(|t| t.id == id.as_str()).map(|t| t.id).ok_or_else(|| {
            let known: Vec<&str> = fontcraft_tools::tool::TOOLS.iter().map(|t| t.id).collect();
            format!("no tool `{id}`; the tools are {}", known.join(", "))
        }),
        Some(_) => Err("`tool` must be a tool id (tool.list names them)".into()),
    }
}

fn definitions() -> Vec<CommandDef> {
    let mut defs = Vec::new();
    for spec in fontcraft_engine::cmd::command_specs() {
        defs.push(engine_def(spec));
    }
    for module in shell_commands::MODULES {
        // A shell command with a tool's id (`tool.select`) is that tool's definition, made below.
        for command in module.commands.iter().filter(|c| !matches!(kind(c.id), Kind::Tool(_))) {
            defs.push(shell_def(command));
        }
    }
    // One definition per numbered shortcut (Cmd+1 for the first master), its parameters folded into the id.
    for key in crate::shell::numbered_keys::all() {
        if let Ok(binding) = Binding::parse(&key.shortcut) {
            defs.push(craft_ext::ui::runtime::commands::param_def(CommandDef::new(key.command, key.label, Scope::app()).with_default(binding)));
        }
    }
    for tool in fontcraft_tools::tool::TOOLS {
        let group = (!tool.group.is_empty()).then(|| tool.group.to_string());
        defs.push(
            CommandDef::new(CommandRef::new(format!("{TOOL_PREFIX}{}", tool.id)), tool.label, Scope::app())
                .with_kind(CommandKind::Tool { group })
                .with_icon(crate::icons::for_tool(tool.id)),
        );
    }
    defs
}

fn engine_def(spec: &CommandSpec) -> CommandDef {
    let mut def = CommandDef::new(CommandRef::new(spec.id), spec.label, Scope::app());
    if !spec.menu.is_empty() {
        def = def.with_path(spec.menu.iter().map(|s| s.to_string()).collect());
    }
    // A shortcut the keymap cannot read stays unbound; the command still runs from its menu row.
    if let Some(binding) = spec.shortcut.and_then(|text| Binding::parse(text).ok()) {
        def = def.with_default(binding);
    }
    def
}

fn shell_def(command: &UiCommand) -> CommandDef {
    let mut def = CommandDef::new(CommandRef::new(command.id), command.label, Scope::app());
    if !command.menu.is_empty() {
        def = def.with_path(command.menu.iter().map(|s| s.to_string()).collect());
    }
    if let Some(binding) = command.shortcut.and_then(|text| Binding::parse(text).ok()) {
        def = def.with_default(binding);
    }
    def
}

/// What the live state says about the catalog's own rows.
#[derive(Default)]
struct MenuStates<'a> {
    checks: &'a [(&'static str, bool)],
    greyed: &'a [(&'static str, String)],
}

impl MenuStates<'_> {
    /// `row` with its check mark when it is a switch's.
    fn checked(&self, mut row: Row, id: &str) -> Row {
        if let Some((_, on)) = self.checks.iter().find(|(c, _)| *c == id) {
            row.kind = ItemKind::Check(*on);
        }
        row
    }

    /// Why the selection greys out the row of `id`, if it does.
    fn greyed(&self, id: &str) -> Option<&str> {
        self.greyed.iter().find(|(g, _)| *g == id).map(|(_, why)| why.as_str())
    }
}

/// The menu bar: one row per engine and shell command with a menu path, the `recent` rows after File › Open…, then the
/// `extra` rows. `enabled(at)` answers
/// for the engine command at index `at` of `command_specs()`; a row the selection greys out carries why as its tooltip.
fn menu(enabled: impl Fn(usize) -> bool, states: &MenuStates<'_>, recent: &[Row], extra: &[Row]) -> MenuBar {
    let mut rows = Vec::new();
    for (at, spec) in fontcraft_engine::cmd::command_specs().iter().enumerate() {
        if spec.menu.is_empty() || super::menu_alternates::alternate_of(spec.id).is_some() || super::menu_alternates::row_taken_by_dialog(spec.id) {
            continue;
        }
        let mut row = states.checked(Row::new(spec.menu, CommandRef::new(spec.id), spec.label), spec.id);
        row.enabled = enabled(at);
        if row.enabled
            && let Some(why) = states.greyed(spec.id)
        {
            row.enabled = false;
            row.tooltip = Some(why.to_string());
        }
        row.alternate = super::menu_alternates::menu_alternate(spec.id);
        rows.push(row);
        if spec.id == "file.open" {
            rows.extend(recent.iter().cloned());
        }
    }
    for module in shell_commands::MODULES {
        // The recent files' submenu stands for `file.openRecent`.
        let shown =
            |c: &&UiCommand| !c.menu.is_empty() && c.id != super::document_menu::OPEN_RECENT && super::menu_alternates::alternate_of(c.id).is_none();
        for command in module.commands.iter().filter(shown) {
            let mut row = states.checked(Row::new(command.menu, CommandRef::new(command.id), command.label), command.id);
            row.alternate = super::menu_alternates::menu_alternate(command.id);
            rows.push(row);
        }
    }
    rows.extend(extra.iter().cloned());
    craft::menu::from_items(MENU_ORDER, &rows)
}
