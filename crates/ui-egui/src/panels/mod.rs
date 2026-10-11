//! The Palette, the Info box, the Edit View bottom bar and the Selection Info window.
//!
//! | Module | Draws |
//! |---|---|
//! | [`palette`] | the Palette sidebar: sections with chevrons, and the contextual area |
//! | [`dimensions`] | Palette › Dimensions: the measurement notepad of the current master |
//! | [`fit_curve`] | Palette › Fit Curve: range fields, eight steps, plus, minus, balance |
//! | [`layers`] | Palette › Layers: the layers of the current glyph |
//! | [`transformations`] | Palette › Transformations: origin, mirror, scale, rotate, slant, align, booleans |
//! | [`stroke`] | Palette › Stroke: the stroke of the selected paths (contextual) |
//! | [`info_box`] | the Info box over the canvas: the selection part, or the annotation palette |
//! | [`info_box_metrics`] | the glyph Info box under the current glyph: name, sidebearings, width |
//! | [`info_box_kerning`] | the Info box's kerning part and the preceding glyph's box |
//! | [`bottom_bar`] | the zoom controls along the lower edge of an Edit View tab |
//! | [`widgets`] | fields with the Info box editing rules, buttons, the nine-point grid |
//! | [`selection_info`] | the Selection Info window: the selected glyphs' properties |
//!
//! Panels read engine state (the font and queries such as `selection.getInfo`) and act only by
//! queueing commands with explicit targets; the app runs them after the frame. Their own
//! settings (collapsed sections, the Info box point, the Fit Curve range) are UI state in [`PanelState`], which
//! only this module's shell commands ([`COMMANDS`]) change.

pub mod bottom_bar;
pub mod dimensions;
pub mod fit_curve;
pub mod info_box;
pub mod info_box_kerning;
pub mod info_box_metrics;
pub mod layers;
pub mod palette;
pub mod selection_info;
pub mod stroke;
pub mod transformations;
pub mod widgets;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use crate::app::FontcraftApp;
use crate::shell::{Args, UiCommand};
use crate::tools_glue;

/// Commands queued while drawing, run by the app after the frame.
pub type Pending = Vec<(&'static str, Value)>;

/// The Palette sections, in order: (id, title).
pub const SECTIONS: [(&str, &str); 4] =
    [("dimensions", "Dimensions"), ("fitCurve", "Fit Curve"), ("layers", "Layers"), ("transformations", "Transformations")];
/// The metric lines a Transformations origin can sit on.
pub const METRICS: [&str; 5] = ["baseline", "halfXHeight", "xHeight", "halfCapHeight", "capHeight"];

/// The panels' own settings.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct PanelState {
    /// The Info box point that X and Y report and edit (`infoBox.setReferencePoint`).
    pub reference_point: String,
    /// Width and height of the Info box scale together (`infoBox.setSizeLock`).
    pub size_locked: bool,
    /// Ids of the collapsed Palette sections.
    pub collapsed: Vec<String>,
    /// The Fit Curve range fields; `None`: the font's `Fit Curve Panel Settings`, else 48 and 74.
    pub fit_curve_range: Option<[f64; 2]>,
    /// Layers list filters.
    pub layers_current_master_only: bool,
    pub hide_backup_layers: bool,
    /// The glyph inspector at the bottom left of Font View is hidden (`fontView.toggleInspector`).
    pub inspector_hidden: bool,
    /// The inspector shows production name, script, category and subcategory
    /// (`fontView.toggleInspectorDetails`).
    pub inspector_details: bool,
    /// The Selection Info window is open (`edit.infoForSelection`).
    pub selection_info: bool,
    /// The Kerning window: open, the search, the sort and the selected rows.
    pub kerning_window: crate::shell::kerning_window::KerningWindowState,
    /// The inspector shows the vertical metric fields (`fontView.toggleMetricsOrientation`).
    pub inspector_vertical: bool,
}

impl Default for PanelState {
    fn default() -> Self {
        Self {
            reference_point: "bottomLeft".into(),
            size_locked: false,
            collapsed: Vec::new(),
            fit_curve_range: None,
            layers_current_master_only: false,
            hide_backup_layers: false,
            inspector_hidden: false,
            inspector_details: false,
            selection_info: false,
            kerning_window: Default::default(),
            inspector_vertical: false,
        }
    }
}

/// Draws the Palette (when shown), and for an Edit View tab its bottom bar and, while Show Info
/// is on, the Info box over the canvas; then the Selection Info window while it is open. Call
/// before the canvas: the Info box sits on the space the panels leave.
pub fn show(app: &mut FontcraftApp, ui: &mut egui::Ui, pending: &mut Pending) {
    if app.state.show_palette {
        palette::show(app, ui, pending);
    }
    if app.state.tab().is_some() {
        bottom_bar::show(app, ui, pending);
        if app.state.display.info {
            info_box::show(app, ui, pending);
        }
    }
    selection_info::show(app, ui, pending);
}

/// The glyph the panels describe: the edited glyph, else the first glyph selected in Font View.
pub fn current_glyph(app: &FontcraftApp) -> Option<String> {
    app.state.editing().map(str::to_string).or_else(|| app.state.selected_glyphs.first().cloned())
}

