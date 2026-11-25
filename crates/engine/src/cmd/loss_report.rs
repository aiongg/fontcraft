//! `file.lossReport`: what a save or a UFO export would leave out, answered without writing
//! (`roundtrip.lossReport`, M1.20). The window asks about its `fresh` entries before a person's
//! Save, Save As and UFO export.

use serde_json::Value;
#[cfg(not(target_arch = "wasm32"))]
use serde_json::json;

use super::*;
#[cfg(not(target_arch = "wasm32"))]
use crate::params::Params;

pub fn specs() -> Vec<CommandSpec> {
    vec![cmd!(
        query "file.lossReport",
        "Loss Report",
        [],
        None,
        "{format?: \"fontcraft\"|\"glyphs\"|\"glyphspackage\"|\"ufo\"|\"ufoz\"|\"designspace\", version?: 2|3|4, smartComponents?: \"components\"|\"outlines\", exportUfo?: bool, decomposeSmartStuff?: bool} what saving the document in this format would leave out, without writing anything: what file.save and file.saveAs return as losses. The format defaults to the one Save writes. exportUfo: the report of file.exportUfo instead (decomposeSmartStuff as there). privateData lists what a UFO holds only in glyph lib keys that other UFO tools ignore (corner components, caps, brushes, segments, smart components kept as components). fresh lists the entries of both that the document's file, as last saved, does not already leave out; it is empty when a save loses nothing new. glyphs names the glyphs of an entry, at most 256 → {format, version?, losses: [{family, entries, what, items?, files?, glyphs?}], privateData: [same], fresh: [same], smartComponents: number}",
        has_doc,
        report
    )]
}

/// A loss that a report has and an earlier one did not, or had less of.
#[cfg(not(target_arch = "wasm32"))]
pub(super) struct Fresh {
    pub entry: Value,
    /// The items (or files) the earlier report did not list.
    pub added: Vec<String>,
    /// How many more entries it counts.
    pub grew: u64,
}

/// The entries of the loss report `new` that name something `old` did not.
#[cfg(not(target_arch = "wasm32"))]
pub(super) fn fresh(old: &[Value], new: &[Value]) -> Vec<Fresh> {
    use std::collections::HashSet;
    let text = |v: &Value, key: &str| v.get(key).and_then(Value::as_str).unwrap_or("").to_string();
    let key = |v: &Value| (text(v, "family"), text(v, "what"));
    let entries = |v: &Value| v.get("entries").and_then(Value::as_u64).unwrap_or(0);
    let items = |v: &Value| -> Vec<String> {
        let list = v.get("items").or_else(|| v.get("files")).and_then(Value::as_array);
        list.map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default()
    };
    let mut out = Vec::new();
    for entry in new {
        let was = old.iter().find(|b| key(b) == key(entry));
        let had: HashSet<String> = was.map(items).unwrap_or_default().into_iter().collect();
        let added: Vec<String> = items(entry).into_iter().filter(|i| !had.contains(i)).collect();
        let grew = entries(entry).saturating_sub(was.map(entries).unwrap_or(0));
        if !added.is_empty() || grew > 0 {
            out.push(Fresh { entry: entry.clone(), added, grew });
        }
    }
    out
}

#[cfg(not(target_arch = "wasm32"))]
fn report(s: &mut Session, p: &Value) -> Result<Value> {
    use crate::io::{self, SaveFormat};
    let p = Params::new("file.lossReport", p)?;
    let export = p.bool_or("exportUfo", false)?;
    let doc = s.doc()?;
    let own = SaveFormat::of_source(doc.file_format());
    let format = match (export, p.opt_str("format")?) {
        (true, _) => SaveFormat::Ufo,
        (false, Some(f)) => SaveFormat::parse(f).ok_or_else(|| {
            p.bad(format!("`format` must be \"fontcraft\", \"glyphs\", \"glyphspackage\", \"ufo\", \"ufoz\" or \"designspace\", got \"{f}\""))
        })?,
        (false, None) => own.ok_or_else(|| p.bad("this document is view-only: give the `format` to save it in"))?,
    };
    let version = p.opt_int("version", 2, 4)?.map(|v| v as u32);
    let mut font = fontcraft_model::Font::clone(&doc.font);
    let (version, dropped, outlines) = if export {
        (Some(3), None, p.bool_or("decomposeSmartStuff", true)?)
    } else {
        let (version, dropped) = super::file::prepare(&p, &mut font, format, version)?;
        (version, dropped, super::file::smart_outlines(&p)?)
    };
    let outlines = outlines && format.is_ufo();
    let smart = if format.is_ufo() { super::format_warnings::smart_components(&font) } else { 0 };
    let (drawn, _) = super::file::smart_choice(&font, outlines);
    let written = drawn.as_ref().unwrap_or(&font);
    let losses_of = |font: &fontcraft_model::Font| if export { io::export_losses(font) } else { io::save_losses(font, format) };
    let mut losses = losses_of(written);
    losses.extend(dropped);
    let private = if format.is_ufo() { super::format_warnings::private_features(written) } else { Vec::new() };
    // What the document's file lost when it was read or written is no news on the next save.
    let in_file = doc.path.is_some() && if export { own.is_some_and(SaveFormat::is_ufo) } else { own == Some(format) };
    let before = if in_file { losses_of(&doc.saved) } else { Vec::new() };
    let mut news: Vec<Value> = fresh(&before, &losses).into_iter().map(|f| f.entry).collect();
    if !(doc.path.is_some() && own.is_some_and(SaveFormat::is_ufo)) {
        news.extend(private.iter().cloned());
    }
    Ok(json!({
        "format": format.name(), "version": version, "losses": losses, "privateData": private, "fresh": news, "smartComponents": smart,
    }))
}

#[cfg(target_arch = "wasm32")]
fn report(_: &mut Session, _: &Value) -> Result<Value> {
    Err(crate::EngineError::Unsupported("the loss report in the browser, which writes no files by path".into()))
}
