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
            "{id?: string} one setting's value by id (its default while it is not set), or every setting without an id. Besides the preferences, the ids fontView.filters, fontView.visibleScripts, file.recentDocuments, start.chooserPicks, sampleStrings.groups, audit.userProfiles, com.fontcraft.exportProfiles, sharing.trustedDevices, view.shownReporters and export.keptSettings hold data their own commands write (null while empty) → {id, value} | {settings: {id: value}}",
            always,
            get
        ),
        cmd!(
            "settings.set",
            "Set Setting",
            [],
            None,
            "{id: string, value: any} set one preference, checked against its kind (true or false, a choice, a number in range, text); null returns it to its default. Unknown ids, and the ids other commands write, are refused; export.ttfAutohintBinary, and turning sharing.enableExternalPreview on, are refused while automation roots are in force → {id, value}",
            always,
            set
        ),
        cmd!(
            "settings.resetColors",
            "Standard",
            [],
            None,
            "{} returns every colour of Settings › Appearance, in both the light and the dark set, to its default (the section's Standard button) → {reset: [id]}",
            always,
            reset_colors
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
    // The trusted autohinter is a program the exports run; automation may not choose it.
    if id == crate::external_autohint::SETTING && crate::file_access::current().is_some() {
        return Err(p.bad(format!("{id} names a program to run and cannot be set while automation roots are in force")));
    }
    // External Preview sends the open font to the network; a confined agent may not turn it on.
    if id == super::sharing::ENABLE_SETTING && value == Value::Bool(true) && crate::file_access::current().is_some() {
        return Err(p.bad(format!("{id} serves the open font to the network and cannot be turned on while automation roots are in force")));
    }
    s.settings_mut().set(&id, value).map_err(|e| p.bad(e.to_string()))?;
    let value = s.settings().get(&id).map_err(|e| p.bad(e.to_string()))?;
    Ok(json!({ "id": id, "value": value }))
}

fn reset_colors(s: &mut Session, v: &Value) -> Result<Value> {
    let p = Params::new("settings.resetColors", v)?;
    let ids: Vec<&str> = crate::settings::color_ids().collect();
    // Every id is a live colour setting, for which null (the default) is always accepted, so the reset cannot stop
    // halfway.
    for id in &ids {
        s.settings_mut().set(id, Value::Null).map_err(|e| p.bad(e.to_string()))?;
    }
    Ok(json!({ "reset": ids }))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::indexing_slicing)]

    use serde_json::json;

    use crate::Session;

    /// Standard returns both colour sets to their defaults and leaves the other settings alone.
    #[test]
    fn standard_restores_both_colour_sets() {
        let mut s = Session::new();
        let standard = s.settings().get("appearance.color.cornerNodes.dark").unwrap();
        s.execute("settings.set", &json!({"id": "appearance.color.cornerNodes.light", "value": "#ff0000"})).unwrap();
        s.execute("settings.set", &json!({"id": "appearance.color.cornerNodes.dark", "value": "#00ff0080"})).unwrap();
        s.execute("settings.set", &json!({"id": "appearance.color.canvas.dark", "value": "#000000"})).unwrap();
        s.execute("settings.set", &json!({"id": "appearance.handleSize", "value": "large"})).unwrap();
        let out = s.execute("settings.resetColors", &json!({})).unwrap();
        assert_eq!(out["reset"].as_array().unwrap().len(), 18, "nine items, light and dark");
        assert_eq!(s.settings().get("appearance.color.cornerNodes.dark").unwrap(), standard);
        assert_eq!(s.settings().get("appearance.color.cornerNodes.light").unwrap(), json!("#2d6cdf"));
        assert_eq!(s.settings().get("appearance.color.canvas.dark").unwrap(), json!("#1e1e1e"));
        assert_eq!(s.settings().get("appearance.handleSize").unwrap(), json!("large"), "the handle size is no colour");
    }
}
