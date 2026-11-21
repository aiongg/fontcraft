//! Shortcuts that stand for a numbered series and run one command with the number as a parameter: Cmd+1 to Cmd+9
//! select the first nine masters (`view.selectMaster {master}`), Cmd+Alt+2 to Cmd+Alt+9 show tabs 2 to 9
//! (`view.showTab {index}`; Cmd+Alt+1 is `fontView.show`).
//!
//! Each key is its own catalog definition of the command with its parameters, through
//! `craft_ext::ui::runtime::commands::param_def` (`app::catalog`), so craft's keymap routes the chord: the typing rule
//! and focus apply as to every other menu shortcut, and a surface naming the command with those parameters shows the
//! chord. The app takes fired commands through `fired_command`, which gives back the parameters.

use craft::ui::runtime::commands::CommandRef;
use serde_json::json;

/// Masters with a shortcut: Cmd+1 to Cmd+9.
pub const MASTER_KEYS: u64 = 9;
/// The first and last tab positions with a shortcut: Cmd+Alt+2 to Cmd+Alt+9.
pub const TAB_KEYS: std::ops::RangeInclusive<u64> = 2..=9;

/// One numbered shortcut.
#[derive(Clone, Debug, PartialEq)]
pub struct NumberedKey {
    /// The command with its parameters.
    pub command: CommandRef,
    /// The definition's label, for the keymap and its conflict reports.
    pub label: String,
    /// The shortcut in the engine's notation.
    pub shortcut: String,
}

/// Every numbered shortcut.
pub fn all() -> Vec<NumberedKey> {
    let masters = (0..MASTER_KEYS).map(|master| NumberedKey {
        command: CommandRef::with_params("view.selectMaster", json!({ "master": master })),
        label: format!("Select Master {}", master + 1),
        shortcut: format!("Cmd+{}", master + 1),
    });
    let tabs = TAB_KEYS.map(|index| NumberedKey {
        command: CommandRef::with_params("view.showTab", json!({ "index": index })),
        label: format!("Show Tab {index}"),
        shortcut: format!("Cmd+Alt+{index}"),
    });
    masters.chain(tabs).collect()
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]
mod tests {
    use craft::ui::runtime::commands::CommandRef;
    use craft::ui::testing::input::key_press;
    use egui::{Key, Modifiers};
    use serde_json::json;

    use crate::testing::TestApp;

    #[test]
    fn a_numbered_shortcut_runs_its_command_with_the_number() {
        let mut t = TestApp::new();
        t.app.fire(CommandRef::new("fontView.show"));
        t.frame();
        assert!(t.app.font_view_shows());
        // Through the window's keymap, as a keyboard sends it.
        t.frames.hold(Modifiers::COMMAND | Modifiers::ALT);
        t.frame_with(vec![key_press(Key::Num2, Modifiers::COMMAND | Modifiers::ALT)]);
        t.frames.hold(Modifiers::NONE);
        t.frame();
        assert!(!t.app.font_view_shows(), "Cmd+Alt+2 shows tab 2, the first Edit View tab");
        assert_eq!(t.app.status, "", "the command got its index");
        // The control channel's keys read the same table.
        t.app.fire(CommandRef::new("fontView.show"));
        t.frame();
        let reply = t.ok("ui.key", json!({"key": "2", "cmd": true, "alt": true}));
        assert_eq!((reply["handled"].as_bool(), reply["command"].as_str()), (Some(true), Some("view.showTab")));
        assert!(!t.app.font_view_shows());
        // Cmd+9 asks for the ninth master, which the new font does not have.
        t.ok("ui.key", json!({"key": "9", "cmd": true}));
        assert!(!t.app.status.is_empty(), "a master past the last is refused");
    }
}
