//! File › Export dialogs.
//!
//! - Export (`dialog.export`): one tab per output kind, each a dialog of its own command, for the active document:
//!   OTF (`file.exportStatic`), Variable Fonts (`file.exportVariable`), UFO (`file.exportUfo`) and Metrics
//!   (`file.exportMetrics`). The Variable Fonts tab offers the OTF tab's file formats and folder and no outline
//!   flavour: variable fonts have TrueType outlines. The OTF tab's Autohint follows the outline flavour and is off by
//!   default, since hinting is optional; CBDT is FontCraft's own option, off by default. The UFO tab writes each ticked
//!   master as a UFO, with glyph names turned into production names when its checkbox is on.
//! - Export Profiles (`dialog.exportProfiles`, `file.saveExportProfile`): one profile's settings, saved under a name.
//!   It opens on the profile `name` names, else on the one `basedOn` names, else on the defaults, and lists every
//!   profile above the fields. The command refuses a built-in name.
//! - Export Fonts As (`dialog.exportFontsAs`, `file.exportWithProfiles`): the document through the ticked profiles.
//! - Export Artwork (`dialog.exportArtwork`, `glyph.exportArtwork`): one picture per selected glyph.
//! - Export Window Contents (`dialog.exportWindowContents`, `file.exportWindowContents`): what the canvas shows (the
//!   edited glyph in its shown layer of the current master) as one PDF or SVG page. The format follows the file's
//!   extension.
//!
//! All of them write files or settings, so nothing runs until OK. Next… or Export… with no folder or file asks the
//! chooser for one; over the control channel a missing destination is an error.

use fontcraft_engine::Session;
use fontcraft_engine::cmd::export_profiles::{Profile, profiles};
use fontcraft_engine::cmd::export_settings::{self, export_all_needs_folder, kept_for_active};
use serde_json::{Map, Value, json};

use super::fields::{Field, Kind, MAX_CHOICES};
use super::spec::{DialogSpec, Fields, Shaped, Tab};
use crate::app::FontCraftApp;

pub const SPECS: &[DialogSpec] = &[
    DialogSpec::new("dialog.export", "Export", "file.export", Fields::Tabs(&TABS)),
    PROFILES,
    DialogSpec::new(
        "dialog.exportFontsAs",
        "Export Fonts As",
        "file.exportWithProfiles",
        Fields::Shaped(Shaped { fields: export_fields, given: export_given, params: export_params, link: export_link }),
    )
    .run_on_ok()
    .buttons("Export", ""),
    DialogSpec::new("dialog.exportArtwork", "Export Artwork", "glyph.exportArtwork", Fields::Fixed(artwork_fields)).run_on_ok().buttons("Export", ""),
    WINDOW_CONTENTS,
    EXPORT_ALL_FOLDER,
];

/// The tabs of Export, by the id `file.export {tab}` takes.
pub const TABS: [Tab; 4] = [
    Tab { id: "otf", label: "OTF", dialog: OTF },
    Tab { id: "variable", label: "Variable Fonts", dialog: VARIABLE },
    Tab { id: "ufo", label: "UFO", dialog: UFO },
    Tab { id: "metrics", label: "Metrics", dialog: METRICS },
];

/// Static OpenType fonts.
const OTF: DialogSpec = DialogSpec::new(
    "dialog.exportOtf",
    "Export",
    "file.exportStatic",
    Fields::Shaped(Shaped { fields: otf_fields, given: otf_given, params: kept_params, link: destination_link }),
)
.run_on_ok()
.buttons("Next…", "");

/// One variable font per variable font setting, TrueType only.
const VARIABLE: DialogSpec = DialogSpec::new(
    "dialog.exportVariable",
    "Export",
    "file.exportVariable",
    Fields::Shaped(Shaped { fields: variable_fields, given: variable_given, params: kept_params, link: destination_link }),
)
.run_on_ok()
.buttons("Next…", "");

/// Export All when a document has no ticked export folder: one folder for those documents, asked with the chooser
/// as the dialog opens ([`export_all_fired`]). Never the menu item's dialog: Export All skips the Export dialog.
const EXPORT_ALL_FOLDER: DialogSpec = DialogSpec {
    follow_up: true,
    ..DialogSpec::new("export.promptWhenNoDestination", "Export All", "file.exportAll", Fields::Fixed(export_all_fields))
        .run_on_ok()
        .buttons("Export", "")
};

