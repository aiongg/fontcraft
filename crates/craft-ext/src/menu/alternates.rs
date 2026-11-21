//! Alternate menu rows with their own label and shortcut: while the modifiers of a row's [`Alternate`] are held, the
//! row reads as the alternate (Tidy Up Paths becomes Tidy Up Paths for All Masters), not only runs it.
//!
//! Meant for craft-menu: a `label` on `Alternate` (`src/model.rs`), which the in-window bar's command row shows while
//! the alternate is selected (`src/context.rs`, `command_row`, which today swaps the command only), and an
//! `NSMenuItem` alternate item in the macOS backend (`src/mac.rs`), which AppKit swaps in while the modifiers are held.
//!
//! Until then the app hands the in-window bar a copy of its bar in which every row whose alternate the held modifiers
//! select carries the alternate's label and no fixed shortcut. Craft still swaps the command, so the row's chord
//! column shows the alternate command's own chord, and the row keeps its id (craft salts it with the base command),
//! so hover and keyboard focus stay on it as Alt goes down and up. With nothing held the bar is handed over as it is.

use std::borrow::Cow;

use craft::keymap::Mods;
use craft::menu::{Alternate, CommandRef, Item, MenuBar, Node};
use craft::ui::runtime::pointer;
use egui::Ui;

/// Whether `held` includes every modifier of `alternate`: the rule craft's bar uses to run the alternate.
fn selected_by(alternate: &Alternate, held: Mods) -> bool {
    let m = alternate.mods;
    (!m.cmd || held.cmd) && (!m.ctrl || held.ctrl) && (!m.alt || held.alt) && (!m.shift || held.shift)
}

/// The label `item` shows with `held`: its alternate's, from `label`, when `held` selects one that has a label.
fn alternate_label(item: &Item, held: Mods, label: &impl Fn(&CommandRef) -> Option<String>) -> Option<String> {
    item.alternate.as_ref().filter(|alternate| selected_by(alternate, held)).and_then(|alternate| label(&alternate.command))
}

/// Whether a row of `nodes`, at any depth, changes with `held`.
fn changes(nodes: &[Node], held: Mods, label: &impl Fn(&CommandRef) -> Option<String>) -> bool {
    nodes.iter().any(|node| match node {
        Node::Item(item) => alternate_label(item, held, label).is_some(),
        Node::Submenu { children, .. } => changes(children, held, label),
        _ => false,
    })
}

fn relabel(nodes: &mut [Node], held: Mods, label: &impl Fn(&CommandRef) -> Option<String>) {
    for node in nodes {
        match node {
            Node::Item(item) => {
                if let Some(text) = alternate_label(item, held, label) {
                    item.label = text;
                    item.shortcut = None;
                }
            }
            Node::Submenu { children, .. } => relabel(children, held, label),
            _ => {}
        }
    }
}

/// `bar` as it reads with `held`: each row whose alternate `held` selects shows the label `label` gives the
/// alternate's command, and no fixed shortcut. A row whose alternate has no label stays as it is. Borrowed when no
/// row changes.
pub fn alternate_labels<'a>(bar: &'a MenuBar, held: Mods, label: impl Fn(&CommandRef) -> Option<String>) -> Cow<'a, MenuBar> {
    if !bar.menus.iter().any(|menu| changes(&menu.children, held, &label)) {
        return Cow::Borrowed(bar);
    }
    let mut owned = bar.clone();
    for menu in &mut owned.menus {
        relabel(&mut menu.children, held, &label);
    }
    Cow::Owned(owned)
}

/// [`alternate_labels`] with the modifiers held in this pass, as craft's bar reads them when a row is chosen.
pub fn held_alternate_labels<'a>(ui: &Ui, bar: &'a MenuBar, label: impl Fn(&CommandRef) -> Option<String>) -> Cow<'a, MenuBar> {
    alternate_labels(bar, pointer::frame(ui.ctx()).mods, label)
}

#[cfg(test)]
mod tests {
    use std::borrow::Cow;

    use craft::keymap::Mods;
    use craft::menu::{Alternate, CommandRef, MenuBar, Node, Row, from_items};

    use super::alternate_labels;

    fn bar() -> MenuBar {
        let mut tidy = Row::new(&["Path"], "path.tidy", "Tidy Up Paths");
        tidy.alternate = Some(Alternate::new(Mods::ALT, "path.tidyAll"));
        let plain = Row::new(&["Path"], "path.reverse", "Reverse");
        from_items(&["Path"], &[tidy, plain])
    }

    fn labels(bar: &MenuBar) -> Vec<String> {
        fn walk(nodes: &[Node], out: &mut Vec<String>) {
            for node in nodes {
                match node {
                    Node::Item(item) => out.push(item.label.clone()),
                    Node::Submenu { children, .. } => walk(children, out),
                    _ => {}
                }
            }
        }
        let mut out = Vec::new();
        for menu in &bar.menus {
            walk(&menu.children, &mut out);
        }
        out
    }

    /// With Alt held the row reads as its alternate and keeps its own command (craft swaps it when chosen); without
    /// it, or for an alternate with no label, the bar is handed over unchanged.
    #[test]
    fn a_held_alternate_shows_its_own_label() {
        let bar = bar();
        let label = |command: &CommandRef| (command.id == "path.tidyAll").then(|| "Tidy Up Paths for All Masters".to_string());
        let held = alternate_labels(&bar, Mods::ALT, label);
        assert!(matches!(held, Cow::Owned(_)));
        assert_eq!(labels(&held), ["Tidy Up Paths for All Masters", "Reverse"]);
        let row = held.menus.iter().flat_map(|menu| &menu.children).find_map(|node| match node {
            Node::Item(item) if item.label.starts_with("Tidy") => Some(item),
            _ => None,
        });
        assert_eq!(row.map(|item| item.command.id.clone()), Some("path.tidy".to_string()));
        assert!(matches!(alternate_labels(&bar, Mods::NONE, label), Cow::Borrowed(_)));
        assert!(matches!(alternate_labels(&bar, Mods::ALT, |_: &CommandRef| None), Cow::Borrowed(_)));
    }
}
