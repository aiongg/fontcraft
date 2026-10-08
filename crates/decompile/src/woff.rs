//! WOFF 1: the sfnt tables, each optionally zlib-compressed, behind a 44-byte header and a
//! directory of 20-byte entries (W3C WOFF File Format 1.0). Unwrapping rebuilds the sfnt; the
//! metadata and private blocks are not kept.

use crate::{DecompileError, MAX_SFNT};

/// Most tables a WOFF file may list.
const MAX_TABLES: usize = 1024;

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn bad(what: &str) -> DecompileError {
    DecompileError::Unreadable(format!("WOFF: {what}"))
}

struct Entry {
    tag: u32,
    checksum: u32,
    data: Vec<u8>,
}

/// The sfnt a WOFF 1 file wraps.
pub fn to_sfnt(woff: &[u8]) -> Result<Vec<u8>, DecompileError> {
    let flavor = u32_at(woff, 4).ok_or_else(|| bad("the header is cut short"))?;
    let num_tables = usize::from(u16_at(woff, 12).ok_or_else(|| bad("the header is cut short"))?);
    let total = u32_at(woff, 16).ok_or_else(|| bad("the header is cut short"))? as usize;
    if num_tables == 0 || num_tables > MAX_TABLES {
        return Err(bad("the table count is out of range"));
    }
    if total > MAX_SFNT {
        return Err(bad("the font inside is too large"));
    }
    let mut entries = Vec::with_capacity(num_tables);
    let mut inflated = 0usize;
    for i in 0..num_tables {
        let at = 44 + 20 * i;
        let field = |k: usize| u32_at(woff, at + 4 * k).ok_or_else(|| bad("the table directory is cut short"));
        let (tag, offset, comp_len, orig_len, checksum) = (field(0)?, field(1)? as usize, field(2)? as usize, field(3)? as usize, field(4)?);
        inflated = inflated.saturating_add(orig_len);
        if inflated > MAX_SFNT || comp_len > orig_len {
            return Err(bad("a table's sizes are inconsistent"));
        }
        let raw = woff.get(offset..offset.saturating_add(comp_len)).ok_or_else(|| bad("a table lies outside the file"))?;
        let data = if comp_len == orig_len {
            raw.to_vec()
        } else {
            let out = miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(raw, orig_len).map_err(|_| bad("a table does not inflate"))?;
            if out.len() != orig_len {
                return Err(bad("a table inflates to another size than stated"));
            }
            out
        };
        entries.push(Entry { tag, checksum, data });
    }
    entries.sort_by_key(|e| e.tag);
    // sfnt header, then the table records, then the data, each table padded to four bytes.
    let n = u16::try_from(entries.len()).map_err(|_| bad("too many tables"))?;
    let mut pow = 1u16;
    let mut shift = 0u16;
    while pow.saturating_mul(2) <= n {
        pow *= 2;
        shift += 1;
    }
    let search_range = pow.saturating_mul(16);
    let mut out = Vec::with_capacity(total.max(12));
    out.extend_from_slice(&flavor.to_be_bytes());
    out.extend_from_slice(&n.to_be_bytes());
    out.extend_from_slice(&search_range.to_be_bytes());
    out.extend_from_slice(&shift.to_be_bytes());
    out.extend_from_slice(&n.saturating_mul(16).saturating_sub(search_range).to_be_bytes());
    let mut offset = 12 + 16 * entries.len();
    for e in &entries {
        out.extend_from_slice(&e.tag.to_be_bytes());
        out.extend_from_slice(&e.checksum.to_be_bytes());
        out.extend_from_slice(&u32::try_from(offset).map_err(|_| bad("the font is too large"))?.to_be_bytes());
        out.extend_from_slice(&u32::try_from(e.data.len()).map_err(|_| bad("a table is too large"))?.to_be_bytes());
        offset += e.data.len().next_multiple_of(4);
    }
    for e in &entries {
        out.extend_from_slice(&e.data);
        out.resize(out.len().next_multiple_of(4), 0);
    }
    Ok(out)
}
