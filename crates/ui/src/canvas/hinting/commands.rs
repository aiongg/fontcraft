//! The shell commands of hinting on the canvas: Show Hints, hint selection, the Instructor's direction, pixel size,
//! point numbers, pixel preview, rendering intent and instruction selection, and the Review Hints dialog.
//!
//! Rows: `view.showHints`, `hint.ps.selectNextHint`, `hint.ps.selectPreviousHint`, `hint.ps.reviewHints`,
//! `hint.tt.toggleDirection`, `hint.tt.increasePixelSize`, `hint.tt.decreasePixelSize`, `hint.tt.showPointIndexes`,
//! `hint.tt.selectInstructions`, `hint.tt.showPixelPreview`, `hint.tt.setPixelSize`, `hint.tt.setRenderingIntent`.
//!
//! The Instructor's keys (X, period, comma, plus, minus) reach the toolbox while the Instructor is active, which runs
//! these commands; they have no shortcut of their own, so the letters stay the tools' elsewhere.

use craft::ui::runtime::commands::CommandRef;
use fontcraft_engine::hinting::settings::main_master;
use fontcraft_engine::hinting::{MAX_LAYER_HINTS, PsHint, ps_hints, tt_instructions};
use fontcraft_engine::model::{Font, Layer};
use fontcraft_tools::tt_instructor::TtDirection;
use fontcraft_tools::{Item, Selection};
use serde_json::Value;

use super::{MAX_PPM, MIN_PPM, RenderingIntent, ReviewState, SelectedInstructions, clamp_ppm, hinting_direction, ps_marks};
use crate::app::FontCraftApp;
use crate::shell::commands::{Module, UiCommand};

pub const MODULE: Module = Module { commands: COMMANDS, run };

/// The `instance` value of `hint.tt.showPixelPreview` that previews the edited master.
pub const MASTER: &str = "master";

const COMMANDS: &[UiCommand] = &[
    UiCommand {
        id: "view.showHints",
        label: "Show Hints",
        menu: &["View"],
        shortcut: None,
        params: "{on?: bool} draw the edited layer's PostScript hints (bars, number badges, and the markers the Select tool drags); TrueType hints are not affected (omitted: toggle)",
    },
    UiCommand {
        id: "hint.ps.selectNextHint",
        label: "Select Next Hint",
        menu: &[],
        shortcut: None,
        params: "{} with PostScript hints selected and Show Hints on: select the hint after the last selected one in list order, the first after the last",
    },
    UiCommand {
        id: "hint.ps.selectPreviousHint",
        label: "Select Previous Hint",
        menu: &[],
        shortcut: None,
        params: "{} with PostScript hints selected and Show Hints on: select the hint before the first selected one in list order, the last before the first",
    },
    UiCommand {
        id: "hint.ps.reviewHints",
        label: "Review Hints…",
        menu: &[],
        shortcut: None,
        params: "{glyph?: name (default: the open dialog's glyph when `step`, `direction` or `hint` is given, else the edited glyph), layer?: id (default the main master's layer), direction?: \"horizontal\" | \"vertical\" (default the direction of `hint`, else the one reviewed before, else horizontal), hint?: index (in the layer's PostScript hint list), step?: \"next\" | \"previous\" (from the reviewed hint, within the direction), open?: bool (false closes the dialog)} opens the Review Hints dialog on the layer's PostScript hints of one direction, in list order, and turns on Show Hints. Without `hint` or `step` the first hint of the direction is reviewed; stepping past the first or the last is refused",
    },
    UiCommand {
        id: "hint.tt.toggleDirection",
        label: "Toggle Hinting Direction",
        menu: &[],
        shortcut: None,
        params: "{direction?: \"vertical\" | \"horizontal\" (omitted: the other one)} the TrueType Instructor's hinting direction: vertical instructions move points in y, horizontal ones in x. New instructions take it; the other direction's instructions are drawn dimmed and cannot be selected. A change clears the instruction selection",
    },
    UiCommand {
        id: "hint.tt.increasePixelSize",
        label: "Increase Pixel Size",
        menu: &[],
        shortcut: None,
        params: "{} raise the TrueType Instructor's preview pixel size by one, up to 400 PPM",
    },
    UiCommand {
        id: "hint.tt.decreasePixelSize",
        label: "Decrease Pixel Size",
        menu: &[],
        shortcut: None,
        params: "{} lower the TrueType Instructor's preview pixel size by one, down to 4 PPM",
    },
    UiCommand {
        id: "hint.tt.showPointIndexes",
        label: "Show Point Indexes",
        menu: &[],
        shortcut: None,
        params: "{on?: bool} label each on-curve node of the edited glyph with its point number in the exported TrueType outline (omitted: toggle)",
    },
    UiCommand {
        id: "hint.tt.selectInstructions",
        label: "Select Instructions",
        menu: &[],
        shortcut: None,
        params: "{instructions: [index], mode?: \"replace\" | \"add\" | \"remove\" | \"toggle\" (default replace)} select TrueType instructions of the edited glyph's layer by their position in its instruction list, as clicking their badges does. Instructions of the other direction cannot be selected; an unknown position or another direction changes nothing",
    },
    UiCommand {
        id: "hint.tt.showPixelPreview",
        label: "Show Pixel Preview",
        menu: &[],
        shortcut: None,
        params: "{on?: bool (omitted: toggle, or on when `instance` is given), instance?: name (an instance by name, `#n` for the instance at position n counted from 0, or \"master\" for the edited master)} show or hide the TrueType Instructor's blue outline (the previewed instance before hinting) and red outline (after its instructions ran, grid-fitted), and choose the previewed instance. The pixel backdrop stays",
    },
    UiCommand {
        id: "hint.tt.setPixelSize",
        label: "Set Preview Pixel Size",
        menu: &[],
        shortcut: None,
        params: "{ppm: whole number 4 to 400} the pixels per em of the TrueType Instructor's grid-fitted outline and pixel backdrop",
    },
    UiCommand {
        id: "hint.tt.setRenderingIntent",
        label: "Set Rendering Intent",
        menu: &[],
        shortcut: None,
        params: "{intent: \"grayscale\" | \"cleartype\" | \"directwrite\"} the rendering the TrueType Instructor's grid-fitted outline and pixel backdrop imitate",
    },
];

