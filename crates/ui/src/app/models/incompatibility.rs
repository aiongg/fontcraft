//! The incompatibility indicators: the bar over each glyph of the edit view's text and the corner of each Font View
//! cell whose layers cannot interpolate, with the reason the engine gives (`compatibility.editViewRedBar`,
//! `fontView.cell.badgeIncompatible`).
//!
//! The answers are the engine's (`SessionInterpExt`), kept there until a glyph changes. Working them out for a large
//! font takes longer than a frame, so one projection does at most [`WORK_PER_PASS`] of it: [`scan`] runs before the
//! models are projected and checks the cells in view first, then the rest of the font; [`project`] checks the glyphs of
//! the edit view's text. While answers are missing the next frame is asked for, a cell without one shows no corner, and
//! the sidebar's counts and the filters take the answers there are; they are projected once more when the last one is
//! in. Nothing is drawn while Enforce Compatibility Check is present and off.

use std::collections::HashSet;

use craft::behavior::dock::PanelState;
use craft::ui::contract::{PlainData, Tracked};
use fontcraft_engine::Session;
use fontcraft_engine::prelude::*;

use super::ProjectCx;
use super::compatibility::CompatibilityModel;
use super::font_view::{FontViewModel, GlyphRow};
use super::text::{MAX_DRAWN_GLYPHS, TextModel};
use crate::app::FontCraftApp;
use crate::shell::dock::PanelId;

/// How much checking one projection does, in compared nodes: about two milliseconds.
pub const WORK_PER_PASS: usize = 20_000;
/// Most cells in view one pass asks for.
pub const MAX_CELLS: usize = 4_096;
/// Most pairs of layers a reason names.
const MAX_REASONS: usize = 4;

/// What a Font View cell shows of its glyph's compatibility.
#[derive(PlainData, Clone, Debug, Default, PartialEq, Eq)]
pub enum CellCompat {
    /// No corner: the layers interpolate, or the font shows no indicators.
    #[default]
    Fine,
    /// Not checked yet.
    Unchecked,
    /// The layers cannot interpolate, and why.
    Differs(String),
}

/// The bar over one glyph of the text, in the edited glyph's font units.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct Bar {
    pub glyph: String,
    /// The text item; `None` for an edited glyph the text does not hold.
    pub item: Option<usize>,
    /// The glyph's advance and its ascender.
    pub x0: f64,
    pub x1: f64,
    pub y: f64,
    pub reason: String,
}

/// Where the checks stand.
#[derive(Default)]
pub struct Scan {
    /// The cells in view without an answer, as the grid reported them.
    cells: Vec<String>,
    /// The whole font's answers were not all in after the last pass.
    waiting: bool,
    /// The last [`scan`] worked something out.
    moved: bool,
    /// Glyphs of the text wait for their answer.
    text_waiting: bool,
}

impl Scan {
    /// The grid drew these cells without an answer.
    pub fn ask(&mut self, mut cells: Vec<String>) {
        cells.truncate(MAX_CELLS);
        self.cells = cells;
    }

    /// Answers are still missing: another pass is wanted.
    pub fn waiting(&self) -> bool {
        self.waiting || self.text_waiting || self.moved
    }
}

/// Before the projections: checks part of the font for Font View, the sidebar and its filters, whichever show. When the
/// last answer of several passes is in, the models are projected again.
pub fn scan(app: &mut FontCraftApp) {
    let sidebar = app.view.workspace.layout.panel_state(&PanelId::Sidebar) != PanelState::Hidden;
    let cells = std::mem::take(&mut app.models.compat_scan.cells);
    let waited = app.models.compat_scan.waiting;
    let waiting = (sidebar || app.font_view_shows()) && app.session.check_some_compat(&cells, true, WORK_PER_PASS);
    let scan = &mut app.models.compat_scan;
    scan.moved = waited || !cells.is_empty();
    scan.waiting = waiting;
    if waited && !waiting {
        app.view_changed();
    }
}