/// One UFO per master.
const UFO: DialogSpec = DialogSpec::new("dialog.exportUfo", "Export", "file.exportUfo", Fields::Fixed(ufo_fields)).run_on_ok().buttons("Next…", "");

/// One master's spacing and kerning as a metrics file or an AFM file.
const METRICS: DialogSpec =
    DialogSpec::new("dialog.exportMetrics", "Export", "file.exportMetrics", Fields::Fixed(metrics_fields)).run_on_ok().buttons("Next…", "");

const PROFILES: DialogSpec = DialogSpec::new(
    "dialog.exportProfiles",
    "Export Profiles",
    "file.saveExportProfile",
    Fields::Shaped(Shaped { fields: profile_fields, given: profile_given, params: profile_params, link: no_link }),
)
.run_on_ok()
.buttons("Save", "");

const WINDOW_CONTENTS: DialogSpec = DialogSpec::new(
    "dialog.exportWindowContents",
    "Export Window Contents",
    "file.exportWindowContents",
    Fields::Shaped(Shaped { fields: window_fields, given: window_given, params: window_params, link: no_link }),
)
.run_on_ok()
.buttons("Export…", "")
.from_window(window_contents);

fn no_link(_: &mut Map<String, Value>, _: &str) {}

/// The shell command File › Export….
pub const MODULE: crate::shell::commands::Module = crate::shell::commands::Module { commands: COMMANDS, run: run_command };

const COMMANDS: &[crate::shell::commands::UiCommand] = &[crate::shell::commands::UiCommand {
    id: "file.export",
    label: "Export…",
    menu: &["File"],
    shortcut: Some("Cmd+E"),
    params: "{tab?: \"otf\" (default) | \"variable\" | \"ufo\" | \"metrics\", params?: {…}} opens the export dialog of the active document on a tab: static OpenType (file.exportStatic), variable fonts (file.exportVariable), UFO (file.exportUfo) or metrics (file.exportMetrics). `params` gives first values as dialog.open does. Next… runs the tab's command; with no export folder it asks for one in the window and is an error over the control channel",
}];

fn run_command(app: &mut FontCraftApp, command: &craft::ui::runtime::commands::CommandRef) -> Result<(), String> {
    if command.id != "file.export" {
        return Err(format!("no export command `{}`", command.id));
    }
    let given = match command.params.get("params") {
        None | Some(Value::Null) => Map::new(),
        Some(Value::Object(m)) => m.clone(),
        Some(_) => return Err("`params` must be a JSON object".into()),
    };
    let tab = command.params.get("tab").and_then(Value::as_str).unwrap_or("otf");
    let spec = TABS.iter().find(|t| t.id == tab).map(|t| t.dialog).ok_or_else(|| format!("unknown tab `{tab}`: otf, variable, ufo or metrics"))?;
    super::form::open(app, &spec, &given).map(drop)
}

/// The export folder field of every tab.
fn destination() -> Field {
    Field::folder("destination", "Export Folder", "The folder the files are written to; files of the same name are replaced")
}

/// The Export Destination checkbox of the OTF and Variable Fonts tabs.
fn use_destination() -> Field {
    Field::bool("useDestination", "Export Destination", true, "Write into the export folder; unticked, Next… asks for a folder every time")
}

/// The tab's values from the settings the document's last successful export kept, under what `given` names.
fn kept_given(session: &Session, tab: export_settings::Tab, p: &Map<String, Value>) -> Map<String, Value> {
    let mut out = kept_for_active(session, tab);
    out.extend(p.iter().map(|(k, v)| (k.clone(), v.clone())));
    out
}

fn otf_given(session: &Session, p: &Map<String, Value>) -> Result<Map<String, Value>, String> {
    Ok(kept_given(session, export_settings::Tab::Otf, p))
}

fn variable_given(session: &Session, p: &Map<String, Value>) -> Result<Map<String, Value>, String> {
    Ok(kept_given(session, export_settings::Tab::Variable, p))
}

/// The field values are the command's parameters; the command keeps them, `useDestination` with them.
fn kept_params(values: &Map<String, Value>) -> Map<String, Value> {
    values.clone()
}

/// With Export Destination unticked the folder field is emptied, so Next… asks for a folder.
fn destination_link(values: &mut Map<String, Value>, key: &str) {
    if (key.is_empty() || key == "useDestination") && values.get("useDestination") == Some(&Value::Bool(false)) {
        values.insert("destination".into(), json!(""));
    }
}

