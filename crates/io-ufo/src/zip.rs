//! Reading and writing a UFOZ: a ZIP archive holding one UFO under a single root folder (UFO 3
//! specification › Storage Formats). Written from the ZIP file format description (PKWARE
//! APPNOTE): the end-of-central-directory record, the central directory, local headers; entries
//! stored or deflated; ZIP64 sizes and offsets. Encrypted entries are refused.
//!
//! All input is untrusted: every count, size and offset is checked against the archive and
//! against [`MAX_ENTRIES`] and [`MAX_TOTAL`] before anything is allocated or inflated.

use crate::{Files, Result, UfoError, bad};

/// Most entries one archive may hold.
pub const MAX_ENTRIES: usize = 200_000;
/// Most bytes all entries may inflate to, together.
pub const MAX_TOTAL: usize = 512 << 20;

const EOCD: u32 = 0x0605_4b50;
const EOCD64_LOCATOR: u32 = 0x0706_4b50;
const EOCD64: u32 = 0x0606_4b50;
const CENTRAL: u32 = 0x0201_4b50;
const LOCAL: u32 = 0x0403_4b50;

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn u64_at(b: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(at..at + 8)?.try_into().ok()?))
}

fn broken(what: &str) -> UfoError {
    bad("the UFOZ archive", what)
}

fn to_usize(v: u64) -> Result<usize> {
    usize::try_from(v).map_err(|_| broken("a size is too large"))
}

/// Whether bytes start like a ZIP archive.
pub fn is_zip(bytes: &[u8]) -> bool {
    u32_at(bytes, 0) == Some(LOCAL) || u32_at(bytes, 0) == Some(EOCD)
}

