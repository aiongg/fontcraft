//! The shaping font and one shaping call (M8.18).

use std::ops::Range;
use std::sync::Arc;

use crate::{MAX_PRIVATE_USE, ShapeError, Tag};

/// A layout font ready for shaping. Cheap to share; the plan cache lives inside.
#[derive(Debug)]
pub struct ShapingFont {
    bytes: Arc<[u8]>,
    /// Code point to glyph id, for glyphs reached through private-use code points.
    private_use: Vec<(u32, u16)>,
    // M8.18: the plan cache (at most `MAX_PLANS`) and the parsed font data.
}

impl ShapingFont {
    /// Reads `bytes` as a font. `private_use` maps the private-use code points the preview font
    /// gave its unencoded glyphs to their glyph ids (at most [`MAX_PRIVATE_USE`]).
    pub fn new(bytes: Arc<[u8]>, private_use: &[(u32, u16)]) -> Result<Self, ShapeError> {
        if private_use.len() > MAX_PRIVATE_USE {
            return Err(ShapeError::TooManyPrivateUse { limit: MAX_PRIVATE_USE });
        }
        read_fonts::FontRef::new(&bytes).map_err(|e| ShapeError::Font(e.to_string()))?;
        Ok(Self { bytes, private_use: private_use.to_vec() })
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// The glyph a private-use code point stands for.
    pub fn private_use_glyph(&self, code_point: u32) -> Option<u16> {
        self.private_use.iter().find(|(cp, _)| *cp == code_point).map(|(_, g)| *g)
    }
}

/// The direction text runs in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Direction {
    #[default]
    LeftToRight,
    RightToLeft,
    TopToBottom,
    BottomToTop,
}

impl Direction {
    pub fn is_horizontal(self) -> bool {
        matches!(self, Direction::LeftToRight | Direction::RightToLeft)
    }
}

/// One feature turned on or off, for the whole text or a byte range of it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FeatureSetting {
    pub tag: Tag,
    /// 0 turns the feature off, 1 on; higher values pick an alternate.
    pub value: u32,
    /// Byte range of the text; `None` for all of it.
    pub range: Option<Range<usize>>,
}

/// What to shape and how.
#[derive(Clone, Copy)]
pub struct ShapeRequest<'a> {
    /// One run: one direction, one script. At most [`crate::MAX_TEXT_BYTES`].
    pub text: &'a str,
    pub direction: Direction,
    /// The ISO 15924 script (`Latn`, `Arab`); `None` guesses it from the text.
    pub script: Option<Tag>,
    /// A BCP 47 language tag (`tr`), which harfrust maps to an OpenType language system;
    /// `None` for the default language system.
    pub language: Option<&'a str>,
    /// Features beyond the shaper's defaults for the script, or turning defaults off. Edit View
    /// passes its active features and turns every `GPOS` feature off; Text Preview passes none.
    /// At most [`crate::MAX_FEATURES`].
    pub features: &'a [FeatureSetting],
    /// The advance of a glyph id from the live model, in font units, replacing the font's
    /// `hmtx` value; `None` from the function keeps the font's. `None` here reads the font.
    pub advances: Option<&'a dyn Fn(u16) -> Option<i32>>,
}

impl<'a> ShapeRequest<'a> {
    /// Left-to-right text with the default features and the font's advances.
    pub fn new(text: &'a str) -> Self {
        Self { text, direction: Direction::LeftToRight, script: None, language: None, features: &[], advances: None }
    }
}

/// One output glyph, in visual order for horizontal text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ShapedGlyph {
    pub glyph: u16,
    /// Byte offset in the request's text of the first character this glyph came from.
    pub cluster: u32,
    pub x_advance: i32,
    pub y_advance: i32,
    pub x_offset: i32,
    pub y_offset: i32,
}

/// Shapes one run of text.
pub fn shape(font: &ShapingFont, req: &ShapeRequest) -> Result<Vec<ShapedGlyph>, ShapeError> {
    // M8.18: harfrust with the cached plan, private-use code points and live advances.
    let _ = (font, req);
    Err(ShapeError::NotBuilt)
}
