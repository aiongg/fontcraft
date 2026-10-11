//! The layout-only font Edit View and Text Preview shape with (M8.19;
//! `docs/plans/M8.md` § 3, The preview font).
//!
//! Rows: `features.menuNeedsCompile` (the font the menu lists), `features.previewScope`,
//! `text.shaping`, `textPreview.shaping`.
//!
//! The font holds `head`, `maxp`, `cmap`, `hhea`, `hmtx`, `GDEF`, `GSUB`, `GPOS` (for
//! [`PreviewKind::Full`]) and `post`, built for one master. Glyphs without a code point get a
//! private-use code point of plane 15 or 16, kept in [`PreviewFont::private_use`] beside the
//! font, not in its `cmap`; `fontcraft_shaping` reads them from there.

use fontcraft_fea::FeatureSummary;
use fontcraft_features::EntryDiagnostic;
use fontcraft_model::Font;

use crate::CompileError;

/// First private-use code point of plane 15.
pub const PLANE_15_START: u32 = 0xF_0000;
/// Last private-use code point of plane 15.
pub const PLANE_15_END: u32 = 0xF_FFFD;
/// First private-use code point of plane 16.
pub const PLANE_16_START: u32 = 0x10_0000;
/// Last private-use code point of plane 16.
pub const PLANE_16_END: u32 = 0x10_FFFD;

/// Which layout tables the preview font has.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PreviewKind {
    /// `GSUB` only: Edit View, which positions with live kerning and anchors.
    GsubOnly,
    /// `GSUB` and `GPOS`, generated kerning and marks included: Text Preview.
    Full,
}

/// A compiled preview font.
#[derive(Clone, Debug, PartialEq)]
pub struct PreviewFont {
    pub bytes: Vec<u8>,
    /// The master it was built for.
    pub master: String,
    pub kind: PreviewKind,
    /// Glyph names in glyph-id order, `.notdef` first.
    pub glyph_order: Vec<String>,
    /// (private-use code point, glyph id) for each glyph without a code point.
    pub private_use: Vec<(u32, u16)>,
    /// The features of its `GSUB` and `GPOS`, for the Features menu.
    pub features: Vec<FeatureSummary>,
    /// The compiler's warnings, mapped to the entries.
    pub diagnostics: Vec<EntryDiagnostic>,
}

/// The private-use code point given to the `index`-th unencoded glyph: plane 15 first, then
/// plane 16; `None` past both (131,068 glyphs, more than a font holds).
pub fn private_use_code_point(index: usize) -> Option<u32> {
    let index = u32::try_from(index).ok()?;
    let plane15 = PLANE_15_END - PLANE_15_START + 1;
    if index < plane15 {
        return Some(PLANE_15_START + index);
    }
    let rest = index - plane15;
    (rest <= PLANE_16_END - PLANE_16_START).then(|| PLANE_16_START + rest)
}

/// Builds the preview font of `master`. Feature-code errors come back as
/// [`CompileError::Features`].
pub fn layout_font(font: &Font, master: &str, kind: PreviewKind) -> Result<PreviewFont, CompileError> {
    // M8.19
    let _ = (font, master, kind);
    Err(CompileError::Table { table: "GSUB", message: "the preview font is not available yet".into() })
}
