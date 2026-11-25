//! The Designspace section of the Masters tab: a map of the font's axes with the masters, virtual masters and
//! instances on it, the same points as a table, and the problems of the set-up.
//!
//! | Control | Posts |
//! |---|---|
//! | Across, Up (three axes or more) | `font.info.masters.setMapAxes` |
//! | a click on a master | `font.info.masters.select` |
//! | a click on an instance | `font.info.exports.select` |
//! | a drag of a master or instance | `font.master.setAxisLocation` or `font.instance.setAxisLocation` on release, in whole units: one undo step |
//! | a coordinate in the table | the same, for one axis |
//!
//! With one axis the map is a line. Masters are squares, the origin's filled; virtual masters are dashed squares;
//! instances are circles. The selected ones take the accent colour; those with a problem of place (two masters at one
//! location, an extrapolated instance) the warning colour. A virtual master is edited in its parameter, not here.

use std::cell::RefCell;

use craft::behavior::{ColumnKey, ColumnWidth, DisplayOrder, ItemSource};
use craft::theme::variant::Tone;
use craft::theme::{Rgba, Srgba};
use craft::ui::contract::{Bind, ColumnSpec, Entry, Props as _, Show as _, ShowCollection as _, ShowContainer as _};
use craft::ui::display::Text;
use craft::ui::form::Field;
use craft::ui::list::ListRow;
use craft::ui::pickers::Select;
use craft::ui::runtime::commands::CommandRef;
use craft::ui::runtime::env::{Env, Label};
use craft::ui::sections::Section;
use craft::ui::table::Table;
use craft::ui::text::NumberField;
use egui::{Color32, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Ui, Vec2};
use serde_json::{Value, json};

use crate::app::Msg;
use crate::app::models::designspace::{DesignspaceModel, MapPoint, PointKind};
use crate::shell::font_info_masters::select_masters;
use crate::shell::font_info_tabs::select_exports;

craft::ui::words! {
    /// The words of the Designspace section.
    pub struct DesignspaceWords {
        title: "fontcraftDesignspace.title" = "Designspace";
        map: "fontcraftDesignspace.map" = "Designspace map";
        across: "fontcraftDesignspace.across" = "Across";
        up: "fontcraftDesignspace.up" = "Up";
        table: "fontcraftDesignspace.table" = "Masters and instances";
        name: "fontcraftDesignspace.name" = "Name";
        external: "fontcraftDesignspace.external" = "External";
        more: "fontcraftDesignspace.more" = "more problems";
        none: "fontcraftDesignspace.none" = "The font has no master.";
    }
}

/// Most rows the table shows at once.
const TABLE_ROWS: u16 = 10;
/// Height of the map with two axes, and with one, in points.
const MAP_HEIGHT: f32 = 180.0;
const LINE_HEIGHT: f32 = 48.0;
/// Room between the frame and the points at the ends of the axes.
const MARGIN: f32 = 18.0;
/// Half a master's square, an instance's radius, and how far from a point a press still takes it.
const MASTER_HALF: f32 = 5.0;
const INSTANCE_RADIUS: f32 = 4.0;
const REACH: f32 = 9.0;
const NAME_SIZE: f32 = 10.0;
/// How far a drag may take a point past the axis ends, as a share of the axis.
const OVERSHOOT: f64 = 0.25;

/// The name column; the axis columns are numbered, the internal one of each axis before its external one.
const NAME: ColumnKey = ColumnKey::new("name");

fn internal_key(axis: usize) -> ColumnKey {
    ColumnKey::index(u32::try_from(axis.saturating_mul(2)).unwrap_or(u32::MAX))
}

fn external_key(axis: usize) -> ColumnKey {
    ColumnKey::index(u32::try_from(axis.saturating_mul(2).saturating_add(1)).unwrap_or(u32::MAX))
}

pub fn view(ui: &mut Ui, model: &DesignspaceModel, sent: &mut Vec<Msg>) {
    let Some(shown) = model.shown else { return };
    let words = Env::of(ui).words::<DesignspaceWords>();
    Section::new(words.title.clone()).id_salt("fontcraftDesignspace").show(ui, |ui, _| {
        if model.axes.len() > 2 {
            axis_choice(ui, model, &words.across, shown.0, "across", sent);
            axis_choice(ui, model, &words.up, shown.1, "up", sent);
        }
        map(ui, model, &words, shown, sent);
        table(ui, model, &words, sent);
        for p in &model.problems {
            Text::new(p.message.clone()).tone(Tone::Warn).show(ui);
        }
        if model.more_problems > 0 {
            Text::new(format!("{} {}", model.more_problems, words.more.as_str())).tone(Tone::Warn).show(ui);
        }
    });
}

