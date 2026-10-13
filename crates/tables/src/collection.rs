//! Font collections (M12.3; rows `collection list`, `pack`, `unpack`, `ttc.export`,
//! `compile.collectionSharing`). Oracles: AFDKO otf2otc and otc2otf.
//!
//! WOFF and WOFF2 compression live in `fontcraft-container`; the engine calls it directly. A
//! WOFF2 collection is unwrapped first wherever a collection is read.
//!
//! [`pack`] lays a collection out as otf2otc does (AFDKO 5.0.1, Apache-2.0): header version 1,
//! the font directories in input order, then the tables grouped by tag, groups in the order the
//! tags first appear (a tag new to a later font goes in at that font's table position), each
//! group holding the distinct versions of its table in the order they were met. Table bytes are
//! kept as given, `head.checkSumAdjustment` included; checksums are computed. [`unpack`] gives
//! each member with its own copy of every table it uses, within [`MAX_UNPACKED`] in all.

use std::collections::HashMap;
use std::hash::BuildHasher;

use fontcraft_container::sfnt as csfnt;

use crate::limits::{MAX_COLLECTION_FONTS, MAX_UNPACKED};
use crate::sfnt::{Flavour, SfntFont, Tag};
use crate::{Result, TablesError};

/// One font of a collection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CollectionMember {
    pub index: u32,
    pub flavour: Flavour,
    pub tables: Vec<Tag>,
    /// Tables whose bytes are shared with another member.
    pub shared: Vec<Tag>,
    /// Name ID 6, when the font has one.
    pub postscript_name: Option<String>,
}

/// The fonts of a collection file (a single font lists as one member).
pub fn list(bytes: &[u8]) -> Result<Vec<CollectionMember>> {
    let unwrapped = fontcraft_container::to_sfnt(bytes)?;
    let bytes: &[u8] = &unwrapped;
    if bytes.get(..4) != Some(b"ttcf".as_slice()) {
        let font = crate::sfnt::read(bytes, 0)?;
        let postscript_name = font.get(*b"name").and_then(postscript_name);
        return Ok(vec![CollectionMember {
            index: 0,
            flavour: font.flavour,
            tables: font.tables.keys().copied().collect(),
            shared: Vec::new(),
            postscript_name,
        }]);
    }
    let (_, fonts) = csfnt::parse_collection(bytes)?;
    // A table is shared when another member's directory points at the same bytes.
    let base = bytes.as_ptr() as usize;
    let place = |t: &csfnt::Table<'_>| (t.tag, (t.data.as_ptr() as usize).wrapping_sub(base), t.data.len());
    let mut users: HashMap<(Tag, usize, usize), usize> = HashMap::new();
    for font in &fonts {
        for t in &font.tables {
            *users.entry(place(t)).or_default() += 1;
        }
    }
    let mut members = Vec::with_capacity(fonts.len());
    for (i, font) in fonts.iter().enumerate() {
        let flavour = Flavour::from_sfnt_version(font.flavor).ok_or_else(|| TablesError::invalid("a member is not a TrueType or OpenType font"))?;
        let mut tables: Vec<Tag> = font.tables.iter().map(|t| t.tag).collect();
        tables.sort_unstable();
        let mut shared: Vec<Tag> = font.tables.iter().filter(|t| users.get(&place(t)).is_some_and(|&n| n > 1)).map(|t| t.tag).collect();
        shared.sort_unstable();
        let index = u32::try_from(i).map_err(|_| TablesError::invalid("too many fonts"))?;
        members.push(CollectionMember { index, flavour, tables, shared, postscript_name: font.get(*b"name").and_then(postscript_name) });
    }
    Ok(members)
}

/// How [`pack`] builds a collection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PackOptions {
    /// Store byte-identical tables once.
    pub share_tables: bool,
    /// The `ttcf` major version: 1, or 2 when a `DSIG` is kept.
    pub version: u16,
}

impl Default for PackOptions {
    fn default() -> Self {
        Self { share_tables: true, version: 1 }
    }
}

