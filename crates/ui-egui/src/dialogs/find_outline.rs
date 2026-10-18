//! Find / Replace Outline (`dialog.findOutline`): searches the font for the paths of a pattern
//! glyph with `path.findOutlinePattern` and replaces them with the paths of a replacement glyph
//! with `path.replaceOutlinePattern`. The pattern and the replacement are drawn in glyphs of
//! their own (on each master, so every master can have its own shapes); the glyphs searched are
//! the selected ones, or every glyph when none is selected.
//!
//! | Opened by | OK |
//! |---|---|
//! | Edit › Find Outline… ([`FIND`]) | Find All: searches, then shows the matches in [`FOUND`] |
//! | the result of Find All ([`FOUND`]) | Replace All with the replacement glyph |
//! | Edit › Replace Outline… ([`DIALOG`]) | Replace All; the "…" menu copies the list of matches |
//!
//! Nothing changes until Replace All, which is one undo step.

use fontcraft_engine::Session;
use fontcraft_model::MAX_NAME_LEN;
use serde_json::{Map, Value, json};

use super::form::Field;
use super::{Action, After, Dialog, Fields, Mode};

const FIND_COMMAND: &str = "path.findOutlinePattern";
const REPLACE_COMMAND: &str = "path.replaceOutlinePattern";
/// The command's largest tolerance, in font units.
const MAX_TOLERANCE: f64 = 10.0;
/// Matches the list shows; a last row counts the others.
const MAX_ROWS: usize = 1_000;

/// Edit › Replace Outline…: Replace All runs `path.replaceOutlinePattern`; the "…" menu's Copy
/// Matches puts the list `path.findOutlinePattern` gives on the clipboard.
pub const DIALOG: Dialog = Dialog {
    mode: Mode::RunOnOk,
    buttons: ["Replace All", "Cancel"],
    actions: &[Action { id: "copyOutlineMatches", label: "Copy Matches", command: FIND_COMMAND }],
    ..Dialog::new("dialog.findOutline", "Replace Outline", REPLACE_COMMAND, Fields::Fixed(replace_fields))
};

/// Edit › Find Outline…: Find All runs `path.findOutlinePattern`, and [`FOUND`] shows its matches.
pub const FIND: Dialog = Dialog {
    mode: Mode::RunOnOk,
    buttons: ["Find All", "Cancel"],
    ..Dialog::new("dialog.findOutline", "Find Outline", FIND_COMMAND, Fields::Fixed(find_fields))
};

/// What Find All found, with the search's fields and a replacement glyph; Replace All runs
/// `path.replaceOutlinePattern` on the same glyphs.
pub const FOUND: Dialog = Dialog {
    mode: Mode::RunOnOk,
    buttons: ["Replace All", "Close"],
    follow_up: true,
    after: Some(After { command: FIND_COMMAND, params: found_params }),
    ..Dialog::new("dialog.findOutline", "Find Outline", REPLACE_COMMAND, Fields::Targeted(found_fields))
};

fn pattern_field() -> Field {
    Field::text("patternGlyph", "Pattern Glyph", MAX_NAME_LEN, "The glyph whose paths, on each layer searched, are the shape to find")
}

fn replacement_field() -> Field {
    Field::text("replacementGlyph", "Replacement Glyph", MAX_NAME_LEN, "The glyph whose paths take the place of each match")
}

fn option_fields() -> [Field; 3] {
    [
        Field::bool("allowFlipped", "Allow Flipped", false, "Also find mirrored and upside-down copies"),
        Field::bool(
            "endpointsLoose",
            "Loose End Points",
            false,
            "The end nodes of an open pattern give only the direction and least length of the segments leading to them",
        ),
        Field::number(
            "tolerance",
            "Tolerance",
            0.0,
            MAX_TOLERANCE,
            0.5,
            "The largest distance between a pattern node and the node it matches, in units",
        ),
    ]
}

fn find_fields(_: &Session) -> Vec<Field> {
    let mut fields = vec![pattern_field()];
    fields.extend(option_fields());
    fields
}

fn replace_fields(_: &Session) -> Vec<Field> {
    let mut fields = vec![pattern_field(), replacement_field()];
    fields.extend(option_fields());
    fields
}

/// The rows of the match list, given under `matches` by [`found_params`].
fn found_fields(_: &mut Session, given: &Map<String, Value>) -> Result<Vec<Field>, String> {
    let rows: Vec<String> = given
        .get("matches")
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default();
    let mut fields =
        vec![Field::rows("matches", "Matches", rows, "Where the pattern was found, one row per match"), pattern_field(), replacement_field()];
    fields.extend(option_fields());
    Ok(fields)
}

