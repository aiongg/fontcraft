//! The Attributes panel's view model: what the selected paths of a full-colour layer have in common.
//!
//! It reads the canvas's selection, which moves without the engine or the view state, so [`project`] runs on every
//! projection and skips the work while the stamp and the selection stay.

use craft::ui::contract::{PlainData, Tracked};
use fontcraft_engine::color::palette::{self as color_palette, Palettes};
use fontcraft_engine::model::{Color, Dimension, GradientKind, Shape, ShapeAttrs};
use fontcraft_tools::Selection;
use serde_json::{Value, json};

use super::palette::Targets;
use super::{ProjectCx, Stamp};
use crate::canvas::color::knobs::selected_paths;
use crate::canvas::tools::{Tools, scene, targets};

/// A colour as four unmultiplied bytes: red, green, blue, alpha.
pub type Rgba8 = [u8; 4];

/// The palette row of the text colour.
pub const FOREGROUND: u16 = 0xFFFF;
/// The text colour on the light canvas.
const TEXT: Rgba8 = [0, 0, 0, 255];

/// One shadow as the panel lists it: blur and offset.
#[derive(PlainData, Clone, Copy, Debug, Default, PartialEq)]
pub struct ShadowLine {
    pub blur: f64,
    pub x: f64,
    pub y: f64,
}

/// The Attributes panel. A value is `None` when the selected paths differ.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct AttributesModel {
    /// The targets of the `attributes.*` commands with the selected paths as `shapes`; `None` while the edited layer is
    /// not a full-colour layer or no path is selected.
    pub targets: Option<Targets>,
    pub stroke_width: Option<f64>,
    /// `center`, `inside` or `outside`.
    pub alignment: Option<String>,
    pub stroke_color: Option<Rgba8>,
    pub fill_color: Option<Rgba8>,
    /// Some selected path has a gradient: the fill row hides.
    pub any_gradient: bool,
    /// `linear`, `radial` or `conic`.
    pub gradient_type: Option<String>,
    /// The first gradient's stops: offset from 0 to 1 and colour.
    pub stops: Vec<(f64, Rgba8)>,
    /// The first path's shadow and inner shadow.
    pub shadow: Option<ShadowLine>,
    pub inner_shadow: Option<ShadowLine>,
}

/// The stroke alignments and their stored positions.
pub const ALIGNMENTS: [(&str, f64); 3] = [("center", 0.0), ("inside", -1.0), ("outside", 1.0)];

/// A component from 0 to 1 as a byte; anything not finite is 0.
fn byte(v: f64) -> u8 {
    if v.is_finite() { (v.clamp(0.0, 1.0) * 255.0).round() as u8 } else { 0 }
}

/// A model colour as the swatches show it: a palette entry is looked up in palette 0 of `palettes`, its alpha
/// multiplied in; the text colour is black. `None` for a palette row the palettes do not have.
pub fn resolve(color: &Color, palettes: &Palettes) -> Option<Rgba8> {
    match color {
        Color::Rgba([r, g, b, a]) => Some([byte(*r), byte(*g), byte(*b), byte(*a)]),
        Color::Palette { index, alpha } => {
            let [r, g, b, a] = if *index == FOREGROUND {
                TEXT
            } else {
                let entry = palettes.get(0, usize::from(*index))?;
                [entry.r, entry.g, entry.b, entry.a]
            };
            let alpha = if alpha.is_finite() { alpha.clamp(0.0, 1.0) } else { 0.0 };
            Some([r, g, b, (f64::from(a) * alpha).round() as u8])
        }
    }
}

/// `#RRGGBBAA`.
pub fn hex([r, g, b, a]: Rgba8) -> String {
    format!("#{r:02X}{g:02X}{b:02X}{a:02X}")
}

/// `#RRGGBB` or `#RRGGBBAA`, either case.
pub fn parse_hex(text: &str) -> Option<Rgba8> {
    color_palette::Rgba::parse_hex(text.trim()).map(|c| [c.r, c.g, c.b, c.a])
}

