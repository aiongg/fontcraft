//! gzip (RFC 1952) around the native file's bytes (`roundtrip.nativeGzip`).

use crate::{FormatError, MAX_BYTES};

const MAGIC: [u8; 2] = [0x1f, 0x8b];

pub fn is_gzip(bytes: &[u8]) -> bool {
    bytes.get(..2) == Some(&MAGIC[..])
}

fn crc32(data: &[u8]) -> u32 {
    let mut table = [0u32; 256];
    for (i, slot) in table.iter_mut().enumerate() {
        let mut c = i as u32;
        for _ in 0..8 {
            c = if c & 1 == 1 { 0xEDB8_8320 ^ (c >> 1) } else { c >> 1 };
        }
        *slot = c;
    }
    let mut crc = !0u32;
    for b in data {
        let i = usize::from((crc as u8) ^ b);
        crc = table.get(i).copied().unwrap_or(0) ^ (crc >> 8);
    }
    !crc
}

/// `data` as a gzip stream, without a file name or time, so the same data gives the same bytes.
pub fn compress(data: &[u8]) -> Vec<u8> {
    let mut out = vec![MAGIC[0], MAGIC[1], 8, 0, 0, 0, 0, 0, 0, 255];
    out.extend(miniz_oxide::deflate::compress_to_vec(data, 9));
    out.extend_from_slice(&crc32(data).to_le_bytes());
    out.extend_from_slice(&(data.len() as u32).to_le_bytes());
    out
}

/// The data a gzip stream holds, at most [`MAX_BYTES`].
pub fn decompress(bytes: &[u8]) -> Result<Vec<u8>, FormatError> {
    let bad = |what: &str| FormatError::NotFontcraft(format!("gzip: {what}"));
    let flags = *bytes.get(3).ok_or_else(|| bad("the header is cut short"))?;
    if bytes.get(2) != Some(&8) {
        return Err(bad("not deflate-compressed"));
    }
    let mut at = 10usize;
    let u16_at = |i: usize| bytes.get(i..i + 2).and_then(|b| <[u8; 2]>::try_from(b).ok()).map(|b| usize::from(u16::from_le_bytes(b)));
    if flags & 0x04 != 0 {
        at += 2 + u16_at(at).ok_or_else(|| bad("the extra field is cut short"))?;
    }
    for flag in [0x08, 0x10] {
        if flags & flag != 0 {
            let rest = bytes.get(at..).ok_or_else(|| bad("the header is cut short"))?;
            at += rest.iter().position(|b| *b == 0).ok_or_else(|| bad("a header string is not terminated"))? + 1;
        }
    }
    if flags & 0x02 != 0 {
        at += 2;
    }
    let body = bytes.get(at..bytes.len().saturating_sub(8)).ok_or_else(|| bad("the stream is cut short"))?;
    let data = miniz_oxide::inflate::decompress_to_vec_with_limit(body, MAX_BYTES)
        .map_err(|_| bad("the data does not inflate, or is larger than the limit"))?;
    let trailer = bytes.get(bytes.len().saturating_sub(8)..).ok_or_else(|| bad("the trailer is missing"))?;
    let crc = trailer.get(..4).and_then(|b| b.try_into().ok()).map(u32::from_le_bytes);
    if crc != Some(crc32(&data)) {
        return Err(bad("the checksum does not match"));
    }
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_round_trips_and_damage_is_caught() {
        let data = b"{\"format\": \"fontcraft\"}\n".repeat(100);
        let z = compress(&data);
        assert!(is_gzip(&z) && z.len() < data.len());
        assert_eq!(decompress(&z).unwrap(), data);
        assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
        let mut bad = z.clone();
        let n = bad.len();
        bad[n - 6] ^= 1;
        assert!(decompress(&bad).is_err());
        for cut in 0..z.len() {
            let _ = decompress(&z[..cut]);
        }
    }
}
