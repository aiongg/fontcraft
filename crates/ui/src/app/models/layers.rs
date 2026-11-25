//! The Layers panel's and the master switcher's view models.

use std::collections::{HashMap, HashSet};

use craft::ui::contract::{PlainData, Tracked};
use fontcraft_engine::Session;
use fontcraft_engine::color::layers::{self as color_layers, ColorLayerKind};
use fontcraft_engine::color::palette as color_palette;
use fontcraft_engine::interp::labels::{brace_label, bracket_label};
use fontcraft_engine::model::{Font, Layer, Master, PaletteEntry};
use fontcraft_geom::{Affine, PathEl, Shape as _};
use serde_json::{Value, json};

use super::ProjectCx;
use crate::app::ViewState;
use crate::shell::masters::{self, LayersView};

/// Most rows the panel lists.
pub const MAX_ROWS: usize = 500;
/// Most palette colours a colour layer's chooser lists.
pub const MAX_CHOOSER_ROWS: usize = 256;
/// The `context` of the commands the row menu offers.
pub const ROW_CONTEXT: &str = "layers.row";
/// Most points of a glyph master icon.
const MAX_ICON_POINTS: usize = 2048;
/// Room left around a glyph icon, as a share of the side.
const GLYPH_MARGIN: f64 = 0.1;

/// The part of a special layer's label: `{…}` for an intermediate layer, `[…]` for an alternate one.
#[derive(PlainData, Clone, Copy, Debug, PartialEq, Eq)]
pub enum Part {
    Intermediate,
    Alternate,
}

impl Part {
    /// The name `layers.editSpecialLayerSettings` takes.
    pub fn name(self) -> &'static str {
        match self {
            Part::Intermediate => "intermediate",
            Part::Alternate => "alternate",
        }
    }
}

/// The layers of the panel's glyphs as the Layers panel lists them: the glyphs selected in Font View, those of the text
/// selection, else the shown glyph. With several glyphs the rows are the layers all of them have ([`shared`]).
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct LayersModel {
    /// The glyphs the rows belong to and the panel's commands change; empty without an open document or glyph.
    pub glyphs: Vec<String>,
    /// Every row in the first glyph's palette order, filters ignored: `layers.reorder` counts hidden rows too.
    pub all: Vec<LayerRow>,
    /// The ids of the rows the filters keep, in order.
    pub shown: Vec<String>,
    /// The row the glyphs are shown in, when it is one row.
    pub active_layer: Option<String>,
    /// The active layer's master, else the current master.
    pub active_master: Option<String>,
    /// The selected rows; the active layer's row when nothing else is.
    pub selection: Vec<String>,
    pub current_master_only: bool,
    pub hide_backups: bool,
    /// The commands of a row's context menu, in order.
    pub row_menu: Vec<RowCommand>,
    /// The row whose context menu is open.
    pub menu_row: Option<String>,
}

impl LayersModel {
    pub fn row(&self, id: &str) -> Option<&LayerRow> {
        self.all.iter().find(|r| r.id == id)
    }
}

/// One layer of the first glyph and the layers it stands for in the others.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct LayerRow {
    /// The layer's id in the first glyph.
    pub id: String,
    /// The ids of the layers it stands for in the glyphs where the layer has another id, with the glyph's place in
    /// [`LayersModel::glyphs`], in that order.
    pub others: Vec<(usize, String)>,
    /// Its name; the master's name for a master layer.
    pub name: String,
    /// `master`, `backup`, or the kind `layers.list` reports.
    pub kind: String,
    pub master: String,
    pub visible: bool,
    /// `[…]`: an alternate layer's ranges.
    pub bracket: Option<String>,
    /// `{…}`: an intermediate layer's coordinates.
    pub brace: Option<String>,
    pub color: Option<ColorEntry>,
}

impl LayerRow {
    pub fn is_master(&self) -> bool {
        self.kind == "master"
    }

    pub fn is_backup(&self) -> bool {
        self.kind == "backup"
    }

    /// The layer's id in the glyph at `index`.
    pub fn layer_in(&self, index: usize) -> &str {
        self.others.binary_search_by_key(&index, |(at, _)| *at).ok().and_then(|at| self.others.get(at)).map_or(&self.id, |(_, id)| id)
    }