/// Every file in the archive, by path with `/`.
pub fn read_archive(zip: &[u8]) -> Result<Files> {
    // The end record is in the last 22 + 65,535 bytes (its comment is at most that long).
    let search_from = zip.len().saturating_sub(22 + 0xffff);
    let eocd = (search_from..zip.len().saturating_sub(21))
        .rev()
        .find(|&i| u32_at(zip, i) == Some(EOCD))
        .ok_or_else(|| broken("no end of central directory"))?;
    let mut count = u64::from(u16_at(zip, eocd + 10).ok_or_else(|| broken("a short end record"))?);
    let mut cd_size = u64::from(u32_at(zip, eocd + 12).ok_or_else(|| broken("a short end record"))?);
    let mut cd_offset = u64::from(u32_at(zip, eocd + 16).ok_or_else(|| broken("a short end record"))?);
    if (count == 0xffff || cd_size == 0xffff_ffff || cd_offset == 0xffff_ffff) && eocd >= 20 && u32_at(zip, eocd - 20) == Some(EOCD64_LOCATOR) {
        let at = to_usize(u64_at(zip, eocd - 20 + 8).ok_or_else(|| broken("a short ZIP64 locator"))?)?;
        if u32_at(zip, at) != Some(EOCD64) {
            return Err(broken("the ZIP64 end record is missing"));
        }
        count = u64_at(zip, at + 32).ok_or_else(|| broken("a short ZIP64 end record"))?;
        cd_size = u64_at(zip, at + 40).ok_or_else(|| broken("a short ZIP64 end record"))?;
        cd_offset = u64_at(zip, at + 48).ok_or_else(|| broken("a short ZIP64 end record"))?;
    }
    let count = to_usize(count)?;
    if count > MAX_ENTRIES {
        return Err(broken(&format!("more than {MAX_ENTRIES} entries")));
    }
    let (cd_offset, cd_size) = (to_usize(cd_offset)?, to_usize(cd_size)?);
    if cd_offset.checked_add(cd_size).is_none_or(|end| end > zip.len()) {
        return Err(broken("the central directory lies outside the archive"));
    }
    let mut out = Files::new();
    let mut total = 0usize;
    let mut at = cd_offset;
    for _ in 0..count {
        if u32_at(zip, at) != Some(CENTRAL) {
            return Err(broken("a damaged central directory"));
        }
        let field = |off: usize| u16_at(zip, at + off).ok_or_else(|| broken("a short directory entry"));
        let flags = field(8)?;
        let method = field(10)?;
        let mut csize = u64::from(u32_at(zip, at + 20).ok_or_else(|| broken("a short directory entry"))?);
        let mut usize_ = u64::from(u32_at(zip, at + 24).ok_or_else(|| broken("a short directory entry"))?);
        let name_len = usize::from(field(28)?);
        let extra_len = usize::from(field(30)?);
        let comment_len = usize::from(field(32)?);
        let mut local = u64::from(u32_at(zip, at + 42).ok_or_else(|| broken("a short directory entry"))?);
        let name = zip.get(at + 46..at + 46 + name_len).ok_or_else(|| broken("a short directory entry"))?;
        let name = std::str::from_utf8(name).map_err(|_| broken("an entry name is not UTF-8"))?.replace('\\', "/");
        // ZIP64 sizes and offset, in the order of the fields that overflowed.
        let extra = zip.get(at + 46 + name_len..at + 46 + name_len + extra_len).ok_or_else(|| broken("a short directory entry"))?;
        let mut e = 0;
        while e + 4 <= extra.len() {
            let (id, len) = (u16_at(extra, e).unwrap_or(0), usize::from(u16_at(extra, e + 2).unwrap_or(0)));
            if id == 1 {
                let mut p = e + 4;
                for slot in [&mut usize_, &mut csize, &mut local] {
                    if *slot == 0xffff_ffff {
                        *slot = u64_at(extra, p).ok_or_else(|| broken("a short ZIP64 field"))?;
                        p += 8;
                    }
                }
            }
            e += 4 + len;
        }
        at += 46 + name_len + extra_len + comment_len;
        if name.ends_with('/') {
            continue;
        }
        if flags & 1 != 0 {
            return Err(UfoError::Unsupported("encrypted UFOZ archives".into()));
        }
        let (csize, size, local) = (to_usize(csize)?, to_usize(usize_)?, to_usize(local)?);
        total = total
            .checked_add(size)
            .filter(|t| *t <= MAX_TOTAL)
            .ok_or_else(|| broken(&format!("the files inflate to more than {MAX_TOTAL} bytes")))?;
        if u32_at(zip, local) != Some(LOCAL) {
            return Err(broken("a damaged local header"));
        }
        let data_at = local + 30 + usize::from(u16_at(zip, local + 26).unwrap_or(0)) + usize::from(u16_at(zip, local + 28).unwrap_or(0));
        let data = zip
            .get(data_at..data_at.checked_add(csize).ok_or_else(|| broken("an entry lies outside the archive"))?)
            .ok_or_else(|| broken("an entry lies outside the archive"))?;
        let bytes = match method {
            0 => data.to_vec(),
            8 => miniz_oxide::inflate::decompress_to_vec_with_limit(data, size).map_err(|_| broken(&format!("`{name}` does not inflate")))?,
            m => return Err(UfoError::Unsupported(format!("ZIP compression method {m} in `{name}`"))),
        };
        if bytes.len() != size {
            return Err(broken(&format!("`{name}` is not the size its directory entry says")));
        }
        out.insert(name, bytes);
    }
    Ok(out)
}

/// The UFO inside a UFOZ: the files under its single root folder.
pub fn read_ufoz(zip: &[u8]) -> Result<Files> {
    let all = read_archive(zip)?;
    let mut roots = all.keys().filter_map(|k| k.split_once('/').map(|(r, _)| r)).collect::<Vec<_>>();
    roots.dedup();
    let [root] = roots.as_slice() else { return Err(broken("it must hold exactly one folder at its top")) };
    let prefix = format!("{root}/");
    if all.keys().any(|k| !k.starts_with(&prefix)) {
        return Err(broken("it holds files outside its one folder"));
    }
    Ok(all.into_iter().filter_map(|(k, v)| k.strip_prefix(&prefix).map(|r| (r.to_string(), v))).collect())
}

/// Most files [`write_ufoz`] writes: the count an archive without ZIP64 records can hold.
const MAX_WRITE_ENTRIES: usize = 0xffff;

