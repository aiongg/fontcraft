//! The command catalog, the menu bar and the enabled state of every command.
//!
//! Engine commands come from `fontcraft_engine::command_specs()`: label, menu path, default shortcut and `enabled`.
//! Shell commands come from `shell::commands::MODULES`, tools from `fontcraft_tools::TOOLS`. All three reach the
//! framework as one `AppCatalog`, so keys, menu rows and the tool rail fire them the same way.

use craft::menu::{MenuBar, Row};
use craft::ui::runtime::commands::{AppCatalog, Binding, CommandDef, CommandInput, CommandKind, CommandRef, CommandState, Scope};
use fontcraft_engine::Session;
use fontcraft_engine::cmd::CommandSpec;

use crate::shell::commands::{self as shell_commands, UiCommand};

/// The menus of the menu bar, in order. A row whose path starts elsewhere is not shown.
pub const MENU_ORDER: &[&str] = &["File", "Edit", "Glyph", "Path", "Filter", "View", "Script", "Window", "Help"];

/// The prefix of a tool's command id: `tool.select` makes the Select tool current.
pub const TOOL_PREFIX: &str = "tool.";

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
        Catalog { app, input, menu: menu(|_| true), error, generation }
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
        self.menu = menu(|at| enabled.get(at).copied().unwrap_or(true));
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

fn definitions() -> Vec<CommandDef> {
    let mut defs = Vec::new();
    for spec in fontcraft_engine::cmd::command_specs() {
        defs.push(engine_def(spec));
    }
    for module in shell_commands::MODULES {
        for command in module.commands {
            defs.push(shell_def(command));
        }
    }
    for tool in fontcraft_tools::tool::TOOLS {
        let group = (!tool.group.is_empty()).then(|| tool.group.to_string());
        defs.push(
            CommandDef::new(CommandRef::new(format!("{TOOL_PREFIX}{}", tool.id)), tool.label, Scope::app()).with_kind(CommandKind::Tool { group }),
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

/// The menu bar: one row per engine and shell command with a menu path. `enabled(at)` answers for the engine
/// command at index `at` of `command_specs()`.
fn menu(enabled: impl Fn(usize) -> bool) -> MenuBar {
    let mut rows = Vec::new();
    for (at, spec) in fontcraft_engine::cmd::command_specs().iter().enumerate() {
        if spec.menu.is_empty() {
            continue;
        }
        let mut row = Row::new(spec.menu, CommandRef::new(spec.id), spec.label);
        row.enabled = enabled(at);
        rows.push(row);
    }
    for module in shell_commands::MODULES {
        for command in module.commands.iter().filter(|c| !c.menu.is_empty()) {
            rows.push(Row::new(command.menu, CommandRef::new(command.id), command.label));
        }
    }
    craft::menu::from_items(MENU_ORDER, &rows)
}
