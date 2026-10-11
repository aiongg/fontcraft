//! The Edit View camera and the tab's text layout: where a tab's glyphs sit on the canvas.
//!
//! A tab's [`TabView`] holds a scale and the text point at the canvas centre. The tools crate's
//! [`fontcraft_tools::View`] is the transform for the edited glyph, placed where the glyph sits
//! in the text; [`tools_view`] turns one into the other, so the canvas and the tools share
//! one transform.

use std::cell::RefCell;
use std::sync::Arc;

use fontcraft_engine::Session;
use fontcraft_geom::Point;
use fontcraft_model::{Font, Layer, Master, metric};
use fontcraft_spacing::kerning::KernIndex;
use fontcraft_textlayout::{DEFAULT_LINE_WIDTH, KerningMode, Layout, Options};
use fontcraft_tools::View;

use crate::state::{Tab, TabView, WritingDirection};

/// The master a tab draws its glyphs in: the one chosen with `view.selectMaster`, else the
/// first.
pub fn tab_master<'f>(font: &'f Font, tab: &Tab) -> Option<&'f Master> {
    tab.master.as_deref().and_then(|id| font.masters.iter().find(|m| m.id == id)).or_else(|| font.masters.first())
}

/// The layer a tab draws glyph `name` in: the layer picked for it in the Layers section, else its
/// layer of the tab's master.
pub fn shown_layer<'f>(font: &'f Font, tab: &Tab, name: &str) -> Option<&'f Layer> {
    let glyph = font.glyph(name)?;
    if let Some(id) = tab.layers.get(name)
        && let Some(layer) = glyph.layers.iter().find(|l| l.id == *id)
    {
        return Some(layer);
    }
    glyph.layer(&tab_master(font, tab)?.id)
}

/// The active layer: the layer the tab edits its edited glyph in.
pub fn active_layer<'f>(font: &'f Font, tab: &Tab) -> Option<&'f Layer> {
    shown_layer(font, tab, tab.edited_glyph()?)
}

/// The side of the active layer the tab edits, as a layer (`fontcraft_tools::side_view`): the
/// canvas draws it as the edited outline and its other side behind it.
pub fn edited_layer<'f>(font: &'f Font, tab: &Tab) -> Option<std::borrow::Cow<'f, Layer>> {
    active_layer(font, tab).map(|l| fontcraft_tools::side_view(l, tab.side()))
}

/// The active master: the master of the active layer, else the tab's master. The metric lines,
/// zones and fitting follow it.
pub fn active_master<'f>(font: &'f Font, tab: &Tab) -> Option<&'f Master> {
    active_layer(font, tab).and_then(|l| font.masters.iter().find(|m| m.id == l.master)).or_else(|| tab_master(font, tab))
}

/// Most tab layouts kept at once.
const KEPT_LAYOUTS: usize = 8;

/// What a tab's layout depends on besides the font's glyphs.
#[derive(Clone, Debug, PartialEq)]
struct LayoutKey {
    tab: u64,
    /// The document's id and revision.
    doc: (u64, u64),
    text: u64,
    master: String,
    direction: WritingDirection,
    kerning: KerningMode,
}

/// The tabs' text layouts, each kept while its document revision, text revision, master,
/// direction and kerning mode stay the same, so a frame lays text out only after a change. It
/// also keeps the kerning index the layouts read.
#[derive(Default)]
pub struct LayoutCache {
    inner: RefCell<LayoutCacheInner>,
    /// The sidebearings the text-mode metrics labels show.
    pub sidebearings: RefCell<super::text::Sidebearings>,
}

#[derive(Default)]
struct LayoutCacheInner {
    kern: Option<Arc<KernIndex>>,
    entries: Vec<(LayoutKey, Arc<Layout>)>,
}

impl LayoutCache {
    /// Hands the cache the session's kerning index when a layout of `tab` would have to be
    /// computed and its own index no longer describes the font.
    pub fn prepare(&self, session: &mut Session, tab: &Tab) {
        let Ok(font) = session.font() else { return };
        let Some(key) = key(session, font, tab) else { return };
        let inner = self.inner.borrow();
        let fresh =
            inner.entries.iter().any(|(k, _)| *k == key) || !tab.kerning.shows_kerning() || inner.kern.as_ref().is_some_and(|k| k.is_current(font));
        drop(inner);
        if !fresh && let Ok(index) = session.kern_index() {
            self.inner.borrow_mut().kern = Some(index);
        }
    }
}

fn key(session: &Session, font: &Font, tab: &Tab) -> Option<LayoutKey> {
    let doc = session.active().map(|d| (d.id, d.revision))?;
    let master = tab_master(font, tab)?.id.clone();
    Some(LayoutKey { tab: tab.id, doc, text: tab.text.revision(), master, direction: tab.direction, kerning: tab.kerning })
}