/// The parameter `key` as a flag; an error when it is there and not a bool.
fn flag(params: &Value, key: &str) -> Result<Option<bool>, String> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(*b)),
        Some(_) => Err(format!("`{key}` must be true or false")),
    }
}

/// The parameter `key` as text; an error when it is there and not text.
fn text<'a>(params: &'a Value, key: &str) -> Result<Option<&'a str>, String> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.as_str())),
        Some(_) => Err(format!("`{key}` must be text")),
    }
}

fn run(app: &mut FontCraftApp, command: &CommandRef) -> Result<(), String> {
    let params = &command.params;
    match command.id.as_str() {
        "view.showHints" => {
            let on = flag(params, "on")?.unwrap_or(!app.view.display.hints);
            app.view.display.hints = on;
            if !on {
                // Hidden hints cannot be acted on.
                app.tools.selection.hints.clear();
            }
            Ok(())
        }
        "hint.ps.selectNextHint" => step(app, true),
        "hint.ps.selectPreviousHint" => step(app, false),
        "hint.ps.reviewHints" => review(app, params),
        "hint.tt.toggleDirection" => {
            let s = &mut app.view.hinting.instructor;
            let direction = match text(params, "direction")? {
                Some(name) => TtDirection::parse(name).ok_or("`direction` must be \"vertical\" or \"horizontal\"")?,
                None => s.direction.other(),
            };
            if direction != s.direction {
                s.selected.instructions.clear();
            }
            s.direction = direction;
            Ok(())
        }
        "hint.tt.increasePixelSize" | "hint.tt.decreasePixelSize" => {
            let s = &mut app.view.hinting.instructor;
            let step = if command.id == "hint.tt.increasePixelSize" { 1 } else { -1 };
            s.ppm = clamp_ppm(i64::from(s.ppm) + step);
            Ok(())
        }
        "hint.tt.showPointIndexes" => {
            let s = &mut app.view.hinting.instructor;
            s.point_indexes = flag(params, "on")?.unwrap_or(!s.point_indexes);
            Ok(())
        }
        "hint.tt.selectInstructions" => select_instructions(app, params),
        "hint.tt.showPixelPreview" => {
            let given = text(params, "instance")?;
            let instance = match given {
                None => app.view.hinting.instructor.instance.clone(),
                Some(MASTER) => None,
                Some(name) => {
                    let font = app.session.font().map_err(|e| e.to_string())?;
                    super::pixels::instance_index(font, name).ok_or_else(|| format!("the font has no instance `{name}`"))?;
                    Some(name.to_string())
                }
            };
            let s = &mut app.view.hinting.instructor;
            s.pixel_preview = match (flag(params, "on")?, given) {
                (Some(on), _) => on,
                (None, Some(_)) => true,
                (None, None) => !s.pixel_preview,
            };
            s.instance = instance;
            Ok(())
        }
        "hint.tt.setPixelSize" => {
            let ppm = params.get("ppm").and_then(Value::as_f64).ok_or("`ppm` is required")?;
            if ppm.fract() != 0.0 || ppm < f64::from(MIN_PPM) || ppm > f64::from(MAX_PPM) {
                return Err(format!("`ppm`: {ppm} is not a whole pixel size from {MIN_PPM} to {MAX_PPM}"));
            }
            app.view.hinting.instructor.ppm = clamp_ppm(ppm as i64);
            Ok(())
        }
        "hint.tt.setRenderingIntent" => {
            let name = text(params, "intent")?.ok_or("`intent` is required")?;
            let intent = RenderingIntent::parse(name)
                .ok_or_else(|| format!("unknown intent `{name}`; the intents are grayscale, cleartype and directwrite"))?;
            app.view.hinting.instructor.intent = intent;
            Ok(())
        }
        other => Err(format!("no hinting command `{other}`")),
    }
}