fn axis_choice(ui: &mut Ui, model: &DesignspaceModel, label: &Label, shown: usize, key: &str, sent: &mut Vec<Msg>) {
    let entries: Vec<Entry<usize>> = model.axes.iter().enumerate().map(|(i, a)| Entry::new(i, a.name.clone())).collect();
    Field::new(label.clone()).show(ui, |ui, _| {
        if let Some(&at) = Select::new(label.clone(), Bind::given(shown), &entries).show(ui).events.committed()
            && at != shown
            && let Some(axis) = model.axes.get(at)
        {
            let mut params = serde_json::Map::new();
            params.insert(key.to_string(), json!(axis.tag));
            sent.push(Msg::Shell(CommandRef::with_params("font.info.masters.setMapAxes", Value::Object(params))));
        }
    });
}

// ── The map ─────────────────────────────────────────────────────────────────────────────────────

/// Where coordinates lie in the map's rect.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Frame {
    rect: Rect,
    /// The axes across and up.
    axes: (usize, usize),
    x: (f64, f64),
    /// `None`: one axis, a line.
    y: Option<(f64, f64)>,
}

impl Frame {
    fn new(model: &DesignspaceModel, rect: Rect, axes: (usize, usize)) -> Option<Frame> {
        let range = |i: usize| model.axes.get(i).map(|a| (a.min, a.max));
        let y = if axes.0 == axes.1 { None } else { range(axes.1) };
        Some(Frame { rect: rect.shrink(MARGIN), axes, x: range(axes.0)?, y })
    }

    /// `v` as a share of the range; the middle of a range without extent.
    fn share(v: f64, (lo, hi): (f64, f64)) -> f32 {
        if hi > lo { ((v - lo) / (hi - lo)) as f32 } else { 0.5 }
    }

    fn pos(&self, location: &[f64]) -> Option<Pos2> {
        let value = |i: usize| location.get(i).copied().filter(|v| v.is_finite());
        let x = self.rect.left() + Self::share(value(self.axes.0)?, self.x) * self.rect.width();
        let y = match self.y {
            Some(range) => self.rect.bottom() - Self::share(value(self.axes.1)?, range) * self.rect.height(),
            None => self.rect.center().y,
        };
        Some(Pos2::new(x, y))
    }

    /// The coordinates at `p`, in whole units; at most [`OVERSHOOT`] past the ends. An axis without extent has none.
    fn coordinates(&self, p: Pos2) -> Vec<(usize, f64)> {
        let at = |share: f32, (lo, hi): (f64, f64)| -> Option<f64> {
            let share = f64::from(share);
            (hi > lo && share.is_finite()).then(|| (lo + share.clamp(-OVERSHOOT, 1.0 + OVERSHOOT) * (hi - lo)).round())
        };
        let x = at((p.x - self.rect.left()) / self.rect.width().max(1.0), self.x).map(|v| (self.axes.0, v));
        let y = self.y.and_then(|range| at((self.rect.bottom() - p.y) / self.rect.height().max(1.0), range)).map(|v| (self.axes.1, v));
        x.into_iter().chain(y).collect()
    }
}

/// The point a press at `p` takes: the nearest within reach, masters before instances.
fn hit(model: &DesignspaceModel, frame: &Frame, p: Pos2) -> Option<PointKind> {
    let rank = |k: PointKind| match k {
        PointKind::Master(_) => 0,
        PointKind::Instance(_) => 1,
        PointKind::Virtual(_) => 2,
    };
    let near = model.points.iter().filter(|m| !matches!(m.kind, PointKind::Virtual(_))).filter_map(|m| {
        let d = frame.pos(&m.location)?.distance(p);
        (d <= REACH).then_some((m.kind, d))
    });
    near.min_by(|a, b| a.1.total_cmp(&b.1).then(rank(a.0).cmp(&rank(b.0)))).map(|(kind, _)| kind)
}

