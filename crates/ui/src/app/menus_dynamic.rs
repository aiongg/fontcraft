//! Menu rows that come from the installed modules and the Macros folder rather than from the command catalog. Each
//! row runs a fixed command with parameters naming the module or macro, so the catalog stays fixed:
//!
//! | Menu | Rows | Command |
//! |---|---|---|
//! | Script | one per macro of the Macros folder in the listing's order (folders first, then titles in natural order); subfolders as submenus; the description as tooltip; unreadable files greyed out | `script.run {"macro": path}` |
//! | Filter | one per filter module, by name, below a separator | `plugin.runFilter {"id"}` with the module's defaults |
//! | View | one Show row per reporter module, by name, checked while shown | `view.showReporter {"id", "on"}` |
//! | View › Font View Strike | Outlines, then one row per bitmap strike of the active font (`12`, or `12@4` for a grey one), the shown one checked; only when the font has strikes | `view.setFontViewStrike {"strike"}` |
//!
//! The rows are rebuilt when the extensions, the shown reporters, the font's strikes or the strike shown change
//! ([`refresh`]). The selection becomes the
//! targets when a row fires ([`fill_targets`]): Font View's selected glyphs, or the edited glyph and layer with the
//! selected nodes. Rows: `script.menu.mirrorsFolder`, `script.menu.subfolders`, `script.menu.tooltip`,
//! `view.reporterPluginsInViewMenu`, `filter.menu.pluginFiltersListed`, `fontView.batch.pluginsAndScripts`.

use std::hash::{DefaultHasher, Hash, Hasher};

use craft::menu::{ItemKind, Row};
use craft::ui::runtime::commands::CommandRef;
use fontcraft_engine::Session;
use fontcraft_engine::extensions::MAX_MACRO_FOLDER_DEPTH;
use fontcraft_engine::macros::natural_cmp;
use fontcraft_engine::plugins::{Installed, Kind};
use serde_json::{Map, Value, json};

use super::FontCraftApp;
use super::font_view::StrikeView;
use super::targets;
use crate::shell::font_view::SET_STRIKE;

/// The command a filter module's row runs.
pub const RUN_FILTER: &str = "plugin.runFilter";
/// The command a macro's row runs.
pub const RUN_MACRO: &str = "script.run";
/// The command a reporter's row runs.
pub const SHOW_REPORTER: &str = "view.showReporter";
/// Most rows one menu section lists; the rest are left out.
pub const MAX_ROWS: usize = 512;

/// The label of a separator row (`craft::menu::from_items`).
const SEPARATOR: &str = "---";

/// Rebuilds the dynamic rows when what they show changed: the extensions' revision, safe mode or the shown reporters.
pub fn refresh(app: &mut FontCraftApp) {
    let FontCraftApp { session, view, catalog, .. } = app;
    let reporters = &view.extensions.reporters;
    let mut h = DefaultHasher::new();
    session.extensions().revision().hash(&mut h);
    session.extensions().safe_mode.hash(&mut h);
    reporters.hash(&mut h);
    let strike = view.font_view.strike;
    strike.map(|s| (s.ppem, s.depth)).hash(&mut h);
    if let Ok(font) = session.font() {
        for s in font.bitmap_strikes.iter() {
            (s.ppem, s.depth).hash(&mut h);
        }
    }
    catalog.set_dynamic(h.finish(), || rows(session, reporters, strike));
}

/// Every dynamic row, the Script, Filter and View sections in turn.
pub fn rows(session: &Session, reporters: &[String], strike: Option<StrikeView>) -> Vec<Row> {
    let mut out = Vec::new();
    section(&mut out, "Script", script_rows(session));
    section(&mut out, "Filter", filter_rows(session));
    section(&mut out, "View", reporter_rows(session, reporters));
    out.extend(strike_rows(session, strike));
    out
}

/// The submenu the strike rows sit in.
pub const STRIKE_MENU: [&str; 2] = ["View", "Font View Strike"];

