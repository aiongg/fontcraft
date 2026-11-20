//! `CPAL` (M11.6, M13.65): every palette with the one `Color Palette for CPAL` names first, the
//! colours COLR added after the palette rows. Version 1 when the `Color Palettes` value gives a
//! palette a background style or a name, or a colour a name (the version 4 form): background
//! styles become palette types, names become palette and palette entry labels, each a new
//! `name` ID. Otherwise version 0 with no labels or types, as both reference compilers write it.
//!
//! Rows: `CPAL`, `color.export.colorPaletteForCpal`, `color.export.defaultPalette`,
//! `color.palette.multiplePalettes`, `color.generateDarkPalette`.
//!
//! The version 1 layout (the order of the arrays after the colour records, and an offset of 0 for
//! an array that would hold only defaults) follows the compile method of fontTools'
//! `ttLib/tables/C_P_A_L_.py` (MIT); see `NOTICE`.

use std::collections::HashMap;

use fontcraft_color::Rgba;
use fontcraft_color::export::CpalPlan;
use write_fonts::tables::cpal::{ColorRecord, Cpal};

use crate::CompileError;

/// Palette type bit 0: the palette suits light backgrounds.
const USABLE_WITH_LIGHT_BACKGROUND: u32 = 0x0001;
/// Palette type bit 1: the palette suits dark backgrounds.
const USABLE_WITH_DARK_BACKGROUND: u32 = 0x0002;
/// A label that names nothing.
const NO_NAME_ID: u16 = 0xFFFF;
/// The font-specific name IDs run from 256 to 32,767.
const FIRST_FONT_NAME_ID: u16 = 256;
const LAST_FONT_NAME_ID: u16 = 32_767;
/// Most UTF-16 bytes the label names may add to the `name` table, whose string offsets are 16-bit;
/// past it, palettes and colours go without labels.
const MAX_LABEL_BYTES: usize = 32_768;

/// The table and the `name` records its labels point to, (ID, text), Windows English.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Built {
    pub table: Vec<u8>,
    pub names: Vec<(u16, String)>,
}

/// The table: `plan`'s palettes, each extended by `added` (the colours COLR uses that are not
/// palette references, the same rows in every palette). Without palettes, `added` is the one
/// palette; a font whose layers use only the text colour gets one empty palette. Labels take
/// name IDs from `first_name_id` (at least 256) upwards, one per distinct text.
pub fn build(plan: &CpalPlan, added: &[Rgba], first_name_id: u16, warnings: &mut Vec<String>) -> Result<Built, CompileError> {
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

    let types: Vec<u32> = (0..palettes.len())
        .map(|p| {
            plan.info
                .get(p)
                .map_or(0, |i| (if i.light { USABLE_WITH_LIGHT_BACKGROUND } else { 0 }) | (if i.dark { USABLE_WITH_DARK_BACKGROUND } else { 0 }))
        })
        .collect();
    let mut labels = Labels::new(first_name_id);
    let palette_labels: Vec<u16> = (0..palettes.len()).map(|p| labels.id(plan.info.get(p).and_then(|i| i.name.as_deref()), warnings)).collect();
    // The rows COLR added have no names.
    let entry_labels: Vec<u16> = (0..entries).map(|row| labels.id(plan.color_names.get(row).and_then(|n| n.as_deref()), warnings)).collect();

    let version_1 = types.iter().any(|t| *t != 0) || palette_labels.iter().chain(&entry_labels).any(|l| *l != NO_NAME_ID);
    if !version_1 {
        let records: Vec<ColorRecord> = palettes.iter().flatten().map(|c| ColorRecord::new(c.b, c.g, c.r, c.a)).collect();
        let cpal = Cpal::new(num_entries, num_palettes, num_records, Some(records), indices);
        let table = write_fonts::dump_table(&cpal).map_err(|e| CompileError::Table { table: "CPAL", message: e.to_string() })?;
        return Ok(Built { table, names: Vec::new() });
    }
    let header = Header { num_entries, num_palettes, num_records };
    let table = version_1_table(&header, &palettes, &indices, &types, &palette_labels, &entry_labels).ok_or_else(too_big)?;
    Ok(Built { table, names: labels.records })
}

/// Name IDs for label texts, handed out in order; the same text gets the same ID.
struct Labels {
    next: Option<u16>,
    ids: HashMap<String, u16>,
    records: Vec<(u16, String)>,
    bytes: usize,
    warned: bool,
}