/// The command that puts `kind` at these coordinates, the other axes left as they are.
fn place(kind: PointKind, coordinates: &[(usize, f64)]) -> Option<Msg> {
    let last = coordinates.iter().map(|(i, _)| *i).max()?;
    let location: Vec<Value> = (0..=last).map(|i| coordinates.iter().find(|(k, _)| *k == i).map_or(Value::Null, |(_, v)| json!(v))).collect();
    Some(match kind {
        PointKind::Master(i) => Msg::run("font.master.setAxisLocation", json!({ "masters": [i], "location": location })),
        PointKind::Instance(i) => Msg::run("font.instance.setAxisLocation", json!({ "instances": [i], "location": location })),
        PointKind::Virtual(_) => return None,
    })
}

fn select(kind: PointKind) -> Option<Msg> {
    match kind {
        PointKind::Master(i) => Some(Msg::Shell(select_masters(&[i]))),
        PointKind::Instance(i) => Some(Msg::Shell(select_exports(&[i]))),
        PointKind::Virtual(_) => None,
    }
}

fn map(ui: &mut Ui, model: &DesignspaceModel, words: &DesignspaceWords, axes: (usize, usize), sent: &mut Vec<Msg>) {
    let height = if axes.0 == axes.1 { LINE_HEIGHT } else { MAP_HEIGHT };
    let width = ui.available_width().max(MARGIN * 4.0);
    let (rect, response) = ui.allocate_exact_size(Vec2::new(width, height), Sense::click_and_drag());
    let name = words.map.as_str().to_string();
    response.widget_info(|| egui::WidgetInfo::labeled(egui::WidgetType::Other, true, name.clone()));
    let Some(frame) = Frame::new(model, rect, axes) else { return };
    let memory = response.id.with("drag");
    let mut dragged: Option<PointKind> = ui.data(|d| d.get_temp(memory));
    if response.drag_started() {
        dragged = ui.input(|i| i.pointer.press_origin()).and_then(|p| hit(model, &frame, p));
    }
    let pointer = response.interact_pointer_pos();
    if response.drag_stopped() || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
        let moved = dragged.zip(pointer).filter(|_| response.drag_stopped());
        sent.extend(moved.and_then(|(kind, p)| place(kind, &frame.coordinates(p))));
        dragged = None;
    } else if response.clicked() {
        sent.extend(pointer.and_then(|p| hit(model, &frame, p)).and_then(select));
    }
    ui.data_mut(|d| match dragged {
        Some(kind) => {
            d.insert_temp(memory, kind);
        }
        None => d.remove::<PointKind>(memory),
    });
    if ui.is_rect_visible(rect) {
        let moving = dragged.zip(pointer);
        paint(ui, model, &frame, rect, moving);
    }
}

fn color(c: Rgba) -> Color32 {
    Color32::from(Srgba::from(c))
}

fn paint(ui: &Ui, model: &DesignspaceModel, frame: &Frame, rect: Rect, moving: Option<(PointKind, Pos2)>) {
    let env = Env::of(ui);
    let colors = &env.tokens.color;
    let (ink, muted, accent, warn) = (color(colors.text.normal), color(colors.text.muted), color(colors.accent.solid), color(colors.status.warn.fg));
    let painter = ui.painter().with_clip_rect(rect.intersect(ui.clip_rect()));
    let hairline = 1.0 / painter.pixels_per_point().max(0.01);
    let line = Stroke::new(hairline, muted);
    match frame.y {
        Some(_) => {
            painter.rect_stroke(frame.rect, 0.0, line, StrokeKind::Inside);
        }
        None => {
            painter.line_segment([frame.rect.left_center(), frame.rect.right_center()], line);
        }
    }
    for p in &model.points {
        let free = moving.filter(|(kind, _)| *kind == p.kind).map(|(_, at)| at);
        let Some(at) = free.or_else(|| frame.pos(&p.location)) else { continue };
        let tint = if p.problem {
            warn
        } else if p.selected {
            accent
        } else {
            ink
        };
        mark(&painter, p, at, tint, hairline);
        if !matches!(p.kind, PointKind::Instance(_)) || p.selected {
            let below = at + Vec2::new(0.0, MASTER_HALF + hairline);
            painter.text(below, egui::Align2::CENTER_TOP, &p.name, egui::FontId::proportional(NAME_SIZE), tint);
        }
    }
}

