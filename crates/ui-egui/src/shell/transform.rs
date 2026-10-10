//! The Transformations section's settings: the origin, the two scale fields with their lock, and
//! the rotate and slant angles.
//!
//! The settings are UI state. Engine transform commands receive the origin as a parameter each
//! time ([`TransformSettings::origin_param`]); the Rotate and Scale tools place the reference
//! point with `transform.setOrigin {point}` (`docs/spec/04-palette.md` › Transformations).

use fontcraft_engine::cmd::transform::{BoxPoint, MetricPoint};
use fontcraft_geom::Point;
use fontcraft_model::MAX_COORDINATE;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{Args, UiCommand};
use crate::app::FontcraftApp;

pub const COMMANDS: &[UiCommand] = &[
    (
        "transform.setOrigin",
        "Set Transformation Origin",
        &[],
        "",
        "{mode?: \"box\" | \"reference\" | \"metrics\" | \"layer\", boxPoint?: \"topLeft\" | \"top\" | \"topRight\" | \"left\" | \"center\" | \"right\" | \"bottomLeft\" | \"bottom\" | \"bottomRight\", metric?: \"baseline\" | \"halfXHeight\" | \"xHeight\" | \"halfCapHeight\" | \"capHeight\", point?: [x, y]} choose the fixed point of the Transformations section. `point` places the reference point (what a click with the Rotate or Scale tool does); without `mode` the mode stays. `boxPoint` and `metric` are kept for their modes → the settings, as transform.getSettings returns them",
    ),
    (
        "transform.setScaleLock",
        "Lock Scale Proportions",
        &[],
        "",
        "{locked?: bool} lock the vertical scale field to the horizontal one (omitted: toggle) → {locked}",
    ),
    (
        "transform.getSettings",
        "Get Transformation Settings",
        &[],
        "",
        "{} the Transformations section's settings → {origin: {mode, boxPoint, metric, point}, scale: {x, y, locked}, rotateAngle, slantAngle}",
    ),
    (
        "transform.setSettings",
        "Set Transformation Values",
        &[],
        "",
        "{scaleX?: percent (non-zero, within ±1000000), scaleY?: percent, rotate?: degrees (within ±3600), slant?: degrees (between -90 and 90)} the values of the Transformations fields → the settings, as transform.getSettings returns them",
    ),
];

/// Largest scale percentage and rotation angle the Transformations fields hold.
const MAX_SCALE: f64 = 1_000_000.0;
const MAX_ANGLE: f64 = 3_600.0;

/// Which fixed point Palette transformations use.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum OriginMode {
    /// One of nine points of the selection's box.
    #[default]
    Box,
    /// The point placed with the Rotate or Scale tool.
    Reference,
    /// A metric height of the master, at half the advance width.
    Metrics,
    /// The centre of the layer (CJK glyphs).
    Layer,
}

impl OriginMode {
    pub fn name(self) -> &'static str {
        match self {
            OriginMode::Box => "box",
            OriginMode::Reference => "reference",
            OriginMode::Metrics => "metrics",
            OriginMode::Layer => "layer",
        }
    }
}

/// The Transformations section's state.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
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
        Self {
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
    /// The reference point, when one is placed.
    pub fn reference(&self) -> Option<Point> {
        self.point.map(|[x, y]| Point::new(x, y))
    }

    /// The `origin` parameter of the engine's transform commands for these settings. The
    /// reference mode without a placed point falls back to the box centre.
    pub fn origin_param(&self) -> Value {
        match (self.mode, self.point) {
            (OriginMode::Box, _) => json!({"mode": "box", "boxPoint": self.box_point}),
            (OriginMode::Reference, None) => json!({"mode": "box", "boxPoint": "center"}),
            (OriginMode::Reference, Some(p)) => json!({"mode": "reference", "point": p}),
            (OriginMode::Metrics, _) => json!({"mode": "metrics", "metric": self.metric}),
            (OriginMode::Layer, _) => json!({"mode": "layer"}),
        }
    }

    /// The horizontal and vertical percentages a Scale button applies: with the lock closed, the
    /// horizontal field for both.
    pub fn scale_percent(&self) -> (f64, f64) {
        let [x, y] = self.scale;
        if self.scale_locked { (x, x) } else { (x, y) }
    }

    /// What `transform.getSettings` returns.
    pub fn to_json(&self) -> Value {
        let [x, y] = self.scale;
        json!({
            "origin": {"mode": self.mode.name(), "boxPoint": self.box_point, "metric": self.metric, "point": self.point},
            "scale": {"x": x, "y": y, "locked": self.scale_locked},
            "rotateAngle": self.rotate_angle,
            "slantAngle": self.slant_angle,
        })
    }
}

