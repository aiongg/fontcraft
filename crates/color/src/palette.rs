//! Colour palettes (M11.2): the `Color Palettes` parameter as typed data, a master's own palettes,
//! and the edits the palette commands make.
//!
//! Rows: `colorPalette.*`, `color.palette.fontLevelParameter`, `color.palette.indexFromZero`,
//! `color.palette.multiplePalettes`, `color.palette.perMaster`, `Color Palettes`.
//!
//! Contract (`docs/plans/M11.md` § 3, § 4 › M11.2):
//! - Every palette has the same number of rows ([`Palettes::rows`]); [`Palettes::new`] refuses a
//!   ragged table, and every edit keeps the invariant.
//! - Rows count from 0. Row 65,535 is the text colour and is never stored.
//! - [`read`] parses the version 2 (`"r,g,b,a"` strings), version 3 (`[r, g, b, a]` integers
//!   0–255) and version 4 (floats 0–1) forms. A master's own `Color Palettes` wins over the font's
//!   for that master.
//! - A new row is black in every palette; a new palette copies palette 0; the last palette cannot
//!   be removed; a removed row renumbers the rows after it (the engine renumbers the layers in
//!   the same undo step and refuses while a layer uses the row).
//! - Pure functions; an edit that fails returns an error and changes nothing.

use fontcraft_model::plist::{Array, Value};
use fontcraft_model::{Font, Param};

use crate::{ColorError, MAX_PALETTE_ROWS, MAX_PALETTES};

/// The parameter's name, on the font or on a master.
pub const COLOR_PALETTES_PARAM: &str = "Color Palettes";
/// The palette index `COLR` and the palette layers use for the text colour.
pub const FOREGROUND_INDEX: u16 = 0xFFFF;

/// A palette colour: red, green, blue and alpha, 0–255 each (what `CPAL` stores).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba {
    /// The colour of a new row.
    pub const BLACK: Rgba = Rgba { r: 0, g: 0, b: 0, a: 255 };

    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Self { r, g, b, a }
    }

    /// `#RRGGBBAA`.
    pub fn hex(&self) -> String {
        format!("#{:02X}{:02X}{:02X}{:02X}", self.r, self.g, self.b, self.a)
    }

    /// `#RRGGBB` (opaque) or `#RRGGBBAA`, either case; `None` for anything else.
    pub fn parse_hex(text: &str) -> Option<Rgba> {
        // M11.2: the commands' colour parameter.
        let _ = text;
        None
    }

    /// The colour as components from 0 to 1, for drawing.
    pub fn to_f64(&self) -> [f64; 4] {
        [self.r, self.g, self.b, self.a].map(|c| f64::from(c) / 255.0)
    }
}

/// A font's or a master's palettes: `palettes()[p][row]`. Every palette has [`Palettes::rows`]
/// entries.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Palettes {
    rows: usize,
    palettes: Vec<Vec<Rgba>>,
}

impl Palettes {
    /// Palettes from a table, refused when ragged or beyond [`MAX_PALETTES`] or
    /// [`MAX_PALETTE_ROWS`]. No palettes at all is the empty value.
    pub fn new(palettes: Vec<Vec<Rgba>>) -> Result<Palettes, ColorError> {
        if palettes.len() > MAX_PALETTES {
            return Err(ColorError::LimitExceeded { what: "palettes", limit: MAX_PALETTES });
        }
        let rows = palettes.first().map_or(0, Vec::len);
        if rows > MAX_PALETTE_ROWS {
            return Err(ColorError::LimitExceeded { what: "colours in a palette", limit: MAX_PALETTE_ROWS });
        }
        if palettes.iter().any(|p| p.len() != rows) {
            return Err(ColorError::BadPalettes("the palettes have different numbers of colours".into()));
        }
        Ok(Palettes { rows, palettes })
    }

    /// The number of colours in every palette.
    pub fn rows(&self) -> usize {
        self.rows
    }

    pub fn palettes(&self) -> &[Vec<Rgba>] {
        &self.palettes
    }

    pub fn palette(&self, index: usize) -> Option<&[Rgba]> {
        self.palettes.get(index).map(Vec::as_slice)
    }

    pub fn get(&self, palette: usize, row: usize) -> Option<Rgba> {
        self.palettes.get(palette)?.get(row).copied()
    }

    pub fn is_empty(&self) -> bool {
        self.palettes.is_empty()
    }

    /// A row of `color` appended to every palette (Add Color). With no palette yet, the first
    /// palette is made.
    pub fn add_row(&self, color: Rgba) -> Result<Palettes, ColorError> {
        // M11.2
        let _ = color;
        Err(ColorError::NotYetImplemented("adding a palette colour"))
    }

    /// Row `row` removed from every palette; the rows after it move down by one.
    pub fn remove_row(&self, row: usize) -> Result<Palettes, ColorError> {
        // M11.2
        let _ = row;
        Err(ColorError::NotYetImplemented("removing a palette colour"))
    }

    /// Colour `row` of palette `palette` set to `color`.
    pub fn set(&self, palette: usize, row: usize, color: Rgba) -> Result<Palettes, ColorError> {
        // M11.2
        let _ = (palette, row, color);
        Err(ColorError::NotYetImplemented("setting a palette colour"))
    }

    /// A copy of palette 0 appended.
    pub fn add_palette(&self) -> Result<Palettes, ColorError> {
        // M11.2
        Err(ColorError::NotYetImplemented("adding a palette"))
    }

    /// Palette `index` removed; refused for the last one ([`ColorError::LastPalette`]).
    pub fn remove_palette(&self, index: usize) -> Result<Palettes, ColorError> {
        // M11.2
        let _ = index;
        Err(ColorError::NotYetImplemented("removing a palette"))
    }
}

/// The palettes that apply to `master` (its own `Color Palettes` when it has one, else the
/// font's), or the font's when `master` is `None`. No parameter: empty palettes.
pub fn read(font: &Font, master: Option<&str>) -> Result<Palettes, ColorError> {
    // M11.2
    let _ = (font, master);
    Err(ColorError::NotYetImplemented("reading colour palettes"))
}

/// A `Color Palettes` value in any of the three forms.
pub fn parse(value: &Value) -> Result<Palettes, ColorError> {
    // M11.2
    let _ = value;
    Err(ColorError::NotYetImplemented("reading colour palettes"))
}

/// The `Color Palettes` parameter that stores `palettes`, in the version 4 form (floats 0–1).
pub fn to_param(palettes: &Palettes) -> Param {
    // M11.2: the colours; the placeholder writes an empty list.
    let _ = palettes;
    Param::new(COLOR_PALETTES_PARAM, Value::Array(Array::default()))
}
