//! Shortcuts that run a command with parameters: Cmd+1 to Cmd+9 for the first nine documents, layers or masters, one
//! command with an index.
//!
//! Meant for craft-ui-contract, `src/runtime/commands/mod.rs`: `AppCatalog::new` indexing its definitions by the
//! command's binding key (`CommandRef::key`: the id, then the parameters) instead of by the bare id, so one id may
//! have a definition per parameter set, each with its own default binding. The keymap already keys its catalog that
//! way, finds the bindings of a command with parameters and shows their chord on a surface that names the command with
//! those parameters, and a key press fires the definition's command, parameters included.
//!
//! The catalog's index is craft's, so this stand-in hands it a definition whose id is the whole binding key and whose
//! parameters are empty ([`param_def`]). The keymap's key for it is the same text, so every surface that names the
//! command with its parameters (a menu row, a tooltip) finds the binding and shows its chord, and the typing rule and
//! focus apply as to any other app binding. A key press fires that definition's command, the key as its id;
//! [`fired_command`] turns it back into the command with its parameters, so the app takes the fired commands through it.

use craft::ui::runtime::commands::{CommandDef, CommandRef};

/// The form of `def` the app catalog takes when its command carries parameters: the command's binding key as the
/// id, no parameters. A command without parameters comes back as it is.
pub fn param_def(mut def: CommandDef) -> CommandDef {
    let key = def.command.key();
    if key != def.command.id {
        def.command = CommandRef::new(key);
    }
    def
}

/// The command a fired `command` stands for: the id and parameters of a [`param_def`] definition, or `command` itself.
/// Ids never hold whitespace (`CommandRef::parse_key`), so an id with a space can only be a binding key.
pub fn fired_command(command: CommandRef) -> CommandRef {
    if !command.params.is_null() || !command.id.contains(' ') {
        return command;
    }
    CommandRef::parse_key(&command.id).unwrap_or(command)
}

#[cfg(test)]
mod tests {
    use craft::keymap::{Keymap, Platform, Scopes};
    use craft::ui::runtime::commands::{AppCatalog, Binding, CommandDef, CommandRef, Scope};

    use super::{fired_command, param_def};

    fn select(index: usize) -> CommandRef {
        CommandRef::parse_key(&format!("view.selectItem {{\"index\":{index}}}")).expect("a binding key")
    }

    fn defs() -> Vec<CommandDef> {
        let plain = CommandDef::new(CommandRef::new("view.selectItem"), "Select Item", Scope::app());
        let numbered = (0..3).map(|i| {
            let binding = Binding::parse(&format!("Cmd+{}", i + 1)).expect("a chord");
            param_def(CommandDef::new(select(i), format!("Select Item {}", i + 1), Scope::app()).with_default(binding))
        });
        std::iter::once(plain).chain(numbered).collect()
    }

    #[test]
    fn one_id_takes_a_binding_per_parameter_set() {
        assert!(AppCatalog::new(defs(), 1).is_ok(), "the catalog refuses no definition");
        let keymap = Keymap::new(defs(), Scopes::new(), Platform::Mac);
        let cmd2 = Binding::parse("Cmd+2").expect("a chord").trigger.chord;
        // A surface that names the command with its parameters shows the chord; the bare command has none.
        assert_eq!(keymap.shown_chord(&select(1)), Some(cmd2));
        assert_eq!(keymap.shown_chord(&CommandRef::new("view.selectItem")), None);
    }

    #[test]
    fn a_fired_definition_gives_back_the_command_and_its_parameters() {
        let def = param_def(CommandDef::new(select(2), "Select Item 3", Scope::app()));
        assert_eq!(fired_command(def.command), select(2));
        assert_eq!(fired_command(CommandRef::new("view.selectItem")), CommandRef::new("view.selectItem"));
        assert_eq!(fired_command(select(0)), select(0));
    }
}
