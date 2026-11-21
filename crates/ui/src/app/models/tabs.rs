//! The document area's tab strip: one entry per Edit View tab (`shell::tabs`), after Font View.

use craft::ui::contract::{PlainData, Tracked};

use super::ProjectCx;

/// Most glyph names a tab title shows; more end in the strip's "more" mark.
pub const TITLE_GLYPHS: usize = 4;

/// One Edit View tab as the strip shows it.
#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct TabEntry {
    pub id: u64,
    /// The first [`TITLE_GLYPHS`] glyphs the tab shows.
    pub glyphs: Vec<String>,
    /// The tab shows more glyphs than `glyphs`.
    pub more: bool,
}

#[derive(PlainData, Clone, Debug, Default, PartialEq)]
pub struct TabsModel {
    pub tabs: Vec<TabEntry>,
    /// The showing tab's id; `None` while Font View shows.
    pub active: Option<u64>,
}

pub fn project(model: &mut Tracked<TabsModel>, cx: &mut ProjectCx<'_>) {
    let font = cx.session.font().ok();
    let view = cx.view;
    let tabs = view
        .tabs
        .tabs
        .iter()
        .map(|tab| {
            let mut glyphs = crate::shell::tabs::tab_glyphs(font, view, tab);
            let more = glyphs.len() > TITLE_GLYPHS;
            glyphs.truncate(TITLE_GLYPHS);
            TabEntry { id: tab.id, glyphs, more }
        })
        .collect();
    super::set(model, TabsModel { tabs, active: view.tabs.active });
}