/// The layout of a tab's text in the tab's master, with its direction and kerning mode, and the
/// line height of the master (`EditView Line Height`, else the computed one). Cached in `cache`;
/// `None` without a document or a master.
pub fn text_layout(session: &Session, cache: &LayoutCache, tab: &Tab) -> Option<Arc<Layout>> {
    let font = session.font().ok()?;
    let key = key(session, font, tab)?;
    let mut inner = cache.inner.borrow_mut();
    if let Some((_, layout)) = inner.entries.iter().find(|(k, _)| *k == key) {
        return Some(layout.clone());
    }
    let index = match &inner.kern {
        Some(index) if tab.kerning.shows_kerning() && index.is_current(font) => Some(index.clone()),
        _ if tab.kerning.shows_kerning() => {
            let index = Arc::new(KernIndex::new(font));
            inner.kern = Some(index.clone());
            Some(index)
        }
        _ => None,
    };
    let options = Options {
        direction: tab.direction.layout(),
        kerning: tab.kerning,
        line_width: DEFAULT_LINE_WIDTH,
        index: index.as_deref(),
        ..Default::default()
    };
    let layout = Arc::new(fontcraft_textlayout::layout(font, &key.master, &tab.text, &options).ok()?);
    inner.entries.retain(|(k, _)| k.tab != key.tab);
    if inner.entries.len() >= KEPT_LAYOUTS {
        inner.entries.remove(0);
    }
    inner.entries.push((key, layout.clone()));
    Some(layout)
}

/// Where the edited glyph's origin sits in the tab's text, in font units (`[0, 0]` when nothing
/// is edited).
pub fn edited_origin(session: &Session, cache: &LayoutCache, tab: &Tab) -> [f64; 2] {
    let Some(index) = tab.edited_index() else { return [0.0, 0.0] };
    text_layout(session, cache, tab).and_then(|l| l.placed.get(index).map(|p| [p.x, p.y])).unwrap_or([0.0, 0.0])
}

/// The advance width of the edited glyph in its active layer (0 when there is none).
pub fn edited_width(font: &Font, tab: &Tab) -> f64 {
    active_layer(font, tab).map_or(0.0, |l| if l.width.is_finite() { l.width.max(0.0) } else { 0.0 })
}

/// The edited glyph's transform on a canvas whose centre is `center` (screen points); `origin`
/// is where the glyph sits in the text ([`edited_origin`]).
pub fn tools_view(view: &TabView, origin: [f64; 2], center: Point) -> View {
    let s = TabView::clamped_scale(view.scale);
    View::new(s, Point::new(center.x + (origin[0] - view.center[0]) * s, center.y + (view.center[1] - origin[1]) * s))
}

/// Descender and ascender of a master, in font units.
pub fn vertical_span(font: &Font, master: &Master) -> (f64, f64) {
    let upm = f64::from(font.units_per_em);
    let ascender = font.metric_pos(master, metric::ASCENDER, upm * 0.8);
    let descender = font.metric_pos(master, metric::DESCENDER, -upm * 0.2);
    if ascender > descender { (descender, ascender) } else { (-upm * 0.2, upm * 0.8) }
}

/// The camera that fits the edited glyph's descender-to-ascender span (and its width, when that
/// is the tighter limit) into a canvas of `canvas` points, leaving `margin` (a fraction) free on
/// every side.
pub fn fit(session: &Session, cache: &LayoutCache, tab: &Tab, canvas: [f64; 2], margin: f64) -> TabView {
    let Ok(font) = session.font() else { return TabView::default() };
    let Some(master) = active_master(font, tab) else { return TabView::default() };
    let (descender, ascender) = vertical_span(font, master);
    let [x, y] = edited_origin(session, cache, tab);
    let width = edited_width(font, tab);
    let mut v = zoom_to_rect([x, y + descender, width, ascender - descender], canvas, margin);
    if width <= 0.0 {
        v.center[0] = x;
    }
    v
}

/// The camera that shows the line rectangle `[x, y, width, height]` as large as fits.
pub fn zoom_to_rect(rect: [f64; 4], canvas: [f64; 2], margin: f64) -> TabView {
    let [x, y, w, h] = rect;
    let room = 1.0 - 2.0 * margin.clamp(0.0, 0.45);
    let fits = |space: f64, size: f64| if size > 0.0 && space > 0.0 { space * room / size } else { f64::INFINITY };
    let scale = fits(canvas[0], w).min(fits(canvas[1], h));
    TabView { scale: TabView::clamped_scale(if scale.is_finite() { scale } else { 1.0 }), center: [x + w / 2.0, y + h / 2.0] }
}
