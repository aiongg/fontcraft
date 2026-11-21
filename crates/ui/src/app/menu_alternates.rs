//! The Option (Alt) alternates of menu rows: while Alt is held, choosing a row runs its alternate instead, and the
//! alternate has no row of its own.
//!
//! The menu bar carries each pair as a `craft::menu::Alternate` on the base row. While Alt is held the row reads as
//! the alternate: the shell hands the bar over through `craft_ext::menu::held_alternate_labels`, which gives the row
//! the alternate's label ([`alternate_label`]), and the chord column shows the alternate command's own shortcut.

use craft::keymap::Mods;
use craft::menu::Alternate;

/// `(base, alternate)`: the command a row runs, and the one it runs while Alt is held. Both have menu paths in the
/// engine; the delete and Expand Outline pairs have none and wait for the context menus.
pub const ALTERNATES: &[(&str, &str)] = &[
    ("path.tidyUpPaths", "path.tidyUpPathsForAllMasters"),
    ("path.correctPathDirection", "path.correctPathDirectionForAllMasters"),
    ("path.reverseContours", "path.reverseContoursForAllMasters"),
    ("path.roundCoordinates", "path.roundCoordinatesForAllMasters"),
    ("path.removeOverlap", "path.removeOverlapForAllMasters"),
    ("glyph.setAnchors", "glyph.setAnchorsForAllMasters"),
    ("glyph.resetAnchors", "glyph.resetAnchorsForAllMasters"),
    ("glyph.decomposeGlyph", "glyph.decomposeGlyphForAllMasters"),
    ("glyph.updateMetrics", "glyph.updateMetricsForAllMasters"),
    ("path.addExtremes", "path.forceExtremes"),
    ("path.selectionToBackground", "path.addSelectionToBackground"),
    ("path.assignBackground", "path.clearBackground"),
];

/// `(engine command, dialog)`: engine commands whose menu row the dialog's own row takes. The dialog runs the command
/// and shows its result; the engine command keeps its id for keys, the command palette and agents.
pub const DIALOG_ROWS: &[(&str, &str)] = &[
    ("kerning.auditExceptions", "dialog.auditKerningExceptions"),
    ("kerning.matchAcrossMasters", "dialog.matchKerning"),
    ("font.overlayFonts", "dialog.overlayFonts"),
];

/// Whether the menu row of the engine command `id` is left out for a dialog's row ([`DIALOG_ROWS`]).
pub fn row_taken_by_dialog(id: &str) -> bool {
    DIALOG_ROWS.iter().any(|(command, _)| *command == id)
}

/// The command the row of `id` runs while Alt is held.
pub fn alternate(id: &str) -> Option<&'static str> {
    ALTERNATES.iter().find(|(base, _)| *base == id).map(|(_, alt)| *alt)
}

/// The row the command `id` is the Alt alternate of; such a command has no row of its own.
pub fn alternate_of(id: &str) -> Option<&'static str> {
    ALTERNATES.iter().find(|(_, alt)| *alt == id).map(|(base, _)| *base)
}

/// The label the row shows while Alt is held, when `id` is an alternate: the command's own label.
pub fn alternate_label(id: &str) -> Option<&'static str> {
    alternate_of(id)?;
    fontcraft_engine::command_specs().iter().find(|spec| spec.id == id).map(|spec| spec.label)
}

/// The menu alternate of the row of `id`.
pub fn menu_alternate(id: &str) -> Option<Alternate> {
    alternate(id).map(|alt| Alternate::new(Mods::ALT, alt))
}

#[cfg(test)]
mod tests {
    use craft::menu::Node;

    use super::{ALTERNATES, alternate_of};
    use crate::app::catalog::Catalog;

    fn rows<'a>(nodes: &'a [Node], out: &mut Vec<&'a craft::menu::Item>) {
        for node in nodes {
            match node {
                Node::Item(item) => out.push(item),
                Node::Submenu { children, .. } => rows(children, out),
                _ => {}
            }
        }
    }

    /// Every pair names two engine commands with menu rows; the menu bar shows the base with the alternate on it
    /// and no row for the alternate.
    #[test]
    fn alternates_ride_on_their_base_rows() {
        let specs = fontcraft_engine::command_specs();
        let missing: Vec<&str> =
            ALTERNATES.iter().flat_map(|(base, alt)| [*base, *alt]).filter(|id| !specs.iter().any(|s| s.id == *id && !s.menu.is_empty())).collect();
        assert!(missing.is_empty(), "commands without a menu row: {missing:?}");
        let catalog = Catalog::build();
        let mut items = Vec::new();
        for menu in &catalog.menu.menus {
            rows(&menu.children, &mut items);
        }
        for (base, alt) in ALTERNATES {
            let row = items.iter().find(|i| i.command.id == *base).unwrap_or_else(|| panic!("a row for `{base}`"));
            assert_eq!(row.alternate.as_ref().map(|a| a.command.id.as_str()), Some(*alt));
            assert!(row.alternate.as_ref().is_some_and(|a| a.mods.alt));
            assert!(!items.iter().any(|i| i.command.id == *alt), "`{alt}` has no row of its own");
        }
        assert_eq!(alternate_of("path.forceExtremes"), Some("path.addExtremes"));
    }

    /// With Alt held, a row with an alternate reads as the alternate: Tidy Up Paths shows the label of
    /// `path.tidyUpPathsForAllMasters`; without Alt the bar is unchanged.
    #[test]
    fn held_alt_shows_the_alternate_label() {
        let catalog = Catalog::build();
        let label = |command: &craft::menu::CommandRef| super::alternate_label(&command.id).map(str::to_string);
        let held = craft_ext::menu::alternate_labels(&catalog.menu, craft::keymap::Mods::ALT, label);
        let mut items = Vec::new();
        for menu in &held.menus {
            rows(&menu.children, &mut items);
        }
        let row = items.iter().find(|i| i.command.id == "path.tidyUpPaths").expect("the Tidy Up Paths row");
        assert_eq!(Some(row.label.as_str()), super::alternate_label("path.tidyUpPathsForAllMasters"));
        assert_ne!(Some(row.label.as_str()), fontcraft_engine::command_specs().iter().find(|s| s.id == "path.tidyUpPaths").map(|s| s.label));
        assert!(matches!(craft_ext::menu::alternate_labels(&catalog.menu, craft::keymap::Mods::NONE, label), std::borrow::Cow::Borrowed(_)));
    }

    /// Audit Kerning Exceptions…, Match Kerning… and Overlay Fonts… show once each: the dialogs' rows, not the engine
    /// commands' (M13.21b, M13.66).
    #[test]
    fn dialog_rows_take_the_engine_rows() {
        let catalog = Catalog::build();
        let mut items = Vec::new();
        for menu in &catalog.menu.menus {
            rows(&menu.children, &mut items);
        }
        for (command, dialog) in super::DIALOG_ROWS {
            assert!(items.iter().any(|i| i.command.id == *dialog), "a row for `{dialog}`");
            assert!(!items.iter().any(|i| i.command.id == *command), "no row for `{command}`");
        }
    }
}
