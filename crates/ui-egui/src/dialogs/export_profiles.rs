//! Export profiles and artwork dialogs.
//!
//! - Export Profiles (`file.saveExportProfile`): one profile's settings, saved under a name. It
//!   opens on the profile its `name` parameter names, else on the profile `basedOn` names, else
//!   on the defaults, and lists every profile above the fields. Built-in profiles are read-only:
//!   saving under a built-in name is refused by the command.
//! - Export Fonts As (`file.exportWithProfiles`): the active document through the ticked
//!   profiles into a folder.
//! - Export Artwork (`glyph.exportArtwork`): one picture per selected glyph into a folder.
//!
//! All three write files or settings, so nothing runs until OK.

use fontcraft_engine::Session;
use fontcraft_engine::cmd::export_profiles::{Profile, profiles};
use serde_json::{Map, Value, json};

use super::form::{Field, Kind, MAX_CHOICES};
use super::{Dialog, Fields, Mode, Shaped};

/// Edits and saves one export profile.
pub const DIALOG: Dialog = Dialog {
    mode: Mode::RunOnOk,
    buttons: ["Save", "Cancel"],
    ..Dialog::new("dialog.exportProfiles", "Export Profiles", "file.saveExportProfile", Fields::Shaped(Shaped { fields, given, params, link }))
};

/// Exports the active document through chosen profiles.
pub const EXPORT: Dialog = Dialog {
    mode: Mode::RunOnOk,
    buttons: ["Export", "Cancel"],
    ..Dialog::new("dialog.exportFontsAs", "Export Fonts As", "file.exportWithProfiles", Fields::Fixed(export_fields))
};

/// Writes one picture per glyph.
pub const ARTWORK: Dialog = Dialog {
    mode: Mode::RunOnOk,
    buttons: ["Export", "Cancel"],
    ..Dialog::new("dialog.exportArtwork", "Export Artwork", "glyph.exportArtwork", Fields::Fixed(artwork_fields))
};

/// The settings fields, keyed as in a profile's params.
const SETTINGS: [&str; 5] = ["folder", "outlines", "containers", "removeOverlap", "autohint"];

fn names(session: &Session) -> Vec<String> {
    profiles(session).0.into_iter().take(MAX_CHOICES).map(|p| p.name).collect()
}

fn fields(session: &Session) -> Vec<Field> {
    let (all, problems) = profiles(session);
    let mut rows: Vec<String> = all.iter().map(|p| if p.built_in { format!("{} (built-in)", p.name) } else { p.name.clone() }).collect();
    rows.extend(problems);
    let mut based: Vec<String> = vec![String::new()];
    based.extend(all.iter().take(MAX_CHOICES - 1).map(|p| p.name.clone()));
    let mut labels: Vec<String> = vec!["None".into()];
    labels.extend(based.iter().skip(1).cloned());
    vec![
        Field::rows("existing", "Profiles", rows, "The export profiles; built-in ones are read-only"),
        Field::text("name", "Name", 128, "The profile to create or update; a built-in name is refused"),
        Field {
            key: "basedOn".into(),
            label: "Based On".into(),
            kind: Kind::Choice { choices: based, labels },
            default: json!(""),
            doc: "The profile whose settings a new profile starts from".into(),
            group: None,
        },
        Field::text(
            "folder",
            "Output Subfolder",
            1024,
            "The subfolder of the export folder the fonts go into; empty: the profile's name for a new profile, unchanged otherwise",
        )
        .in_group("Output"),
        Field::choice(
            "outlines",
            "Outline Format",
            &[("truetype", "TrueType"), ("cff", "PostScript/CFF")],
            "truetype",
            "truetype: quadratic outlines in .ttf files; cff: PostScript outlines in .otf files",
        )
        .in_group("Output"),
        Field::multi(
            "containers",
            "File Format",
            &[("plain", "OTF/TTF"), ("woff", "WOFF"), ("woff2", "WOFF2")],
            &["plain"],
            "The containers each font is written in",
        )
        .in_group("Output"),
        Field::bool("removeOverlap", "Remove Overlap", true, "Merge overlapping paths and components in every exported glyph").in_group("Outlines"),
        Field::bool("autohint", "Autohint", false, "Add automatic hints to the exported fonts").in_group("Hinting"),
    ]
}

