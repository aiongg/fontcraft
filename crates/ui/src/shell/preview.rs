//! The commands of both previews: the Preview Area under the edit view and the Preview Panel (`PanelId::Preview`).
//! Their settings are view state ([`PreviewState`] in `ViewState::preview`); `app::models::preview` projects the
//! lines they draw and `panels::preview` shows them.

use std::collections::BTreeMap;

use craft::behavior::dock::PanelState;
use craft::ui::contract::PlainData;
use craft::ui::runtime::commands::CommandRef;
use fontcraft_engine::cmd::preview::MAX_BLUR;
use serde_json::{Map, Value};

use super::commands::{Module, UiCommand};
use super::dock::PanelId;
use crate::app::FontCraftApp;
use crate::app::models::preview::{instance_index, instance_key, is_static};

pub const MODULE: Module = Module { commands: COMMANDS, run };

/// The tallest Preview Area, in screen points.
pub const MAX_AREA_HEIGHT: f64 = 4000.0;
/// The largest offset, in screen points either way.
pub const MAX_OFFSET: f64 = 1.0e6;

/// One preview's options.
#[derive(PlainData, Clone, Copy, Debug, PartialEq)]
pub struct PreviewOptions {
    /// Light text on a dark ground (`preview.invert`).
    pub invert: bool,
    /// Upside down (`preview.flip`).
    pub flip: bool,
    /// Blur in screen points, 0 to [`MAX_BLUR`] (`preview.setBlur`).
    pub blur: f64,
    /// Keep the edited glyph in the middle (`preview.alwaysCenterActiveGlyph`).
    pub always_center: bool,
    /// The sideways shift in screen points (`preview.setOffset`).
    pub offset: f64,
}

impl Default for PreviewOptions {
    fn default() -> Self {
        Self { invert: false, flip: false, blur: 0.0, always_center: true, offset: 0.0 }
    }
}

impl PreviewOptions {
    pub fn blur(&self) -> f64 {
        if self.blur.is_finite() { self.blur.clamp(0.0, MAX_BLUR) } else { 0.0 }
    }

    pub fn offset(&self) -> f64 {
        if self.offset.is_finite() { self.offset.clamp(-MAX_OFFSET, MAX_OFFSET) } else { 0.0 }
    }
}

/// What a preview's line shows: the choice in its instance pop-up.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub enum Shows {
    /// The current master (the dash).
    #[default]
    Master,
    /// One instance, by its name, or `#n` for the instance at position n when the name does not find it.
    Instance(String),
    /// The edited glyph once for each visible static instance.
    All,
    /// The location the sliders set (`view.setLocation`).
    Location,
}

/// Which preview a command changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Target {
    Area,
    Panel,
}

impl Target {
    pub fn name(self) -> &'static str {
        match self {
            Target::Area => "area",
            Target::Panel => "panel",
        }
    }
}

/// Both previews' settings.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PreviewState {
    /// The Preview Area shows under the edit view.
    pub area_open: bool,
    /// The area's height in screen points; `None` for the standard one.
    pub area_height: Option<f64>,
    pub area: PreviewOptions,
    pub panel: PreviewOptions,
    pub area_shows: Shows,
    pub panel_shows: Shows,
    /// The axis both previews sweep along (`view.animateAxis`); `None` while none is animated.
    pub sweep: Option<super::axis_sweep::AxisSweep>,
    /// The location [`Shows::Location`] draws: internal coordinates by axis tag. An axis without one is at its origin.
    pub location: BTreeMap<String, f64>,
}

impl PreviewState {
    pub fn shows(&self, target: Target) -> &Shows {
        match target {
            Target::Area => &self.area_shows,
            Target::Panel => &self.panel_shows,
        }
    }

    pub(super) fn shows_mut(&mut self, target: Target) -> &mut Shows {
        match target {
            Target::Area => &mut self.area_shows,
            Target::Panel => &mut self.panel_shows,
        }
    }

    pub fn options(&self, target: Target) -> &PreviewOptions {
        match target {
            Target::Area => &self.area,
            Target::Panel => &self.panel,
        }
    }