fn mark(painter: &egui::Painter, p: &MapPoint, at: Pos2, tint: Color32, hairline: f32) {
    let stroke = Stroke::new(hairline * 2.0, tint);
    let square = Rect::from_center_size(at, Vec2::splat(MASTER_HALF * 2.0));
    match p.kind {
        PointKind::Master(_) => {
            if p.origin {
                painter.rect_filled(square, 0.0, tint);
            } else {
                painter.rect_stroke(square, 0.0, stroke, StrokeKind::Middle);
            }
        }
        PointKind::Virtual(_) => {
            let corners = [square.left_top(), square.right_top(), square.right_bottom(), square.left_bottom(), square.left_top()];
            painter.extend(Shape::dashed_line(&corners, stroke, MASTER_HALF * 0.5, MASTER_HALF * 0.5));
        }
        PointKind::Instance(_) => {
            painter.circle_stroke(at, INSTANCE_RADIUS, stroke);
        }
    }
}

// ── The table ───────────────────────────────────────────────────────────────────────────────────

/// The points in order.
struct Positions(usize);

impl DisplayOrder<usize> for Positions {
    fn len(&self) -> usize {
        self.0
    }

    fn key(&self, at: usize) -> usize {
        at
    }

    fn index_of(&self, key: &usize) -> Option<usize> {
        (*key < self.0).then_some(*key)
    }
}

impl ItemSource<usize> for Positions {}

fn number(v: f64) -> String {
    let r = (v * 100.0).round() / 100.0;
    format!("{}", if r == 0.0 { 0.0 } else { r })
}

fn table(ui: &mut Ui, model: &DesignspaceModel, words: &DesignspaceWords, sent: &mut Vec<Msg>) {
    let mut columns = vec![ColumnSpec::new(NAME, words.name.clone(), ColumnWidth::fill(1.0))];
    for (i, axis) in model.axes.iter().enumerate() {
        columns.push(ColumnSpec::new(internal_key(i), axis.name.clone(), ColumnWidth::Content).numeric());
        columns.push(ColumnSpec::new(external_key(i), words.external.clone(), ColumnWidth::Content).numeric());
    }
    let typed: RefCell<Vec<(PointKind, usize, f64)>> = RefCell::new(Vec::new());
    let _ = Table::<usize>::new(words.table.clone(), columns).empty(words.none.clone()).max_rows(TABLE_ROWS).striped(true).show_items(
        ui,
        &Positions(model.points.len()),
        &mut |at, icx| {
            let Some(p) = model.points.get(*at) else { return ListRow::new(Label::Empty) };
            let mut row = ListRow::new(p.name.clone()).cells(icx.cells()).striped(icx.row.striped).strong(matches!(p.kind, PointKind::Master(_)));
            for (i, axis) in model.axes.iter().enumerate() {
                let Some(value) = p.location.get(i).copied() else { continue };
                if matches!(p.kind, PointKind::Virtual(_)) {
                    row = row.cell(internal_key(i), number(value));
                } else {
                    let label = format!("{} {}", p.name, axis.name);
                    let field = NumberField::new(label, Bind::given(Some(value))).id_salt(("fontcraftDesignspace.value", *at, i));
                    let (typed, kind) = (&typed, p.kind);
                    row = row.cell_field(internal_key(i), field, move |out| {
                        if let Some(Some(v)) = out.edit.events.committed()
                            && *v != value
                        {
                            typed.borrow_mut().push((kind, i, *v));
                        }
                    });
                }
                if let Some(v) = p.external.get(i) {
                    row = row.cell(external_key(i), number(*v));
                }
            }
            row
        },
    );
    sent.extend(typed.take().into_iter().filter_map(|(kind, axis, v)| place(kind, &[(axis, v)])));
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::panic, clippy::indexing_slicing)]
mod tests {
    use std::sync::Arc;

    use craft::host::Host;
    use craft::ui::testing::Fixture;
    use egui::{Pos2, Rect, Vec2};
    use fontcraft_engine::Session;
    use serde_json::{Value, json};

    use super::{Frame, hit, place, view};
    use crate::app::models::designspace::{DesignspaceModel, PointKind};
    use crate::app::{APP, FontCraftApp, Msg};

