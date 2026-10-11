//! Hinted rendering: grid-fitted outlines and pixel images of a compiled font at a pixel size,
//! with its TrueType programs run by skrifa's interpreter or its CFF hints applied by skrifa's
//! PostScript hinter (owner decision M10 § 7, question 4 (a): FreeType's behaviour, not
//! Windows').
//!
//! Execution bounds, checked in skrifa 0.48's source: the TrueType interpreter stops a program
//! after 1,000,000 instructions (`MAX_RUN_INSTRUCTIONS`), also counts instructions skipped in
//! untaken branches against that budget, limits loop calls and the call stack, and reports
//! `ExceededExecutionBudget` instead of running on. That is the cap
//! ([`MAX_INSTRUCTIONS_PER_GLYPH`]); skrifa gives no way to lower it. A glyph whose program
//! fails or exceeds it is drawn unhinted and [`HintedOutline::message`] says why.
//!
//! Rows: `hint.tt.renderPreview`, `hint.ps.renderPreview`, `hinting.tt.tool.gridFittedOutline`
//! and `hinting.tt.tool.pixelBackdrop` (engine side), `hint.tt.setRenderingIntent` (the mapping).
//! M10.23 fills the bodies marked `// M10.23:`; fields are private so it may add caches.

use std::sync::Arc;

use fontcraft_geom::BezPath;

use crate::{Image, RenderError};

/// The instructions skrifa runs for one program before it gives up (its own constant).
pub const MAX_INSTRUCTIONS_PER_GLYPH: usize = 1_000_000;
/// Pixel sizes accepted.
pub const MIN_PPEM: u16 = 1;
pub const MAX_PPEM: u16 = 400;
/// Largest font accepted, in bytes.
pub const MAX_FONT_BYTES: usize = 1 << 28;

/// The rendering a preview imitates (the Instructor's intent pop-up).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Intent {
    /// Grayscale anti-aliasing (skrifa's smooth normal target).
    #[default]
    Grayscale,
    /// GDI ClearType: horizontal subpixels, no vertical oversampling; the intent in which the
    /// GDI-only rounding mode rounds.
    GdiClearType,
    /// DirectWrite ClearType: horizontal subpixels with symmetric smoothing.
    DirectWrite,
}

/// A grid-fitted outline in pixels at the requested size, y up.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct HintedOutline {
    pub path: BezPath,
    /// The hinted advance width, in pixels.
    pub advance: f64,
    /// Whether hinting ran; `false` with a message when the glyph was drawn unhinted.
    pub hinted: bool,
    pub message: Option<String>,
}

/// Renders the glyphs of one compiled font, hinted.
#[derive(Clone, Debug)]
pub struct HintedRenderer {
    font: Arc<[u8]>,
    intent: Intent,
}

impl HintedRenderer {
    /// A renderer for `font` (a TrueType or CFF sfnt) in `intent`. The font must parse.
    pub fn new(font: Arc<[u8]>, intent: Intent) -> Result<Self, RenderError> {
        if font.len() > MAX_FONT_BYTES {
            return Err(RenderError::Model(format!("the font is larger than {MAX_FONT_BYTES} bytes")));
        }
        // M10.23: parse with skrifa and keep what the hinting instances need.
        Ok(Self { font, intent })
    }

    pub fn intent(&self) -> Intent {
        self.intent
    }

    /// The font bytes the renderer was made with.
    pub fn font(&self) -> &Arc<[u8]> {
        &self.font
    }

    /// The grid-fitted outline of glyph id `glyph` at `ppem` pixels per em.
    pub fn outline(&self, glyph: u32, ppem: u16) -> Result<HintedOutline, RenderError> {
        check_ppem(ppem)?;
        // M10.23: skrifa HintingInstance at ppem, outline through a pen.
        let _ = glyph;
        Err(RenderError::Model("hinted rendering is not available yet".into()))
    }

    /// The glyph's pixels at `ppem`, one image pixel per device pixel, anti-aliased as the
    /// intent does it.
    pub fn bitmap(&self, glyph: u32, ppem: u16) -> Result<Image, RenderError> {
        check_ppem(ppem)?;
        // M10.23: rasterise the hinted outline with tiny-skia.
        let _ = glyph;
        Err(RenderError::Model("hinted rendering is not available yet".into()))
    }
}

fn check_ppem(ppem: u16) -> Result<(), RenderError> {
    if (MIN_PPEM..=MAX_PPEM).contains(&ppem) { Ok(()) } else { Err(RenderError::BadNumber("the pixel size")) }
}
