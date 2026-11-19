//! The installed command tables: every command the process knows, in menu order.
//!
//! The tables are installed once per process ([`install`]), before any session runs a command.
//! Lookups ([`find`], [`command_specs`], [`lookup`]) read the installed tables; before the
//! install they find nothing, and [`lookup`] says so with [`EngineError::NotInstalled`].

use std::any::Any;
use std::collections::HashMap;
use std::sync::OnceLock;

use super::CommandSpec;
use crate::{EngineError, Result};

/// One area's command table: its specs in the order its menus list them.
pub type CommandTable = fn() -> Vec<CommandSpec>;

/// Engine-wide hooks a command area adds to the command pipeline. None yet.
#[derive(Default)]
pub struct Hooks {}

/// A session slot ([`crate::slots`]) that every new session starts with, for state that must
/// exist before anything can create it through `&mut Session`: a handle a reader holding only
/// `&Session` shares with another thread. Other slots are created on first use.
#[derive(Clone, Copy)]
pub struct SlotInit(fn() -> Box<dyn Any + Send + Sync>);

impl SlotInit {
    /// A slot of type `T` holding its default value.
    pub fn of<T: Any + Send + Sync + Default>() -> Self {
        SlotInit(|| Box::new(T::default()))
    }

    pub(crate) fn create(&self) -> Box<dyn Any + Send + Sync> {
        (self.0)()
    }
}

/// What [`install`] installs: the command tables in registry order, the hooks and the
/// per-session state of the command areas.
#[derive(Default)]
pub struct Install {
    pub commands: Vec<CommandTable>,
    pub hooks: Hooks,
    pub slots: Vec<SlotInit>,
}

/// The installed tables and what was built from them.
struct Installed {
    tables: Vec<CommandTable>,
    slots: Vec<SlotInit>,
    /// The commands, or why the tables were refused.
    registry: std::result::Result<Registry, String>,
}

struct Registry {
    specs: Vec<CommandSpec>,
    index: HashMap<&'static str, usize>,
}

static INSTALLED: OnceLock<Installed> = OnceLock::new();

/// Installs the command tables for the whole process. The first call builds the registry; a
/// later call with the same tables does nothing, and one with other tables is an error. Tables
/// that register one id twice are refused, and every later call reports that.
pub fn install(install: Install) -> Result<()> {
    let Install { commands, hooks: _, slots } = install;
    let done = INSTALLED.get_or_init(|| Installed { registry: build(&commands), tables: commands.clone(), slots: slots.clone() });
    let same = done.tables.len() == commands.len()
        && done.tables.iter().zip(&commands).all(|(a, b)| std::ptr::fn_addr_eq(*a, *b))
        && done.slots.len() == slots.len()
        && done.slots.iter().zip(&slots).all(|(a, b)| std::ptr::fn_addr_eq(a.0, b.0));
    if !same {
        return Err(EngineError::Internal("other command tables are already installed in this process".into()));
    }
    done.registry.as_ref().map(|_| ()).map_err(|why| EngineError::Internal(why.clone()))
}

fn build(tables: &[CommandTable]) -> std::result::Result<Registry, String> {
    let specs: Vec<CommandSpec> = tables.iter().flat_map(|table| table()).collect();
    let mut index = HashMap::with_capacity(specs.len());
    let mut twice = Vec::new();
    for (i, spec) in specs.iter().enumerate() {
        if index.insert(spec.id, i).is_some() {
            twice.push(spec.id);
        }
    }
    if twice.is_empty() {
        Ok(Registry { specs, index })
    } else {
        Err(format!("the command tables register these ids more than once: {}", twice.join(", ")))
    }
}

fn registry() -> Result<&'static Registry> {
    match INSTALLED.get().map(|done| &done.registry) {
        Some(Ok(r)) => Ok(r),
        Some(Err(why)) => Err(EngineError::Internal(why.clone())),
        None => Err(EngineError::NotInstalled),
    }
}

/// The slots every new session starts with; none before the install.
pub(crate) fn slot_inits() -> &'static [SlotInit] {
    INSTALLED.get().map_or(&[], |done| done.slots.as_slice())
}

/// Whether the command tables are installed.
pub fn installed() -> bool {
    registry().is_ok()
}

/// The installed command `id`, if there is one.
pub fn find(id: &str) -> Option<&'static CommandSpec> {
    let r = registry().ok()?;
    r.index.get(id).and_then(|&i| r.specs.get(i))
}

/// The installed command `id`: [`EngineError::NotInstalled`] before the install,
/// [`EngineError::UnknownCommand`] when no table has it.
pub fn lookup(id: &str) -> Result<&'static CommandSpec> {
    let r = registry()?;
    r.index.get(id).and_then(|&i| r.specs.get(i)).ok_or_else(|| EngineError::UnknownCommand(id.to_string()))
}

/// Every installed command, in registry order (the order menus and listings show them in);
/// empty before the install.
pub fn command_specs() -> &'static [CommandSpec] {
    registry().map_or(&[], |r| r.specs.as_slice())
}