/// `hint.ps.selectNextHint` and `hint.ps.selectPreviousHint`.
fn step(app: &mut FontCraftApp, forward: bool) -> Result<(), String> {
    crate::canvas::tools::apply::sync(app);
    let marks = ps_marks(&mut app.session, &app.view);
    if !marks.shown {
        return Err("turn on View › Show Hints and edit a glyph first".into());
    }
    let selected = marks.selected(&app.tools.selection);
    let from = if forward { selected.last() } else { selected.first() };
    let from = *from.ok_or("select a hint first")?;
    let count = marks.hints.len();
    let next = if forward { from.saturating_add(1).checked_rem(count).unwrap_or(0) } else { from.checked_sub(1).unwrap_or(count.saturating_sub(1)) };
    app.tools.selection = Selection::from_items([Item::Hint(next)]);
    Ok(())
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Replace,
    Add,
    Remove,
    Toggle,
}

fn select_instructions(app: &mut FontCraftApp, params: &Value) -> Result<(), String> {
    let Some(Value::Array(given)) = params.get("instructions") else {
        return Err("`instructions` must be a list of positions in the instruction list".into());
    };
    if given.len() > MAX_LAYER_HINTS {
        return Err(format!("`instructions` names more than {MAX_LAYER_HINTS} instructions"));
    }
    let wanted: Vec<usize> = given
        .iter()
        .map(|v| v.as_u64().and_then(|n| usize::try_from(n).ok()))
        .collect::<Option<_>>()
        .ok_or("`instructions` must hold whole numbers")?;
    let mode = match text(params, "mode")? {
        None | Some("replace") => Mode::Replace,
        Some("add") => Mode::Add,
        Some("remove") => Mode::Remove,
        Some("toggle") => Mode::Toggle,
        Some(other) => return Err(format!("unknown mode `{other}`; the modes are replace, add, remove and toggle")),
    };
    if app.view.edit_background {
        return Err("instructions belong to the foreground; the background is being edited".into());
    }
    let target = crate::canvas::tools::target(&app.session, &app.view).ok_or("no glyph is being edited")?;
    let font = app.session.font().map_err(|e| e.to_string())?;
    let layer = font.glyph(&target.glyph).and_then(|g| g.layers.iter().find(|l| l.id == target.layer)).ok_or("the edited layer is gone")?;
    let list = tt_instructions(layer).map_err(|e| e.to_string())?;
    let direction = app.view.hinting.instructor.direction;
    for &i in &wanted {
        let Some((_, t)) = list.get(i) else {
            return Err(format!("the layer has no instruction {i} (it has {})", list.len()));
        };
        if t.direction != hinting_direction(direction) {
            return Err(format!("instruction {i} is {}: switch the direction to select it", direction.other().as_str()));
        }
    }
    let count = list.len();
    let state = &mut app.view.hinting.instructor;
    let mut current = state.selected_in(&target.glyph, &target.layer);
    current.retain(|&i| i < count);
    match mode {
        Mode::Replace => current = wanted.into_iter().collect(),
        Mode::Add => current.extend(wanted),
        Mode::Remove => current.retain(|i| !wanted.contains(i)),
        Mode::Toggle => {
            for i in wanted {
                if !current.remove(&i) {
                    current.insert(i);
                }
            }
        }
    }
    state.selected = SelectedInstructions { glyph: target.glyph, layer: target.layer, instructions: current };
    Ok(())
}

