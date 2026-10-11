//! The user's settings, headless: `settings.get` and `settings.set` (`crate::settings`).

use serde_json::{Value, json};

use super::*;
use crate::params::Params;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        cmd!(
            query "settings.get",
            "Get Setting",
            [],
            None,
            "{id?: string} one setting's value by id (its default while it is not set), or every setting without an id. Besides the preferences, the ids fontView.filters, fontView.visibleScripts, file.recentDocuments, start.chooserPicks and sampleStrings.groups hold data their own commands write (null while empty) → {id, value} | {settings: {id: value}}",
            always,
            get
        ),
        cmd!(
            "settings.set",
            "Set Setting",
            [],
            None,
            "{id: string, value: any} set one preference, checked against its kind (true or false, a choice, a number in range, text); null returns it to its default. Unknown ids, and the ids other commands write, are refused → {id, value}",
            always,
            set
        ),
    ]
}

fn get(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("settings.get", v)?;
    match p.opt_str("id")? {
        Some(id) => Ok(json!({ "id": id, "value": s.settings().get(id).map_err(|e| p.bad(e.to_string()))? })),
        None => Ok(json!({ "settings": s.settings().all() })),
    }
}

fn set(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("settings.set", v)?;
    let id = p.str("id")?.to_string();
    // `Params::get` reads null as absent; here null means "back to the default".
    let value = v.get("value").cloned().ok_or_else(|| p.bad("`value` is required (null returns the setting to its default)"))?;
    s.settings_mut().set(&id, value).map_err(|e| p.bad(e.to_string()))?;
    let value = s.settings().get(&id).map_err(|e| p.bad(e.to_string()))?;
    Ok(json!({ "id": id, "value": value }))
}