/// Field values from parameters in the command's shape: the named profile's settings (or the
/// base's), with `settings` on top.
fn given(session: &Session, p: &Map<String, Value>) -> Result<Map<String, Value>, String> {
    let (all, _) = profiles(session);
    let find = |key: &str| -> Option<&Profile> {
        let name = p.get(key).and_then(Value::as_str)?;
        all.iter().find(|q| q.name == name)
    };
    let mut out = Map::new();
    // A copy writes into its own folder, so the base's folder is not taken over.
    let start = match find("name").filter(|q| !q.built_in) {
        Some(own) => Some((own, true)),
        None => find("basedOn").map(|base| (base, false)),
    };
    if let Some((start, own)) = start {
        for key in SETTINGS.iter().filter(|k| own || **k != "folder") {
            if let Some(v) = start.params.get(*key) {
                out.insert((*key).into(), v.clone());
            }
        }
    }
    if let Some(Value::Object(settings)) = p.get("settings") {
        for key in SETTINGS {
            if let Some(v) = settings.get(key) {
                out.insert(key.into(), v.clone());
            }
        }
    }
    for key in ["name", "basedOn"] {
        if let Some(v) = p.get(key) {
            out.insert(key.into(), v.clone());
        }
    }
    Ok(out)
}

fn params(values: &Map<String, Value>) -> Map<String, Value> {
    let mut settings = Map::new();
    for key in SETTINGS {
        match values.get(key) {
            Some(Value::String(s)) if key == "folder" && s.trim().is_empty() => {}
            Some(v) => {
                settings.insert(key.into(), v.clone());
            }
            None => {}
        }
    }
    let mut out = Map::new();
    out.insert("name".into(), values.get("name").cloned().unwrap_or(json!("")));
    if let Some(based) = values.get("basedOn").and_then(Value::as_str).filter(|s| !s.is_empty()) {
        out.insert("basedOn".into(), json!(based));
    }
    out.insert("settings".into(), Value::Object(settings));
    out
}

fn link(_: &mut Map<String, Value>, _: &str) {}

fn export_fields(session: &Session) -> Vec<Field> {
    let names = names(session);
    let first: Vec<&String> = names.iter().take(1).collect();
    vec![
        Field {
            key: "profiles".into(),
            label: "Profiles".into(),
            kind: Kind::Multi { choices: names.clone(), labels: Vec::new() },
            default: json!(first),
            doc: "The profiles to export through; each writes into its own subfolder".into(),
            group: None,
        },
        Field::folder("destination", "Export Folder", "The folder the profiles' subfolders are made in; files of the same name are replaced"),
    ]
}

fn artwork_fields(session: &Session) -> Vec<Field> {
    vec![
        Field::choice("format", "Format", &[("svg", "SVG"), ("pdf", "PDF"), ("png", "PNG")], "svg", "The picture format, one file per glyph"),
        super::import_metrics::master_field(session, "layer", "Master", "The master whose layers are drawn"),
        Field::number("pixelsPerEm", "Pixels per Em", 1.0, 16_384.0, 256.0, "The size of PNG pictures; SVG and PDF are in font units"),
        Field::folder("directory", "Export Folder", "The folder the pictures are written to; files of the same name are replaced"),
    ]
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

    use serde_json::{Value, json};

    use crate::{FontcraftApp, control};

    fn call(app: &mut FontcraftApp, method: &str, params: Value) -> Value {
        let r = control::handle(app, &json!({"id": 1, "method": method, "params": params}));
        assert_eq!(r["ok"], true, "{method}: {r}");
        r["result"].clone()
    }

    /// `dialog.exportProfiles`: the dialog saves a copy of a built-in profile with its own
    /// settings, and opens again on the saved profile.
    #[test]
    fn export_profiles_dialog_saves_a_profile_based_on_a_built_in_one() {
        let mut app = FontcraftApp::default();
        let params = json!({"name": "Web Only WOFF2", "basedOn": "Web", "settings": {"containers": ["woff2"]}});
        call(&mut app, "engine.execute", json!({"command": "dialog.open", "params": {"dialog": "dialog.exportProfiles", "params": params}}));
        let done = call(&mut app, "ui.dialog.confirm", json!({}));
        assert_eq!(done["command"], "file.saveExportProfile", "{done}");
        let list = app.run("file.listExportProfiles", &json!({})).unwrap();
        let saved = list["profiles"].as_array().unwrap().iter().find(|p| p["name"] == "Web Only WOFF2").unwrap().clone();
        assert_eq!(saved["params"]["containers"], json!(["woff2"]));
        assert_eq!(saved["params"]["outlines"], "truetype");
        assert_eq!(saved["params"]["folder"], "Web Only WOFF2");

        // Opening on the saved profile fills its settings.
        let opened = call(
            &mut app,
            "engine.execute",
            json!({"command": "dialog.open", "params": {"dialog": "dialog.exportProfiles", "params": {"name": "Web Only WOFF2"}}}),
        );
        assert!(opened.to_string().contains("woff2"), "{opened}");
        call(&mut app, "ui.dialog.cancel", json!({}));
    }
}