/// Why `glyph` cannot interpolate, on one line.
fn reason(session: &mut Session, glyph: &str) -> String {
    let reasons = session.compat_reasons(glyph);
    let more = reasons.len().saturating_sub(MAX_REASONS);
    let mut text = reasons.into_iter().take(MAX_REASONS).collect::<Vec<_>>().join("; ");
    if more > 0 {
        text.push_str(&format!("; and {more} more"));
    }
    text
}

fn states(session: &mut Session, rows: &[GlyphRow]) -> Vec<CellCompat> {
    if !session.compat_indicators() {
        return vec![CellCompat::Fine; rows.len()];
    }
    let names: Vec<&str> = rows.iter().map(|r| r.name.as_str()).collect();
    let known = session.known_compat_states(&names);
    let state = |(row, known): (&GlyphRow, Option<bool>)| match known {
        Some(true) => CellCompat::Fine,
        None => CellCompat::Unchecked,
        Some(false) => CellCompat::Differs(reason(session, &row.name)),
    };
    rows.iter().zip(known).map(state).collect()
}

/// Sets the rows' states from the answers there are.
pub fn mark(session: &mut Session, rows: &mut [GlyphRow]) {
    let states = states(session, rows);
    for (row, state) in rows.iter_mut().zip(states) {
        row.compat = state;
    }
}

/// After the projections: `projected` says that the models were projected on this pass, else only the answers may have
/// moved.
pub fn project(
    scan: &mut Scan,
    font_view: &mut Tracked<FontViewModel>,
    compatibility: &mut Tracked<CompatibilityModel>,
    text: &TextModel,
    projected: bool,
    cx: &mut ProjectCx<'_>,
) {
    if scan.moved && !projected && !font_view.rows.is_empty() {
        let states = states(cx.session, &font_view.rows);
        if font_view.rows.iter().zip(&states).any(|(row, state)| row.compat != *state) {
            font_view.edit(|m| {
                for (row, state) in m.rows.iter_mut().zip(states) {
                    row.compat = state;
                }
            });
        }
    }
    if !projected && !scan.text_waiting && !scan.moved {
        return;
    }
    let mut bars = Vec::new();
    let mut line = String::new();
    scan.text_waiting = false;
    if cx.view.tabs.font_view_shows() {
        // The one selected glyph's reason.
        if let [row] = font_view.selected.as_slice()
            && let Some(GlyphRow { name, compat: CellCompat::Differs(reason), .. }) = font_view.rows.get(*row)
        {
            line = told(name, reason);
        }
    } else if let Some((found, waiting)) = text_bars(text, cx) {
        scan.text_waiting = waiting;
        if let Some(bar) = found.first().filter(|b| Some(b.glyph.as_str()) == super::glyph::shown_glyph(cx).as_deref()) {
            line = told(&bar.glyph, &bar.reason);
        }
        bars = found;
    }
    if compatibility.bars != bars || compatibility.reason != line {
        compatibility.edit(|m| {
            m.bars = bars;
            m.reason = line;
        });
    }
}

/// The status line's text.
fn told(glyph: &str, reason: &str) -> String {
    format!("{glyph} cannot interpolate: {reason}")
}

