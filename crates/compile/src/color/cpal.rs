//! `CPAL` (M11.6): version 0, every palette with the one `Color Palette for CPAL` names first,
//! the colours COLR added after the palette rows, no labels or types.
//!
//! Rows: `CPAL`, `color.export.colorPaletteForCpal`, `color.export.defaultPalette`.

use fontcraft_color::Rgba;
use fontcraft_color::export::CpalPlan;
use write_fonts::tables::cpal::{ColorRecord, Cpal};

use crate::CompileError;

/// The table: `plan`'s palettes, each extended by `added` (the colours COLR uses that are not
/// palette references, the same rows in every palette). Without palettes, `added` is the one
/// palette; a font whose layers use only the text colour gets one empty palette.
pub fn build(plan: &CpalPlan, added: &[Rgba]) -> Result<Vec<u8>, CompileError> {
    let mut palettes: Vec<Vec<Rgba>> = plan.palettes.palettes().iter().map(|p| p.iter().chain(added).copied().collect()).collect();
    if palettes.is_empty() {
        palettes.push(added.to_vec());
    }
    let entries = palettes.first().map_or(0, Vec::len);
    let too_big = || CompileError::Table {
        table: "CPAL",
        message: format!("{} palettes of {entries} colours are more than the table holds (65,535 colours in all)", palettes.len()),
    };
    let num_entries = u16::try_from(entries).map_err(|_| too_big())?;
    let num_palettes = u16::try_from(palettes.len()).map_err(|_| too_big())?;
    let num_records = entries.checked_mul(palettes.len()).and_then(|n| u16::try_from(n).ok()).ok_or_else(too_big)?;
    let indices = (0..palettes.len())
        .map(|p| p.checked_mul(entries).and_then(|n| u16::try_from(n).ok()).ok_or_else(too_big))
        .collect::<Result<Vec<u16>, _>>()?;
    let records: Vec<ColorRecord> = palettes.iter().flatten().map(|c| ColorRecord::new(c.b, c.g, c.r, c.a)).collect();
    let cpal = Cpal::new(num_entries, num_palettes, num_records, Some(records), indices);
    write_fonts::dump_table(&cpal).map_err(|e| CompileError::Table { table: "CPAL", message: e.to_string() })
}

#[cfg(test)]
mod tests {
    use fontcraft_color::export::CpalPlan;
    use fontcraft_color::{Palettes, Rgba};
    use read_fonts::{FontData, FontRead, tables::cpal::Cpal};

    use super::build;

    #[test]
    fn cpal_writes_every_palette_with_the_added_rows_in_each() {
        let red = Rgba::new(255, 0, 0, 255);
        let palettes =
            Palettes::new(vec![vec![Rgba::new(1, 2, 3, 4), Rgba::new(5, 6, 7, 8)], vec![Rgba::new(9, 10, 11, 12), Rgba::new(13, 14, 15, 16)]])
                .unwrap();
        let bytes = build(&CpalPlan { palettes }, &[red]).unwrap();
        let cpal = Cpal::read(FontData::new(&bytes)).unwrap();
        assert_eq!(cpal.version(), 0);
        assert_eq!((cpal.num_palettes(), cpal.num_palette_entries(), cpal.num_color_records()), (2, 3, 6));
        assert_eq!(cpal.color_record_indices().iter().map(|i| i.get()).collect::<Vec<_>>(), [0, 3]);
        let records: Vec<(u8, u8, u8, u8)> = cpal.color_records_array().unwrap().unwrap().iter().map(|c| (c.red, c.green, c.blue, c.alpha)).collect();
        assert_eq!(records, [(1, 2, 3, 4), (5, 6, 7, 8), (255, 0, 0, 255), (9, 10, 11, 12), (13, 14, 15, 16), (255, 0, 0, 255)]);
        assert!(cpal.palette_types_array().is_none() && cpal.palette_labels_array().is_none(), "no labels or types");

        // No palettes at all: the added colours are the one palette.
        let bytes = build(&CpalPlan::default(), &[red]).unwrap();
        let cpal = Cpal::read(FontData::new(&bytes)).unwrap();
        assert_eq!((cpal.num_palettes(), cpal.num_palette_entries()), (1, 1));
    }
}
