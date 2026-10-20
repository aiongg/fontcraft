//! gzip streams (RFC 1952), as `SVG ` tables store compressed documents: written with the header
//! fontTools writes, and read with every member, CRC and length checked and the inflated bytes
//! capped by the caller's budget.

use miniz_oxide::inflate::TINFLStatus;
use miniz_oxide::inflate::core::{DecompressorOxide, decompress, inflate_flags};

/// The first two bytes of every gzip member.
pub const MAGIC: [u8; 2] = [0x1F, 0x8B];

/// Why a gzip stream could not be read.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum GunzipError {
    /// The data would inflate past the budget.
    #[error("the data inflates past its budget")]
    TooLarge,
    /// The stream is damaged or not gzip.
    #[error("{0}")]
    Invalid(&'static str),
}

/// The CRC-32 table of the gzip polynomial.
pub fn crc_table() -> [u32; 256] {
    let mut table = [0u32; 256];
    for (i, slot) in table.iter_mut().enumerate() {
        let mut c = i as u32;
        for _ in 0..8 {
            c = if c & 1 == 1 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
        }
        *slot = c;
    }
    table
}

pub fn crc32(table: &[u32; 256], data: &[u8]) -> u32 {
    let mut crc = !0u32;
    for b in data {
        crc = table.get(usize::from((crc as u8) ^ b)).copied().unwrap_or(0) ^ (crc >> 8);
    }
    !crc
}

/// `data` as one gzip member with the header fontTools writes (no name, time 0, best
/// compression, unknown system).
pub fn gzip(data: &[u8]) -> Vec<u8> {
    let mut out = vec![MAGIC[0], MAGIC[1], 8, 0, 0, 0, 0, 0, 2, 255];
    out.extend(miniz_oxide::deflate::compress_to_vec(data, 9));
    out.extend_from_slice(&crc32(&crc_table(), data).to_le_bytes());
    // The length field holds the length modulo 2^32.
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out
}

fn le32(src: &[u8], at: usize) -> Option<u32> {
    src.get(at..at.checked_add(4)?).and_then(|b| <[u8; 4]>::try_from(b).ok()).map(u32::from_le_bytes)
}

/// The data of the gzip stream `src`, at most `left` bytes. Every member is read, and zero
/// bytes after the last one are skipped, as Python's gzip module reads a stream.
pub fn gunzip(src: &[u8], left: usize) -> Result<Vec<u8>, GunzipError> {
    use GunzipError::Invalid;
    // The last four bytes are the length of the last member (or zero padding), and no member
    // is longer than the whole stream: a declared length past the budget is refused at once.
    if let Some(declared) = src.len().checked_sub(4).and_then(|at| le32(src, at))
        && declared as usize > left
    {
        return Err(GunzipError::TooLarge);
    }
    let table = crc_table();
    let mut state = Box::<DecompressorOxide>::default();
    let mut buf = Vec::new();
    let mut out = Vec::new();
    let mut at = 0usize;
    loop {
        let rest = src.get(at..).unwrap_or_default();
        if rest.get(..2) != Some(&MAGIC[..]) {
            return Err(Invalid("a member does not start with the gzip signature"));
        }
        if rest.get(2) != Some(&8) {
            return Err(Invalid("a member is not deflate-compressed"));
        }
        let flags = *rest.get(3).ok_or(Invalid("a header is cut short"))?;
        let mut h = 10usize;
        if flags & 0x04 != 0 {
            let len = rest.get(h..h + 2).ok_or(Invalid("an extra field is cut short"))?;
            h += 2 + usize::from(u16::from_le_bytes([len.first().copied().unwrap_or(0), len.get(1).copied().unwrap_or(0)]));
        }
        for flag in [0x08u8, 0x10] {
            if flags & flag != 0 {
                let field = rest.get(h..).ok_or(Invalid("a header is cut short"))?;
                h += field.iter().position(|b| *b == 0).ok_or(Invalid("a header string is not terminated"))? + 1;
            }
        }
        if flags & 0x02 != 0 {
            h += 2;
        }
        let body = rest.get(h..).ok_or(Invalid("a header is cut short"))?;
        let start = out.len();
        let used = inflate_member(body, &mut state, &mut buf, &mut out, left)
            .map_err(|e| if e { GunzipError::TooLarge } else { Invalid("the data does not inflate") })?;
        let trailer = h + used;
        let crc = le32(rest, trailer).ok_or(Invalid("a member's trailer is cut short"))?;
        let size = le32(rest, trailer + 4).ok_or(Invalid("a member's trailer is cut short"))?;
        let member = out.get(start..).unwrap_or_default();
        if crc != crc32(&table, member) {
            return Err(Invalid("the checksum does not match"));
        }
        if size != member.len() as u32 {
            return Err(Invalid("the length does not match"));
        }
        at += trailer + 8;
        // Zero padding after a member, then either the end or another member.
        while src.get(at) == Some(&0) {
            at += 1;
        }
        if at >= src.len() {
            return Ok(out);
        }
    }
}

/// Inflates the raw deflate stream at the start of `input` through `buf`, appending to `out`,
/// which may hold at most `cap` bytes. Returns the input bytes the stream took; the error says
/// whether the cap was reached (`true`) or the stream is broken (`false`).
fn inflate_member(input: &[u8], state: &mut DecompressorOxide, buf: &mut Vec<u8>, out: &mut Vec<u8>, cap: usize) -> Result<usize, bool> {
    let flags = inflate_flags::TINFL_FLAG_USING_NON_WRAPPING_OUTPUT_BUF;
    state.init();
    let base = out.len();
    // The buffer grows by doubling up to one byte past the cap, so filling it means too much.
    let room = cap.saturating_sub(base).saturating_add(1);
    let first = input.len().saturating_mul(4).max(64);
    buf.resize(buf.len().max(first).min(room), 0);
    let (mut pos, mut taken) = (0usize, 0usize);
    loop {
        let rest = input.get(taken..).unwrap_or_default();
        let (status, used, written) = decompress(state, rest, buf, pos, flags);
        taken = taken.saturating_add(used);
        pos = pos.saturating_add(written);
        if base.saturating_add(pos) > cap {
            return Err(true);
        }
        match status {
            TINFLStatus::Done => {
                out.extend_from_slice(buf.get(..pos).unwrap_or_default());
                return Ok(taken);
            }
            TINFLStatus::HasMoreOutput => {
                if buf.len() >= room {
                    return Err(true);
                }
                let size = buf.len().saturating_mul(2).min(room);
                buf.resize(size, 0);
            }
            _ => return Err(false),
        }
    }
}