/// The bars of the edit view, the edited glyph's first, and whether glyphs of the text still wait for their answer.
fn text_bars(text: &TextModel, cx: &mut ProjectCx<'_>) -> Option<(Vec<Bar>, bool)> {
    let edited = super::glyph::shown_glyph(cx)?;
    if !cx.session.compat_indicators() {
        return None;
    }
    let mut names = vec![edited.clone()];
    if let Some(layout) = text.layout.as_ref().filter(|_| text.shown) {
        let mut seen: HashSet<&str> = HashSet::from([edited.as_str()]);
        let others = layout.placed.iter().take(MAX_DRAWN_GLYPHS).filter_map(|p| p.glyph.as_deref());
        names.extend(others.filter(|n| seen.insert(*n)).map(str::to_string));
    }
    let waiting = cx.session.check_some_compat(&names, false, WORK_PER_PASS);
    let mut reasons = std::collections::HashMap::new();
    for name in names {
        if cx.session.known_compat(&name).is_some_and(|c| !c.ok) {
            let reason = reason(cx.session, &name);
            reasons.insert(name, reason);
        }
    }
    if reasons.is_empty() {
        return Some((Vec::new(), waiting));
    }
    let font = cx.session.font().ok()?;
    let glyph = font.glyph(&edited)?;
    let mut out = Vec::new();
    let mut push = |name: &str, item: Option<usize>, at: [f64; 2], width: f64, ascender: f64| {
        let Some(reason) = reasons.get(name) else { return };
        let width = if width.is_finite() { width.max(0.0) } else { 0.0 };
        let bar = Bar { glyph: name.to_string(), item, x0: at[0], x1: at[0] + width, y: at[1] + ascender, reason: reason.clone() };
        if [bar.x0, bar.x1, bar.y].iter().all(|v| v.is_finite()) {
            out.push(bar);
        }
    };
    let shown = crate::shell::masters::shown_layer(font, &cx.view.layers, glyph);
    let ascender = match (&text.layout, shown.and_then(|l| crate::shell::masters::layer_master(font, &cx.view.layers, l))) {
        (_, Some(master)) => super::canvas::vertical_span(font, master).1,
        (Some(layout), None) => layout.ascender,
        (None, None) => return Some((Vec::new(), waiting)),
    };
    push(&edited, text.edited.filter(|_| text.shown), [0.0, 0.0], shown.map_or(0.0, |l| l.width), ascender);
    if let Some(layout) = text.layout.as_ref().filter(|_| text.shown) {
        for (i, p) in layout.placed.iter().enumerate().take(MAX_DRAWN_GLYPHS) {
            if Some(i) == text.edited {
                continue;
            }
            let Some(name) = p.glyph.as_deref() else { continue };
            let own = cx.view.text.text.layer_of(i, name);
            let width = font.glyph(name).and_then(|g| super::text::item_layer(font, cx.view, g, own)).map_or(p.advance, |l| l.width);
            push(name, Some(i), [p.x - text.origin[0], p.y - text.origin[1]], width, layout.ascender);
        }
    }
    Some((out, waiting))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::indexing_slicing, clippy::panic)]
mod tests {
    use std::sync::Arc;

    use craft::host::Host;
    use craft::ui::runtime::commands::CommandRef;
    use fontcraft_engine::Session;
    use fontcraft_engine::model::{Glyph, Layer, Path, Shape};
    use fontcraft_tools::{Modifiers, PointerEvent, PointerKind};
    use serde_json::{Value, json};

    use super::CellCompat;
    use crate::app::{APP, FontCraftApp, Msg, ViewMsg};
    use crate::canvas::tools::ToolInput;

