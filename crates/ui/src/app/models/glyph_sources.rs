//! The Glyph Sources panel's view model: the shown glyph's sources, how much each contributes at the location the
//! previews show (`shell::axis_sweep::previewed_location`), and the masters the glyph has no layer for.

use craft::behavior::dock::PanelState;
use craft::ui::contract::{PlainData, Tracked};
use fontcraft_engine::cmd::glyph_sources::{SourceKind, influence};
use fontcraft_engine::interp::{Designspace, Location};
use fontcraft_engine::prelude::*;

use super::ProjectCx;
use crate::panels::widgets::format_number;
use crate::shell::axis_sweep::{has_range, previewed_location};
use crate::shell::dock::PanelId;
use crate::shell::masters;

/// Most sources and most missing masters the panel lists; the rest are counted.
pub const MAX_ROWS: usize = 64;

/// What the panel shows.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct GlyphSourcesModel {
    /// The glyph shown; `None` without a font or a glyph.
    pub glyph: Option<String>,
    /// Why the sources cannot be listed or weighed.
    pub problem: Option<String>,
    /// The axes with a range.
    pub axes: Vec<AxisRow>,
    /// The previewed location, internal coordinates by axis tag: where Add Source adds one.
    pub location: Vec<(String, f64)>,
    /// A source stands at the location already.
    pub source_there: bool,
    /// The outline at the location shows on the canvas.
    pub outline_shows: bool,
    pub sources: Vec<SourceEntry>,
    /// Sources past [`MAX_ROWS`], left out.
    pub more: usize,
    /// The names of the masters the glyph has no layer for.
    pub missing: Vec<String>,
}

/// One axis and the previewed coordinate on it. The slider and the field show the external one.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct AxisRow {
    pub tag: String,
    pub name: String,
    /// The internal coordinate.
    pub value: f64,
    /// The external coordinate and the axis' external range.
    pub external: f64,
    pub min: f64,
    pub max: f64,
    /// The axis is at its origin: there is nothing to reset.
    pub at_origin: bool,
    /// The axis is being animated.
    pub animated: bool,
}

/// One source of the glyph.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct SourceEntry {
    /// The layer's id.
    pub layer: String,
    pub name: String,
    /// Its location, as `tag value` pairs.
    pub location: String,
    pub active: bool,
    /// The origin master's layer, which always takes part.
    pub origin: bool,
    pub alternate: bool,
    /// Its share of the glyph at the previewed location.
    pub weight: f64,
    /// It can be deleted: it is no master layer.
    pub deletable: bool,
    /// How it differs from the layers it should interpolate with; empty when it does not.
    pub mismatch: String,
}

/// Projects the panel while it is open; closed, it holds nothing.
pub fn project(model: &mut Tracked<GlyphSourcesModel>, cx: &mut ProjectCx<'_>) {
    let shown = cx.view.workspace.layout().panel_state(&PanelId::GlyphSources) != PanelState::Hidden;
    let value = if shown { build(cx).unwrap_or_default() } else { GlyphSourcesModel::default() };
    super::set(model, value);
}

fn build(cx: &mut ProjectCx<'_>) -> Option<GlyphSourcesModel> {
    let designspace = cx.session.designspace();
    // Which layers differ from the rest of their group, and how.
    let shown = masters::shown_glyph(cx.session.font().ok()?, cx.view)?.name.clone();
    let compat = if cx.session.compat_indicators() { cx.session.glyph_compat(&shown) } else { None };
    let font = cx.session.font().ok()?;
    let glyph = masters::shown_glyph(font, cx.view)?;
    let outline_shows = cx.view.display.outline_at_location;
    let mut model = GlyphSourcesModel { glyph: Some(glyph.name.clone()), outline_shows, ..GlyphSourcesModel::default() };
    let designspace = match designspace {
        Ok(d) => d,
        Err(e) => {
            model.problem = Some(e);
            return Some(model);
        }
    };
    let here = previewed_location(font, &designspace, cx.view);
    let at = Location(designspace.axes().iter().map(|a| here.get(a.font_axis).copied().unwrap_or(a.internal.default)).collect());
    let animated = cx.view.preview.sweep.as_ref().map(|s| s.axis.as_str());
    model.axes = designspace
        .axes()
        .iter()
        .zip(&at.0)
        .filter(|(a, _)| has_range(a))
        .map(|(a, v)| AxisRow {
            tag: a.tag.clone(),
            name: a.name.clone(),
            value: *v,
            external: a.map.to_user(*v),
            min: a.external.minimum.min(a.external.maximum),
            max: a.external.minimum.max(a.external.maximum),
            at_origin: near(*v, a.internal.default),
            animated: animated == Some(a.tag.as_str()),
        })
        .collect();
    model.location = designspace.axes().iter().zip(&at.0).map(|(a, v)| (a.tag.clone(), *v)).collect();
    let found = match influence(font, &designspace, &glyph.name, &at) {
        Ok(found) => found,
        Err(e) => {
            model.problem = Some(e);
            return Some(model);
        }
    };
    model.more = found.sources.len().saturating_sub(MAX_ROWS);
    let there = |l: &Location| l.0.len() == at.0.len() && l.0.iter().zip(&at.0).all(|(a, b)| near(*a, *b));
    model.source_there = found.sources.iter().any(|r| r.kind != SourceKind::Alternate && there(&r.location));
    let mismatch = |layer: &str| {
        let found = compat.iter().flat_map(|c| &c.first_mismatch).find(|m| m.layer_b == layer);
        found.map(|m| m.describe(font)).unwrap_or_default()
    };
    model.sources = found
        .sources
        .into_iter()
        .take(MAX_ROWS)
        .map(|r| SourceEntry {
            location: location_text(&designspace, &r.location),
            mismatch: mismatch(&r.layer),
            deletable: r.kind != SourceKind::Master,
            layer: r.layer,
            name: r.name,
            active: r.active,
            origin: r.origin,
            alternate: r.kind == SourceKind::Alternate,
            weight: r.weight,
        })
        .collect();
    model.missing = found
        .missing
        .iter()
        .take(MAX_ROWS)
        .map(|id| font.masters.iter().find(|m| m.id == *id).map_or_else(|| id.clone(), |m| m.name.clone()))
        .collect();
    Some(model)
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 * (1.0 + b.abs())
}

/// `location` as `tag value` pairs.
fn location_text(designspace: &Designspace, location: &Location) -> String {
    designspace.axes().iter().zip(&location.0).map(|(a, v)| format!("{} {}", a.tag, format_number(*v))).collect::<Vec<_>>().join(", ")
}
