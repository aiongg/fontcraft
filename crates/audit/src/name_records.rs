//! The one reader of `name` table records every check and the report export use. A hostile table
//! can point 65,535 records at the same 64 KiB string, so the text decoded is charged to a byte
//! budget the caller holds ([`MAX_NAME_TEXT_BYTES`] for one font).

use read_fonts::{FontRef, TableProvider};

/// Most bytes of name strings decoded for one font; records past it are read without their text.
pub const MAX_NAME_TEXT_BYTES: usize = 4 << 20;

/// One record of the `name` table.
pub(crate) struct NameText {
    pub platform: u16,
    pub encoding: u16,
    pub language: u16,
    pub id: u16,
    /// `None` when the encoding is not one we decode (anything but Unicode, Windows Unicode and
    /// Macintosh Roman), the string is out of bounds, or the budget ran out.
    pub text: Option<String>,
    /// Whether the string lies inside the table's string data.
    pub in_bounds: bool,
}

/// The records [`read`] kept, and whether the budget ran out before all their text was decoded.
pub(crate) struct NameRecords {
    pub records: Vec<NameText>,
    pub over_budget: bool,
}

/// Why a `name` table could not be read at all.
pub(crate) const NO_NAME_TABLE: &str = "the font has no readable 'name' table";
pub(crate) const SHORT_NAME_TABLE: &str = "the 'name' table is shorter than its record count says";

/// Reads the records of `font`'s `name` table that `keep(platform, encoding, language, id)`
/// accepts, decoding their text while `budget` (bytes of string data) lasts and charging it.
/// Fails with a reason when the table is missing or shorter than its record count.
pub(crate) fn read(font: &FontRef<'_>, budget: &mut usize, keep: impl Fn(u16, u16, u16, u16) -> bool) -> Result<NameRecords, &'static str> {
    let name = font.name().map_err(|_| NO_NAME_TABLE)?;
    let data = name.string_data();
    let records = name.name_record();
    // read-fonts answers an empty list when the records run past the end of the table.
    if records.len() != usize::from(name.count()) {
        return Err(SHORT_NAME_TABLE);
    }
    let mut out = Vec::new();
    let mut over_budget = false;
    for r in records {
        let (platform, encoding, language, id) = (r.platform_id(), r.encoding_id(), r.language_id(), r.name_id().to_u16());
        if !keep(platform, encoding, language, id) {
            continue;
        }
        let len = usize::from(r.length());
        let string = r.string(data).ok();
        let in_bounds = string.is_some();
        let decodable = matches!((platform, encoding), (0, _) | (1, 0) | (3, 0) | (3, 1) | (3, 10));
        let text = match string {
            Some(s) if decodable => {
                if len > *budget {
                    over_budget = true;
                    None
                } else {
                    *budget -= len;
                    Some(s.chars().collect::<String>())
                }
            }
            _ => None,
        };
        out.push(NameText { platform, encoding, language, id, text, in_bounds });
    }
    Ok(NameRecords { records: out, over_budget })
}

/// Every record.
pub(crate) fn all(_: u16, _: u16, _: u16, _: u16) -> bool {
    true
}

/// The reason a check gives when the budget ran out.
pub(crate) fn over_budget_reason() -> String {
    format!("the name table holds more than {MAX_NAME_TEXT_BYTES} bytes of text; the records past that were not read")
}