/// View › Font View Strike: Outlines, then each strike of the active font, the one shown checked. None when the font
/// has no strikes.
fn strike_rows(session: &Session, shown: Option<StrikeView>) -> Vec<Row> {
    let Ok(font) = session.font() else { return Vec::new() };
    if font.bitmap_strikes.is_empty() {
        return Vec::new();
    }
    let magnification = shown.map_or(1, |s| s.magnification);
    let mut outlines = Row::new(&STRIKE_MENU, CommandRef::with_params(SET_STRIKE, json!({"strike": null})), "Outlines");
    outlines.kind = ItemKind::Check(shown.is_none());
    let mut out = vec![outlines];
    for s in font.bitmap_strikes.iter().take(MAX_ROWS) {
        let params = json!({"strike": {"ppem": s.ppem, "depth": s.depth}, "magnification": magnification});
        let mut row = Row::new(&STRIKE_MENU, CommandRef::with_params(SET_STRIKE, params), s.label());
        row.kind = ItemKind::Check(shown.is_some_and(|v| v.ppem == s.ppem && v.depth == s.depth));
        out.push(row);
    }
    out
}

/// `rows` after a separator at the end of the menu `menu`, when there are any.
fn section(out: &mut Vec<Row>, menu: &str, rows: Vec<Row>) {
    if rows.is_empty() {
        return;
    }
    out.push(Row::new(&[menu], CommandRef::new(SEPARATOR), SEPARATOR));
    out.extend(rows);
}

/// The Script menu: one row per macro, subfolders as submenus, the description as tooltip. A file that cannot be
/// read is listed greyed out with the reason.
fn script_rows(session: &Session) -> Vec<Row> {
    session
        .extensions()
        .macros
        .iter()
        .take(MAX_ROWS)
        .map(|e| {
            let mut path: Vec<&str> = std::iter::once("Script").chain(e.path.split('/')).collect();
            path.pop();
            path.truncate(MAX_MACRO_FOLDER_DEPTH.saturating_add(1));
            let mut row = Row::new(&path, CommandRef::with_params(RUN_MACRO, json!({"macro": e.path})), e.title.clone());
            row.enabled = e.error.is_none();
            row.tooltip = match &e.error {
                Some(error) => Some(format!("This macro cannot be read: {error}")),
                None => Some(e.description.clone()).filter(|d| !d.trim().is_empty()),
            };
            row
        })
        .collect()
}

/// The installed modules of `kind`, in the order of their names.
fn modules_of(session: &Session, kind: Kind) -> Vec<&Installed> {
    let mut list: Vec<&Installed> = session.extensions().registry.list().iter().filter(|m| m.plugin.kind() == kind).collect();
    list.sort_by(|a, b| {
        let (a, b) = (a.plugin.manifest(), b.plugin.manifest());
        natural_cmp(&a.name, &b.name).then_with(|| a.id.cmp(&b.id))
    });
    list.truncate(MAX_ROWS);
    list
}

/// The module's description, if it has one.
fn description(m: &Installed) -> Option<String> {
    Some(m.plugin.manifest().description.clone()).filter(|d| !d.trim().is_empty())
}

/// The Filter menu: one row per filter module, run with its default parameters on the selection. A module that
/// cannot run now (turned off, safe mode) is greyed out with the reason as tooltip.
fn filter_rows(session: &Session) -> Vec<Row> {
    let ext = session.extensions();
    modules_of(session, Kind::Filter)
        .into_iter()
        .map(|m| {
            let id = m.plugin.id();
            let mut row = Row::new(&["Filter"], CommandRef::with_params(RUN_FILTER, json!({"id": id})), m.plugin.manifest().name.clone());
            let why = ext.runnable(id, Kind::Filter).err();
            row.enabled = why.is_none();
            row.tooltip = why.or_else(|| description(m));
            row
        })
        .collect()
}

/// The View menu: one Show row per reporter module, checked while it is shown. A module that cannot run now keeps
/// its row, so its switch can still be turned off; the tooltip says why nothing is drawn.
fn reporter_rows(session: &Session, reporters: &[String]) -> Vec<Row> {
    let ext = session.extensions();
    modules_of(session, Kind::Reporter)
        .into_iter()
        .map(|m| {
            let id = m.plugin.id();
            let shown = reporters.iter().any(|r| r == id);
            let command = CommandRef::with_params(SHOW_REPORTER, json!({"id": id, "on": !shown}));
            let mut row = Row::new(&["View"], command, format!("Show {}", m.plugin.manifest().name));
            row.kind = ItemKind::Check(shown);
            row.tooltip = ext.runnable(id, Kind::Reporter).err().or_else(|| description(m));
            row
        })
        .collect()
}

