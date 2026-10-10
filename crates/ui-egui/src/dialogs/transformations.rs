//! Path › Transformations: translate, scale, slant and Cursivy of `path.transformations` about
//! an origin, previewed on the selected points or, with none selected, on the whole layer.
//!
//! The fields sit under the headings Translate (X, Y), Origin, Scale (horizontal, vertical, and
//! a lock button) and Slant (kind, angle), in the reference's order. The dialog shapes them into
//! the command's `translate`, `scale`, `slant` and `cursivy`, leaving out a translation of 0 and
//! a scale of 100 %. While the lock is on, the two scale fields hold the same value. The "…"
//! menu copies the values as a Filter or PreFilter custom parameter.

use fontcraft_model::MAX_COORDINATE;
use serde_json::{Map, Value, json};

use super::form::Field;
use super::{Action, Dialog, Fields, Shaped};

pub const DIALOG: Dialog = Dialog {
    actions: &ACTIONS,
    ..Dialog::new("dialog.transformations", "Transformations", "path.transformations", Fields::Shaped(Shaped { fields, given, params, link }))
};

const ACTIONS: [Action; 2] = [
    Action { id: "copyFilterParameter", label: "Copy Filter Parameter", command: "path.transformations.copyFilterParameter" },
    Action { id: "copyPreFilterParameter", label: "Copy PreFilter Parameter", command: "path.transformations.copyPreFilterParameter" },
];

/// The command refuses ±90° and beyond, where the shear has no finite factor.
const MAX_SLANT: f64 = 89.9;
/// The largest scale percentage the command takes.
const MAX_PERCENT: f64 = 1_000_000.0;

/// The origins with their menu labels, in the order of their export-string codes (0 cap height
/// … 4 baseline).
const ORIGINS: [(&str, &str); 5] = [
    ("capHeight", "Cap Height"),
    ("halfCapHeight", "Half Cap Height"),
    ("xHeight", "x-Height"),
    ("halfXHeight", "Half x-Height"),
    ("baseline", "Baseline"),
];

fn fields(_: &fontcraft_engine::Session) -> Vec<Field> {
    vec![
        Field::number("translateX", "X", -MAX_COORDINATE, MAX_COORDINATE, 0.0, "Horizontal move in font units, after the scale and slant")
            .in_group("Translate"),
        Field::number("translateY", "Y", -MAX_COORDINATE, MAX_COORDINATE, 0.0, "Vertical move in font units, after the scale and slant")
            .in_group("Translate"),
        Field::choice(
            "origin",
            "Origin",
            &ORIGINS,
            "baseline",
            "The fixed point of the scale and slant: a metric height of the master, at half the advance width",
        )
        .in_group("Origin"),
        Field::number("scaleX", "Horizontal", -MAX_PERCENT, MAX_PERCENT, 100.0, "Horizontal scale in percent").in_group("Scale"),
        Field::number("scaleY", "Vertical", -MAX_PERCENT, MAX_PERCENT, 100.0, "Vertical scale in percent; follows the horizontal scale while the lock is on")
            .in_group("Scale"),
        Field::lock("scaleLocked", "Proportional", true, "Locked: the vertical scale stays equal to the horizontal one").in_group("Scale"),
        Field::choice(
            "slantKind",
            "Kind",
            &[("slant", "Slant"), ("cursivy", "Cursivy")],
            "slant",
            "Slant: a plain shear about the origin; Cursivy: a slant with optical correction of the curves, which needs the master's horizontal and vertical stems",
        )
        .in_group("Slant"),
        Field::number("slant", "Angle", -MAX_SLANT, MAX_SLANT, 0.0, "Degrees of the shear about the origin").in_group("Slant"),
    ]
}

/// Field values from `dialog.open`'s parameters in the command's shape: `translate: {x, y}`,
/// `scale: {x, y?}` (a missing or equal `y` keeps the lock on), `cursivy: bool`.
fn given(p: &Map<String, Value>) -> Map<String, Value> {
    let mut out = Map::new();
    for (key, value) in p {
        match (key.as_str(), value) {
            ("translate", Value::Object(t)) => {
                out.insert("translateX".into(), t.get("x").cloned().unwrap_or(json!(0.0)));
                out.insert("translateY".into(), t.get("y").cloned().unwrap_or(json!(0.0)));
            }
            ("scale", Value::Object(s)) => {
                let x = s.get("x").cloned().unwrap_or(json!(100.0));
                let y = s.get("y").filter(|y| !y.is_null()).cloned().unwrap_or_else(|| x.clone());
                out.insert("scaleLocked".into(), json!(x.as_f64() == y.as_f64()));
                out.insert("scaleX".into(), x);
                out.insert("scaleY".into(), y);
            }
            ("cursivy", Value::Bool(on)) => {
                out.insert("slantKind".into(), json!(if *on { "cursivy" } else { "slant" }));
            }
            _ => {
                out.insert(key.clone(), value.clone());
            }
        }
    }
    out
}