fn export_all_fields(_: &Session) -> Vec<Field> {
    vec![Field::folder("fallbackDestination", "Export Folder", "The folder for the documents that have no export folder of their own")]
}

/// Export All fired by the person: runs straight away when every document has a ticked export folder (returns
/// false), else opens [`EXPORT_ALL_FOLDER`] with the folder chooser, and OK follows the folder chosen.
pub fn export_all_fired(app: &mut FontCraftApp, id: &str, params: &Value) -> bool {
    let empty = params.is_null() || params.as_object().is_some_and(Map::is_empty);
    if id != "file.exportAll" || !empty || !export_all_needs_folder(&app.session) {
        return false;
    }
    let opened = super::form::open(app, &EXPORT_ALL_FOLDER, &Map::new()).and_then(|_| super::form::choose(app, "fallbackDestination", true));
    if let Err(e) = opened {
        app.status = e;
    }
    true
}

/// A pop-up of the active document's masters by name; the value is the master id, the first master at first (the
/// dialog opens on the current master).
pub fn master_field(session: &Session, key: &str, label: &str, doc: &str) -> Field {
    let masters: Vec<(String, String)> =
        session.font().map(|f| f.masters.iter().take(MAX_CHOICES).map(|m| (m.id.clone(), m.name.clone())).collect()).unwrap_or_default();
    Field {
        key: key.into(),
        label: label.into(),
        default: json!(masters.first().map(|(id, _)| id.clone()).unwrap_or_default()),
        kind: Kind::Choice { choices: masters.iter().map(|(id, _)| id.clone()).collect(), labels: masters.into_iter().map(|(_, n)| n).collect() },
        doc: doc.into(),
        group: None,
        same_as: None,
    }
}

fn otf_fields(_: &Session) -> Vec<Field> {
    vec![
        Field::choice(
            "outlines",
            "Outline Flavor",
            &[("cff", "PostScript/CFF"), ("truetype", "TrueType")],
            "cff",
            "cff: PostScript outlines in a .otf file; truetype: quadratic outlines in a .ttf file",
        ),
        containers(),
        Field::bool("removeOverlap", "Remove Overlap", true, "Merge overlapping paths and components in every exported glyph"),
        Field::bool(
            "autohint",
            "Autohint",
            false,
            "PostScript/CFF: hint every glyph without manual PostScript hints; TrueType: hint the whole font, in place of any manual TrueType hints. An instance's Autohint parameter overrides this",
        ),
        Field::bool("cbdt", "CBDT", false, "Also write CBDT and CBLC bitmap tables from the iColor layers, for renderers that do not draw sbix"),
        use_destination(),
        destination(),
    ]
}

fn containers() -> Field {
    Field::multi(
        "containers",
        "File Format",
        &[("plain", "OTF/TTF"), ("woff", "WOFF"), ("woff2", "WOFF2")],
        &["plain"],
        "The containers to write each font in: the plain .otf or .ttf file, WOFF, WOFF2",
    )
}

fn variable_fields(_: &Session) -> Vec<Field> {
    vec![containers(), use_destination(), destination()]
}

fn ufo_fields(session: &Session) -> Vec<Field> {
    let masters: Vec<(String, String)> =
        session.font().map(|f| f.masters.iter().take(MAX_CHOICES).map(|m| (m.id.clone(), m.name.clone())).collect()).unwrap_or_default();
    let ids: Vec<&String> = masters.iter().map(|(id, _)| id).collect();
    vec![
        Field {
            key: "masters".into(),
            label: "Masters".into(),
            kind: Kind::Multi {
                choices: masters.iter().map(|(id, _)| id.clone()).collect(),
                labels: masters.iter().map(|(_, name)| name.clone()).collect(),
            },
            default: json!(ids),
            doc: "The masters to write, each as its own UFO".into(),
            group: None,
            same_as: None,
        },
        Field::bool(
            "productionNames",
            "Convert Glyph Names to Production Names",
            false,
            "Write each glyph under its production name (its own, else the glyph data's), with components, kerning, groups and the glyph order following; the document keeps its names",
        ),
        Field::bool(
            "decomposeSmartStuff",
            "Decompose Smart Stuff",
            true,
            "Draw smart components as outlines; off keeps them as components with their values in the glyph lib",
        ),
        destination(),
    ]
}

