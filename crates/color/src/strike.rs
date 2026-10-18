//! Bitmap strikes (M11.8; [`pick_size`] M11.10): the iColor layers of a master grouped by size,
//! for `sbix`, `CBDT` and the SBIX to SVG conversion.
//!
//! Rows: `sbix`, `color.sbix.imagePerSize`, `color.sbix.sameAspectRatio`,
//! `color.sbix.masterLayerBounds`, `color.sbix.sizeScalesImage`,
//! `color.export.sbixToSvgSizeChoice`, `CBDT`.
//!
//! Contract (`docs/plans/M11.md` § 3):
//! - One strike per distinct iColor size, sorted by size, at most [`crate::MAX_STRIKES`];
//!   `sbix` writes them at [`SBIX_PPI`] pixels per inch.
//! - A strike glyph names its layer and the stored image path; the table builder reads the bytes
//!   through an [`crate::AssetStore`] and computes the origin offsets in pixels from
//!   [`StrikeGlyph::placement`]. A missing file leaves the glyph out of that strike, with a
//!   warning, at build time.
//! - Images of one glyph that differ in width ÷ height are exported, with a warning
//!   ([`aspect_problems`]).

use fontcraft_model::{Font, Transform};

use crate::export::Problem;

/// The pixels per inch every `sbix` strike states.
pub const SBIX_PPI: u16 = 72;

/// The images of one size.
#[derive(Clone, Debug, PartialEq)]
pub struct Strike {
    /// Pixels per em.
    pub ppem: u16,
    /// In glyph order of the font.
    pub glyphs: Vec<StrikeGlyph>,
}

/// One glyph's image in a strike.
#[derive(Clone, Debug, PartialEq)]
pub struct StrikeGlyph {
    pub glyph: String,
    /// The iColor layer's id.
    pub layer: String,
    /// The image's stored path, as the [`crate::AssetStore`] takes it.
    pub image: String,
    /// Where the image sits on the layer, in font units (the image's transform).
    pub placement: Transform,
}

/// The strikes of `master_id`'s iColor layers.
pub fn strikes(font: &Font, master_id: &str) -> Vec<Strike> {
    // M11.8
    let _ = (font, master_id);
    Vec::new()
}

/// The size SBIX to SVG takes for a glyph with iColor layers of `sizes` when `wanted` is asked:
/// the exact size, else the smallest larger one, else the largest smaller one.
pub fn pick_size(sizes: &[u16], wanted: u16) -> Option<u16> {
    if sizes.contains(&wanted) {
        return Some(wanted);
    }
    let larger = sizes.iter().copied().filter(|&s| s > wanted).min();
    larger.or_else(|| sizes.iter().copied().filter(|&s| s < wanted).max())
}

/// A warning for each glyph whose images in `strikes` differ in width ÷ height. `image_size`
/// gives an image's pixel size by stored path (`None` when it cannot be read).
pub fn aspect_problems(strikes: &[Strike], image_size: &dyn Fn(&str) -> Option<(u32, u32)>) -> Vec<Problem> {
    // M11.8
    let _ = (strikes, image_size);
    Vec::new()
}