/// The search's parameters and one row per match (at most [`MAX_ROWS`], then a count of the
/// rest).
fn found_params(request: &Value, result: &Value) -> Option<Map<String, Value>> {
    let mut given = request.as_object().cloned().unwrap_or_default();
    let count = result.get("count").and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok()).unwrap_or(0);
    let mut rows: Vec<Value> = result.get("text").and_then(Value::as_str).unwrap_or_default().lines().take(MAX_ROWS).map(|l| json!(l)).collect();
    if count == 0 {
        rows.push(json!("No matches"));
    } else if count > rows.len() {
        rows.push(json!(format!("and {} more", count.saturating_sub(rows.len()))));
    }
    given.insert("matches".into(), Value::Array(rows));
    Some(given)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

    use std::sync::Arc;

    use fontcraft_model::Font;
    use serde_json::{Value, json};

    use crate::{FontcraftApp, control};

    fn call(app: &mut FontcraftApp, method: &str, params: Value) -> Value {
        let r = control::handle(app, &json!({"id": 1, "method": method, "params": params}));
        assert_eq!(r["ok"], true, "{method}: {r}");
        r["result"].clone()
    }

    fn square(x: f64, y: f64, size: f64) -> Value {
        json!([{"x": x, "y": y}, {"x": x + size, "y": y}, {"x": x + size, "y": y + size}, {"x": x, "y": y + size}])
    }

    /// `a` holds a square dot, `b` a diamond of the same size, `i` and `j` the dot among other paths.
    fn dotted() -> FontcraftApp {
        let mut app = FontcraftApp::default();
        let mut add = |glyph: &str, nodes: Value| {
            app.run("path.addPath", &json!({"glyph": glyph, "nodes": nodes})).unwrap();
        };
        add("a", square(0.0, 0.0, 40.0));
        add("b", json!([{"x": 20, "y": 0}, {"x": 40, "y": 20}, {"x": 20, "y": 40}, {"x": 0, "y": 20}]));
        add("i", square(0.0, 100.0, 40.0));
        add("i", square(300.0, 600.0, 40.0));
        add("j", square(100.0, 600.0, 40.0));
        app
    }

    fn font(app: &FontcraftApp) -> Arc<Font> {
        app.session.doc().unwrap().font.clone()
    }

    /// Replace All in the dialog gives the font the command gives with the same answers.
    #[test]
    fn find_outline_dialog_replaces_as_the_command_does() {
        let mut app = dotted();
        let opened = call(
            &mut app,
            "engine.execute",
            json!({"command": "dialog.open", "params": {"dialog": "dialog.findOutline", "params": {"patternGlyph": "a", "replacementGlyph": "b"}}}),
        );
        assert_eq!(opened["command"], "path.replaceOutlinePattern", "{opened}");
        let copied = crate::dialogs::click(&mut app, "copyOutlineMatches").unwrap();
        assert_eq!(copied["text"].as_str().unwrap().lines().count(), 3, "the dot in i, the dot in j and the square stem of i: {copied}");
        call(&mut app, "ui.dialog.confirm", json!({}));

        let mut direct = dotted();
        let r = direct
            .run(
                "path.replaceOutlinePattern",
                &json!({"patternGlyph": "a", "replacementGlyph": "b", "allowFlipped": false, "endpointsLoose": false, "tolerance": 0.5}),
            )
            .unwrap();
        assert_eq!(r["replaced"], 3, "{r}");
        assert!(font(&app).same_as(&font(&direct)), "the dialog and the command give the same font");
    }

    /// Find All shows the matches, and Replace All from there replaces them.
    #[test]
    fn find_all_lists_the_matches_and_replaces_them_from_the_list() {
        let mut app = dotted();
        let before = font(&app);
        let opened = call(
            &mut app,
            "engine.execute",
            json!({"command": "dialog.open", "params": {"command": "path.findOutlinePattern", "params": {"patternGlyph": "a"}}}),
        );
        assert_eq!(opened["command"], "path.findOutlinePattern", "{opened}");
        call(&mut app, "ui.dialog.confirm", json!({}));
        assert!(Arc::ptr_eq(&font(&app), &before), "Find All changes nothing");

        let found = call(&mut app, "ui.dialog.inspect", json!({}));
        assert_eq!(found["command"], "path.replaceOutlinePattern", "{found}");
        let rows = found["fields"].as_array().unwrap().iter().find(|f| f["id"] == "matches").unwrap()["value"].clone();
        assert_eq!(rows.as_array().unwrap().len(), 3, "{rows}");
        assert_eq!(found["params"]["patternGlyph"], "a");
        call(&mut app, "ui.dialog.set", json!({"field": "replacementGlyph", "value": "b"}));
        call(&mut app, "ui.dialog.confirm", json!({}));

        let mut direct = dotted();
        direct.run("path.replaceOutlinePattern", &json!({"patternGlyph": "a", "replacementGlyph": "b"})).unwrap();
        assert!(font(&app).same_as(&font(&direct)));
    }
}