    /// Masters Light (30) and Bold (120) and an instance between them; `n` has a second path on Bold, `o` matches, and
    /// `extra` more glyphs like `o`.
    fn session(extra: usize) -> Session {
        let mut s = Session::with_document();
        let mut run = |id: &str, params: Value| {
            s.execute(id, &params).unwrap_or_else(|e| panic!("{id}: {e}"));
        };
        run("font.axes.add", json!({"name": "Weight", "tag": "wght"}));
        run("font.master.add", json!({}));
        run("font.master.setAxisLocation", json!({"masters": [0], "location": [30]}));
        run("font.master.setAxisLocation", json!({"masters": [1], "location": [120]}));
        s.edit("glyphs", |font| {
            let masters: Vec<String> = font.masters.iter().map(|m| m.id.clone()).collect();
            font.instances = Arc::new(vec![fontcraft_engine::model::Instance { name: "Medium".into(), location: vec![70.0], ..Default::default() }]);
            let names = ["n".to_string(), "o".to_string()].into_iter().chain((0..extra).map(|i| format!("x{i}")));
            for name in names {
                let mut g = Glyph::new(name.clone());
                for (i, id) in masters.iter().enumerate() {
                    let mut l = Layer::new(id.as_str(), 500.0);
                    l.shapes.push(Shape::Path(Path::rect(50.0, 0.0, 250.0, 500.0)));
                    if name == "n" && i == 1 {
                        l.shapes.push(Shape::Path(Path::rect(300.0, 0.0, 400.0, 500.0)));
                    }
                    g.layers.push(l);
                }
                if font.glyphs.get(&name).is_some() {
                    let mut slot = font.glyph_mut(&name)?;
                    *slot = g;
                } else {
                    font.glyphs.push(g);
                }
            }
            Ok(())
        })
        .unwrap();
        s
    }

    fn app(extra: usize) -> FontCraftApp {
        FontCraftApp::with_edit_tab(session(extra), Arc::new(Host::builder(APP).build()))
    }

    fn shell(app: &mut FontCraftApp, id: &str, params: Value) {
        app.sent.push(Msg::Shell(CommandRef::with_params(id, params)));
        app.drain();
        assert!(app.status.is_empty(), "{id}: {}", app.status);
    }

    fn cell(app: &FontCraftApp, name: &str) -> CellCompat {
        let at = app.models.font_view.row_of(name).unwrap();
        app.models.font_view.rows[at].compat.clone()
    }

    /// Drains until no answer is missing.
    fn settle(app: &mut FontCraftApp) {
        for _ in 0..2_000 {
            app.drain();
            if !app.models.compat_scan.waiting() {
                return;
            }
        }
        panic!("the checks come to an end");
    }

    fn parameter_off(app: &mut FontCraftApp) {
        app.session
            .edit("indicators off", |font| {
                let off = fontcraft_engine::model::plist::Value::Bool(false);
                Arc::make_mut(&mut font.info).params.set(fontcraft_engine::interp::params::ENFORCE_COMPATIBILITY_CHECK, off);
                Ok(())
            })
            .unwrap();
        settle(app);
    }