/// The CRC-32 ZIP stores for each file (the ISO 3309 polynomial, reflected).
fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = 0xffff_ffff_u32;
    for &b in bytes {
        crc ^= u32::from(b);
        for _ in 0..8 {
            let mask = (crc & 1).wrapping_neg();
            crc = (crc >> 1) ^ (0xedb8_8320 & mask);
        }
    }
    !crc
}

/// A UFOZ of the UFO `files`, under the folder `root` (such as `Font.ufo`): each file deflated,
/// or stored when deflating does not make it smaller, with UTF-8 names and the earliest ZIP
/// date, so the same UFO always gives the same bytes. An archive that would need ZIP64 records
/// (more than 65,535 files, or 4 GiB) is refused.
pub fn write_ufoz(files: &Files, root: &str) -> Result<Vec<u8>> {
    if files.len() > MAX_WRITE_ENTRIES {
        return Err(broken(&format!("a UFOZ holds at most {MAX_WRITE_ENTRIES} files")));
    }
    if root.is_empty() || root.contains(['/', '\\']) {
        return Err(broken(&format!("`{root}` cannot be its folder")));
    }
    let too_big = || broken("it would be larger than 4 GiB");
    let mut out = Vec::new();
    let mut central = Vec::new();
    for (path, data) in files {
        let name = format!("{root}/{path}");
        let name_len = u16::try_from(name.len()).map_err(|_| broken(&format!("the path `{name}` is too long")))?;
        let deflated = miniz_oxide::deflate::compress_to_vec(data, 6);
        let (method, body): (u16, &[u8]) = if deflated.len() < data.len() { (8, &deflated) } else { (0, data) };
        let offset = u32::try_from(out.len()).map_err(|_| too_big())?;
        // The fields both headers share: version needed, flags (bit 11: UTF-8 names), method,
        // time, date (1980-01-01), CRC, sizes and name length.
        let mut common = Vec::with_capacity(24);
        common.extend(20u16.to_le_bytes());
        common.extend(0x0800u16.to_le_bytes());
        common.extend(method.to_le_bytes());
        common.extend(0u16.to_le_bytes());
        common.extend(0x0021u16.to_le_bytes());
        common.extend(crc32(data).to_le_bytes());
        common.extend(u32::try_from(body.len()).map_err(|_| too_big())?.to_le_bytes());
        common.extend(u32::try_from(data.len()).map_err(|_| too_big())?.to_le_bytes());
        common.extend(name_len.to_le_bytes());
        out.extend(LOCAL.to_le_bytes());
        out.extend(&common);
        out.extend(0u16.to_le_bytes());
        out.extend(name.as_bytes());
        out.extend(body);
        central.extend(CENTRAL.to_le_bytes());
        central.extend(20u16.to_le_bytes());
        central.extend(&common);
        // Extra field and comment lengths, disk, internal and external attributes.
        central.extend([0u8; 12]);
        central.extend(offset.to_le_bytes());
        central.extend(name.as_bytes());
    }
    let count = u16::try_from(files.len()).map_err(|_| too_big())?;
    let cd_offset = u32::try_from(out.len()).map_err(|_| too_big())?;
    let cd_size = u32::try_from(central.len()).map_err(|_| too_big())?;
    out.extend(central);
    out.extend(EOCD.to_le_bytes());
    out.extend([0u8; 4]);
    out.extend(count.to_le_bytes());
    out.extend(count.to_le_bytes());
    out.extend(cd_size.to_le_bytes());
    out.extend(cd_offset.to_le_bytes());
    out.extend([0u8; 2]);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal archive: each file stored (method 0) or deflated (method 8).
    fn zip(files: &[(&str, &[u8], bool)]) -> Vec<u8> {
        let mut out = Vec::new();
        let mut central = Vec::new();
        for (name, data, deflate) in files {
            let body = if *deflate { miniz_oxide::deflate::compress_to_vec(data, 6) } else { data.to_vec() };
            let offset = out.len() as u32;
            let method: u16 = if *deflate { 8 } else { 0 };
            let mut h = Vec::new();
            h.extend(LOCAL.to_le_bytes());
            h.extend([20, 0, 0, 0]);
            h.extend(method.to_le_bytes());
            h.extend([0; 8]);
            h.extend((body.len() as u32).to_le_bytes());
            h.extend((data.len() as u32).to_le_bytes());
            h.extend((name.len() as u16).to_le_bytes());
            h.extend([0, 0]);
            h.extend(name.as_bytes());
            out.extend(&h);
            out.extend(&body);
            let mut c = Vec::new();
            c.extend(CENTRAL.to_le_bytes());
            c.extend([20, 0, 20, 0, 0, 0]);
            c.extend(method.to_le_bytes());
            c.extend([0; 8]);
            c.extend((body.len() as u32).to_le_bytes());
            c.extend((data.len() as u32).to_le_bytes());
            c.extend((name.len() as u16).to_le_bytes());
            c.extend([0; 12]);
            c.extend(offset.to_le_bytes());
            c.extend(name.as_bytes());
            central.extend(c);
        }
        let cd_offset = out.len() as u32;
        out.extend(&central);
        out.extend(EOCD.to_le_bytes());
        out.extend([0; 4]);
        out.extend((files.len() as u16).to_le_bytes());
        out.extend((files.len() as u16).to_le_bytes());
        out.extend((central.len() as u32).to_le_bytes());
        out.extend(cd_offset.to_le_bytes());
        out.extend([0, 0]);
        out
    }

    #[test]
    fn a_ufoz_holds_one_ufo() {
        let z = zip(&[("Font.ufo/metainfo.plist", b"meta", true), ("Font.ufo/glyphs/a.glif", b"glif", false)]);
        assert!(is_zip(&z));
        let files = read_ufoz(&z).unwrap();
        assert_eq!(files.get("metainfo.plist").map(Vec::as_slice), Some(&b"meta"[..]));
        assert_eq!(files.get("glyphs/a.glif").map(Vec::as_slice), Some(&b"glif"[..]));
    }

    #[test]
    fn a_written_ufoz_reads_back() {
        assert_eq!(crc32(b"123456789"), 0xcbf4_3926, "the check value of CRC-32");
        let mut files = Files::new();
        files.insert("metainfo.plist".into(), b"meta".to_vec());
        files.insert("glyphs/a.glif".into(), "<glyph/>".repeat(100).into_bytes());
        let z = write_ufoz(&files, "Font.ufo").unwrap();
        assert!(is_zip(&z));
        assert_eq!(read_ufoz(&z).unwrap(), files);
        assert_eq!(write_ufoz(&files, "Font.ufo").unwrap(), z, "the same UFO gives the same bytes");
        assert!(write_ufoz(&files, "a/b").is_err());
    }

    #[test]
    fn broken_archives_are_errors() {
        assert!(read_ufoz(&zip(&[("a.plist", b"x", false)])).is_err(), "no root folder");
        assert!(read_ufoz(&zip(&[("A.ufo/a", b"x", false), ("B.ufo/b", b"y", false)])).is_err(), "two root folders");
        let good = zip(&[("A.ufo/a", b"hello", true)]);
        for cut in [0, 10, good.len() / 2, good.len() - 5] {
            assert!(read_archive(good.get(..cut).unwrap()).is_err(), "cut at {cut}");
        }
        let mut lying = good.clone();
        // Claim a larger uncompressed size in the central directory.
        let cd = lying.windows(4).position(|w| w == CENTRAL.to_le_bytes()).unwrap();
        lying.splice(cd + 24..cd + 28, 999u32.to_le_bytes());
        assert!(read_archive(&lying).is_err());
        let mut huge = good;
        huge.splice(cd + 24..cd + 28, u32::MAX.to_le_bytes());
        assert!(read_archive(&huge).is_err());
    }

    #[test]
    fn the_corpus_ufoz_opens() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/fonttools/Tests/ufoLib/testdata/TestFont1 (UFO3).ufoz");
        let Ok(bytes) = std::fs::read(&path) else { return };
        let files = read_ufoz(&bytes).unwrap();
        let font = crate::ufo::read(&files).unwrap();
        assert!(!font.glyphs.is_empty());
    }
}
