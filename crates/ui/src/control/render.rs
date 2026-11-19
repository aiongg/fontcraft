//! `ui.render`: glyphs filled by the nonzero rule, black on white, without a window (the `render` crate).

use base64::Engine as _;
use fontcraft_engine::file_access;
use fontcraft_render::{Image, LineOptions};
use serde_json::{Value, json};

use super::{Method, object, set};
use crate::app::FontCraftApp;

pub const METHODS: &[(&str, Method)] = &[("ui.render", render)];

/// `{path?, glyphs?, text?, master?, layer?, size?, scale?}` → `{width, height, glyphs, master?, path? | png?}`.
/// `glyphs` names them, `text` maps characters to glyphs; neither: the glyph the glyph panels show. `master`
/// defaults to the first. `layer` draws that layer of the one glyph named. The same request gives the same bytes.
fn render(app: &mut FontCraftApp, params: &Value) -> Result<Value, String> {
    const M: &str = "ui.render";
    let obj = object(M, params)?;
    let font = app.session.font().map_err(|e| e.to_string())?;
    let get = |key: &str| obj.get(key).filter(|v| !v.is_null());
    let number = |key: &'static str| match get(key) {
        None => Ok(None),
        Some(v) => v.as_f64().map(Some).ok_or_else(|| format!("{M}: `{key}` must be a number")),
    };
    let path = match get("path") {
        None => None,
        Some(Value::String(p)) if !p.is_empty() => Some(p.as_str()),
        Some(_) => return Err(format!("{M}: `path` must be a file path")),
    };
    let glyphs: Vec<String> = match (get("glyphs"), get("text")) {
        (Some(_), Some(_)) => return Err(format!("{M}: give `glyphs` or `text`, not both")),
        (Some(list), None) => {
            let list = list.as_array().ok_or_else(|| format!("{M}: `glyphs` must be a list of glyph names"))?;
            if list.len() > fontcraft_render::MAX_LINE_GLYPHS {
                return Err(format!("{M}: at most {} glyphs", fontcraft_render::MAX_LINE_GLYPHS));
            }
            list.iter()
                .map(|v| v.as_str().map(str::to_string).ok_or_else(|| format!("{M}: `glyphs` must be a list of glyph names")))
                .collect::<Result<_, _>>()?
        }
        (None, Some(text)) => {
            glyphs_for_text(font, text.as_str().ok_or_else(|| format!("{M}: `text` must be a string"))?).map_err(|e| format!("{M}: {e}"))?
        }
        (None, None) => app.models.glyph_metrics.glyph.iter().cloned().collect(),
    };
    if glyphs.is_empty() {
        return Err(format!("{M}: no glyphs: give `glyphs` or `text`, or show a glyph"));
    }
    let master = match get("master") {
        None => None,
        Some(Value::String(id)) if font.master(id).is_ok() => Some(id.clone()),
        Some(Value::String(id)) => return Err(format!("{M}: no master has id `{id}`")),
        Some(_) => return Err(format!("{M}: `master` must be a master id")),
    };
    let options = LineOptions { master: master.clone(), size: number("size")?, scale: number("scale")?, ..Default::default() };
    let image = match get("layer") {
        None => {
            let names: Vec<&str> = glyphs.iter().map(String::as_str).collect();
            fontcraft_render::render_line(font, &names, &options)
        }
        Some(Value::String(layer)) => {
            let [name] = glyphs.as_slice() else { return Err(format!("{M}: `layer` needs exactly one glyph")) };
            let glyph = font.glyph(name).ok_or_else(|| format!("{M}: no glyph named `{name}`"))?;
            fontcraft_render::render_layer(font, glyph, Some(layer), &options)
        }
        Some(_) => return Err(format!("{M}: `layer` must be a layer id")),
    }
    .map_err(|e| format!("{M}: {e}"))?;
    let mut result = deliver(M, &image, path)?;
    set(&mut result, "glyphs", json!(glyphs));
    if get("layer").is_none() {
        set(&mut result, "master", json!(master.or_else(|| font.masters.first().map(|m| m.id.clone()))));
    }
    Ok(result)
}

/// The glyphs that encode the characters of `text`, in order.
fn glyphs_for_text(font: &fontcraft_model::Font, text: &str) -> Result<Vec<String>, String> {
    if text.chars().count() > fontcraft_render::MAX_LINE_GLYPHS {
        return Err(format!("`text` holds at most {} characters", fontcraft_render::MAX_LINE_GLYPHS));
    }
    let mut by_code: std::collections::HashMap<u32, &str> = std::collections::HashMap::new();
    for glyph in font.glyphs.iter() {
        for u in &glyph.unicodes {
            by_code.entry(*u).or_insert(glyph.name.as_str());
        }
    }
    text.chars()
        .map(|c| by_code.get(&u32::from(c)).map(|n| n.to_string()).ok_or_else(|| format!("no glyph encodes U+{:04X} `{c}`", u32::from(c))))
        .collect()
}

/// Writes `image` as PNG to `path` (checked against the write root), or returns it as base64 `png`.
fn deliver(method: &str, image: &Image, path: Option<&str>) -> Result<Value, String> {
    let png = image.png().map_err(|e| format!("{method}: {e}"))?;
    let mut result = json!({"width": image.width, "height": image.height});
    match path {
        Some(path) => {
            file_access::check_write(std::path::Path::new(path)).map_err(|e| format!("{method}: {e}"))?;
            fontcraft_engine::extensions::write_file(std::path::Path::new(path), &png).map_err(|e| format!("{method}: cannot write {e}"))?;
            set(&mut result, "path", json!(path));
        }
        None => set(&mut result, "png", json!(base64::engine::general_purpose::STANDARD.encode(&png))),
    }
    Ok(result)
}
