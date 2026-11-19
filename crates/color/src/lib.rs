//! FontCraft colour fonts (L2): palettes as typed data, colour layer kinds and their paint order,
//! the paint graph built from the layers, the instance switches resolved into one export plan,
//! bitmap strikes, the SVG document writer, and the asset-store trait image bytes come through.
//! Pure functions over a [`fontcraft_model::Font`]; no I/O. Plan: `docs/plans/M11.md` § 3 and
//! § 4. Behaviour notes: `docs/spec/15-color-fonts.md`; rows in
//! `spec/inventory/15-color-fonts.toml`.
//!
//! | Module | Package | What it gives |
//! |---|---|---|
//! | [`palette`] | M11.2 | `Color Palettes` read and written, a master's own palettes, the edit helpers |
//! | [`dark`] | M13.65 | a palette for dark backgrounds, derived in OKLab |
//! | [`layers`] | M11.3 | colour layer kinds, labels, a glyph's colour layers in paint order |
//! | [`paint`] | M11.7 (M11.6 for solid layers, M11.19 for variation) | the paint graph of a glyph |
//! | [`export`] | M11.6 | the eight instance switches, the export plan every table builder reads |
//! | [`strike`] | M11.8 (M11.10 for [`strike::pick_size`]) | bitmap strikes from iColor layers |
//! | [`svg`] | M11.9 (M11.10 for bitmaps) | SVG documents from paints and bitmaps, the `SVG ` table |
//! | [`svg_import`] | M11.10 | a referenced SVG file rewritten as a glyph document |
//! | [`svg_draw`] | M11.14 | an svg layer's file as filled outlines for the editor |
//! | [`svg_filter`] | M11.17d | an SVG file's filters as data for a raster drawing |
//! | [`assets`] | M11.1 | [`AssetStore`]: stored image path → bytes; [`MemoryAssets`] |
//!
//! Who calls what: Edit View, Font View, Text Preview, the SVG writer and the COLR writer all read
//! the same paint graph ([`paint::glyph_paint`]), so what the canvas shows is what export writes.
//! `fontcraft-compile` reads [`export::plan_master`] and builds the tables it names;
//! `color.describeExport` reads [`export::plan`]. Outlines are not drawn here: a
//! [`paint::ShapeRef`] names shapes, and each caller draws them with its own outline code.
//!
//! Limits are named in [`error`]; every function returns [`ColorError`] instead of panicking.
#![forbid(unsafe_code)]

pub mod assets;
pub mod dark;
pub mod error;
pub mod export;
pub mod layers;
pub mod paint;
pub mod palette;
pub mod strike;
pub mod svg;
pub mod svg_draw;
pub mod svg_filter;
pub mod svg_import;

pub use assets::{AssetError, AssetStore, MemoryAssets};
pub use error::{
    ColorError, MAX_ASSET_BYTES, MAX_COLOR_LAYERS, MAX_LAYER_GLYPHS, MAX_PAINT_DEPTH, MAX_PALETTE_ROWS, MAX_PALETTES, MAX_STOPS, MAX_STRIKES,
    MAX_SVG_DEPTH, MAX_SVG_DOCUMENT_BYTES, MAX_SVG_FILE_BYTES, MAX_SVG_PAINT_VISITS, MAX_SVG_TABLE_BYTES,
};
pub use export::{ExportPlan, Problem, Switches};
pub use layers::{ColorLayerKind, GlyphColor};
pub use paint::{ColorLine, ColorRef, ColorStop, Paint, ShapeRef, StrokeSpec};
pub use palette::{Palettes, Rgba};
pub use strike::{Strike, StrikeGlyph};

#[cfg(test)]
mod tests {
    use fontcraft_model::Font;

    use crate::{AssetStore, ColorError, MemoryAssets, export, palette};

    #[test]
    fn placeholders_fail_or_plan_nothing() {
        let font = Font::new("Test");
        // An empty plan writes no table, so compiling a font is unchanged until M11.6.
        let Some(master) = font.masters.first() else { return };
        let plan = export::plan_master(&font, &master.id, &export::Switches::default());
        assert!(plan.tables().is_empty());
        assert!(matches!(palette::read(&font, None), Err(ColorError::NotYetImplemented(_)) | Ok(_)));
        let mut assets = MemoryAssets::new();
        assets.insert("a.png", vec![1, 2, 3]);
        assert!(assets.exists("a.png") && !assets.exists("b.png"));
        assert_eq!(assets.read("a.png").map(|b| b.len()), Ok(3));
    }
}