    fn options_mut(&mut self, target: Target) -> &mut PreviewOptions {
        match target {
            Target::Area => &mut self.area,
            Target::Panel => &mut self.panel,
        }
    }

    /// The area's height: the one set, else `standard`, held between `least` and [`MAX_AREA_HEIGHT`].
    pub fn area_height(&self, standard: f64, least: f64) -> f64 {
        self.area_height.filter(|h| h.is_finite()).unwrap_or(standard).min(MAX_AREA_HEIGHT).max(least)
    }
}

const fn command(
    id: &'static str,
    label: &'static str,
    menu: &'static [&'static str],
    shortcut: Option<&'static str>,
    params: &'static str,
) -> UiCommand {
    UiCommand { id, label, menu, shortcut, params }
}

const COMMANDS: &[UiCommand] = &[
    command(
        "view.togglePreviewArea",
        "Preview",
        &[],
        None,
        "{on?: bool, height?: points} show or hide the Preview Area under the edit view (omitted `on`: toggle); `height` sets the area's height, held to the smallest the theme allows and 4000",
    ),
    command(
        "window.previewPanel",
        "Preview Panel",
        &["Window"],
        None,
        "{on?: bool} show or hide the Preview Panel, which previews the edit view's text with its own settings (omitted: toggle)",
    ),
    command(
        "preview.invert",
        "Invert",
        &[],
        None,
        "{target?: \"area\" | \"panel\" (default area), on?: bool} light text on a dark ground instead of dark on light (omitted: toggle)",
    ),
    command(
        "preview.flip",
        "Flip",
        &[],
        None,
        "{target?: \"area\" | \"panel\" (default area), on?: bool} draw the preview upside down (omitted: toggle)",
    ),
    command(
        "preview.setBlur",
        "Blur",
        &[],
        None,
        "{target?: \"area\" | \"panel\" (default area), amount: points} blur the preview by `amount` screen points, held to 0..10; 0 is sharp",
    ),
    command(
        "preview.alwaysCenterActiveGlyph",
        "Always Center Active Glyph",
        &[],
        None,
        "{target?: \"area\" | \"panel\" (default area), on?: bool} on: the edited glyph stays in the middle of the preview; off: the text keeps flush with the left edge (right edge for right-to-left text) where it fits, and fills the width where it does not (omitted: toggle)",
    ),
    command(
        "preview.setOffset",
        "Scroll Preview",
        &[],
        None,
        "{target?: \"area\" | \"panel\" (default area), x?: points, dx?: points} shift the preview line sideways to `x` screen points from where it would sit, or by `dx` more",
    ),
    command(
        "preview.setInstance",
        "Preview Instance",
        &[],
        None,
        "{target?: \"area\" | \"panel\" (default area), instance: name | index | \"master\" | \"all\" | \"location\"} choose what the preview draws: a static instance (by name, `#n` or index from 0), interpolated as its static instance would be; \"master\" (the dash) for the current master, following each master switch; \"all\" for the edited glyph once per visible instance; \"location\" for the location view.setLocation sets. An instance of one of these names is given by index",
    ),
    command(
        "preview.showAllInstances",
        "Show All Instances",
        &[],
        None,
        "{target?: \"area\" | \"panel\" (default area)} draw the edited glyph once for every static instance whose eye is on (preview.toggleInstanceVisibility), side by side, instead of the text",
    ),
];