    #[test]
    fn a_broken_master_shows_the_bar_and_the_reason_until_it_is_mended_or_hidden() {
        let mut app = app(0);
        shell(&mut app, "editView.setText", json!({"glyphs": ["o", "n", "o", "n"]}));
        shell(&mut app, "editView.setCursor", json!({"index": 0}));
        settle(&mut app);
        let bars = app.models.compatibility.bars.clone();
        assert_eq!(bars.iter().map(|b| (b.glyph.as_str(), b.item)).collect::<Vec<_>>(), [("n", Some(1)), ("n", Some(3))]);
        assert_eq!((bars[0].x0, bars[0].x1), (500.0, 1_000.0), "over the glyph's advance");
        let reason = bars[0].reason.clone();
        assert!(reason.ends_with("differ: 1 shapes against 2"), "{reason}");
        let masters = app.session.font().unwrap().masters.clone();
        assert!(reason.starts_with(&format!("{} and {}", masters[0].name, masters[1].name)), "the layers' names: {reason}");
        assert_eq!(app.models.compatibility.reason, "", "the edited glyph interpolates");

        // A click on the second bar edits that glyph and shows Master Compatibility.
        app.view.canvas.rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1_280.0, 800.0));
        let camera = crate::canvas::tools::apply::camera(&app);
        let rect = crate::canvas::compatibility::bar_rect(&camera, &bars[0], &app.models.canvas_tokens.compat).unwrap();
        let pos = camera.to_font(rect.center());
        for kind in [PointerKind::Down, PointerKind::Up] {
            app.sent.push(Msg::Tool(ToolInput::Pointer(PointerEvent { kind, pos, mods: Modifiers::NONE })));
        }
        settle(&mut app);
        assert!(app.view.display.master_compatibility, "{}", app.status);
        assert_eq!(app.view.text.edited_index(), Some(1));
        assert_eq!(app.models.compatibility.reason, format!("n cannot interpolate: {reason}"));

        app.session
            .edit("mend", |font| {
                font.glyph_mut("n")?.layers[1].shapes.pop();
                Ok(())
            })
            .unwrap();
        settle(&mut app);
        assert!(app.models.compatibility.bars.is_empty() && app.models.compatibility.reason.is_empty(), "mended");
        app.session.execute("edit.undo", &json!({})).unwrap();
        settle(&mut app);
        assert_eq!(app.models.compatibility.bars.len(), 2, "undo brings the bars back");
        parameter_off(&mut app);
        assert!(app.models.compatibility.bars.is_empty(), "Enforce Compatibility Check off hides them");
    }

    #[test]
    fn font_view_marks_the_cells_and_names_the_selected_glyphs_reason() {
        let mut app = app(0);
        shell(&mut app, "fontView.show", json!({}));
        settle(&mut app);
        assert!(matches!(cell(&app, "n"), CellCompat::Differs(r) if r.contains("1 shapes against 2")));
        assert_eq!(cell(&app, "o"), CellCompat::Fine);
        shell(&mut app, "fontView.setSelection", json!({"glyphs": ["n"]}));
        assert!(app.models.compatibility.reason.starts_with("n cannot interpolate: "), "{}", app.models.compatibility.reason);
        assert!(app.models.compatibility.bars.is_empty(), "no canvas shows");
        parameter_off(&mut app);
        assert_eq!(cell(&app, "n"), CellCompat::Fine);

        // Without an instance between the masters nothing needs them both.
        app.session.execute("edit.undo", &json!({})).unwrap();
        app.session
            .edit("no instances", |font| {
                font.instances = Arc::default();
                Ok(())
            })
            .unwrap();
        settle(&mut app);
        assert_eq!(cell(&app, "n"), CellCompat::Fine);
    }

    /// Opening a font of 3,000 glyphs checks a slice of them, the cells in view first; the others follow pass by pass
    /// and the filter's count is right at the end.
    #[test]
    fn a_large_font_answers_the_cells_in_view_first_and_the_rest_in_slices() {
        let mut app = app(3_000);
        shell(&mut app, "fontView.show", json!({}));
        let unchecked = |app: &FontCraftApp| app.models.font_view.rows.iter().filter(|r| r.compat == CellCompat::Unchecked).count();
        assert!(unchecked(&app) > 1_000, "one frame leaves most of the font unchecked");
        assert!(app.models.compat_scan.waiting());
        let in_view: Vec<String> = (2_950..2_990).map(|i| format!("x{i}")).collect();
        app.sent.push(Msg::View(ViewMsg::UncheckedCells(in_view.clone())));
        let revision = app.view_revision;
        app.drain();
        assert_eq!(app.view_revision, revision, "the cells in view do not project every model again");
        assert!(in_view.iter().all(|n| cell(&app, n) == CellCompat::Fine), "the cells in view are answered on the next pass");
        assert!(unchecked(&app) > 1_000);
        settle(&mut app);
        assert_eq!(unchecked(&app), 0);
        let filter = app.models.sidebar.rows.iter().find(|r| r.path.last().is_some_and(|p| p == "Incompatible masters"));
        if let Some(filter) = filter {
            assert_eq!(filter.count, 1, "the sidebar counts the answers once they are in");
        }

        app.session
            .edit("move", |font| {
                font.glyph_mut("x7")?.layers[0].width = 510.0;
                Ok(())
            })
            .unwrap();
        app.drain();
        assert_eq!(unchecked(&app), 0, "an edit of one glyph is checked on its own pass");
    }
}