/// The target object of command `id` for the showing tab's selection: glyph and layer plus the
/// selection's lists the command takes. `None` without an Edit View tab.
pub fn edit_targets(app: &FontcraftApp, id: &str) -> Option<Map<String, Value>> {
    match tools_glue::with_targets(app, id, Value::Null) {
        Value::Object(m) => Some(m),
        _ => None,
    }
}

/// Whether a target object names part of the layer rather than the whole layer.
pub fn has_lists(targets: &Map<String, Value>) -> bool {
    ["points", "segments", "shapes", "anchors", "guides", "annotations"].iter().any(|k| targets.contains_key(*k))
}

/// `targets` with the entries of `extra` (an object) added.
pub fn with(mut targets: Map<String, Value>, extra: Value) -> Value {
    if let Value::Object(more) = extra {
        targets.extend(more);
    }
    Value::Object(targets)
}

pub const COMMANDS: &[UiCommand] = &[
    (
        "palette.toggleSection",
        "Collapse / Expand Section",
        &[],
        "",
        "{section: \"dimensions\" | \"fitCurve\" | \"layers\" | \"transformations\", expanded?: bool (omitted: toggle)} collapse a Palette section to its title row or expand it → {section, expanded}",
    ),
    (
        "infoBox.setReferencePoint",
        "Set Info Box Reference Point",
        &[],
        "",
        "{point: \"topLeft\" | \"top\" | \"topRight\" | \"left\" | \"center\" | \"right\" | \"bottomLeft\" | \"bottom\" | \"bottomRight\"} the point of the selection's box that the Info box X and Y report and edit → {point}",
    ),
    (
        "infoBox.setSizeLock",
        "Lock Info Box Proportions",
        &[],
        "",
        "{locked?: bool (omitted: toggle)} locked: editing the Info box width or height scales both → {locked}",
    ),
    (
        "fitCurve.setRange",
        "Set Fit Curve Range",
        &[],
        "",
        "{min?: percent, max?: percent} the Fit Curve range fields, each held to 1..100; step 1 applies min, step 8 max → {min, max}",
    ),
    (
        "layers.onlyShowLayersFromCurrentMaster",
        "Only Show Layers from Current Master",
        &[],
        "",
        "{on?: bool} list only the layers of the current master in the Layers section (omitted: toggle)",
    ),
    ("layers.hideBackupLayers", "Hide Backup Layers", &[], "", "{on?: bool} leave backup layers out of the Layers section (omitted: toggle)"),
];

fn one_of(args: &Args, key: &str, allowed: &[&str]) -> Result<Option<String>, String> {
    match args.string(key)? {
        None => Ok(None),
        Some(v) if allowed.contains(&v) => Ok(Some(v.to_string())),
        Some(v) => Err(args.err(format!("`{key}` must be one of {}, got \"{v}\"", allowed.join(", ")))),
    }
}

pub fn run(app: &mut FontcraftApp, id: &str, args: &Args) -> Result<Value, String> {
    let panels = &mut app.state.panels;
    match id {
        "palette.toggleSection" => {
            let ids: Vec<&str> = SECTIONS.iter().map(|s| s.0).collect();
            let section = one_of(args, "section", &ids)?.ok_or_else(|| args.err("give `section`"))?;
            let was = !panels.collapsed.contains(&section);
            let expanded = args.flag("expanded")?.unwrap_or(!was);
            panels.collapsed.retain(|s| *s != section);
            if !expanded {
                panels.collapsed.push(section.clone());
            }
            Ok(json!({"section": section, "expanded": expanded}))
        }
        "infoBox.setReferencePoint" => {
            let point = one_of(args, "point", &widgets::BOX_POINTS)?.ok_or_else(|| args.err("give `point`"))?;
            panels.reference_point.clone_from(&point);
            Ok(json!({ "point": point }))
        }
        "infoBox.setSizeLock" => {
            panels.size_locked = args.flag("locked")?.unwrap_or(!panels.size_locked);
            Ok(json!({"locked": panels.size_locked}))
        }
        "fitCurve.setRange" => {
            let (min, max) = (args.number("min")?, args.number("max")?);
            if min.is_none() && max.is_none() {
                return Err(args.err("give `min`, `max` or both"));
            }
            let [dmin, dmax] = match panels.fit_curve_range {
                Some(r) => r,
                None => fit_curve::font_range(app.session.font().ok()),
            };
            let (min, max) = (min.unwrap_or(dmin).clamp(1.0, 100.0), max.unwrap_or(dmax).clamp(1.0, 100.0));
            app.state.panels.fit_curve_range = Some([min, max]);
            Ok(json!({"min": min, "max": max}))
        }
        "layers.onlyShowLayersFromCurrentMaster" => {
            panels.layers_current_master_only = args.toggle(panels.layers_current_master_only)?;
            Ok(json!({"on": panels.layers_current_master_only}))
        }
        "layers.hideBackupLayers" => {
            panels.hide_backup_layers = args.toggle(panels.hide_backup_layers)?;
            Ok(json!({"on": panels.hide_backup_layers}))
        }
        other => Err(format!("unknown command `{other}`")),
    }
}
