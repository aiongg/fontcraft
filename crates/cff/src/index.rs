//! CFF INDEX structures: a count, an offset size, offsets and the item data.
//!
//! Layout rules ported from fontTools `cffLib` (`IndexCompiler`, `calcOffSize`; MIT, Copyright
//! (c) 2017 Just van Rossum and contributors): offsets start at 1, the offset size is the smallest
//! that holds the last offset, and an empty INDEX is its two-byte count alone.

use crate::Error;

/// The smallest offset size (1 to 4 bytes) that holds `largest`.
pub(crate) fn off_size(largest: usize) -> u8 {
    if largest < 0x100 {
        1
    } else if largest < 0x1_0000 {
        2
    } else if largest < 0x100_0000 {
        3
    } else {
        4
    }
}

/// The encoded size of an INDEX whose items have these lengths.
pub(crate) fn index_len(item_lengths: impl Iterator<Item = usize>) -> usize {
    let mut count = 0usize;
    let mut data = 0usize;
    for len in item_lengths {
        count += 1;
        data = data.saturating_add(len);
    }
    if count == 0 {
        return 2;
    }
    let last_offset = data.saturating_add(1);
    let size = usize::from(off_size(last_offset));
    // count, offSize, (count + 1) offsets, data
    (2usize + 1).saturating_add((count + 1).saturating_mul(size)).saturating_add(data)
}

/// Append an INDEX of `items`.
pub(crate) fn write_index<T: AsRef<[u8]>>(items: &[T], out: &mut Vec<u8>) -> Result<(), Error> {
    let count = u16::try_from(items.len()).map_err(|_| Error::TooManyGlyphs(items.len()))?;
    out.extend_from_slice(&count.to_be_bytes());
    if items.is_empty() {
        return Ok(());
    }
    let data: usize = items.iter().map(|i| i.as_ref().len()).sum();
    let last_offset = data.saturating_add(1);
    if last_offset > u32::MAX as usize {
        return Err(Error::TableTooLarge(last_offset));
    }
    let size = off_size(last_offset);
    out.push(size);
    let mut offset = 1usize;
    push_offset(offset, size, out);
    for item in items {
        offset += item.as_ref().len();
        push_offset(offset, size, out);
    }
    for item in items {
        out.extend_from_slice(item.as_ref());
    }
    Ok(())
}

/// Append `offset` big-endian in `size` bytes (the caller has checked that it fits).
fn push_offset(offset: usize, size: u8, out: &mut Vec<u8>) {
    let bytes = (offset as u32).to_be_bytes();
    let skip = 4usize.saturating_sub(usize::from(size));
    out.extend_from_slice(bytes.get(skip..).unwrap_or(&bytes));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index(items: &[Vec<u8>]) -> Vec<u8> {
        let mut out = Vec::new();
        write_index(items, &mut out).unwrap();
        assert_eq!(out.len(), index_len(items.iter().map(Vec::len)), "index_len agrees");
        out
    }

    #[test]
    fn an_empty_index_is_its_count() {
        assert_eq!(index(&[]), [0, 0]);
    }

    #[test]
    fn offset_size_grows_at_the_byte_boundaries() {
        // Last offset 255 (254 data bytes + 1) fits in one byte; 256 needs two.
        let one = index(&[vec![7; 254]]);
        assert_eq!(&one[..5], &[0, 1, 1, 1, 255]);
        let two = index(&[vec![7; 255]]);
        assert_eq!(&two[..7], &[0, 1, 2, 0, 1, 1, 0]);
        // 0xFFFF and 0x10000.
        assert_eq!(index(&[vec![0; 0xFFFE]])[2], 2);
        assert_eq!(index(&[vec![0; 0xFFFF]])[2], 3);
        assert_eq!(off_size(0xFF_FFFF), 3);
        assert_eq!(off_size(0x100_0000), 4);
    }

    #[test]
    fn offsets_start_at_one() {
        assert_eq!(index(&[b"ab".to_vec(), b"c".to_vec()]), [0, 2, 1, 1, 3, 4, b'a', b'b', b'c']);
    }
}