/// `get` of every item when all agree.
fn common<T: PartialEq, I>(items: &[I], get: impl Fn(&I) -> T) -> Option<T> {
    let mut values = items.iter().map(get);
    let first = values.next()?;
    values.all(|v| v == first).then_some(first)
}

fn alignment(a: &ShapeAttrs) -> &'static str {
    let position = match a.stroke_pos {
        Some(Dimension::Number(n)) => n,
        _ => 0.0,
    };
    ALIGNMENTS.iter().min_by(|x, y| (x.1 - position).abs().total_cmp(&(y.1 - position).abs())).map_or("center", |a| a.0)
}

/// The Attributes model and what it was last projected from.
#[derive(Default)]
pub struct AttributesModels {
    seen: Option<(Stamp, Selection)>,
    pub attributes: Tracked<AttributesModel>,
}

/// Projects the Attributes model when the stamp or the selection moved.
pub fn project(models: &mut AttributesModels, cx: &mut ProjectCx<'_>, tools: &Tools, stamp: Stamp) {
    let key = (stamp, tools.selection.clone());
    if models.seen.as_ref() == Some(&key) {
        return;
    }
    models.seen = Some(key);
    super::set(&mut models.attributes, attributes(cx, &tools.selection).unwrap_or_default());
}

fn attributes(cx: &ProjectCx<'_>, selection: &Selection) -> Option<AttributesModel> {
    if cx.view.edit_background {
        return None;
    }
    let font = cx.session.font().ok()?;
    let scene = scene(font, cx.session.glyph_data().ok().as_deref(), cx.view, None)?;
    if !scene.layer.info.attrs.full_color {
        return None;
    }
    let paths: Vec<usize> = selected_paths(selection).into_iter().filter(|&s| scene.path(s).is_some()).collect();
    let attrs: Vec<ShapeAttrs> = paths
        .iter()
        .filter_map(|&i| match scene.layer.shapes.get(i) {
            Some(Shape::Path(p)) => Some(p.info.attrs.clone()),
            _ => None,
        })
        .collect();
    if attrs.is_empty() {
        return None;
    }
    let spec = fontcraft_engine::find_command("attributes.setFillColor")?;
    let mut t = targets::selection_targets(&scene, selection, spec);
    t.remove("points");
    t.insert("shapes".into(), json!(paths));
    let palettes = color_palette::read(font, Some(&scene.layer.master)).unwrap_or_default();
    let colour_of = |c: &Option<Color>| c.as_ref().and_then(|c| resolve(c, &palettes));
    let gradient = attrs.iter().find_map(|a| a.gradient.as_ref());
    let stops = gradient
        .map(|g| {
            g.stops
                .iter()
                .map(|s| (if s.offset.is_finite() { s.offset.clamp(0.0, 1.0) } else { 0.0 }, resolve(&s.color, &palettes).unwrap_or([0, 0, 0, 0])))
                .collect()
        })
        .unwrap_or_default();
    let shadow = |s: Option<&fontcraft_engine::model::Shadow>| s.map(|s| ShadowLine { blur: s.blur, x: s.offset[0], y: s.offset[1] });
    let first = attrs.first();
    Some(AttributesModel {
        targets: Some(Targets { json: Value::Object(t).to_string() }),
        stroke_width: common(&attrs, |a| match a.stroke_width {
            Some(Dimension::Number(n)) => Some(n),
            _ => None,
        })
        .flatten(),
        alignment: common(&attrs, alignment).map(str::to_string),
        stroke_color: common(&attrs, |a| colour_of(&a.stroke_color)).flatten(),
        fill_color: common(&attrs, |a| colour_of(&a.fill_color)).flatten(),
        any_gradient: gradient.is_some(),
        gradient_type: common(&attrs, |a| {
            a.gradient.as_ref().map(|g| match g.kind {
                GradientKind::Linear { .. } => "linear",
                GradientKind::Radial { .. } => "radial",
                GradientKind::Conic { .. } => "conic",
            })
        })
        .flatten()
        .map(str::to_string),
        stops,
        shadow: shadow(first.and_then(|a| a.shadow.as_ref())),
        inner_shadow: shadow(first.and_then(|a| a.shadow_in.as_ref())),
    })
}