/// The command's parameters from the field values.
fn params(v: &Map<String, Value>) -> Map<String, Value> {
    let number = |key: &str, neutral: f64| v.get(key).and_then(Value::as_f64).unwrap_or(neutral);
    let (tx, ty) = (number("translateX", 0.0), number("translateY", 0.0));
    let (sx, sy) = (number("scaleX", 100.0), number("scaleY", 100.0));
    let mut out = Map::new();
    if tx != 0.0 || ty != 0.0 {
        out.insert("translate".into(), json!({"x": tx, "y": ty}));
    }
    if sx != 100.0 || sy != 100.0 {
        out.insert("scale".into(), json!({"x": sx, "y": sy}));
    }
    out.insert("slant".into(), json!(number("slant", 0.0)));
    out.insert("cursivy".into(), json!(v.get("slantKind").and_then(Value::as_str) == Some("cursivy")));
    out.insert("origin".into(), v.get("origin").cloned().unwrap_or(json!("baseline")));
    out
}

/// While the lock is on, the scale field not just typed into takes the other's value.
fn link(v: &mut Map<String, Value>, changed: &str) {
    if v.get("scaleLocked").and_then(Value::as_bool) != Some(true) {
        return;
    }
    let (from, to) = if changed == "scaleY" { ("scaleY", "scaleX") } else { ("scaleX", "scaleY") };
    if let Some(value) = v.get(from).cloned() {
        v.insert(to.into(), value);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use fontcraft_model::Font;
    use serde_json::{Value, json};

    use crate::{FontcraftApp, control};

    fn call(app: &mut FontcraftApp, method: &str, params: Value) -> Value {
        let r = control::handle(app, &json!({"id": 1, "method": method, "params": params}));
        assert_eq!(r["ok"], true, "{method}: {r}");
        r["result"].clone()
    }

    fn fails(app: &mut FontcraftApp, method: &str, params: Value) -> String {
        let r = control::handle(app, &json!({"id": 1, "method": method, "params": params}));
        assert_eq!(r["ok"], false, "{method} should fail: {r}");
        r["error"].as_str().unwrap_or_default().to_string()
    }

    /// Glyph A with a 200-unit square, edited in a tab.
    fn square_app() -> FontcraftApp {
        let mut app = FontcraftApp::default();
        app.run("path.addRectangle", &json!({"glyph": "A", "x": 100, "y": 100, "width": 200, "height": 200})).unwrap();
        app.run("view.editSelectedGlyphs", &json!({"glyphs": ["A"]})).unwrap();
        app
    }

    fn font(app: &FontcraftApp) -> Arc<Font> {
        app.session.doc().unwrap().font.clone()
    }

    fn open(app: &mut FontcraftApp, params: Value) -> Value {
        call(app, "engine.execute", json!({"command": "dialog.open", "params": {"dialog": "dialog.transformations", "params": params}}))
    }

    #[test]
    fn the_dialog_previews_slants_the_layer_and_copies_the_filter_parameter() {
        let mut app = square_app();
        let original = font(&app);
        let opened = open(&mut app, json!({}));
        assert_eq!(opened["title"], "Transformations");
        assert_eq!(opened["params"]["glyph"], "A", "no points selected: the whole layer");
        assert_eq!(opened["params"]["origin"], "baseline");
        assert!(font(&app).same_as(&original), "the opening values change nothing");

        call(&mut app, "ui.dialog.set", json!({"field": "slant", "value": 10}));
        assert!(!font(&app).same_as(&original), "the canvas shows the slanted square");
        let error = fails(&mut app, "ui.dialog.set", json!({"field": "slant", "value": 90}));
        assert!(error.contains("from -89.9 to 89.9"), "{error}");
        fails(&mut app, "ui.dialog.set", json!({"field": "origin", "value": "middle"}));

        // The "…" menu's first item copies the dialog's values as an export filter: in the reply
        // and, at the end of the next frame, on the system clipboard.
        let shown = call(&mut app, "ui.dialog.inspect", json!({}));
        assert_eq!(shown["actions"][0]["id"], "copyFilterParameter");
        let copied = call(&mut app, "ui.click", json!({"id": "copyFilterParameter"}));
        assert_eq!(copied["text"], "Transformations;Slant:10;SlantCorrection:0;Origin:4;");
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| app.ui(ui));
        output.textures_delta.clear();
        assert!(
            output.platform_output.commands.contains(&egui::OutputCommand::CopyText("Transformations;Slant:10;SlantCorrection:0;Origin:4;".into())),
            "{:?}",
            output.platform_output.commands
        );
        let pre = call(&mut app, "ui.click", json!({"id": "copyPreFilterParameter"}));
        assert_eq!(pre["result"]["parameter"], "PreFilter");
        assert_eq!(call(&mut app, "ui.dialog.inspect", json!({}))["open"], true, "an action leaves the dialog open");

        call(&mut app, "ui.dialog.set", json!({"field": "origin", "value": "xHeight"}));
        call(&mut app, "ui.dialog.confirm", json!({}));
        let mut direct = square_app();
        direct.run("path.transformations", &json!({"glyph": "A", "slant": 10, "origin": "xHeight"})).unwrap();
        assert!(font(&app).same_as(&font(&direct)), "the dialog and the command give the same font");
    }

    #[test]
    fn selected_points_move_alone_and_cancel_puts_the_font_back() {
        let mut app = square_app();
        let original = font(&app);
        let points = json!([[0, 0], [0, 1]]);
        open(&mut app, json!({"glyph": "A", "points": points, "slant": 20}));
        call(&mut app, "ui.click", json!({"id": "ok"}));
        let mut direct = square_app();
        direct.run("path.transformations", &json!({"glyph": "A", "points": points, "slant": 20})).unwrap();
        assert!(font(&app).same_as(&font(&direct)));
        assert!(!font(&app).same_as(&original));

        let before = font(&app);
        open(&mut app, json!({"glyph": "A", "slant": -15, "origin": "capHeight"}));
        assert!(!font(&app).same_as(&before), "the preview shows the slant");
        call(&mut app, "ui.click", json!({"id": "cancel"}));
        assert!(Arc::ptr_eq(&font(&app), &before), "Cancel puts back the very font the dialog opened on");

        // Cursivy, the other kind of slant, is refused with a message, and OK keeps nothing.
        open(&mut app, json!({"glyph": "A", "slant": 10}));
        let shown = call(&mut app, "ui.dialog.set", json!({"field": "slantKind", "value": "cursivy"}));
        assert_eq!(shown["params"]["cursivy"], true);
        assert!(shown["error"].as_str().is_some_and(|e| e.contains("Cursivy")), "{shown}");
        fails(&mut app, "ui.dialog.confirm", json!({}));
        call(&mut app, "ui.dialog.cancel", json!({}));
        assert!(Arc::ptr_eq(&font(&app), &before));
    }

    #[test]
    fn translate_and_scale_fields_shape_the_commands_objects_and_the_lock_keeps_the_scales_equal() {
        let mut app = square_app();
        let opened = open(&mut app, json!({"translate": {"x": 20}, "scale": {"x": 80}}));
        assert_eq!(
            (opened["params"]["translate"].clone(), opened["params"]["scale"].clone()),
            (json!({"x": 20.0, "y": 0.0}), json!({"x": 80.0, "y": 80.0}))
        );
        let value = |shown: &Value, key: &str| shown["fields"].as_array().unwrap().iter().find(|f| f["id"] == key).unwrap()["value"].clone();
        assert_eq!(value(&opened, "scaleLocked"), true, "a scale without y opens locked");

        // Locked: the vertical field follows the horizontal one, and the other way round.
        let shown = call(&mut app, "ui.dialog.set", json!({"field": "scaleX", "value": 50}));
        assert_eq!(value(&shown, "scaleY"), 50.0);
        let shown = call(&mut app, "ui.dialog.set", json!({"field": "scaleY", "value": 60}));
        assert_eq!(value(&shown, "scaleX"), 60.0);
        // Unlocked: each keeps its own value.
        call(&mut app, "ui.click", json!({"id": "scaleLocked"}));
        let shown = call(&mut app, "ui.dialog.set", json!({"field": "scaleY", "value": 120}));
        assert_eq!((value(&shown, "scaleX"), value(&shown, "scaleY")), (json!(60.0), json!(120.0)));
        assert_eq!(shown["params"]["scale"], json!({"x": 60.0, "y": 120.0}));

        // Neutral values are left out of the parameters.
        call(&mut app, "ui.dialog.set", json!({"field": "translateX", "value": 0}));
        let shown = call(&mut app, "ui.dialog.inspect", json!({}));
        assert!(shown["params"].get("translate").is_none(), "{shown}");

        call(&mut app, "ui.dialog.set", json!({"field": "translateY", "value": -30}));
        let done = call(&mut app, "ui.dialog.confirm", json!({}));
        let mut direct = square_app();
        direct.run("path.transformations", &done["params"]).unwrap();
        let mut by_hand = square_app();
        by_hand.run("path.transformations", &json!({"glyph": "A", "translate": {"x": 0, "y": -30}, "scale": {"x": 60, "y": 120}})).unwrap();
        assert!(font(&app).same_as(&font(&direct)));
        assert!(font(&app).same_as(&font(&by_hand)), "the dialog's fields mean what the command's objects mean");
    }
}