    /// Weight and Width with Thin (100, 100), Black (900, 100) and Thin Narrow (100, 60), and an instance between.
    fn app() -> FontCraftApp {
        let mut s = Session::with_document();
        let mut run = |id: &str, params: Value| {
            s.execute(id, &params).unwrap_or_else(|e| panic!("{id}: {e}"));
        };
        run("font.axes.add", json!({"name": "Weight", "tag": "wght"}));
        run("font.axes.add", json!({"name": "Width", "tag": "wdth"}));
        run("font.master.add", json!({"name": "Black"}));
        run("font.master.add", json!({"name": "Thin Narrow"}));
        for (i, at) in [[100, 100], [900, 100], [100, 60]].iter().enumerate() {
            run("font.master.setAxisLocation", json!({"masters": [i], "location": at}));
        }
        run("font.instance.add", json!({"styleName": "Between", "axes": {"wght": 500, "wdth": 80}}));
        let mut app = FontCraftApp::with_edit_tab(s, Arc::new(Host::builder(APP).build()));
        app.sent.push(Msg::Shell(craft::ui::runtime::commands::CommandRef::with_params("font.info.selectTab", json!({"tab": "masters"}))));
        app.drain();
        app
    }

    fn model(app: &FontCraftApp) -> DesignspaceModel {
        app.models.font_info.masters_tab.designspace.clone()
    }

    /// A press takes the master under it; the drag's command moves it to the coordinates there, in one undo step.
    #[test]
    fn a_drag_moves_a_master_in_one_undo_step() {
        let mut app = app();
        let m = model(&app);
        let frame = Frame::new(&m, Rect::from_min_size(Pos2::ZERO, Vec2::new(236.0, 136.0)), (0, 1)).unwrap();
        let black = frame.pos(&[900.0, 100.0]).unwrap();
        assert_eq!(hit(&m, &frame, black + Vec2::new(3.0, 2.0)), Some(PointKind::Master(1)));
        assert_eq!(hit(&m, &frame, black - Vec2::new(40.0, 0.0)), None);
        let steps = app.session.doc().unwrap().history.undo.len();
        let to = frame.coordinates(frame.pos(&[700.0, 80.0]).unwrap());
        assert_eq!(to, [(0, 700.0), (1, 80.0)]);
        app.sent.push(place(PointKind::Master(1), &to).unwrap());
        app.drain();
        assert!(app.status.is_empty(), "{}", app.status);
        assert_eq!(app.session.doc().unwrap().history.undo.len(), steps + 1);
        assert_eq!(app.session.font().unwrap().masters[1].location, [700.0, 80.0]);
        assert!(place(PointKind::Virtual(0), &to).is_none());
    }

    /// A duplicated master is named with its original, and marked on the map, until it moves.
    #[test]
    fn a_shared_location_is_named_until_the_master_moves() {
        let mut app = app();
        let shared = |app: &FontCraftApp| model(app).problems.iter().any(|p| p.code == "masters-share-location");
        assert!(!shared(&app), "{:?}", model(&app).problems);
        app.run_now(Msg::run("font.master.duplicate", json!({"masters": [1]})), false).unwrap();
        app.drain();
        let m = model(&app);
        assert!(m.problems.iter().any(|p| p.code == "masters-share-location" && p.message.contains("Black")), "{:?}", m.problems);
        assert_eq!(m.points.iter().filter(|p| p.problem).count(), 2);
        let copy = app.session.font().unwrap().masters.iter().rposition(|m| m.location == [900.0, 100.0]).unwrap();
        app.run_now(Msg::run("font.master.setAxisLocation", json!({"masters": [copy], "location": [900, 60]})), false).unwrap();
        app.drain();
        assert!(!shared(&app), "{:?}", model(&app).problems);
    }

    /// The section shows the map, a field per coordinate and the problems.
    #[test]
    fn the_section_shows_the_map_the_coordinates_and_the_problems() {
        let mut app = app();
        app.run_now(Msg::run("font.master.setAxisLocation", json!({"masters": [2], "location": [900, 100]})), false).unwrap();
        app.drain();
        let m = model(&app);
        let mut fixture = Fixture::new(move |ui| view(ui, &m, &mut Vec::new()));
        fixture.settle();
        assert!(fixture.by_label("Designspace map").is_some(), "the map");
        assert!(fixture.by_label("Black Weight").is_some(), "a field of the table");
        assert!(fixture.by_label("The masters Black and Thin Narrow sit at the same location.").is_some(), "the problem");
    }
}