impl Labels {
    fn new(first: u16) -> Self {
        Labels { next: Some(first.max(FIRST_FONT_NAME_ID)), ids: HashMap::new(), records: Vec::new(), bytes: 0, warned: false }
    }

    fn id(&mut self, text: Option<&str>, warnings: &mut Vec<String>) -> u16 {
        let Some(text) = text.filter(|t| !t.is_empty()) else { return NO_NAME_ID };
        if let Some(id) = self.ids.get(text) {
            return *id;
        }
        let bytes = self.bytes.saturating_add(2 * text.encode_utf16().count());
        match self.next.filter(|id| *id <= LAST_FONT_NAME_ID && bytes <= MAX_LABEL_BYTES) {
            Some(id) => {
                self.next = id.checked_add(1);
                self.bytes = bytes;
                self.ids.insert(text.to_string(), id);
                self.records.push((id, text.to_string()));
                id
            }
            None => {
                if !self.warned {
                    self.warned = true;
                    warnings.push(format!(
                        "CPAL: the palette and colour names need more name IDs (up to {LAST_FONT_NAME_ID}) or name table space than is left, so some have no label"
                    ));
                }
                NO_NAME_ID
            }
        }
    }
}

struct Header {
    num_entries: u16,
    num_palettes: u16,
    num_records: u16,
}