    /// The layers the row stands for in `glyphs` ([`LayersModel::glyphs`]): each id with the glyphs that have it, the
    /// row's own id first.
    pub fn layers<'g>(&self, glyphs: &'g [String]) -> Vec<(&str, Vec<&'g str>)> {
        let mut others = self.others.iter().peekable();
        let mut out: Vec<(&str, Vec<&'g str>)> = vec![(self.id.as_str(), Vec::new())];
        let mut at: HashMap<&str, usize> = HashMap::from([(self.id.as_str(), 0)]);
        for (index, glyph) in glyphs.iter().enumerate() {
            let id = others.next_if(|(other, _)| *other == index).map_or(self.id.as_str(), |(_, id)| id.as_str());
            let group = *at.entry(id).or_insert_with(|| {
                out.push((id, Vec::new()));
                out.len().saturating_sub(1)
            });
            if let Some((_, names)) = out.get_mut(group) {
                names.push(glyph);
            }
        }
        out.retain(|(_, names)| !names.is_empty());
        out
    }

    /// Whether the layer has the status `part` names.
    pub fn has(&self, part: Part) -> bool {
        match part {
            Part::Intermediate => self.brace.is_some(),
            Part::Alternate => self.bracket.is_some(),
        }
    }
}

/// What a colour layer's row shows after its name.
#[derive(PlainData, Clone, Debug, PartialEq)]
pub enum ColorEntry {
    /// A Color Palette layer: its entry (`None` for the text colour) and palette 0 of the palettes that apply to its
    /// master.
    Palette {
        label: String,
        current: Option<u16>,
        palette: Vec<[u8; 4]>,
    },
    Full {
        label: String,
    },
    IColor {
        label: String,
        size: u16,
    },
    Svg {
        label: String,
    },
}

/// A command of the row menu.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct RowCommand {
    pub id: String,
    pub label: String,
    /// Why the command cannot run now, whatever the row.
    pub refused: Option<String>,
}

/// The masters the master switcher shows.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct MastersModel {
    pub masters: Vec<MasterEntry>,
    /// The master glyphs are drawn in.
    pub current: Option<String>,
}

#[derive(PlainData, Clone, Debug, PartialEq)]
pub struct MasterEntry {
    pub id: String,
    pub name: String,
    pub icon: MasterIcon,
}

/// A master's icon (`font.master.setIcon`).
#[derive(PlainData, Clone, Debug, PartialEq)]
pub enum MasterIcon {
    /// A built-in icon: a lowercase n at a weight row and a width column of the 5 by 5 grid.
    Builtin { weight: u8, width: u8 },
    /// A glyph of the font in the master, as polylines on a unit square with y pointing down.
    Outline(Vec<Vec<[f32; 2]>>),
}

/// The icon a master without one shows.
pub const DEFAULT_ICON: &str = "Regular";

/// The built-in icons per row of the grid.
pub const ICON_WIDTHS: usize = 5;

pub fn project(model: &mut Tracked<LayersModel>, cx: &mut ProjectCx<'_>) {
    let value = layers(cx.session, cx.view).unwrap_or_default();
    super::set(model, value);
}

pub fn project_masters(model: &mut Tracked<MastersModel>, cx: &mut ProjectCx<'_>) {
    let value = cx.session.font().map(|font| masters(font, &cx.view.layers)).unwrap_or_default();
    super::set(model, value);
}

fn layers(session: &mut Session, view: &ViewState) -> Option<LayersModel> {
    let (glyphs, all) = panel_rows(session, view)?;
    let font = session.font().ok()?;
    let (active_layer, active_master) = active(font, view, &glyphs, &all);
    let shown = filtered(&all, view, active_master.as_deref()).map(|r| r.id.clone()).collect();
    let mut selection: Vec<String> = view.layers.selection.iter().filter(|id| all.iter().any(|r| r.id == **id)).cloned().collect();
    if selection.is_empty() {
        selection.extend(active_layer.clone());
    }
    Some(LayersModel {
        glyphs,
        all,
        shown,
        active_layer,
        active_master,
        selection,
        current_master_only: view.layers.current_master_only,
        hide_backups: view.layers.hide_backups,
        row_menu: row_menu(session),
        menu_row: view.layers.menu_row.clone(),
    })
}

