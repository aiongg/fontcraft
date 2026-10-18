//! The popovers that edit an intermediate or alternate layer from the Layers palette (M9.21).
//! Filling one runs the command its fields map to; Cancel changes nothing.
//!
//! Rows: `layers.editSpecialLayerSettings`, `popover.intermediateLayer`,
//! `popover.alternateLayer`.
//!
//! Both are dialogs ([`crate::dialogs`]) that preview their command while they are open:
//!
//! - Intermediate: one field per font axis, keyed by the axis tag (`wght`), labelled with the
//!   axis name and the range of the masters and virtual masters on it. OK runs
//!   `layers.setIntermediate` with `coordinates: {tag: number | null}`.
//! - Alternate: a `min` and a `max` field per axis, keyed `wght.min` and `wght.max`, under a
//!   heading with the axis name and range. OK runs `layers.setAlternate` with
//!   `ranges: {tag: {min?, max?}}`; an axis with both fields empty is left out, so all empty
//!   gives the `[]` partner layer.
//!
//! A field holds the text of a number, so that it can be empty: an empty coordinate takes the
//! master's, an empty bound is open. Text that is not a number goes to the command as it is,
//! which refuses it; the dialog shows the message and OK keeps nothing.
//!
//! The dialogs open on the layer's present values: choosing Intermediate or Alternate in a
//! row's context menu, or double-clicking the `{…}` or `[…]` part of a label, runs
//! `layers.editSpecialLayerSettings` with that part.

use fontcraft_engine::Session;
use fontcraft_engine::interp::Designspace;
use fontcraft_model::{Font, Layer};
use serde_json::{Map, Value, json};

use super::current_glyph;
use crate::app::FontcraftApp;
use crate::canvas::view;
use crate::dialogs::form::{Field, format_number};
use crate::dialogs::{self, Dialog, Fields, Shaped};
use crate::shell::{Args, UiCommand};

/// The intermediate layer's coordinates (`popover.intermediateLayer`).
pub const INTERMEDIATE: Dialog = Dialog::new(
    "popover.intermediateLayer",
    "Intermediate Layer",
    "layers.setIntermediate",
    Fields::Shaped(Shaped { fields: intermediate_fields, given: intermediate_given, params: intermediate_params, link: no_link }),
);

/// The alternate layer's axis ranges (`popover.alternateLayer`).
pub const ALTERNATE: Dialog = Dialog::new(
    "popover.alternateLayer",
    "Alternate Layer",
    "layers.setAlternate",
    Fields::Shaped(Shaped { fields: alternate_fields, given: alternate_given, params: alternate_params, link: no_link }),
);

/// Longest text a coordinate field takes.
const MAX_NUMBER_TEXT: usize = 32;

/// The part of a special layer's label each dialog edits, as `part` names it.
const PARTS: [(&str, Dialog); 2] = [("intermediate", INTERMEDIATE), ("alternate", ALTERNATE)];

/// The shell commands of this module.
pub const COMMANDS: &[UiCommand] = &[(
    "layers.editSpecialLayerSettings",
    "Edit Special Layer Settings",
    &[],
    "",
    "{glyph?: name (default: the edited glyph), layer?: id (default: the active layer), part: \"intermediate\" | \"alternate\"} opens the coordinate editor (popover.intermediateLayer) or the range editor (popover.alternateLayer) of a layer, filled with its present values, as double-clicking the {…} or […] part of its label in the Layers palette does. Fields are keyed by axis tag (`wght`) or by tag and bound (`wght.min`, `wght.max`) and hold the text of a number; empty inherits the master's coordinate or leaves the bound open. OK runs layers.setIntermediate or layers.setAlternate; Cancel changes nothing. Then ui.dialog.set, ui.dialog.confirm, ui.dialog.cancel → ui.dialog.inspect's reply",
)];

