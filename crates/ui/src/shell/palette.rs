//! The settings of the Info box and the Palette sections: the Info box's reference point and size lock, the
//! Transformations section's origin, scale, rotate and slant values, and the Fit Curve range.
//!
//! They are view state. The engine's commands get them as parameters each time: the Info box passes its reference
//! point to `selection.getInfo` and `selection.setPosition`, a Transformations button passes the origin and its value
//! to its `transform.*` command (`docs/spec/04-palette.md` › Transformations).

use craft::ui::runtime::commands::CommandRef;
use fontcraft_engine::Session;
use fontcraft_engine::cmd::fit_curve;
use fontcraft_engine::cmd::transform::{BoxPoint, MetricPoint};
use fontcraft_engine::model::MAX_COORDINATE;
use serde_json::{Value, json};

use super::commands::{Module, UiCommand};
use super::params::{flag, string};
use crate::app::{FontCraftApp, ViewState};
use crate::panels::widgets::BOX_POINTS;

/// Largest scale percentage and rotation angle the Transformations fields hold.
const MAX_SCALE: f64 = 1_000_000.0;
const MAX_ANGLE: f64 = 3_600.0;
/// The Fit Curve range fields hold percentages within this range.
const FIT_CURVE_PERCENT: (f64, f64) = (1.0, 100.0);

/// The metric heights the Transformations origin can sit on, as the transform commands name them.
pub const METRICS: [&str; 5] = ["baseline", "halfXHeight", "xHeight", "halfCapHeight", "capHeight"];

pub const MODULE: Module = Module { commands: COMMANDS, run };

const COMMANDS: &[UiCommand] = &[
    UiCommand {
        id: "infoBox.setReferencePoint",
        label: "Set Info Box Reference Point",
        menu: &[],
        shortcut: None,
        params: "{point: \"topLeft\" | \"top\" | \"topRight\" | \"left\" | \"center\" | \"right\" | \"bottomLeft\" | \"bottom\" | \"bottomRight\"} the point of the selection's box that the Info box X and Y report and edit",
    },
    UiCommand {
        id: "infoBox.setSizeLock",
        label: "Lock Info Box Proportions",
        menu: &[],
        shortcut: None,
        params: "{locked?: bool (omitted: toggle)} locked: editing the Info box width or height scales both",
    },
    UiCommand {
        id: "transform.setOrigin",
        label: "Set Transformation Origin",
        menu: &[],
        shortcut: None,
        params: "{mode?: \"box\" | \"reference\" | \"metrics\" | \"layer\", boxPoint?: \"topLeft\" | \"top\" | \"topRight\" | \"left\" | \"center\" | \"right\" | \"bottomLeft\" | \"bottom\" | \"bottomRight\", metric?: \"baseline\" | \"halfXHeight\" | \"xHeight\" | \"halfCapHeight\" | \"capHeight\", point?: [x, y] | null} choose the fixed point of the Transformations section. `point` places the reference point (what a click with the Rotate or Scale tool does), which those tools turn and scale about in every glyph; without `mode` the mode stays. `point: null` takes the reference point away: the tools turn about the centre of the selection's box again, and mode \"reference\" becomes \"box\". `boxPoint` and `metric` are kept for their modes",
    },
    UiCommand {
        id: "transform.setScaleLock",
        label: "Lock Scale Proportions",
        menu: &[],
        shortcut: None,
        params: "{locked?: bool} lock the vertical scale field to the horizontal one (omitted: toggle)",
    },
    UiCommand {
        id: "transform.setSettings",
        label: "Set Transformation Values",
        menu: &[],
        shortcut: None,
        params: "{scaleX?: percent (non-zero, within ±1000000), scaleY?: percent, rotate?: degrees (within ±3600), slant?: degrees (between -90 and 90)} the values of the Transformations fields",
    },
    UiCommand {
        id: "fitCurve.setRange",
        label: "Set Fit Curve Range",
        menu: &[],
        shortcut: None,
        params: "{min?: percent, max?: percent} the Fit Curve range fields, each held to 1..100; step 1 applies min, step 8 max",
    },
];

/// Which fixed point the Transformations buttons use.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum OriginMode {
    /// One of the nine points of the selection's box.
    #[default]
    Box,
    /// The point placed with the Rotate or Scale tool.
    Reference,
    /// A metric height of the master, at half the advance width.
    Metrics,
    /// The centre of the layer (CJK glyphs).
    Layer,
}

/// The Transformations section's settings.
#[derive(Clone, Debug, PartialEq)]
pub struct TransformSettings {
    pub mode: OriginMode,
    /// The box point of the box mode, by its parameter name.
    pub box_point: String,
    /// The metric of the metrics mode, by its parameter name.
    pub metric: String,
    /// The reference point, in font units; `None` until one is placed.
    pub point: Option<[f64; 2]>,
    /// The horizontal and vertical scale fields, in percent.
    pub scale: [f64; 2],
    /// Locked: the horizontal field drives both axes.
    pub scale_locked: bool,
    pub rotate_angle: f64,
    pub slant_angle: f64,
}