/// The layer `layer` of `glyph`.
pub fn layer<'f>(font: &'f Font, glyph: &str, layer: &str) -> Result<&'f Layer, String> {
    let g = font.glyph(glyph).ok_or_else(|| format!("there is no glyph `{glyph}`"))?;
    g.layers.iter().find(|l| l.id == layer).ok_or_else(|| format!("glyph `{glyph}` has no layer `{layer}`"))
}

/// The layer's hints of one direction, with their positions in the hint list.
pub fn hints_of(layer: &Layer, horizontal: bool) -> Result<Vec<(usize, PsHint)>, String> {
    Ok(ps_hints(layer).map_err(|e| e.to_string())?.into_iter().filter(|(_, h)| h.horizontal == horizontal).collect())
}

/// The hint after (`forward`) or before `from` among `list`, by list position.
pub fn neighbour(list: &[(usize, PsHint)], from: Option<usize>, forward: bool) -> Option<usize> {
    let from = from?;
    if forward { list.iter().map(|(i, _)| *i).find(|i| *i > from) } else { list.iter().rev().map(|(i, _)| *i).find(|i| *i < from) }
}

fn direction_name(horizontal: bool) -> &'static str {
    if horizontal { "horizontal" } else { "vertical" }
}

/// `hint.ps.reviewHints`.
fn review(app: &mut FontCraftApp, params: &Value) -> Result<(), String> {
    if flag(params, "open")? == Some(false) {
        app.view.hinting.review.open = false;
        return Ok(());
    }
    let previous = app.view.hinting.review.clone();
    let edited = crate::canvas::tools::target(&app.session, &app.view).map(|t| t.glyph);
    let font = app.session.font().map_err(|e| e.to_string())?;
    let glyph = match text(params, "glyph")? {
        Some(g) => g.to_string(),
        None if previous.open && ["step", "direction", "hint"].iter().any(|k| params.get(k).is_some()) => previous.glyph.clone(),
        None => edited.ok_or("no glyph is being edited: give `glyph`")?,
    };
    let layer_id = match text(params, "layer")? {
        Some(l) => l.to_string(),
        None if previous.open && previous.glyph == glyph => previous.layer.clone(),
        None => main_master(font).to_string(),
    };
    let l = layer(font, &glyph, &layer_id)?;
    let all = ps_hints(l).map_err(|e| e.to_string())?;
    let given = match params.get("hint") {
        None | Some(Value::Null) => None,
        Some(v) => {
            let i = v.as_u64().and_then(|i| usize::try_from(i).ok()).ok_or("`hint` must be a position in the hint list")?;
            Some(all.iter().find(|(n, _)| *n == i).ok_or_else(|| format!("the layer has no PostScript hint {i}"))?)
        }
    };
    let horizontal = match text(params, "direction")? {
        Some("horizontal") => true,
        Some("vertical") => false,
        Some(other) => return Err(format!("`direction` must be \"horizontal\" or \"vertical\", got \"{other}\"")),
        None => match given {
            Some((_, h)) => h.horizontal,
            None if previous.open && previous.glyph == glyph => previous.horizontal,
            None => true,
        },
    };
    if let Some((i, h)) = given
        && h.horizontal != horizontal
    {
        return Err(format!("hint {i} is not a {} hint", direction_name(horizontal)));
    }
    let list = hints_of(l, horizontal)?;
    let same_place = previous.open && previous.glyph == glyph && previous.layer == layer_id && previous.horizontal == horizontal;
    let hint = match (text(params, "step")?, given) {
        (Some(step @ ("next" | "previous")), _) => {
            let from = if same_place { previous.hint } else { None };
            let from = from.filter(|f| list.iter().any(|(i, _)| i == f)).ok_or("no hint is reviewed to step from")?;
            let forward = step == "next";
            let next = neighbour(&list, Some(from), forward);
            Some(next.ok_or_else(|| format!("this is the {} {} hint", if forward { "last" } else { "first" }, direction_name(horizontal)))?)
        }
        (Some(other), _) => return Err(format!("`step` must be \"next\" or \"previous\", got \"{other}\"")),
        (None, Some((i, _))) => Some(*i),
        (None, None) => list.first().map(|(i, _)| *i),
    };
    let target = crate::canvas::tools::target(&app.session, &app.view);
    app.view.hinting.review = ReviewState { open: true, glyph, layer: layer_id, horizontal, hint };
    app.view.display.hints = true;
    // The canvas marks the reviewed hint when it edits the reviewed layer.
    let review = &app.view.hinting.review;
    if let (Some(hint), Some(target)) = (hint, target)
        && target.glyph == review.glyph
        && target.layer == review.layer
    {
        app.tools.selection = Selection::from_items([Item::Hint(hint)]);
    }
    Ok(())
}