pub fn run(app: &mut FontcraftApp, id: &str, args: &Args) -> Result<Value, String> {
    match id {
        "layers.editSpecialLayerSettings" => {
            let part = args.string("part")?.ok_or_else(|| args.err("give `part`: intermediate or alternate"))?;
            let dialog = PARTS
                .iter()
                .find(|(p, _)| *p == part)
                .map(|(_, d)| *d)
                .ok_or_else(|| args.err(format!("unknown part `{part}`: intermediate or alternate")))?;
            let (glyph, layer) = target(app, args)?;
            let mut given = Map::new();
            given.insert("glyph".into(), json!(glyph));
            given.insert("layer".into(), json!(layer));
            dialogs::open(app, &dialog, &given).map_err(|e| args.err(e))
        }
        other => Err(format!("unknown command `{other}`")),
    }
}

/// The glyph and layer `layers.editSpecialLayerSettings` acts on: the given ones, else the edited
/// glyph and its active layer (the first master's layer outside Edit View).
fn target(app: &FontcraftApp, args: &Args) -> Result<(String, String), String> {
    let glyph = match args.string("glyph")? {
        Some(g) => g.to_string(),
        None => current_glyph(app).ok_or_else(|| args.err("no glyph is edited or selected: give `glyph`"))?,
    };
    let font = app.session.font().map_err(|e| args.err(e))?;
    let g = font.glyph(&glyph).ok_or_else(|| args.err(format!("there is no glyph `{glyph}`")))?;
    let layer = match args.string("layer")? {
        Some(l) => l.to_string(),
        None => app
            .state
            .tab()
            .filter(|_| app.state.editing() == Some(glyph.as_str()))
            .and_then(|tab| view::active_layer(font, tab))
            .or_else(|| font.masters.first().and_then(|m| g.layer(&m.id)))
            .map(|l| l.id.clone())
            .ok_or_else(|| args.err(format!("glyph `{glyph}` has no layer to edit: give `layer`")))?,
    };
    Ok((glyph, layer))
}

// ── Fields ──────────────────────────────────────────────────────────────────────────────────────

/// The name an axis is shown with: its name, else its tag.
fn axis_name(font: &Font, i: usize) -> String {
    font.axes.get(i).map(|a| if a.name.trim().is_empty() { a.tag.clone() } else { a.name.clone() }).unwrap_or_default()
}

/// Each font axis's name with the range of the masters and virtual masters on it, as the
/// fields show it: `Weight (100 to 900)`. Just the name when the font cannot interpolate.
fn axis_headings(font: &Font) -> Vec<String> {
    let designspace = Designspace::new(font).ok();
    (0..font.axes.len())
        .map(|i| {
            let range = designspace.as_ref().and_then(|ds| {
                let d = ds.axes().iter().position(|a| a.font_axis == i)?;
                let values = ds.masters().iter().map(|m| &m.location).chain(ds.virtual_masters().iter().map(|v| &v.location));
                let (lo, hi) =
                    values.filter_map(|l| l.0.get(d).copied()).fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| (lo.min(v), hi.max(v)));
                (lo.is_finite() && hi.is_finite()).then_some((lo, hi))
            });
            match range {
                Some((lo, hi)) => format!("{} ({} to {})", axis_name(font, i), format_number(lo), format_number(hi)),
                None => axis_name(font, i),
            }
        })
        .collect()
}

fn intermediate_fields(session: &Session) -> Vec<Field> {
    let Ok(font) = session.font() else { return Vec::new() };
    font.axes
        .iter()
        .zip(axis_headings(font))
        .map(|(axis, heading)| {
            Field::text(
                &axis.tag,
                &heading,
                MAX_NUMBER_TEXT,
                "The layer's internal coordinate on this axis; empty takes the coordinate of its master",
            )
        })
        .collect()
}