impl Default for TransformSettings {
    fn default() -> Self {
        TransformSettings {
            mode: OriginMode::Box,
            box_point: "center".into(),
            metric: "baseline".into(),
            point: None,
            scale: [110.0, 110.0],
            scale_locked: true,
            rotate_angle: 90.0,
            slant_angle: 10.0,
        }
    }
}

impl TransformSettings {
    /// The `origin` parameter of the engine's transform commands. The reference mode without a placed point falls
    /// back to the box centre.
    pub fn origin_param(&self) -> Value {
        match (self.mode, self.point) {
            (OriginMode::Box, _) => json!({"mode": "box", "boxPoint": self.box_point}),
            (OriginMode::Reference, None) => json!({"mode": "box", "boxPoint": "center"}),
            (OriginMode::Reference, Some(p)) => json!({"mode": "reference", "point": p}),
            (OriginMode::Metrics, _) => json!({"mode": "metrics", "metric": self.metric}),
            (OriginMode::Layer, _) => json!({"mode": "layer"}),
        }
    }

    /// The horizontal and vertical percentages a Scale button applies: with the lock closed, the horizontal field for
    /// both.
    pub fn scale_percent(&self) -> (f64, f64) {
        let [x, y] = self.scale;
        if self.scale_locked { (x, x) } else { (x, y) }
    }
}

/// The Info box's and the Palette sections' view state.
#[derive(Clone, Debug, PartialEq)]
pub struct PaletteView {
    /// The point of the selection's box the Info box reports and edits.
    pub reference_point: String,
    /// Editing the Info box width or height scales both.
    pub size_locked: bool,
    pub transform: TransformSettings,
    /// The Fit Curve range fields; `None`: the font's range.
    pub fit_curve_range: Option<[f64; 2]>,
    /// The values typed into the Modules panel's forms (`super::palette_modules`).
    pub modules: super::palette_modules::ModuleForms,
    /// The panels `window.palette` closed, which showing the Palette opens again, and whether each was in front of its
    /// group.
    pub hidden: Vec<(super::dock::PanelId, bool)>,
}

impl Default for PaletteView {
    fn default() -> Self {
        PaletteView {
            reference_point: "bottomLeft".into(),
            size_locked: false,
            transform: TransformSettings::default(),
            fit_curve_range: None,
            modules: Default::default(),
            hidden: Vec::new(),
        }
    }
}

/// The Fit Curve range: the fields' when set, else the font's, else the commands' default.
pub fn fit_curve_range_of(session: &Session, view: &ViewState) -> [f64; 2] {
    view.palette.fit_curve_range.unwrap_or_else(|| {
        let (min, max) = session.font().ok().and_then(fit_curve::font_range).unwrap_or(fit_curve::DEFAULT_RANGE);
        [min, max]
    })
}

/// The number at `key`; `None` when absent or null. Refuses a value that is not a finite number.
fn number(p: &Value, key: &str) -> Result<Option<f64>, String> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_f64().filter(|n| n.is_finite()).map(Some).ok_or_else(|| format!("`{key}` must be a number")),
    }
}

/// The point `[x, y]` at `key`, each within the coordinate limit.
fn point(p: &Value, key: &str) -> Result<Option<[f64; 2]>, String> {
    let Some(v) = p.get(key).filter(|v| !v.is_null()) else { return Ok(None) };
    let list = v.as_array().filter(|l| l.len() == 2).ok_or_else(|| format!("`{key}` must be [x, y]"))?;
    let at = |i: usize| list.get(i).and_then(Value::as_f64).filter(|n| n.is_finite() && n.abs() <= MAX_COORDINATE);
    match (at(0), at(1)) {
        (Some(x), Some(y)) => Ok(Some([x, y])),
        _ => Err(format!("`{key}` must be two numbers within ±{MAX_COORDINATE}")),
    }
}

fn run(app: &mut FontCraftApp, command: &CommandRef) -> Result<(), String> {
    let p = &command.params;
    match command.id.as_str() {
        "infoBox.setReferencePoint" => {
            let name = string(p, "point")?.ok_or("give `point`")?;
            let point = BOX_POINTS.iter().find(|n| **n == name).ok_or_else(|| format!("`point` must be one of {}", BOX_POINTS.join(", ")))?;
            app.view.palette.reference_point = (*point).to_string();
            Ok(())
        }
        "infoBox.setSizeLock" => {
            let locked = flag(p, "locked")?.unwrap_or(!app.view.palette.size_locked);
            app.view.palette.size_locked = locked;
            Ok(())
        }
        "transform.setOrigin" => {
            app.view.palette.transform = set_origin(&app.view.palette.transform, p)?;
            Ok(())
        }
        "transform.setScaleLock" => {
            let t = &mut app.view.palette.transform;
            t.scale_locked = flag(p, "locked")?.unwrap_or(!t.scale_locked);
            Ok(())
        }
        "transform.setSettings" => {
            app.view.palette.transform = set_settings(&app.view.palette.transform, p)?;
            Ok(())
        }
        "fitCurve.setRange" => {
            let (min, max) = (number(p, "min")?, number(p, "max")?);
            if min.is_none() && max.is_none() {
                return Err("give `min`, `max` or both".into());
            }
            let [old_min, old_max] = fit_curve_range_of(&app.session, &app.view);
            let (low, high) = FIT_CURVE_PERCENT;
            let hold = |n: f64| n.max(low).min(high);
            app.view.palette.fit_curve_range = Some([hold(min.unwrap_or(old_min)), hold(max.unwrap_or(old_max))]);
            Ok(())
        }
        other => Err(format!("no palette command `{other}`")),
    }
}

