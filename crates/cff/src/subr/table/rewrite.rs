//! Rewriting a whole `CFF ` or `CFF2` table: DICT entries removed, the charstrings replaced, and
//! the table laid out again (row `tables.recompileCff`). Everything else is copied as
//! [`Table::write`] copies it.

use super::super::{CharstringSet, SubrError};
use super::{Entry, Table};

/// What [`rewrite_table`] removes from the DICTs.
#[derive(Clone, Copy, Debug, Default)]
pub struct TableRewrite<'a> {
    /// Operators (`12 n` written as `1200 + n`) removed from the Top DICT and every Font DICT.
    pub top: &'a [u16],
    /// Operators removed from every Private DICT.
    pub private: &'a [u16],
}

/// The table with the DICT entries `rewrite` names removed and the charstrings `edit` returns
/// from the table's own. `edit` must keep the number of glyphs and Font DICTs. Returns the table
/// and the operators removed, each once, in the order first found.
pub fn rewrite_table(
    data: &[u8],
    cff2: bool,
    rewrite: &TableRewrite<'_>,
    edit: &mut dyn FnMut(&CharstringSet) -> Result<CharstringSet, SubrError>,
) -> Result<(Vec<u8>, Vec<u16>), SubrError> {
    let mut table = Table::read(data, cff2)?;
    let mut removed: Vec<u16> = Vec::new();
    let mut drop = |entries: &mut Vec<Entry>, ops: &[u16]| {
        entries.retain(|e| {
            let gone = ops.contains(&e.op);
            if gone && !removed.contains(&e.op) {
                removed.push(e.op);
            }
            !gone
        });
    };
    drop(&mut table.top, rewrite.top);
    if let Some(dicts) = table.font_dicts.as_mut() {
        for d in dicts.iter_mut() {
            drop(d, rewrite.top);
        }
    }
    for p in table.privates.iter_mut() {
        drop(p, rewrite.private);
    }
    let set = edit(&table.set)?;
    Ok((table.write(&set)?, removed))
}