fn run(app: &mut FontCraftApp, command: &CommandRef) -> Result<(), String> {
    let empty = Map::new();
    let p = command.params.as_object().unwrap_or(&empty);
    match command.id.as_str() {
        "view.togglePreviewArea" => {
            let height = number(p, "height")?;
            let preview = &mut app.view.preview;
            preview.area_open = flag(p, "on")?.unwrap_or(!preview.area_open);
            if let Some(h) = height {
                let tokens = crate::panels::preview::PreviewTokens::standard();
                preview.area_height = Some(h);
                preview.area_height = Some(preview.area_height(tokens.area_height, tokens.area_min_height));
            }
            Ok(())
        }
        "window.previewPanel" => {
            let workspace = &mut app.view.workspace;
            let shown = workspace.layout().panel_state(&PanelId::Preview) != PanelState::Hidden;
            let on = flag(p, "on")?.unwrap_or(!shown);
            if on != shown {
                workspace.set_visible(PanelId::Preview, on, None, false, |p| p.permissions()).map_err(|e| format!("{e:?}"))?;
            }
            Ok(())
        }
        "preview.invert" | "preview.flip" | "preview.alwaysCenterActiveGlyph" => {
            let target = target(p)?;
            let options = app.view.preview.options_mut(target);
            let slot = match command.id.as_str() {
                "preview.invert" => &mut options.invert,
                "preview.flip" => &mut options.flip,
                _ => &mut options.always_center,
            };
            *slot = flag(p, "on")?.unwrap_or(!*slot);
            Ok(())
        }
        "preview.setBlur" => {
            let target = target(p)?;
            let amount = number(p, "amount")?.ok_or("give `amount` (screen points, 0 to 10)")?.clamp(0.0, MAX_BLUR);
            app.view.preview.options_mut(target).blur = amount;
            Ok(())
        }
        "preview.setOffset" => {
            let target = target(p)?;
            let options = app.view.preview.options_mut(target);
            let x = match (number(p, "x")?, number(p, "dx")?) {
                (Some(_), Some(_)) => return Err("give `x` or `dx`, not both".into()),
                (Some(x), None) => x,
                (None, Some(dx)) => options.offset() + dx,
                (None, None) => return Err("give `x` or `dx` (screen points)".into()),
            };
            options.offset = x.clamp(-MAX_OFFSET, MAX_OFFSET);
            Ok(())
        }
        "preview.setInstance" => {
            let target = target(p)?;
            let font = app.session.font().map_err(|e| e.to_string())?;
            let given = p.get("instance").ok_or("give `instance`: a name, an index, \"master\", \"all\" or \"location\"")?;
            let index = match given {
                Value::String(s) if s == "master" || s == "all" || s == "location" => None,
                Value::String(s) => Some(instance_index(font, s).ok_or_else(|| format!("the font has no instance `{s}`"))?),
                Value::Number(n) => Some(
                    n.as_u64()
                        .and_then(|i| usize::try_from(i).ok())
                        .filter(|i| *i < font.instances.len())
                        .ok_or_else(|| format!("the font has {} instances; there is no instance {n}", font.instances.len()))?,
                ),
                _ => return Err("`instance` must be a name, an index, \"master\", \"all\" or \"location\"".into()),
            };
            let shows = match (index, given.as_str()) {
                (Some(i), _) if !is_static(font, i) => {
                    return Err(format!("`{}` is a variable font setting; the preview shows static instances", instance_key(font, i)));
                }
                (Some(i), _) => Shows::Instance(instance_key(font, i)),
                (None, Some("all")) => Shows::All,
                (None, Some("location")) => Shows::Location,
                (None, _) => Shows::Master,
            };
            *app.view.preview.shows_mut(target) = shows;
            Ok(())
        }
        "preview.showAllInstances" => {
            let target = target(p)?;
            *app.view.preview.shows_mut(target) = Shows::All;
            Ok(())
        }
        other => Err(format!("no preview command `{other}`")),
    }
}

fn target(p: &Map<String, Value>) -> Result<Target, String> {
    match p.get("target") {
        None | Some(Value::Null) => Ok(Target::Area),
        Some(Value::String(s)) if s == "area" => Ok(Target::Area),
        Some(Value::String(s)) if s == "panel" => Ok(Target::Panel),
        Some(other) => Err(format!("`target` must be \"area\" or \"panel\", got {other}")),
    }
}

fn flag(p: &Map<String, Value>, key: &str) -> Result<Option<bool>, String> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(_) => Err(format!("`{key}` must be true or false")),
    }
}

fn number(p: &Map<String, Value>, key: &str) -> Result<Option<f64>, String> {
    match p.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_f64().filter(|n| n.is_finite()).map(Some).ok_or_else(|| format!("`{key}` must be a number")),
    }
}
