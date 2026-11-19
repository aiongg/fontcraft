//! View models: what each panel shows, copied out of the engine into `Tracked` values.
//!
//! One file per area. Each file owns its model types and a `project` function; [`Models::project`] calls every
//! one of them with a one-line arm, and only when the [`Stamp`] moved since the last call.

pub mod font_info;
pub mod glyph;

use craft::ui::contract::Tracked;
use fontcraft_engine::Session;

use super::ViewState;

/// Every view model the panels read.
#[derive(Default)]
pub struct Models {
    /// The stamp the models were last projected at.
    seen: Option<Stamp>,
    /// The shown glyph's width and sidebearings (`panels::glyph_metrics`).
    pub glyph_metrics: Tracked<glyph::GlyphMetrics>,
    /// The Font Info panel (`panels::font_info`).
    pub font_info: Tracked<font_info::FontInfoModel>,
}

/// What a projection reads: the session, for its data and its queries, and the view state.
pub struct ProjectCx<'a> {
    /// The session. Projections read it and run queries through `Session::query`, which refuses edits.
    pub session: &'a mut Session,
    pub view: &'a ViewState,
}

/// When the models need projecting again: the open documents, the active one and its revision, and the view
/// state's revision.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stamp {
    documents: usize,
    active: Option<(u64, u64)>,
    view: u64,
}

impl Stamp {
    /// The stamp of `session` and the view state at `view_revision`.
    pub fn of(session: &Session, view_revision: u64) -> Stamp {
        Stamp { documents: session.documents().len(), active: session.active().map(|d| (d.id, d.revision)), view: view_revision }
    }
}

impl Models {
    /// Projects every model when `stamp` differs from the last projection; returns whether it did.
    pub fn project(&mut self, stamp: Stamp, cx: &mut ProjectCx<'_>) -> bool {
        if self.seen == Some(stamp) {
            return false;
        }
        self.seen = Some(stamp);
        glyph::project(&mut self.glyph_metrics, cx);
        font_info::project(&mut self.font_info, cx);
        true
    }
}

/// Replaces `model`'s value with `value` only when they differ, so an unchanged model keeps its revision.
pub fn set<T: craft::ui::contract::PlainData + PartialEq>(model: &mut Tracked<T>, value: T) {
    if **model != value {
        model.edit(|m| *m = value);
    }
}