fn alternate_fields(session: &Session) -> Vec<Field> {
    let Ok(font) = session.font() else { return Vec::new() };
    font.axes
        .iter()
        .zip(axis_headings(font))
        .flat_map(|(axis, heading)| {
            [
                Field::text(
                    &format!("{}.min", axis.tag),
                    "min",
                    MAX_NUMBER_TEXT,
                    "The lowest internal coordinate on this axis where the alternate shape is used; empty for no lower bound",
                )
                .in_group(&heading),
                Field::text(
                    &format!("{}.max", axis.tag),
                    "max",
                    MAX_NUMBER_TEXT,
                    "The highest internal coordinate on this axis where the alternate shape is used; empty for no upper bound",
                )
                .in_group(&heading),
            ]
        })
        .collect()
}

fn no_link(_: &mut Map<String, Value>, _: &str) {}

// ── Opening values ──────────────────────────────────────────────────────────────────────────────

/// The layer `given` names with `glyph` and `layer`; `None` when it names no layer.
fn given_layer<'f>(font: &'f Font, given: &Map<String, Value>) -> Result<Option<&'f Layer>, String> {
    let (Some(glyph), Some(id)) = (given.get("glyph").and_then(Value::as_str), given.get("layer").and_then(Value::as_str)) else {
        return Ok(None);
    };
    let g = font.glyph(glyph).ok_or_else(|| format!("there is no glyph `{glyph}`"))?;
    g.layers.iter().find(|l| l.id == id).map(Some).ok_or_else(|| format!("glyph `{glyph}` has no layer `{id}`"))
}

/// A coordinate as its field holds it: the text of a number, empty for none.
fn field_text(key: &str, v: &Value) -> Result<Value, String> {
    match v {
        Value::Null => Ok(json!("")),
        Value::Number(n) => {
            n.as_f64().filter(|x| x.is_finite()).map(|x| json!(format_number(x))).ok_or_else(|| format!("`{key}` is not a usable number"))
        }
        Value::String(_) => Ok(v.clone()),
        _ => Err(format!("`{key}` must be a number or null")),
    }
}

/// The parameters of `given` other than `drop`, with numbers given for the fields `keys` turned
/// into their text.
fn pass_through(given: &Map<String, Value>, drop: &str, keys: &[String]) -> Result<Map<String, Value>, String> {
    let mut out = Map::new();
    for (k, v) in given.iter().filter(|(k, _)| k.as_str() != drop) {
        let value = if keys.contains(k) { field_text(k, v)? } else { v.clone() };
        out.insert(k.clone(), value);
    }
    Ok(out)
}

fn no_axes(title: &str) -> String {
    format!("{title}: the font has no axes; add one in Font Info › Axes first")
}

/// `coordinates: {tag: number | null}` to the fields; without it, the layer's own coordinates.
fn intermediate_given(session: &Session, given: &Map<String, Value>) -> Result<Map<String, Value>, String> {
    let font = session.font().map_err(|e| e.to_string())?;
    if font.axes.is_empty() {
        return Err(no_axes(INTERMEDIATE.title));
    }
    let layer = given_layer(font, given)?;
    if layer.is_some_and(Layer::is_master_layer) {
        return Err("a master layer cannot be an intermediate layer: add a layer under the master and make that one intermediate".into());
    }
    let tags: Vec<String> = font.axes.iter().map(|a| a.tag.clone()).collect();
    let mut out = pass_through(given, "coordinates", &tags)?;
    match given.get("coordinates") {
        Some(Value::Object(m)) => {
            for (tag, v) in m {
                if !tags.contains(tag) {
                    return Err(format!("`coordinates` names the axis `{tag}`, which the font does not have"));
                }
                out.insert(tag.clone(), field_text(&format!("coordinates.{tag}"), v)?);
            }
        }
        None | Some(Value::Null) => {
            for (tag, c) in tags.iter().zip(layer.map(|l| l.info.attrs.coordinates.as_slice()).unwrap_or_default()) {
                if let Some(v) = c.filter(|v| v.is_finite())
                    && !out.contains_key(tag)
                {
                    out.insert(tag.clone(), json!(format_number(v)));
                }
            }
        }
        Some(_) => return Err("`coordinates` must be an object: {tag: number or null}".into()),
    }
    Ok(out)
}