fn metrics_fields(session: &Session) -> Vec<Field> {
    vec![
        master_field(session, "master", "Master", "The master whose spacing and kerning are written"),
        Field::choice(
            "format",
            "Format",
            &[("metrics", "Metrics File"), ("afm", "AFM File")],
            "metrics",
            "Metrics File: every pair, group, width, sidebearing and metrics key, read back exactly; AFM File: widths and glyph pairs for other editors, without groups or keys",
        ),
        Field::text("fileName", "File Name", 200, "The file name without its extension (.tsv or .afm is added); empty: family and master name"),
        destination(),
    ]
}

// ── Export profiles ─────────────────────────────────────────────────────────────────────────────────────────────────

/// The settings fields, keyed as in a profile's params.
const SETTINGS: [&str; 6] = ["folder", "outlines", "containers", "removeOverlap", "autohint", "cbdt"];

fn profile_names(session: &Session) -> Vec<String> {
    profiles(session).0.into_iter().take(MAX_CHOICES).map(|p| p.name).collect()
}

fn profile_fields(session: &Session) -> Vec<Field> {
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
            same_as: None,
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
        Field::bool("variable", "Variable Fonts", false, "Also build the variable fonts, as the Variable Fonts tab of Export does")
            .in_group("Output"),
        Field::bool("removeOverlap", "Remove Overlap", true, "Merge overlapping paths and components in every exported glyph").in_group("Outlines"),
        Field::text(
            "conversionError",
            "Curve Error",
            16,
            "Largest distance of the quadratic outlines from the cubic ones, in em (above 0, at most 1); empty: the default",
        )
        .in_group("Outlines"),
        Field::bool("autohint", "Autohint", false, "Add automatic hints to the exported fonts").in_group("Hinting"),
        Field::bool("cbdt", "CBDT", false, "Also write CBDT and CBLC bitmap tables from the iColor layers, for renderers that do not draw sbix")
            .in_group("Colour"),
        Field::bool("colr", "COLR", true, "Write the COLR and CPAL tables of palette layers").in_group("Colour"),
        Field::bool("svg", "SVG", true, "Write the SVG table of SVG layers").in_group("Colour"),
        Field::bool("sbix", "sbix", true, "Write the sbix table of iColor layers").in_group("Colour"),
        Field::choice(
            "subroutinize",
            "Subroutines",
            &[("", "Font's Setting"), ("true", "On"), ("false", "Off")],
            "",
            "Move repeated parts of CFF outlines into subroutines; the font's setting follows its Disable Subroutines parameter",
        )
        .in_group("Size"),
        Field::choice(
            "postFormat",
            "post Table",
            &[("", "Default"), ("2", "Format 2: glyph names"), ("3", "Format 3: no glyph names")],
            "",
            "Format 3 leaves the glyph names out of the font, which makes it smaller",
        )
        .in_group("Size"),
        Field::choice(
            "productionNames",
            "Production Names",
            &[("", "Font's Setting"), ("true", "On"), ("false", "Off")],
            "",
            "Rename the glyphs to their production names in the exported fonts",
        )
        .in_group("Glyphs"),
        Field::text(
            "keepGlyphs",
            "Keep Glyphs",
            65_536,
            "Only these glyphs (and their components) are exported; names separated by spaces; empty: all",
        )
        .in_group("Glyphs"),
        Field::text("removeGlyphs", "Remove Glyphs", 65_536, "These glyphs are left out of the exported fonts; names separated by spaces")
            .in_group("Glyphs"),
        Field::bool("skipFeatures", "Skip Features", false, "Build no OpenType layout features").in_group("Layout"),
    ]
}

/// The compile-group fields, keyed as in a profile's `compile` group.
const COMPILE: [&str; 11] = [
    "variable",
    "skipFeatures",
    "productionNames",
    "postFormat",
    "conversionError",
    "subroutinize",
    "keepGlyphs",
    "removeGlyphs",
    "colr",
    "svg",
    "sbix",
];

/// A compile-group value as its field shows it.
fn compile_field_value(key: &str, v: &Value) -> Value {
    match (key, v) {
        ("keepGlyphs" | "removeGlyphs", Value::Array(names)) => json!(names.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(" ")),
        ("productionNames" | "subroutinize", Value::Bool(b)) => json!(b.to_string()),
        ("postFormat" | "conversionError", Value::Number(n)) => json!(n.to_string()),
        _ => v.clone(),
    }
}

