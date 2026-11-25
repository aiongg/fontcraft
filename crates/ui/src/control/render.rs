//! `ui.render`: glyphs drawn by the engine (`preview.render`, `preview.compare`) without a window.

use base64::Engine as _;
use fontcraft_engine::cmd::preview_line::{RENDER_KEYS, request};
use fontcraft_engine::file_access;
use serde_json::{Map, Value, json};

use super::{Method, object, set};
use crate::app::FontCraftApp;

pub const METHODS: &[(&str, Method)] = &[("ui.render", render)];

/// `{path?, …}` → the drawing command's reply, its image as `png` or written to `path`. The arguments are
/// [`RENDER_KEYS`]; with neither `glyphs` nor `text`, the glyph the glyph panels show, except in a comparison, which
/// then draws what changed. Without a place to draw at (`master`, `instance`, `location`, `layer`), the master the
/// window draws in. The same request gives the same bytes.
fn render(app: &mut FontCraftApp, params: &Value) -> Result<Value, String> {
    const M: &str = "ui.render";
    let obj = object(M, params)?;
    let given = |key: &str| obj.get(key).filter(|v| !v.is_null());
    let path = match given("path") {
        None => None,
        Some(Value::String(p)) if !p.is_empty() => Some(p.clone()),
        Some(_) => return Err(format!("{M}: `path` must be a file path")),
    };
    if given("glyphs").is_some() && given("text").is_some() {
        return Err(format!("{M}: give `glyphs` or `text`, not both"));
    }
    let mut args: Map<String, Value> = RENDER_KEYS.iter().filter_map(|k| given(k).map(|v| ((*k).to_string(), v.clone()))).collect();
    let font = app.session.font().map_err(|e| e.to_string())?;
    if given("glyphs").is_none() && given("text").is_none() && given("compare").is_none() {
        let shown: Vec<&String> = app.models.glyph_metrics.glyph.iter().collect();
        if shown.is_empty() {
            return Err(format!("{M}: no glyphs: give `glyphs` or `text`, or show a glyph"));
        }
        args.insert("glyphs".into(), json!(shown));
    }
    if ["master", "instance", "location", "layer"].iter().all(|k| given(k).is_none())
        && let Some(master) = crate::shell::masters::current_master(font, &app.view.layers)
    {
        args.insert("master".into(), json!(master.id));
    }
    let (command, params) = request(&args);
    let mut result = app.session.query(command, &params).map_err(|e| format!("{M}: {e}"))?;
    if let (Some(path), Some(png)) = (path, result.get("png").and_then(Value::as_str)) {
        let bytes = base64::engine::general_purpose::STANDARD.decode(png).map_err(|e| format!("{M}: {e}"))?;
        let file = std::path::Path::new(&path);
        file_access::check_write(file).map_err(|e| format!("{M}: {e}"))?;
        fontcraft_engine::extensions::write_file(file, &bytes).map_err(|e| format!("{M}: cannot write {e}"))?;
        if let Some(fields) = result.as_object_mut() {
            fields.remove("png");
        }
        set(&mut result, "path", json!(path));
    }
    Ok(result)
}