/// `ranges: {tag: {min?, max?}}` to the fields; without it, the layer's own ranges.
fn alternate_given(session: &Session, given: &Map<String, Value>) -> Result<Map<String, Value>, String> {
    let font = session.font().map_err(|e| e.to_string())?;
    if font.axes.is_empty() {
        return Err(no_axes(ALTERNATE.title));
    }
    let layer = given_layer(font, given)?;
    let tags: Vec<String> = font.axes.iter().map(|a| a.tag.clone()).collect();
    let keys: Vec<String> = tags.iter().flat_map(|t| [format!("{t}.min"), format!("{t}.max")]).collect();
    let mut out = pass_through(given, "ranges", &keys)?;
    match given.get("ranges") {
        Some(Value::Object(m)) => {
            for (tag, range) in m {
                if !tags.contains(tag) {
                    return Err(format!("`ranges` names the axis `{tag}`, which the font does not have"));
                }
                let range = range.as_object().ok_or_else(|| format!("`ranges.{tag}` must be an object: {{min?, max?}}"))?;
                for (bound, v) in range {
                    if bound != "min" && bound != "max" {
                        return Err(format!("`ranges.{tag}` takes `min` and `max`, not `{bound}`"));
                    }
                    let key = format!("{tag}.{bound}");
                    out.insert(key.clone(), field_text(&key, v)?);
                }
            }
        }
        None | Some(Value::Null) => {
            for (tag, rule) in tags.iter().zip(layer.map(|l| l.info.attrs.axis_rules.as_slice()).unwrap_or_default()) {
                for (bound, v) in [("min", rule.min), ("max", rule.max)] {
                    let key = format!("{tag}.{bound}");
                    if let Some(v) = v.filter(|v| v.is_finite())
                        && !out.contains_key(&key)
                    {
                        out.insert(key, json!(format_number(v)));
                    }
                }
            }
        }
        Some(_) => return Err("`ranges` must be an object: {tag: {min?, max?}}".into()),
    }
    Ok(out)
}

// ── Parameters ──────────────────────────────────────────────────────────────────────────────────

/// A field's text as the command takes it: empty is null, a number is a number, anything else
/// goes as it is for the command to refuse.
fn field_number(v: &Value) -> Value {
    match v {
        Value::String(s) if s.trim().is_empty() => Value::Null,
        Value::String(s) => s.trim().parse::<f64>().ok().filter(|n| n.is_finite()).map_or_else(|| v.clone(), |n| json!(n)),
        other => other.clone(),
    }
}

fn intermediate_params(values: &Map<String, Value>) -> Map<String, Value> {
    let coordinates: Map<String, Value> = values.iter().map(|(tag, v)| (tag.clone(), field_number(v))).collect();
    let mut m = Map::new();
    m.insert("coordinates".into(), Value::Object(coordinates));
    m
}