/// A collection file of `fonts` (at most [`crate::limits::MAX_COLLECTION_FONTS`]).
pub fn pack(fonts: &[SfntFont], options: &PackOptions) -> Result<Vec<u8>> {
    if fonts.is_empty() {
        return Err(TablesError::BadOptions("a collection needs at least one font".into()));
    }
    if fonts.len() > MAX_COLLECTION_FONTS {
        return Err(TablesError::TooLarge { what: "the number of fonts in a collection", limit: MAX_COLLECTION_FONTS });
    }
    if !matches!(options.version, 1 | 2) {
        return Err(TablesError::BadOptions(format!("the collection header version is 1 or 2, not {}", options.version)));
    }
    // Groups of distinct table versions by tag, in otf2otc's order; each version is an index
    // into `data`. A hash with a random key finds candidates, the bytes decide.
    let state = std::collections::hash_map::RandomState::new();
    let mut groups: Vec<(Tag, Vec<usize>)> = Vec::new();
    let mut data: Vec<(Tag, &[u8])> = Vec::new();
    let mut by_hash: HashMap<(Tag, u64), Vec<usize>> = HashMap::new();
    let mut members = Vec::with_capacity(fonts.len());
    for font in fonts {
        let mut tables = Vec::with_capacity(font.tables.len());
        // The directory is sorted by tag; otf2otc walks it in that order.
        for (position, (tag, bytes)) in font.tables.iter().enumerate() {
            let candidates = by_hash.entry((*tag, state.hash_one(bytes))).or_default();
            let same =
                if options.share_tables { candidates.iter().copied().find(|&i| data.get(i).is_some_and(|d| d.1 == bytes.as_slice())) } else { None };
            let index = match same {
                Some(i) => i,
                None => {
                    data.push((*tag, bytes.as_slice()));
                    let i = data.len() - 1;
                    candidates.push(i);
                    match groups.iter_mut().find(|g| g.0 == *tag) {
                        Some(group) => group.1.push(i),
                        None => groups.insert(position.min(groups.len()), (*tag, vec![i])),
                    }
                    i
                }
            };
            tables.push((*tag, index));
        }
        members.push((font.flavour.sfnt_version(), tables));
    }
    // Lay the tables out group by group and renumber the members' references.
    let mut position = vec![0usize; data.len()];
    let mut laid_out: Vec<csfnt::Table<'_>> = Vec::with_capacity(data.len());
    for (_, versions) in &groups {
        for &i in versions {
            let Some(&(tag, bytes)) = data.get(i) else { continue };
            if let Some(slot) = position.get_mut(i) {
                *slot = laid_out.len();
            }
            laid_out.push(csfnt::Table { tag, data: bytes });
        }
    }
    let members: Vec<csfnt::Member> = members
        .into_iter()
        .map(|(flavor, tables)| csfnt::Member {
            flavor,
            tables: tables.into_iter().map(|(tag, i)| (tag, position.get(i).copied().unwrap_or(0))).collect(),
        })
        .collect();
    Ok(csfnt::assemble_collection(options.version, &members, &laid_out)?)
}

/// Every font of a collection file, each with its own copy of the shared tables.
pub fn unpack(bytes: &[u8]) -> Result<Vec<SfntFont>> {
    let unwrapped = fontcraft_container::to_sfnt(bytes)?;
    let bytes: &[u8] = &unwrapped;
    let count = crate::sfnt::face_count(bytes)?;
    if bytes.get(..4) == Some(b"ttcf".as_slice()) && count == 0 {
        return Err(TablesError::invalid("the collection holds no fonts"));
    }
    // Members share tables, so the copies are counted from the directories before any is made.
    let mut total = 0usize;
    for face in 0..count {
        total = total.saturating_add(crate::sfnt::tables_size(bytes, face)?);
        if total > MAX_UNPACKED {
            return Err(TablesError::TooLarge { what: "the total size of the unpacked fonts", limit: MAX_UNPACKED });
        }
    }
    (0..count).map(|face| crate::sfnt::read(bytes, face)).collect()
}