/// The panel's glyphs and their rows, filters ignored. `None` without a glyph.
pub fn panel_rows(session: &mut Session, view: &ViewState) -> Option<(Vec<String>, Vec<LayerRow>)> {
    let glyphs = masters::panel_glyphs(session.font().ok()?, view);
    let mut all = rows(session, glyphs.first()?);
    let font = session.font().ok()?;
    let layers: Vec<&[Layer]> = glyphs.iter().filter_map(|name| font.glyph(name)).map(|g| g.layers.as_slice()).collect();
    if let [first, others @ ..] = layers.as_slice() {
        shared(&mut all, first, others);
    }
    Some((glyphs, all))
}

/// The rows of the panel's glyphs the filters keep, and the active row.
pub fn shown_rows(session: &mut Session, view: &ViewState) -> Option<(Vec<LayerRow>, Option<String>)> {
    let (glyphs, all) = panel_rows(session, view)?;
    let (layer, master) = active(session.font().ok()?, view, &glyphs, &all);
    Some((filtered(&all, view, master.as_deref()).cloned().collect(), layer))
}

/// Keeps the rows of the first glyph, which has the layers `first`, that every glyph of `others` has too: the master
/// layer of the same master, else the layer with the same id, else the next one under the same master with the same
/// name and the same special attributes. Records the ids that differ.
fn shared(rows: &mut Vec<LayerRow>, first: &[Layer], others: &[&[Layer]]) {
    for (at, layers) in others.iter().enumerate() {
        let mut used: HashSet<&str> = layers.iter().filter(|l| rows.iter().any(|r| r.id == l.id)).map(|l| l.id.as_str()).collect();
        let mut found: Vec<Option<(usize, String)>> = Vec::with_capacity(rows.len());
        let mut kept: Vec<bool> = Vec::with_capacity(rows.len());
        for row in rows.iter() {
            let here = layers.iter().find(|l| l.id == row.id && l.master == row.master);
            if here.is_some() {
                kept.push(true);
                found.push(None);
                continue;
            }
            let like = first.iter().find(|l| l.id == row.id).filter(|_| !row.is_master());
            let twin = like.and_then(|like| {
                layers.iter().find(|l| {
                    l.master == like.master
                        && !l.is_master_layer()
                        && l.info.name == like.info.name
                        && l.info.attrs == like.info.attrs
                        && !used.contains(l.id.as_str())
                })
            });
            kept.push(twin.is_some());
            found.push(twin.map(|l| (at + 1, l.id.clone())));
            used.extend(twin.map(|l| l.id.as_str()));
        }
        for (row, twin) in rows.iter_mut().zip(found) {
            row.others.extend(twin);
        }
        let mut kept = kept.into_iter();
        rows.retain(|_| kept.next().unwrap_or(false));
    }
}

/// The row the panel's glyphs are shown in, when it is one, and its master, else the current master.
fn active(font: &Font, view: &ViewState, glyphs: &[String], all: &[LayerRow]) -> (Option<String>, Option<String>) {
    let current = masters::current_master(font, &view.layers).map(|m| m.id.clone());
    let chosen = crate::app::targets::chosen_glyphs(font, view);
    let layer = if chosen.is_empty() {
        masters::panel_layer(font, view).map(|l| l.id.clone())
    } else {
        let places: HashMap<&str, usize> = glyphs.iter().enumerate().map(|(at, name)| (name.as_str(), at)).collect();
        let in_groups = |row: &&LayerRow| {
            chosen.iter().all(|g| g.glyphs.iter().all(|name| places.get(name.as_str()).is_some_and(|at| row.layer_in(*at) == g.layer)))
        };
        all.iter().find(in_groups).map(|r| r.id.clone())
    };
    let master = layer.as_ref().and_then(|id| all.iter().find(|r| r.id == *id)).map(|r| r.master.clone());
    (layer, master.or(current))
}

fn filtered<'r>(all: &'r [LayerRow], view: &ViewState, master: Option<&str>) -> impl Iterator<Item = &'r LayerRow> {
    let (current_only, no_backups) = (view.layers.current_master_only, view.layers.hide_backups);
    all.iter().filter(move |r| !current_only || Some(r.master.as_str()) == master).filter(move |r| !no_backups || !r.is_backup())
}