/// A filter or macro row's command with the selection's targets added, when its parameters name none: the targets a
/// menu command gets (Font View's selected glyphs in its master, or the edited glyph and layer with the selected
/// nodes). A macro takes them as `glyphs`, and in Edit View their layer as `layers`. Any other command is returned as
/// it is. With no glyph chosen ([`FontCraftApp::targets_chosen`]) a macro runs without targets and a filter refuses.
pub fn fill_targets(app: &mut FontCraftApp, command: CommandRef) -> Result<CommandRef, String> {
    let is_macro = command.id == RUN_MACRO;
    if !is_macro && command.id != RUN_FILTER {
        return Ok(command);
    }
    let Value::Object(mut params) = command.params.clone() else { return Ok(command) };
    if ["glyph", "glyphs", "layer", "layers"].iter().any(|k| params.contains_key(*k)) {
        return Ok(command);
    }
    if !app.targets_chosen() {
        return if is_macro { Ok(command) } else { Err(super::select_glyph_first(&command.id)) };
    }
    let found = app.with_targets(CommandRef::new(RUN_FILTER));
    let mut sets = targets::target_sets(&found, RUN_FILTER);
    if is_macro {
        params.extend(macro_targets(&sets));
        return Ok(CommandRef::with_params(command.id, Value::Object(params)));
    }
    let Some(spec) = fontcraft_engine::find_command(RUN_FILTER) else { return Ok(command) };
    match sets.pop() {
        Some(only) if sets.is_empty() => {
            params.extend(only);
            Ok(CommandRef::with_params(command.id, Value::Object(params)))
        }
        // The text selection's glyphs show several layers: one run per layer.
        Some(last) => {
            sets.push(last);
            let runs = sets.into_iter().map(|set| {
                let mut run = params.clone();
                run.extend(set);
                run
            });
            Ok(targets::batch(spec, runs))
        }
        None => Ok(command),
    }
}

/// The arguments a macro gets from a filter's targets: the edited glyph and its layer as one-item lists, or the
/// selected glyphs alone, each once.
fn macro_targets(sets: &[Map<String, Value>]) -> Map<String, Value> {
    let mut out = Map::new();
    if let Some(one) = sets.first()
        && let Some(glyph) = one.get("glyph")
    {
        out.insert("glyphs".into(), json!([glyph]));
        if let Some(layer) = one.get("layer") {
            out.insert("layers".into(), json!([layer]));
        }
        return out;
    }
    let named = sets.iter().filter_map(|set| set.get("glyphs").and_then(Value::as_array)).flatten();
    let mut seen = std::collections::HashSet::new();
    let glyphs: Vec<&str> = named.filter_map(Value::as_str).filter(|name| seen.insert(*name)).collect();
    if !glyphs.is_empty() {
        out.insert("glyphs".into(), json!(glyphs));
    }
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

    use super::*;

    #[test]
    fn a_macro_in_a_subfolder_is_a_script_submenu_row_and_takes_the_edited_glyph_as_a_list() {
        let targets = json!({"glyph": "A", "layer": "m01", "points": [[0, 1]]});
        let Value::Object(targets) = targets else { panic!() };
        assert_eq!(Value::Object(macro_targets(&[targets])), json!({"glyphs": ["A"], "layers": ["m01"]}));
        let only = json!({"glyphs": ["A", "B"], "layer": "m01"});
        let Value::Object(only) = only else { panic!() };
        assert_eq!(Value::Object(macro_targets(std::slice::from_ref(&only))), json!({"glyphs": ["A", "B"]}));
        let more = json!({"glyphs": ["B", "C"], "layer": "m02"});
        let Value::Object(more) = more else { panic!() };
        assert_eq!(Value::Object(macro_targets(&[only, more])), json!({"glyphs": ["A", "B", "C"]}));

        let mut out = Vec::new();
        section(&mut out, "Script", Vec::new());
        assert!(out.is_empty(), "an empty section adds no separator");
        let row = Row::new(&["Script", "Spacing"], CommandRef::with_params(RUN_MACRO, json!({"macro": "Spacing/Tighten.fcmacro"})), "Tighten");
        section(&mut out, "Script", vec![row]);
        let bar = craft::menu::from_items(&["Script"], &out);
        let script = bar.menus.first().expect("the Script menu");
        assert!(matches!(script.children.first(), Some(craft::menu::Node::Submenu { label, .. }) if label == "Spacing"), "{:?}", script.children);
    }
}