/// `t` after `transform.setOrigin` with `p`; everything is checked before anything changes.
pub fn set_origin(t: &TransformSettings, p: &Value) -> Result<TransformSettings, String> {
    let mut s = t.clone();
    let placed = point(p, "point")?;
    if let Some(placed) = placed {
        s.point = Some(placed);
    }
    let cleared = p.get("point").is_some_and(Value::is_null);
    if cleared {
        s.point = None;
        if s.mode == OriginMode::Reference {
            s.mode = OriginMode::Box;
        }
    }
    let box_point = string(p, "boxPoint")?;
    if let Some(name) = box_point {
        BoxPoint::parse(name).ok_or_else(|| format!("`boxPoint` must be one of {}", BOX_POINTS.join(", ")))?;
        s.box_point = name.into();
    }
    let metric = string(p, "metric")?;
    if let Some(name) = metric {
        MetricPoint::parse(name).ok_or_else(|| format!("`metric` must be one of {}", METRICS.join(", ")))?;
        s.metric = name.into();
    }
    match string(p, "mode")? {
        None if placed.is_none() && !cleared && box_point.is_none() && metric.is_none() => {
            return Err("give `mode`, `point`, `boxPoint` or `metric`".into());
        }
        None => {}
        Some("box") => s.mode = OriginMode::Box,
        Some("reference") => {
            if s.point.is_none() {
                return Err("mode \"reference\" needs `point: [x, y]`: no reference point is placed yet".into());
            }
            s.mode = OriginMode::Reference;
        }
        Some("metrics") => s.mode = OriginMode::Metrics,
        Some("layer") => s.mode = OriginMode::Layer,
        Some(_) => return Err("`mode` must be box, reference, metrics or layer".into()),
    }
    Ok(s)
}

/// `t` after `transform.setSettings` with `p`; everything is checked before anything changes.
pub fn set_settings(t: &TransformSettings, p: &Value) -> Result<TransformSettings, String> {
    let (sx, sy, rotate, slant) = (number(p, "scaleX")?, number(p, "scaleY")?, number(p, "rotate")?, number(p, "slant")?);
    if [sx, sy, rotate, slant].iter().all(Option::is_none) {
        return Err("give `scaleX`, `scaleY`, `rotate` or `slant`".into());
    }
    if [sx, sy].iter().flatten().any(|s| *s == 0.0 || s.abs() > MAX_SCALE) {
        return Err(format!("a scale must be non-zero and within ±{MAX_SCALE} percent"));
    }
    if rotate.is_some_and(|r| r.abs() > MAX_ANGLE) {
        return Err(format!("`rotate` must be within ±{MAX_ANGLE} degrees"));
    }
    if slant.is_some_and(|s| s.abs() >= 90.0) {
        return Err("`slant` must lie between -90 and 90 degrees".into());
    }
    let mut s = t.clone();
    let [x, y] = s.scale;
    s.scale = [sx.unwrap_or(x), sy.unwrap_or(y)];
    s.rotate_angle = rotate.unwrap_or(s.rotate_angle);
    s.slant_angle = slant.unwrap_or(s.slant_angle);
    Ok(s)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{OriginMode, TransformSettings, set_origin, set_settings};

    /// The reference mode needs a placed point; a refused change leaves the settings as they were.
    #[test]
    fn the_origin_checks_before_it_changes() {
        let t = TransformSettings::default();
        assert!(set_origin(&t, &json!({"mode": "reference"})).is_err());
        let placed = set_origin(&t, &json!({"mode": "reference", "point": [10, 20]})).expect("placed");
        assert_eq!((placed.mode, placed.point), (OriginMode::Reference, Some([10.0, 20.0])));
        assert_eq!(placed.origin_param(), json!({"mode": "reference", "point": [10.0, 20.0]}));
        let cleared = set_origin(&placed, &json!({"point": null})).expect("cleared");
        assert_eq!((cleared.mode, cleared.point), (OriginMode::Box, None));
        assert!(set_origin(&placed, &json!({"mode": "reference", "point": null})).is_err());
        assert!(set_origin(&t, &json!({"boxPoint": "middle"})).is_err());
        assert!(set_settings(&t, &json!({"scaleX": 0})).is_err());
        assert!(set_settings(&t, &json!({"slant": 90})).is_err());
        assert_eq!(set_settings(&t, &json!({"scaleX": 50})).expect("scale").scale_percent(), (50.0, 50.0));
    }
}