/// Every layer of `glyph` in palette order (`layers.list`), with its special label and colour entry.
fn rows(session: &mut Session, glyph: &str) -> Vec<LayerRow> {
    let listed = session.query("layers.list", &json!({ "glyph": glyph })).unwrap_or(Value::Null);
    let Some(list) = listed.get("layers").and_then(Value::as_array) else { return Vec::new() };
    let s = |v: &Value, k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
    let mut rows: Vec<LayerRow> = list
        .iter()
        .take(MAX_ROWS)
        .map(|v| LayerRow {
            id: s(v, "id"),
            name: s(v, "name"),
            kind: s(v, "kind"),
            master: s(v, "master"),
            visible: v.get("visible").and_then(Value::as_bool).unwrap_or(true),
            ..LayerRow::default()
        })
        .collect();
    let Some((font, g)) = session.font().ok().and_then(|font| Some((font, font.glyph(glyph)?))) else { return rows };
    let mut palettes: HashMap<&str, Vec<[u8; 4]>> = HashMap::new();
    for row in &mut rows {
        let Some(layer) = g.layers.iter().find(|l| l.id == row.id) else { continue };
        row.bracket = bracket_label(font, layer);
        row.brace = brace_label(font, layer);
        let Some(kind) = color_layers::kind(layer) else { continue };
        let label = color_layers::label(&kind);
        row.color = Some(match kind {
            ColorLayerKind::Palette(entry) => {
                let palette = palettes.entry(layer.master.as_str()).or_insert_with(|| palette_zero(font, &layer.master)).clone();
                let current = match entry {
                    PaletteEntry::Index(i) => Some(i),
                    PaletteEntry::Foreground => None,
                };
                ColorEntry::Palette { label, current, palette }
            }
            ColorLayerKind::FullColor => ColorEntry::Full { label },
            ColorLayerKind::IColor(size) => ColorEntry::IColor { label, size },
            ColorLayerKind::Svg => ColorEntry::Svg { label },
        });
    }
    rows
}

/// Palette 0 of the palettes that apply to `master`: its own, else the font's.
fn palette_zero(font: &Font, master: &str) -> Vec<[u8; 4]> {
    color_palette::read(font, Some(master))
        .ok()
        .and_then(|p| p.palette(0).map(|row| row.iter().take(MAX_CHOOSER_ROWS + 1).map(|c| [c.r, c.g, c.b, c.a]).collect()))
        .unwrap_or_default()
}

/// The commands whose context names the Layers rows, with why each cannot run now.
fn row_menu(session: &Session) -> Vec<RowCommand> {
    fontcraft_engine::command_specs()
        .iter()
        .filter(|spec| spec.context.contains(&ROW_CONTEXT))
        .map(|spec| RowCommand { id: spec.id.to_string(), label: spec.label.to_string(), refused: (spec.enabled)(session).err() })
        .collect()
}

fn masters(font: &Font, view: &LayersView) -> MastersModel {
    MastersModel {
        masters: font.masters.iter().map(|m| MasterEntry { id: m.id.clone(), name: m.name.clone(), icon: icon(font, m) }).collect(),
        current: masters::current_master(font, view).map(|m| m.id.clone()),
    }
}

/// The master custom parameter naming the glyph a master without an icon of its own shows.
pub const ICON_GLYPH_PARAM: &str = "Master Icon Glyph Name";

/// The icon of `master`: its own icon (a built-in one, else the glyph it names drawn in it), else the glyph its Master
/// Icon Glyph Name names, else the Regular icon. A name that is no glyph of the font, or a glyph with no outline in the
/// master, falls through to the next.
pub fn icon(font: &Font, master: &Master) -> MasterIcon {
    let own = master.icon.as_deref().and_then(|name| builtin(name).or_else(|| glyph_icon(font, master, name)));
    let named = || master.params.get(ICON_GLYPH_PARAM).and_then(|v| v.as_str()).and_then(|name| glyph_icon(font, master, name.trim()));
    own.or_else(named).or_else(|| builtin(DEFAULT_ICON)).unwrap_or(MasterIcon::Builtin { weight: 1, width: 2 })
}

/// Glyph `name` of the font drawn in `master`, as an icon.
fn glyph_icon(font: &Font, master: &Master, name: &str) -> Option<MasterIcon> {
    font.glyph(name).and_then(|_| glyph_outline(font, name, &master.id))
}

