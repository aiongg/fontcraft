//! Animate Axis (`view.animateAxis`): the previews sweep back and forth along one axis, from the location they show.
//!
//! The sweep is view state ([`AxisSweep`] in `PreviewState::sweep`): a clock that [`logic`] moves once per frame and
//! that asks for the next frame only after this one. A step's line is made while its frame is projected
//! (`app::models::preview`), so there is never a step waiting behind another; a slow step skips the ones it took the
//! time of. While the window is hidden the clock stands still and asks for no frame. The sweep ends when no view
//! that follows it shows (the previews and the Glyph Sources panel) or its document is no longer the active one.

use craft::behavior::dock::PanelState;
use craft::ui::runtime::commands::CommandRef;
use fontcraft_engine::interp::{AxisInfo, Designspace};
use fontcraft_engine::model::Font;
use fontcraft_engine::model::axes::design_value;
use fontcraft_engine::prelude::*;

use super::commands::{Module, UiCommand};
use super::dock::PanelId;
use super::params::{flag, string};
use super::preview::{Shows, Target};
use crate::app::models::preview::{instance_index, is_static};
use crate::app::{FontCraftApp, ViewState};

pub const MODULE: Module = Module { commands: COMMANDS, run };

const COMMANDS: &[UiCommand] = &[UiCommand {
    id: "view.animateAxis",
    label: "Animate Axis",
    menu: &["View"],
    shortcut: None,
    params: "{axis?: tag (default: the axis under way, else the first with a range), on?: bool} sweep the previews back and forth along one axis, from the location they show (omitted `on`: toggle). Opens the Preview Area when no preview and no Glyph Sources panel shows; closing the last of them ends the sweep",
}];

/// Seconds from one end of the axis to the other.
const SWEEP_SECONDS: f64 = 2.0;
/// Positions along the axis the sweep stops at.
pub const STEPS: u32 = 60;
/// Seconds between two frames of the sweep.
const FRAME: f64 = 1.0 / 30.0;
/// The most one frame moves the clock: after a stall the sweep goes on from where it was.
const MAX_STEP: f64 = 0.25;

/// A sweep under way.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AxisSweep {
    /// The document it runs in.
    pub document: u64,
    /// The axis tag.
    pub axis: String,
    /// 0 to 1 from the axis minimum to its maximum, 1 to 2 on the way back.
    phase: f64,
    /// When the last frame moved the clock; `None` after standing still.
    last: Option<f64>,
}

impl AxisSweep {
    /// A sweep of `axis` in `document` that starts `position` (0 to 1) along the axis, rising.
    pub fn new(document: u64, axis: &str, position: f64) -> AxisSweep {
        let phase = if position.is_finite() { position.clamp(0.0, 1.0) } else { 0.0 };
        AxisSweep { document, axis: axis.to_string(), phase, last: None }
    }

    /// The step the sweep is at, 0 to [`STEPS`].
    pub fn step(&self) -> u32 {
        let t = if self.phase > 1.0 { 2.0 - self.phase } else { self.phase };
        // Saturates, and the phase is within 0..2.
        ((t * f64::from(STEPS)).round() as u32).min(STEPS)
    }

    /// Where the sweep is along the axis, 0 to 1.
    pub fn position(&self) -> f64 {
        f64::from(self.step()) / f64::from(STEPS)
    }

    /// Moves the clock to `now` (seconds).
    pub fn advance(&mut self, now: f64) {
        let passed = self.last.map_or(0.0, |last| now - last);
        self.last = Some(now);
        let passed = if passed.is_finite() { passed.clamp(0.0, MAX_STEP) } else { 0.0 };
        self.phase = (self.phase + passed / SWEEP_SECONDS).rem_euclid(2.0);
    }

    /// Lets the clock stand still until the next [`AxisSweep::advance`].
    pub fn pause(&mut self) {
        self.last = None;
    }
}

/// Whether the masters give `axis` a range. Its internal values can fall from minimum to maximum.
pub fn has_range(axis: &AxisInfo) -> bool {
    (axis.internal.maximum - axis.internal.minimum).abs() > f64::EPSILON
}

/// The location a preview that shows `shows` is at, one internal coordinate per font axis: its instance's, else the
/// current master's. While the sweep runs, its axis is where the sweep is.
pub fn location_of(font: &Font, designspace: &Designspace, view: &ViewState, shows: &Shows) -> Vec<f64> {
    let instance = match shows {
        Shows::Instance(key) => instance_index(font, key).filter(|i| is_static(font, *i)).and_then(|i| font.instances.get(i)),
        Shows::Master | Shows::All => None,
    };
    let base: &[f64] = match instance {
        Some(i) => &i.location,
        None => super::masters::current_master(font, &view.layers).map_or(&[], |m| &m.location),
    };
    let mut out: Vec<f64> = (0..font.axes.len()).map(|i| design_value(base, i)).collect();
    if let Some(sweep) = &view.preview.sweep
        && let Some((_, axis)) = designspace.axis(&sweep.axis)
        && let Some(slot) = out.get_mut(axis.font_axis)
    {
        *slot = axis.internal.minimum + sweep.position() * (axis.internal.maximum - axis.internal.minimum);
    }
    out
}

