//! The glyph closure of a subset (M12.17 through components and `cmap`; M12.18 adds the layout
//! closure): each lookup visited at most once per change of the glyph set, contextual nesting at
//! most [`crate::limits::MAX_LOOKUP_NESTING`], rounds bounded by the glyph count.

use crate::Result;
use crate::sfnt::SfntFont;

use super::{Plan, SubsetOptions};

/// The plan for `options` on `font`: the selected glyphs and everything they pull in.
pub fn plan(font: &SfntFont, options: &SubsetOptions) -> Result<Plan> {
    // M12.17
    let _ = font;
    Ok(Plan { options: options.clone(), ..Plan::default() })
}