/// Name ID 6 of a `name` table: the Windows English record first, then the Macintosh Roman
/// English one, then any record that decodes. `None` when there is none or it is empty.
pub fn postscript_name(name: &[u8]) -> Option<String> {
    let u16_at = |at: usize| Some(u16::from_be_bytes(name.get(at..at.checked_add(2)?)?.try_into().ok()?));
    let count = usize::from(u16_at(2)?);
    let storage = usize::from(u16_at(4)?);
    let mut candidates: Vec<(u8, String)> = Vec::new();
    for i in 0..count {
        let at = 6 + 12 * i;
        let (Some(platform), Some(encoding), Some(language), Some(id), Some(length), Some(offset)) =
            (u16_at(at), u16_at(at + 2), u16_at(at + 4), u16_at(at + 6), u16_at(at + 8), u16_at(at + 10))
        else {
            break;
        };
        if id != 6 {
            continue;
        }
        let start = storage.saturating_add(usize::from(offset));
        let Some(raw) = name.get(start..start.saturating_add(usize::from(length))) else { continue };
        let text = match platform {
            0 | 3 => {
                let units: Vec<u16> = raw.as_chunks::<2>().0.iter().map(|w| u16::from_be_bytes(*w)).collect();
                String::from_utf16_lossy(&units)
            }
            // PostScript names are printable ASCII; other Macintosh Roman bytes are not kept.
            1 => raw.iter().map(|&b| if b.is_ascii() { char::from(b) } else { '?' }).collect(),
            _ => continue,
        };
        let rank = match (platform, encoding, language) {
            (3, 1, 0x409) => 0,
            (1, 0, 0) => 1,
            _ => 2,
        };
        if !text.is_empty() {
            candidates.push((rank, text));
        }
    }
    candidates.sort_by_key(|c| c.0);
    candidates.into_iter().next().map(|c| c.1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn name_table(ps: &str) -> Vec<u8> {
        let s: Vec<u8> = ps.encode_utf16().flat_map(u16::to_be_bytes).collect();
        let mut t = Vec::new();
        for v in [0u16, 1, 18, 3, 1, 0x409, 6, s.len() as u16, 0] {
            t.extend_from_slice(&v.to_be_bytes());
        }
        t.extend_from_slice(&s);
        t
    }

    fn head(revision: u32) -> Vec<u8> {
        let mut h = vec![0u8; 54];
        h[0..4].copy_from_slice(&[0, 1, 0, 0]);
        h[4..8].copy_from_slice(&revision.to_be_bytes());
        h[8..12].copy_from_slice(&0x1234_5678u32.to_be_bytes());
        h[12..16].copy_from_slice(&0x5F0F_3CF5u32.to_be_bytes());
        h[18..20].copy_from_slice(&1000u16.to_be_bytes());
        h
    }

    /// The two fonts the oracle run used (the generator wrote the same bytes).
    fn fonts() -> [SfntFont; 2] {
        let mut a = SfntFont::new(Flavour::TrueType);
        a.insert(*b"OS/2", b"os2-shared!!".to_vec());
        a.insert(*b"cmap", b"cmap-shared".to_vec());
        a.insert(*b"head", head(0x0001_0000));
        a.insert(*b"name", name_table("FontA"));
        a.insert(*b"post", b"post-shared-".to_vec());
        let mut b = SfntFont::new(Flavour::TrueType);
        b.insert(*b"GSUB", b"gsub-B".to_vec());
        b.insert(*b"OS/2", b"os2-shared!!".to_vec());
        b.insert(*b"cmap", b"cmap-shared".to_vec());
        b.insert(*b"head", head(0x0002_0000));
        b.insert(*b"name", name_table("FontBold"));
        b.insert(*b"post", b"post-shared-".to_vec());
        [a, b]
    }

    /// The report `otc2otf -r` prints (AFDKO 5.0.1), rebuilt from a collection file.
    fn report(file: &str, ttc: &[u8]) -> String {
        let (_, fonts) = csfnt::parse_collection(ttc).unwrap();
        let members = list(ttc).unwrap();
        let mut out = format!("Input font: {file}\n\n");
        let mut seen = std::collections::HashSet::new();
        for (font, m) in fonts.iter().zip(&members) {
            out += &format!("Font {}: {}.ttf\n", m.index, m.postscript_name.clone().unwrap());
            out += "    tag     checksum    length    offset\n    ----  ----------  --------  --------\n";
            let mut tables = font.tables.clone();
            tables.sort_by_key(|t| t.tag);
            for t in tables {
                let offset = t.data.as_ptr() as usize - ttc.as_ptr() as usize;
                let row = (t.tag, csfnt::table_checksum(t.tag, t.data), t.data.len(), offset);
                let tag = csfnt::tag_str(t.tag);
                if seen.insert(row) {
                    out += &format!("    {tag:4}  0x{:08X}  {:8}  {:8}\n", row.1, row.2, row.3);
                } else {
                    out += &format!("    {tag:4}  - shared -\n");
                }
            }
            out += "\n";
        }
        out
    }

    /// otf2otc A.ttf B.ttf, then otc2otf -r on the result.
    const OTF2OTC_REPORT: &str = "Input font: pack.ttc

Font 0: FontA.ttf
    tag     checksum    length    offset
    ----  ----------  --------  --------
    OS/2  0x483FB4C0        12       228
    cmap  0x03462DD1        11       240
    head  0x5F1140DD        54       252
    name  0x00FC050C        28       364
    post  0x10484002        12       428

Font 1: FontBold.ttf
    tag     checksum    length    offset
    ----  ----------  --------  --------
    GSUB  0x94B57562         6       220
    OS/2  - shared -
    cmap  - shared -
    head  0x5F1240DD        54       308
    name  0x01CF057F        34       392
    post  - shared -

";

    #[test]
    fn collection_pack_lays_out_tables_as_otf2otc() {
        let ttc = pack(&fonts(), &PackOptions::default()).unwrap();
        assert_eq!(report("pack.ttc", &ttc), OTF2OTC_REPORT);
        let members = list(&ttc).unwrap();
        assert_eq!(members[1].shared, vec![*b"OS/2", *b"cmap", *b"post"]);
        assert_eq!(members[0].flavour, Flavour::TrueType);
        assert_eq!(members[1].postscript_name.as_deref(), Some("FontBold"));
    }

    #[test]
    fn collection_unpack_gives_each_font_its_tables() {
        let input = fonts();
        let ttc = pack(&input, &PackOptions::default()).unwrap();
        let back = unpack(&ttc).unwrap();
        assert_eq!(back.len(), 2);
        for (a, b) in input.iter().zip(&back) {
            assert_eq!(a.tables, b.tables, "every table byte for byte, head included");
        }
        // Without sharing every table is stored once per font, and unpacking gives the same.
        let separate = pack(&input, &PackOptions { share_tables: false, version: 2 }).unwrap();
        assert!(separate.len() > ttc.len());
        assert!(list(&separate).unwrap().iter().all(|m| m.shared.is_empty()));
        assert_eq!(unpack(&separate).unwrap(), back);
        // A WOFF2 collection unpacks to the same fonts apart from head's bit 11.
        let woff2 = fontcraft_container::woff2::encode(&ttc, &fontcraft_container::woff2::Options::default()).unwrap();
        let from_woff2 = unpack(&woff2).unwrap();
        for (a, b) in input.iter().zip(&from_woff2) {
            assert_eq!(a.tables.keys().collect::<Vec<_>>(), b.tables.keys().collect::<Vec<_>>());
            assert_eq!(a.get(*b"name"), b.get(*b"name"));
        }
        // A single font lists as one member and unpacks to its own tables; the write sets head's
        // checksum adjustment (bytes 8 to 11), so that field is left out of the comparison.
        let single = crate::sfnt::write(&input[0], &crate::sfnt::WriteOptions::default()).unwrap();
        assert_eq!(list(&single).unwrap().len(), 1);
        let unpacked = unpack(&single).unwrap();
        assert_eq!(unpacked.len(), 1);
        let masked = |font: &crate::sfnt::SfntFont| {
            let mut tables = font.tables.clone();
            if let Some(adjustment) = tables.get_mut(b"head").and_then(|h| h.get_mut(8..12)) {
                adjustment.fill(0);
            }
            tables
        };
        assert_eq!(masked(&unpacked[0]), masked(&input[0]));
    }

    #[test]
    fn collection_refuses_bad_input() {
        assert!(pack(&[], &PackOptions::default()).is_err());
        assert!(pack(&fonts(), &PackOptions { share_tables: true, version: 3 }).is_err());
        let many = vec![fonts()[0].clone(); MAX_COLLECTION_FONTS + 1];
        assert!(matches!(pack(&many, &PackOptions::default()), Err(TablesError::TooLarge { .. })));
        let ttc = pack(&fonts(), &PackOptions::default()).unwrap();
        for cut in [0, 4, 11, 13, 40, 100, ttc.len() - 1] {
            assert!(list(&ttc[..cut]).is_err(), "cut at {cut}");
            assert!(unpack(&ttc[..cut]).is_err(), "cut at {cut}");
        }
        let mut huge = ttc.clone();
        huge[8..12].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(list(&huge).is_err() && unpack(&huge).is_err(), "a font count past the limit is refused before anything is read");
        assert_eq!(postscript_name(&[0, 0, 0, 9]), None);
    }

    #[test]
    fn collection_unpack_refuses_shared_tables_past_the_total_limit() {
        // Five members share one directory of 250 tables over the same MiB: 250 MiB each is
        // within a font's limit, 1,250 MiB together is not.
        let (members, count, size) = (5usize, 250usize, 1usize << 20);
        let dir = 12 + 4 * members;
        let data_at = dir + 12 + 16 * count;
        let mut ttc = b"ttcf".to_vec();
        ttc.extend_from_slice(&0x0001_0000u32.to_be_bytes());
        ttc.extend_from_slice(&(members as u32).to_be_bytes());
        for _ in 0..members {
            ttc.extend_from_slice(&(dir as u32).to_be_bytes());
        }
        ttc.extend_from_slice(&crate::tests::overlapping_directory(count, data_at, size));
        ttc.resize(data_at + size, 7);
        let started = std::time::Instant::now();
        assert!(matches!(unpack(&ttc), Err(TablesError::TooLarge { limit: MAX_UNPACKED, .. })));
        assert!(started.elapsed() < std::time::Duration::from_secs(1), "refused before any table is copied");
        assert_eq!(list(&ttc).unwrap().len(), members, "listing copies nothing and still works");
    }
}