pub fn run(app: &mut FontcraftApp, id: &str, args: &Args) -> Result<Value, String> {
    match id {
        "transform.setOrigin" => {
            let mut s = app.state.transform.clone();
            if let Some(p) = args.numbers::<2>("point")? {
                if p.iter().any(|v| v.abs() > MAX_COORDINATE) {
                    return Err(args.err(format!("`point` must lie within ±{MAX_COORDINATE}")));
                }
                s.point = Some(p);
            }
            if let Some(name) = args.string("boxPoint")? {
                BoxPoint::parse(name).ok_or_else(|| {
                    args.err(format!("`boxPoint`: \"{name}\" is not topLeft, top, topRight, left, center, right, bottomLeft, bottom or bottomRight"))
                })?;
                s.box_point = name.into();
            }
            if let Some(name) = args.string("metric")? {
                MetricPoint::parse(name)
                    .ok_or_else(|| args.err(format!("`metric`: \"{name}\" is not baseline, halfXHeight, xHeight, halfCapHeight or capHeight")))?;
                s.metric = name.into();
            }
            match args.string("mode")? {
                None if args.get("point").is_none() && args.get("boxPoint").is_none() && args.get("metric").is_none() => {
                    return Err(args.err("give `mode`, `point`, `boxPoint` or `metric`"));
                }
                None => {}
                Some("box") => s.mode = OriginMode::Box,
                Some("reference") => {
                    if s.point.is_none() {
                        return Err(args.err("mode \"reference\" needs `point: [x, y]`: no reference point is placed yet"));
                    }
                    s.mode = OriginMode::Reference;
                }
                Some("metrics") => s.mode = OriginMode::Metrics,
                Some("layer") => s.mode = OriginMode::Layer,
                Some(other) => return Err(args.err(format!("`mode` must be box, reference, metrics or layer, got \"{other}\""))),
            }
            app.state.transform = s;
            Ok(app.state.transform.to_json())
        }
        "transform.setScaleLock" => {
            let locked = args.flag("locked")?.unwrap_or(!app.state.transform.scale_locked);
            app.state.transform.scale_locked = locked;
            Ok(json!({"locked": locked}))
        }
        "transform.setSettings" => {
            let (sx, sy, rotate, slant) = (args.number("scaleX")?, args.number("scaleY")?, args.number("rotate")?, args.number("slant")?);
            if [sx, sy, rotate, slant].iter().all(Option::is_none) {
                return Err(args.err("give `scaleX`, `scaleY`, `rotate` or `slant`"));
            }
            if [sx, sy].iter().flatten().any(|s| *s == 0.0 || s.abs() > MAX_SCALE) {
                return Err(args.err(format!("a scale must be non-zero and within ±{MAX_SCALE} percent")));
            }
            if rotate.is_some_and(|r| r.abs() > MAX_ANGLE) {
                return Err(args.err(format!("`rotate` must be within ±{MAX_ANGLE} degrees")));
            }
            if slant.is_some_and(|s| s.abs() >= 90.0) {
                return Err(args.err("`slant` must lie between -90 and 90 degrees"));
            }
            let t = &mut app.state.transform;
            let [x, y] = t.scale;
            t.scale = [sx.unwrap_or(x), sy.unwrap_or(y)];
            t.rotate_angle = rotate.unwrap_or(t.rotate_angle);
            t.slant_angle = slant.unwrap_or(t.slant_angle);
            Ok(t.to_json())
        }
        "transform.getSettings" => Ok(app.state.transform.to_json()),
        other => Err(format!("unknown command `{other}`")),
    }
}