fn alternate_params(values: &Map<String, Value>) -> Map<String, Value> {
    let mut ranges = Map::new();
    for (key, v) in values {
        let Some((tag, bound)) = key.rsplit_once('.') else { continue };
        let n = field_number(v);
        if n.is_null() {
            continue;
        }
        if let Value::Object(range) = ranges.entry(tag.to_string()).or_insert_with(|| json!({})) {
            range.insert(bound.to_string(), n);
        }
    }
    let mut m = Map::new();
    m.insert("ranges".into(), Value::Object(ranges));
    m
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use egui::{Event, Modifiers, PointerButton, Pos2, Shape};
    use fontcraft_engine::Session;
    use fontcraft_model::{Axis, AxisRule, Font, Glyph, GlyphTable, Layer, Master, Path, Shape as Outline};
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

    fn execute(app: &mut FontcraftApp, command: &str, params: Value) -> Value {
        call(app, "engine.execute", json!({"command": command, "params": params}))
    }

    fn layer(master: &str, id: &str, width: f64) -> Layer {
        let mut l = Layer::new(master, 500.0);
        l.id = id.into();
        l.shapes = vec![Outline::Path(Path::rect(0.0, 0.0, width, 700.0))];
        l
    }

    /// A weight axis with Thin (100) and Black (900); glyph `e` with its master layers, the
    /// layers `extra` adds, and a backup layer `b` under Thin; edited in a tab.
    fn app_with(extra: Vec<Layer>) -> FontcraftApp {
        let mut f = Font::new("Special");
        f.axes = Arc::new(vec![Axis { name: "Weight".into(), tag: "wght".into(), ..Axis::default() }]);
        f.masters = Arc::new(vec![
            Master { id: "thin".into(), name: "Thin".into(), location: vec![100.0], ..Master::default() },
            Master { id: "black".into(), name: "Black".into(), location: vec![900.0], ..Master::default() },
        ]);
        let mut e = Glyph::new("e");
        e.layers = vec![layer("thin", "thin", 100.0), layer("black", "black", 300.0), layer("thin", "b", 150.0)];
        e.layers.extend(extra);
        f.glyphs = GlyphTable::from_glyphs(vec![Arc::new(e)]);
        let mut s = Session::new();
        s.add_document(f, None).unwrap();
        let mut app = FontcraftApp::new(s);
        app.run("view.editSelectedGlyphs", &json!({"glyphs": ["e"]})).unwrap();
        app
    }

    fn font(app: &FontcraftApp) -> Arc<Font> {
        app.session.doc().unwrap().font.clone()
    }

    fn e_layer(app: &FontcraftApp, id: &str) -> Layer {
        font(app).glyph("e").unwrap().layers.iter().find(|l| l.id == id).unwrap().clone()
    }

    #[test]
    fn the_intermediate_popover_gives_the_font_its_command_gives_and_cancel_changes_nothing() {
        let mut app = app_with(Vec::new());
        let opened = execute(&mut app, "layers.editSpecialLayerSettings", json!({"glyph": "e", "layer": "b", "part": "intermediate"}));
        assert_eq!(opened["dialog"], "popover.intermediateLayer");
        let field = &opened["fields"][0];
        assert_eq!((field["id"].clone(), field["label"].clone(), field["value"].clone()), (json!("wght"), json!("Weight (100 to 900)"), json!("")));
        assert!(opened["error"].is_string(), "all fields empty puts the layer on its master: the preview is refused");
        fails(&mut app, "ui.dialog.confirm", json!({}));

        call(&mut app, "ui.dialog.set", json!({"field": "wght", "value": "400"}));
        let done = call(&mut app, "ui.dialog.confirm", json!({}));
        assert_eq!(done["params"], json!({"glyph": "e", "layer": "b", "coordinates": {"wght": 400.0}}));
        let mut direct = app_with(Vec::new());
        execute(&mut direct, "layers.setIntermediate", json!({"glyph": "e", "layer": "b", "coordinates": {"wght": 400}}));
        assert!(font(&app).same_as(&font(&direct)), "the popover and the command give the same font");

        // Reopened, the field holds the layer's coordinate; Cancel puts back the very font.
        let before = font(&app);
        let opened = execute(&mut app, "layers.editSpecialLayerSettings", json!({"glyph": "e", "layer": "b", "part": "intermediate"}));
        assert_eq!(opened["fields"][0]["value"], "400");
        call(&mut app, "ui.dialog.set", json!({"field": "wght", "value": "600"}));
        assert_eq!(e_layer(&app, "b").info.attrs.coordinates, [Some(600.0)], "the change previews");
        call(&mut app, "ui.dialog.cancel", json!({}));
        assert!(Arc::ptr_eq(&font(&app), &before), "Cancel changes nothing");

        // A coordinate outside the masters is the command's refusal, shown in the dialog.
        execute(&mut app, "layers.editSpecialLayerSettings", json!({"glyph": "e", "layer": "b", "part": "intermediate"}));
        let shown = call(&mut app, "ui.dialog.set", json!({"field": "wght", "value": "2000"}));
        assert!(shown["error"].as_str().unwrap_or_default().contains("outside the range"), "{shown}");
        call(&mut app, "ui.dialog.cancel", json!({}));
        assert!(Arc::ptr_eq(&font(&app), &before));

        let master = fails(
            &mut app,
            "engine.execute",
            json!({"command": "layers.editSpecialLayerSettings", "params": {"glyph": "e", "layer": "thin", "part": "intermediate"}}),
        );
        assert!(master.contains("master layer"), "{master}");
        assert_eq!(call(&mut app, "ui.dialog.inspect", json!({}))["open"], false);
    }

    #[test]
    fn the_alternate_popover_gives_the_font_its_command_gives_and_cancel_changes_nothing() {
        let mut app = app_with(Vec::new());
        let opened = execute(&mut app, "dialog.open", json!({"command": "layers.setAlternate", "params": {"glyph": "e", "layer": "b"}}));
        assert_eq!(opened["dialog"], "popover.alternateLayer");
        let ids: Vec<Value> = opened["fields"].as_array().unwrap().iter().map(|f| f["id"].clone()).collect();
        assert_eq!(ids, [json!("wght.min"), json!("wght.max")]);
        assert_eq!(opened["fields"][0]["group"], "Weight (100 to 900)");

        call(&mut app, "ui.dialog.set", json!({"field": "wght.min", "value": "600"}));
        call(&mut app, "ui.dialog.confirm", json!({}));
        let mut direct = app_with(Vec::new());
        execute(&mut direct, "layers.setAlternate", json!({"glyph": "e", "layer": "b", "ranges": {"wght": {"min": 600}}}));
        assert!(font(&app).same_as(&font(&direct)), "the popover and the command give the same font");
        assert_eq!(e_layer(&app, "b").info.attrs.axis_rules, [AxisRule { min: Some(600.0), max: None }]);

        let before = font(&app);
        let opened = execute(&mut app, "layers.editSpecialLayerSettings", json!({"glyph": "e", "layer": "b", "part": "alternate"}));
        assert_eq!((opened["fields"][0]["value"].clone(), opened["fields"][1]["value"].clone()), (json!("600"), json!("")));
        call(&mut app, "ui.dialog.set", json!({"field": "wght.max", "value": "800"}));
        assert_eq!(e_layer(&app, "b").info.attrs.axis_rules, [AxisRule { min: Some(600.0), max: Some(800.0) }]);
        call(&mut app, "ui.dialog.cancel", json!({}));
        assert!(Arc::ptr_eq(&font(&app), &before), "Cancel changes nothing");
    }

    /// A headless window that keeps one egui context across frames.
    struct Window {
        ctx: egui::Context,
    }

    impl Window {
        fn frame(&self, app: &mut FontcraftApp, events: Vec<Event>) -> Vec<Shape> {
            let input =
                egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(1280.0, 800.0))), events, ..Default::default() };
            let mut output = self.ctx.run_ui(input, |ui| app.ui(ui));
            output.textures_delta.clear();
            let mut out: Vec<Shape> = output.shapes.into_iter().map(|c| c.shape).collect();
            let mut flat = Vec::new();
            while let Some(shape) = out.pop() {
                match shape {
                    Shape::Vec(list) => out.extend(list),
                    other => flat.push(other),
                }
            }
            flat
        }

        fn idle(&self, app: &mut FontcraftApp) -> Vec<Shape> {
            for _ in 0..2 {
                self.frame(app, Vec::new());
            }
            self.frame(app, Vec::new())
        }

        fn click(&self, app: &mut FontcraftApp, at: Pos2, button: PointerButton) {
            let event = |pressed| Event::PointerButton { pos: at, button, pressed, modifiers: Modifiers::NONE };
            self.frame(app, vec![Event::PointerMoved(at), event(true)]);
            self.frame(app, vec![event(false)]);
        }
    }

    /// Where the text `text` is drawn, each time it is.
    fn texts_at(shapes: &[Shape], text: &str) -> Vec<egui::Rect> {
        shapes
            .iter()
            .filter_map(|s| match s {
                Shape::Text(t) if t.galley.text() == text => Some(t.galley.rect.translate(t.pos.to_vec2())),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn the_layers_palette_shows_each_label_form_and_opens_its_popover() {
        let mut brace = layer("thin", "brace", 120.0);
        brace.info.attrs.coordinates = vec![Some(400.0)];
        let mut bracket = layer("thin", "bracket", 120.0);
        bracket.info.attrs.axis_rules = vec![AxisRule { min: Some(600.0), max: None }];
        let mut both = layer("black", "both", 320.0);
        both.info.attrs.coordinates = vec![Some(500.5)];
        both.info.attrs.axis_rules = vec![AxisRule { min: None, max: Some(700.0) }];
        let mut partner = layer("black", "partner", 320.0);
        partner.info.attrs.axis_rules = vec![AxisRule::default()];
        let mut app = app_with(vec![brace, bracket, both, partner]);
        let w = Window { ctx: egui::Context::default() };
        let shapes = w.idle(&mut app);
        for label in ["{400}", "[600‹wg]", "[wg‹700]", "{500.5}", "[]"] {
            assert_eq!(texts_at(&shapes, label).len(), 1, "the palette draws {label} once");
        }
        let brace_at = texts_at(&shapes, "{400}")[0];
        let thin = texts_at(&shapes, "Thin")
            .into_iter()
            .filter(|r| r.top() < brace_at.top())
            .max_by(|a, b| a.top().total_cmp(&b.top()))
            .expect("the Thin master row is drawn above its layers");
        assert!(brace_at.left() > thin.left(), "a special layer is indented under its master");
        let (bracket_part, brace_part) = (texts_at(&shapes, "[wg‹700]")[0].center(), texts_at(&shapes, "{500.5}")[0].center());
        assert!((bracket_part.y - brace_part.y).abs() < 1.0 && bracket_part.x < brace_part.x, "a layer that is both shows […] then {{…}}");

        // A double-click on the {…} part opens the coordinate editor with the layer's values.
        w.click(&mut app, brace_part, PointerButton::Primary);
        w.click(&mut app, brace_part, PointerButton::Primary);
        w.idle(&mut app);
        let shown = call(&mut app, "ui.dialog.inspect", json!({}));
        assert_eq!((shown["dialog"].clone(), shown["params"]["layer"].clone()), (json!("popover.intermediateLayer"), json!("both")), "{shown}");
        assert_eq!(shown["fields"][0]["value"], "500.5");
        call(&mut app, "ui.dialog.cancel", json!({}));

        // The context menu's Alternate is ticked on an alternate layer: unticking it clears it.
        let shapes = w.idle(&mut app);
        let bracket_at = texts_at(&shapes, "[600‹wg]")[0].center();
        w.click(&mut app, bracket_at, PointerButton::Secondary);
        let shapes = w.idle(&mut app);
        let item = texts_at(&shapes, "Alternate").first().expect("the row menu shows Alternate").center();
        w.click(&mut app, item, PointerButton::Primary);
        w.idle(&mut app);
        assert_eq!(app.session.journal().last().map(|(id, _)| id.as_str()), Some("layers.clearAlternate"));
        assert!(e_layer(&app, "bracket").info.attrs.axis_rules.is_empty());
    }
}
