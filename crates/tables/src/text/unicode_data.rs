//! Character names from a UnicodeData.txt file, for the comments the TTX dump writes after
//! `cmap` mappings (M12.76; row `dump`, `--unicode-data`). Oracle: ttx `--unicodedata`.
//!
//! The reading follows fontTools 4.66 `unicode._makeunicodes` (MIT, Copyright (c) 2017 Just van
//! Rossum, Behdad Esfahbod and the fontTools authors; see `NOTICE`): the first field of a line
//! is the code point in hex, the second its name; names that start with `<` (`<control>`, the
//! range markers) are skipped, and a later line for the same code point replaces an earlier
//! one. A code point without a name reads as `????`. Lines fontTools stops on with a Python
//! error (no `;`, an empty name, a first field that is not hex) are skipped here.

use std::collections::BTreeMap;

use crate::limits::MAX_CODE_POINTS;
use crate::{Result, TablesError};

/// Largest UnicodeData file read, in bytes (the Unicode 16 file is about 2 MB).
pub const MAX_UNICODE_DATA: usize = 64 << 20;

/// What a code point without a name reads as.
pub const UNKNOWN_NAME: &str = "????";

/// Character names by code point.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UnicodeNames {
    names: BTreeMap<u32, String>,
}

impl UnicodeNames {
    /// Reads UnicodeData text of at most [`MAX_UNICODE_DATA`] bytes, at most
    /// [`crate::limits::MAX_CODE_POINTS`] names.
    pub fn parse(text: &str) -> Result<Self> {
        if text.len() > MAX_UNICODE_DATA {
            return Err(TablesError::TooLarge { what: "the UnicodeData text", limit: MAX_UNICODE_DATA });
        }
        let mut names = BTreeMap::new();
        for line in text.lines() {
            let mut fields = line.split(';');
            let (Some(code), Some(name)) = (fields.next(), fields.next()) else { continue };
            if name.is_empty() || name.starts_with('<') {
                continue;
            }
            let code = code.trim();
            let digits = code.strip_prefix("0x").or_else(|| code.strip_prefix("0X")).unwrap_or(code);
            let Ok(code) = u32::from_str_radix(digits, 16) else { continue };
            if !names.contains_key(&code) && names.len() >= MAX_CODE_POINTS {
                return Err(TablesError::TooLarge { what: "the names of the UnicodeData text", limit: MAX_CODE_POINTS });
            }
            names.insert(code, name.to_string());
        }
        Ok(Self { names })
    }

    /// The name of `code`, or [`UNKNOWN_NAME`].
    pub fn name(&self, code: u32) -> &str {
        self.names.get(&code).map_or(UNKNOWN_NAME, String::as_str)
    }

    /// How many code points have a name.
    pub fn len(&self) -> usize {
        self.names.len()
    }

    pub fn is_empty(&self) -> bool {
        self.names.is_empty()
    }
}