/// The version 1 table; `None` when an offset does not fit 32 bits, which the 16-bit counts
/// rule out.
fn version_1_table(h: &Header, palettes: &[Vec<Rgba>], indices: &[u16], types: &[u32], labels: &[u16], entry_labels: &[u16]) -> Option<Vec<u8>> {
    let any_type = types.iter().any(|t| *t != 0);
    let any_label = labels.iter().any(|l| *l != NO_NAME_ID);
    let any_entry_label = entry_labels.iter().any(|l| *l != NO_NAME_ID);
    let first_record = 12 + 2 * indices.len() + 12;
    let mut pos = first_record + 4 * usize::from(h.num_records);
    let mut place = |present: bool, len: usize| -> Option<u32> {
        if !present {
            return Some(0);
        }
        let at = u32::try_from(pos).ok()?;
        pos += len;
        Some(at)
    };
    let types_at = place(any_type, 4 * types.len())?;
    let labels_at = place(any_label, 2 * labels.len())?;
    let entry_labels_at = place(any_entry_label, 2 * entry_labels.len())?;

    let mut out = Vec::with_capacity(pos);
    for v in [1, h.num_entries, h.num_palettes, h.num_records] {
        out.extend(v.to_be_bytes());
    }
    out.extend(u32::try_from(first_record).ok()?.to_be_bytes());
    for i in indices {
        out.extend(i.to_be_bytes());
    }
    for at in [types_at, labels_at, entry_labels_at] {
        out.extend(at.to_be_bytes());
    }
    for c in palettes.iter().flatten() {
        out.extend([c.b, c.g, c.r, c.a]);
    }
    if any_type {
        types.iter().for_each(|t| out.extend(t.to_be_bytes()));
    }
    if any_label {
        labels.iter().for_each(|l| out.extend(l.to_be_bytes()));
    }
    if any_entry_label {
        entry_labels.iter().for_each(|l| out.extend(l.to_be_bytes()));
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use fontcraft_color::export::CpalPlan;
    use fontcraft_color::palette::PaletteInfo;
    use fontcraft_color::{Palettes, Rgba};
    use read_fonts::{FontData, FontRead, tables::cpal::Cpal};

    use super::build;

    #[test]
    fn cpal_writes_every_palette_with_the_added_rows_in_each() {
        let red = Rgba::new(255, 0, 0, 255);
        let palettes =
            Palettes::new(vec![vec![Rgba::new(1, 2, 3, 4), Rgba::new(5, 6, 7, 8)], vec![Rgba::new(9, 10, 11, 12), Rgba::new(13, 14, 15, 16)]])
                .unwrap();
        let built = build(&CpalPlan { palettes, ..CpalPlan::default() }, &[red], 256, &mut Vec::new()).unwrap();
        let cpal = Cpal::read(FontData::new(&built.table)).unwrap();
        assert_eq!(cpal.version(), 0);
        assert_eq!((cpal.num_palettes(), cpal.num_palette_entries(), cpal.num_color_records()), (2, 3, 6));
        assert_eq!(cpal.color_record_indices().iter().map(|i| i.get()).collect::<Vec<_>>(), [0, 3]);
        let records: Vec<(u8, u8, u8, u8)> = cpal.color_records_array().unwrap().unwrap().iter().map(|c| (c.red, c.green, c.blue, c.alpha)).collect();
        assert_eq!(records, [(1, 2, 3, 4), (5, 6, 7, 8), (255, 0, 0, 255), (9, 10, 11, 12), (13, 14, 15, 16), (255, 0, 0, 255)]);
        assert!(cpal.palette_types_array().is_none() && cpal.palette_labels_array().is_none(), "no labels or types");
        assert!(built.names.is_empty());

        // No palettes at all: the added colours are the one palette.
        let built = build(&CpalPlan::default(), &[red], 256, &mut Vec::new()).unwrap();
        let cpal = Cpal::read(FontData::new(&built.table)).unwrap();
        assert_eq!((cpal.num_palettes(), cpal.num_palette_entries()), (1, 1));
    }

    #[test]
    fn background_styles_and_names_write_version_1_with_types_and_labels() {
        let red = Rgba::new(255, 0, 0, 255);
        let palettes = Palettes::new(vec![vec![Rgba::new(1, 2, 3, 255)], vec![Rgba::new(250, 251, 252, 255)], vec![Rgba::new(7, 7, 7, 7)]]).unwrap();
        let plan = CpalPlan {
            palettes,
            info: vec![
                PaletteInfo { name: Some("Day".into()), light: true, dark: false },
                PaletteInfo { name: Some("Night".into()), light: false, dark: true },
                PaletteInfo::default(),
            ],
            color_names: vec![Some("Day".into())],
        };
        let built = build(&plan, &[red], 300, &mut Vec::new()).unwrap();
        let cpal = Cpal::read(FontData::new(&built.table)).unwrap();
        assert_eq!(cpal.version(), 1);
        assert_eq!((cpal.num_palettes(), cpal.num_palette_entries(), cpal.num_color_records()), (3, 2, 6));
        let records: Vec<(u8, u8, u8, u8)> = cpal.color_records_array().unwrap().unwrap().iter().map(|c| (c.red, c.green, c.blue, c.alpha)).collect();
        assert_eq!(records[2..4], [(250, 251, 252, 255), (255, 0, 0, 255)]);
        let types: Vec<u32> = cpal.palette_types_array().unwrap().unwrap().iter().map(|t| t.get().bits()).collect();
        assert_eq!(types, [1, 2, 0], "light, dark, neither");
        let labels: Vec<u16> = cpal.palette_labels_array().unwrap().unwrap().iter().map(|l| l.get().to_u16()).collect();
        assert_eq!(labels, [300, 301, 0xFFFF]);
        let entry_labels: Vec<u16> = cpal.palette_entry_labels_array().unwrap().unwrap().iter().map(|l| l.get().to_u16()).collect();
        assert_eq!(entry_labels, [300, 0xFFFF], "the same text shares its ID; the added row has no name");
        assert_eq!(built.names, [(300, "Day".to_string()), (301, "Night".to_string())]);

        // Types alone still make version 1, without label arrays.
        let plan = CpalPlan { info: vec![PaletteInfo { dark: true, ..PaletteInfo::default() }], color_names: Vec::new(), ..plan };
        let built = build(&plan, &[], 256, &mut Vec::new()).unwrap();
        let cpal = Cpal::read(FontData::new(&built.table)).unwrap();
        assert_eq!(cpal.version(), 1);
        assert!(cpal.palette_labels_array().is_none() && cpal.palette_entry_labels_array().is_none());
        assert_eq!(cpal.palette_types_array().unwrap().unwrap().iter().map(|t| t.get().bits()).collect::<Vec<_>>(), [2, 0, 0]);

        // Out of name IDs: labels left out with one warning.
        let named = |n: &str| PaletteInfo { name: Some(n.into()), ..PaletteInfo::default() };
        let plan = CpalPlan { info: vec![named("A"), named("B"), named("A")], ..plan };
        let mut warnings = Vec::new();
        let built = build(&plan, &[], 32_767, &mut warnings).unwrap();
        assert_eq!(built.names, [(32_767, "A".to_string())]);
        let cpal = Cpal::read(FontData::new(&built.table)).unwrap();
        let labels: Vec<u16> = cpal.palette_labels_array().unwrap().unwrap().iter().map(|l| l.get().to_u16()).collect();
        assert_eq!(labels, [32_767, 0xFFFF, 32_767]);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
    }
}