/// The built-in icon `name` (`MASTER_ICONS`, row by row).
pub fn builtin(name: &str) -> Option<MasterIcon> {
    let at = fontcraft_engine::cmd::masters::MASTER_ICONS.iter().position(|n| *n == name)?;
    let (weight, width) = (u8::try_from(at / ICON_WIDTHS).ok()?, u8::try_from(at % ICON_WIDTHS).ok()?);
    Some(MasterIcon::Builtin { weight, width })
}

/// Glyph `glyph` in master `master` fitted into the unit square, y down; `None` when it has no outline to draw.
fn glyph_outline(font: &Font, glyph: &str, master: &str) -> Option<MasterIcon> {
    let path = font.outline(glyph, master).ok()?;
    let bounds = path.bounding_box();
    let (w, h) = (bounds.width(), bounds.height());
    if !(w.is_finite() && h.is_finite()) || (w <= 0.0 && h <= 0.0) {
        return None;
    }
    let scale = (1.0 - 2.0 * GLYPH_MARGIN) / w.max(h);
    let to_unit =
        Affine::translate((0.5, 0.5)) * Affine::scale_non_uniform(scale, -scale) * Affine::translate((-bounds.center().x, -bounds.center().y));
    let mut contours: Vec<Vec<[f32; 2]>> = Vec::new();
    let mut count = 0usize;
    let point = |p: fontcraft_geom::Point| [p.x as f32, p.y as f32];
    fontcraft_geom::flatten(&(to_unit * path), 0.002, |el| {
        count += 1;
        if count > MAX_ICON_POINTS {
            return;
        }
        match el {
            PathEl::MoveTo(p) => contours.push(vec![point(p)]),
            PathEl::LineTo(p) => {
                if let Some(c) = contours.last_mut() {
                    c.push(point(p));
                }
            }
            PathEl::ClosePath => {
                if let Some(c) = contours.last_mut()
                    && let Some(first) = c.first().copied()
                {
                    c.push(first);
                }
            }
            _ => {}
        }
    });
    (!contours.is_empty()).then_some(MasterIcon::Outline(contours))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::indexing_slicing)]
mod tests {
    use std::sync::Arc;

    use craft::host::Host;
    use fontcraft_engine::Session;
    use serde_json::json;

    use super::{MasterIcon, icon};
    use crate::app::{APP, FontCraftApp, Msg};

    /// A master without an icon of its own shows the glyph its Master Icon Glyph Name names, in the switcher's model
    /// too; its own icon wins, and a name that is no glyph leaves the Regular icon.
    #[test]
    fn master_icon_glyph_name_draws_the_glyph_for_a_master_without_an_icon() {
        let mut app = FontCraftApp::with_edit_tab(Session::with_document(), Arc::new(Host::builder(APP).build()));
        let mut run = |id: &str, params: serde_json::Value| app.run_now(Msg::run(id, params), false).unwrap();
        run("font.master.add", json!({"name": "Bold"}));
        run("path.addRectangle", json!({"glyph": "A", "x": 0, "y": 0, "width": 300, "height": 600}));
        let regular = icon(app.session.font().unwrap(), &app.session.font().unwrap().masters[0]);
        let first = app.session.font().unwrap().masters[0].id.clone();
        let add = json!({"scope": "master", "targets": first, "name": "Master Icon Glyph Name", "value": "A"});
        app.run_now(Msg::run("font.customParameters.add", add), false).unwrap();
        let font = app.session.font().unwrap();
        assert!(matches!(icon(font, &font.masters[0]), MasterIcon::Outline(ref c) if !c.is_empty()), "the glyph A, drawn");
        assert_eq!(icon(font, &font.masters[1]), regular, "the other master keeps the Regular icon");
        assert!(matches!(app.models.masters.masters[0].icon, MasterIcon::Outline(_)), "the master switcher shows it");

        app.run_now(Msg::run("font.master.setIcon", json!({"masters": [0], "icon": "Bold"})), false).unwrap();
        let font = app.session.font().unwrap();
        assert!(matches!(icon(font, &font.masters[0]), MasterIcon::Builtin { .. }), "the master's own icon wins");

        let unknown = json!({"scope": "master", "targets": 1, "name": "Master Icon Glyph Name", "value": "nosuchglyph"});
        app.run_now(Msg::run("font.customParameters.add", unknown), false).unwrap();
        let font = app.session.font().unwrap();
        assert_eq!(icon(font, &font.masters[1]), regular, "a name that is no glyph leaves the Regular icon");
    }
}