/// A compile-group field's value as the profile keeps it; null for the default, which removes the key.
fn compile_param(key: &str, v: Option<&Value>) -> Value {
    let text = v.and_then(Value::as_str).map(str::trim).unwrap_or_default();
    match key {
        "keepGlyphs" | "removeGlyphs" if !text.is_empty() => json!(text.split_whitespace().collect::<Vec<_>>()),
        "productionNames" | "subroutinize" if !text.is_empty() => json!(text == "true"),
        "postFormat" => text.parse::<i64>().map_or(Value::Null, |n| json!(n)),
        // A number that does not parse goes on as text, so the command names what is wrong with it.
        "conversionError" if !text.is_empty() => text.parse::<f64>().map_or_else(|_| json!(text), |e| json!(e)),
        "variable" | "skipFeatures" => match v {
            Some(Value::Bool(true)) => json!(true),
            _ => Value::Null,
        },
        "colr" | "svg" | "sbix" => match v {
            Some(Value::Bool(false)) => json!(false),
            _ => Value::Null,
        },
        _ => Value::Null,
    }
}

/// Field values from parameters in the command's shape: the named profile's settings (or the base's), with `settings`
/// on top.
fn profile_given(session: &Session, p: &Map<String, Value>) -> Result<Map<String, Value>, String> {
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
        if let Some(Value::Object(group)) = start.params.get("compile") {
            for (key, v) in group.iter().filter(|(k, _)| COMPILE.contains(&k.as_str())) {
                out.insert(key.clone(), compile_field_value(key, v));
            }
        }
    }
    if let Some(Value::Object(settings)) = p.get("settings") {
        for key in SETTINGS {
            if let Some(v) = settings.get(key) {
                out.insert(key.into(), v.clone());
            }
        }
        if let Some(Value::Object(group)) = settings.get("compile") {
            for (key, v) in group.iter().filter(|(k, _)| COMPILE.contains(&k.as_str())) {
                out.insert(key.clone(), compile_field_value(key, v));
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

fn profile_params(values: &Map<String, Value>) -> Map<String, Value> {
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
    let compile: Map<String, Value> = COMPILE.iter().map(|key| ((*key).to_string(), compile_param(key, values.get(*key)))).collect();
    settings.insert("compile".into(), Value::Object(compile));
    let mut out = Map::new();
    out.insert("name".into(), values.get("name").cloned().unwrap_or(json!("")));
    if let Some(based) = values.get("basedOn").and_then(Value::as_str).filter(|s| !s.is_empty()) {
        out.insert("basedOn".into(), json!(based));
    }
    out.insert("settings".into(), Value::Object(settings));
    out
}

fn export_fields(session: &Session) -> Vec<Field> {
    let names = profile_names(session);
    let first: Vec<&String> = names.iter().take(1).collect();
    let fonts: Vec<String> = session.documents().iter().take(MAX_CHOICES).map(|d| d.title.clone()).collect();
    let active: Vec<String> = session.active().map(|d| d.title.clone()).into_iter().collect();
    vec![
        Field {
            key: "fonts".into(),
            label: "Fonts".into(),
            kind: Kind::Multi { choices: fonts, labels: Vec::new() },
            default: json!(active),
            doc: "The open fonts to export; the active one at first".into(),
            group: None,
            same_as: None,
        },
        Field::bool(
            "bySource",
            "Next to Each Source",
            false,
            "Write each font's profile folders next to its source file instead of into the export folder",
        ),
        Field {
            key: "profiles".into(),
            label: "Profiles".into(),
            kind: Kind::Multi { choices: names.clone(), labels: Vec::new() },
            default: json!(first),
            doc: "The profiles to export through; each writes into its own subfolder".into(),
            group: None,
            same_as: None,
        },
        Field::folder("destination", "Export Folder", "The folder the profiles' subfolders are made in; files of the same name are replaced"),
    ]
}

fn export_given(_: &Session, p: &Map<String, Value>) -> Result<Map<String, Value>, String> {
    Ok(p.clone())
}

/// With Next to Each Source the export folder is no parameter.
fn export_params(values: &Map<String, Value>) -> Map<String, Value> {
    let mut out = values.clone();
    if values.get("bySource") == Some(&Value::Bool(true)) {
        out.remove("destination");
    } else {
        out.remove("bySource");
    }
    out
}

/// With Next to Each Source ticked the folder field says so, which is no folder to ask for; unticked, it is
/// emptied again.
fn export_link(values: &mut Map<String, Value>, key: &str) {
    if !key.is_empty() && key != "bySource" {
        return;
    }
    let by_source = values.get("bySource") == Some(&Value::Bool(true));
    let shows_note = values.get("destination").and_then(Value::as_str) == Some(BY_SOURCE_NOTE);
    if by_source {
        values.insert("destination".into(), json!(BY_SOURCE_NOTE));
    } else if shows_note {
        values.insert("destination".into(), json!(""));
    }
}

/// What the folder field shows while Next to Each Source is ticked.
const BY_SOURCE_NOTE: &str = "(next to each source file)";

fn artwork_fields(session: &Session) -> Vec<Field> {
    vec![
        Field::choice("format", "Format", &[("svg", "SVG"), ("pdf", "PDF"), ("png", "PNG")], "svg", "The picture format, one file per glyph"),
        master_field(session, "layer", "Master", "The master whose layers are drawn"),
        Field::number("pixelsPerEm", "Pixels per Em", 1.0, 16_384.0, 256.0, "The size of PNG pictures; SVG and PDF are in font units"),
        Field::folder("directory", "Export Folder", "The folder the pictures are written to; files of the same name are replaced"),
    ]
}

// ── Export Window Contents ──────────────────────────────────────────────────────────────────────────────────────────

/// The command's em sizes, in points (`fontcraft_engine::cmd::view_export::SIZE_RANGE`).
const MIN_SIZE: f64 = 1.0;
const MAX_SIZE: f64 = 10_000.0;

/// The tab's contents: the edited glyph as a display string, the current master, and the glyph's layer when the canvas
/// shows another layer than its master's.
fn window_contents(app: &FontCraftApp) -> Map<String, Value> {
    let mut out = Map::new();
    let Ok(font) = app.session.font() else { return out };
    let Some(glyph) = crate::shell::masters::shown_glyph(font, &app.view) else { return out };
    out.insert("text".into(), json!(format!("/{}", glyph.name)));
    if let Some(master) = crate::shell::masters::current_master(font, &app.view.layers) {
        out.insert("master".into(), json!(master.id));
        if let Some(layer) = crate::shell::masters::panel_layer(font, &app.view).filter(|l| l.id != master.id) {
            let mut picks = Map::new();
            picks.insert(glyph.name.clone(), json!(layer.id));
            out.insert("layers".into(), Value::Object(picks));
        }
    }
    out
}

fn window_fields(session: &Session) -> Vec<Field> {
    let upm = session.font().map_or(1000.0, |f| f64::from(f.units_per_em));
    vec![
        Field::save_file(
            "path",
            "File",
            &["pdf", "svg"],
            "The file to write: .pdf fills the glyphs black like the preview; .svg draws the editing view with nodes, handles and guides",
        ),
        Field::number("size", "Points per Em", MIN_SIZE, MAX_SIZE, upm, "The page's scale: one em in points; large pages are scaled down"),
    ]
}

/// A `format` given in the command's shape names the file's extension; the other parameters pass through.
fn window_given(_: &Session, p: &Map<String, Value>) -> Result<Map<String, Value>, String> {
    let mut out = p.clone();
    out.remove("format");
    if let (Some(path), Some(format)) = (p.get("path").and_then(Value::as_str), p.get("format").and_then(Value::as_str))
        && format_of(path).is_none()
    {
        out.insert("path".into(), json!(format!("{path}.{format}")));
    }
    Ok(out)
}

/// The format a path's extension names: `svg` for .svg, `pdf` for .pdf, `None` for any other.
fn format_of(path: &str) -> Option<&'static str> {
    let extension = std::path::Path::new(path).extension()?.to_str()?.to_ascii_lowercase();
    match extension.as_str() {
        "svg" => Some("svg"),
        "pdf" => Some("pdf"),
        _ => None,
    }
}

fn window_params(values: &Map<String, Value>) -> Map<String, Value> {
    let mut out = Map::new();
    let path = values.get("path").and_then(Value::as_str).map(str::trim).filter(|p| !p.is_empty());
    out.insert("format".into(), json!(path.and_then(format_of).unwrap_or("pdf")));
    if let Some(path) = path {
        out.insert("path".into(), json!(path));
    }
    if let Some(size) = values.get("size").filter(|v| v.is_number()) {
        out.insert("size".into(), size.clone());
    }
    out
}

#[cfg(test)]
#[path = "export_tests.rs"]
mod tests;