/// The location the previews show, for the views that follow them: the Preview Area's while it is open, else the
/// Preview Panel's.
pub fn previewed_location(font: &Font, designspace: &Designspace, view: &ViewState) -> Vec<f64> {
    let panel = view.workspace.layout.panel_state(&PanelId::Preview) != PanelState::Hidden;
    let target = if !view.preview.area_open && panel { Target::Panel } else { Target::Area };
    location_of(font, designspace, view, view.preview.shows(target))
}

/// Whether a view that follows the sweep shows.
fn followed(view: &ViewState) -> bool {
    let front = |panel| view.workspace.layout.panel_state(&panel) == PanelState::Front;
    (view.preview.area_open && !view.tabs.font_view_shows()) || front(PanelId::Preview) || front(PanelId::GlyphSources)
}

/// Moves the sweep once per frame and asks for the next frame.
pub fn logic(app: &mut FontCraftApp, ctx: &egui::Context) {
    let Some(sweep) = &app.view.preview.sweep else { return };
    let document = app.session.active().map(|d| d.id);
    let there = document == Some(sweep.document) && app.session.font().is_ok_and(|f| f.axes.iter().any(|a| a.tag == sweep.axis));
    if !there || !followed(&app.view) {
        app.view.preview.sweep = None;
        app.view_changed();
        return;
    }
    let (now, hidden) = ctx.input(|i| (i.time, i.viewport().visible() == Some(false)));
    let Some(sweep) = &mut app.view.preview.sweep else { return };
    if hidden {
        sweep.pause();
        return;
    }
    sweep.advance(now);
    ctx.request_repaint_after(std::time::Duration::from_secs_f64(FRAME));
}

fn run(app: &mut FontCraftApp, command: &CommandRef) -> Result<(), String> {
    match command.id.as_str() {
        "view.animateAxis" => animate(app, command),
        other => Err(format!("no axis command `{other}`")),
    }
}

fn animate(app: &mut FontCraftApp, command: &CommandRef) -> Result<(), String> {
    let given = string(&command.params, "axis")?.map(str::to_string);
    let running = app.view.preview.sweep.as_ref().map(|s| s.axis.clone());
    let same = running.is_some() && (given.is_none() || given == running);
    let on = flag(&command.params, "on")?.unwrap_or(!same);
    if !on {
        // Turning off another axis than the one under way changes nothing.
        if same {
            app.view.preview.sweep = None;
        }
        return Ok(());
    }
    if same {
        return Ok(());
    }
    let designspace = app.session.designspace()?;
    let axis = match &given {
        Some(tag) => {
            let (_, axis) = designspace.axis(tag).ok_or_else(|| format!("the font has no axis `{tag}`"))?;
            if !has_range(axis) {
                return Err(format!("the masters have no range along axis `{tag}`"));
            }
            axis
        }
        None => {
            let ranged = designspace.axes().iter().find(|a| has_range(a));
            ranged.ok_or("the font's masters have no range along any axis")?
        }
    };
    let document = app.session.active().map(|d| d.id).ok_or("no document open")?;
    let font = app.session.font().map_err(|e| e.to_string())?;
    let here = previewed_location(font, &designspace, &app.view).get(axis.font_axis).copied().unwrap_or(axis.internal.default);
    let position = (here - axis.internal.minimum) / (axis.internal.maximum - axis.internal.minimum);
    app.view.preview.sweep = Some(AxisSweep::new(document, &axis.tag, position));
    if !followed(&app.view) {
        if app.view.tabs.font_view_shows() {
            app.view.preview.sweep = None;
            return Err("open a glyph in an edit view or the Glyph Sources panel to see the animation".into());
        }
        app.view.preview.area_open = true;
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;

    /// The clock rises to the axis maximum, comes back, and a stall or a pause does not jump.
    #[test]
    fn the_sweep_runs_to_both_ends_and_stands_still_while_hidden() {
        let mut sweep = AxisSweep::new(1, "wght", 0.5);
        assert_eq!(sweep.step(), STEPS / 2);
        sweep.advance(10.0);
        assert_eq!(sweep.step(), STEPS / 2, "the first frame starts the clock");
        let mut seen = Vec::new();
        for frame in 1..=200 {
            sweep.advance(10.0 + f64::from(frame) * 0.02);
            seen.push(sweep.step());
        }
        assert!(seen.contains(&0) && seen.contains(&STEPS), "both ends");
        assert!(seen.windows(2).all(|w| matches!(w, [a, b] if a.abs_diff(*b) <= 1)), "one step at a time: {seen:?}");
        let before = sweep.step();
        sweep.pause();
        sweep.advance(10_000.0);
        assert_eq!(sweep.step(), before, "hidden time does not count");
        sweep.advance(f64::NAN);
        sweep.advance(20_000.0);
        assert!(sweep.step() <= STEPS);
    }
}
