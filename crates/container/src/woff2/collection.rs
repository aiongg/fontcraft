//! WOFF2 collections: the collection directory that follows the table directory (W3C WOFF File
//! Format 2.0, "Collection Directory Format"), and the writer and reader around it.
//!
//! Each distinct table is stored once and every font lists the indices of its tables. The writer
//! drops `DSIG` and sets bit 11 of every `head.flags`, as for a single font, and keeps each
//! table's bytes otherwise. A `glyf` table is transformed only when every font that uses it
//! uses the same `loca` and no `head` would need two different `loca` formats; an `hmtx` table
//! only when every font that uses it uses the same `glyf`, `hhea` and `maxp`. Each `loca` follows
//! its `glyf` in the table directory, which the specification asks of collections. The reader
//! rebuilds a collection file with each distinct table once, in directory order.

use std::borrow::Cow;
use std::collections::HashMap;
use std::hash::BuildHasher;

use super::{
    HEAD_FLAGS_AT, HEAD_LOCA_FORMAT_AT, HHEA, HHEA_METRICS_AT, LOSSLESS_TRANSFORM, MAXP, MAXP_GLYPHS_AT, Options, Stored, Woff2Collection,
    Woff2CollectionFont, Woff2Directory, Woff2Table, bad, build, glyf, hmtx, push_255u16, read_255u16, write_file,
};
use crate::sfnt::{self, DSIG, GLYF, HEAD, HMTX, LOCA, Member, OTTO, Table, Tag};
use crate::{Error, MAX_COLLECTION_FONTS, MAX_TABLES, pad4, put_u16, u16_at, u32_at};

/// The collection header versions a WOFF2 collection may record.
const VERSION_1: u32 = 0x0001_0000;
const VERSION_2: u32 = 0x0002_0000;

/// Reads the collection directory at `*at` and moves past it. Every index must name an entry of
/// `tables`, and no font may list a tag twice.
pub(crate) fn read_directory(woff2: &[u8], at: &mut usize, tables: &[Woff2Table]) -> Result<Woff2Collection, Error> {
    let cut = || bad("the collection directory is cut short");
    let version = u32_at(woff2, *at).ok_or_else(cut)?;
    *at = at.saturating_add(4);
    if !matches!(version, VERSION_1 | VERSION_2) {
        return Err(bad("the collection directory has an unknown version"));
    }
    let count = usize::from(read_255u16(woff2, at)?);
    if count == 0 || count > MAX_COLLECTION_FONTS {
        return Err(bad("the collection's font count is out of range"));
    }
    let mut fonts = Vec::with_capacity(count);
    for _ in 0..count {
        let num = usize::from(read_255u16(woff2, at)?);
        if num == 0 || num > MAX_TABLES {
            return Err(bad("a font of the collection has a table count out of range"));
        }
        let flavor = u32_at(woff2, *at).ok_or_else(cut)?;
        *at = at.saturating_add(4);
        if !sfnt::is_font_flavor(flavor) {
            return Err(bad("a font of the collection is neither TrueType nor OpenType"));
        }
        let mut indices = Vec::with_capacity(num);
        let mut tags: Vec<Tag> = Vec::with_capacity(num);
        for _ in 0..num {
            let index = usize::from(read_255u16(woff2, at)?);
            let table = tables.get(index).ok_or_else(|| bad("a font of the collection names a table that does not exist"))?;
            indices.push(index);
            tags.push(table.tag);
        }
        tags.sort_unstable();
        if tags.windows(2).any(|w| w.first() == w.get(1)) {
            return Err(bad("a font of the collection lists a table twice"));
        }
        fonts.push(Woff2CollectionFont { flavor, tables: indices });
    }
    Ok(Woff2Collection { version, fonts })
}

/// The collection file inside a WOFF2 collection whose directory and table stream have been read.
pub(crate) fn decode(dir: &Woff2Directory, collection: &Woff2Collection, data: &[u8]) -> Result<Vec<u8>, Error> {
    let lists: Vec<Vec<usize>> = collection.fonts.iter().map(|f| f.tables.clone()).collect();
    let built = build(dir, data, &lists)?;
    let mut members = Vec::with_capacity(collection.fonts.len());
    for font in &collection.fonts {
        let tables = font
            .tables
            .iter()
            .map(|&i| dir.tables.get(i).map(|t| (t.tag, i)).ok_or_else(|| bad("a font names a missing table")))
            .collect::<Result<_, _>>()?;
        members.push(Member { flavor: font.flavor, tables });
    }
    let list: Vec<Table<'_>> = dir.tables.iter().zip(&built).map(|(t, data)| Table { tag: t.tag, data }).collect();
    let version = if collection.version == VERSION_2 { 2 } else { 1 };
    sfnt::assemble_collection(version, &members, &list)
}

/// One distinct table of the collection being written.
struct Distinct<'a> {
    tag: Tag,
    /// The table as the sfnt holds it after wrapping (`head` with bit 11, a normalised `glyf`).
    data: Cow<'a, [u8]>,
    /// The transformed bytes, for a transformed table (empty for `loca`).
    transformed: Option<Vec<u8>>,
}

/// Wraps a collection file in WOFF2.
pub(crate) fn encode(ttc: &[u8], options: &Options) -> Result<Vec<u8>, Error> {
    let (header, fonts) = sfnt::parse_collection(ttc)?;
    // Distinct tables by content: a hash with a random key finds candidates, the bytes decide.
    let state = std::collections::hash_map::RandomState::new();
    let mut by_hash: HashMap<(Tag, u64), Vec<usize>> = HashMap::new();
    let mut distinct: Vec<Distinct<'_>> = Vec::new();
    let mut members: Vec<(u32, Vec<(Tag, usize)>)> = Vec::with_capacity(fonts.len());
    for font in &fonts {
        let mut list = Vec::with_capacity(font.tables.len());
        for t in font.tables.iter().filter(|t| t.tag != DSIG) {
            let candidates = by_hash.entry((t.tag, state.hash_one(t.data))).or_default();
            let found = candidates.iter().copied().find(|&i| distinct.get(i).is_some_and(|d| d.data.as_ref() == t.data));
            let index = match found {
                Some(i) => i,
                None => {
                    distinct.push(Distinct { tag: t.tag, data: Cow::Borrowed(t.data), transformed: None });
                    candidates.push(distinct.len() - 1);
                    distinct.len() - 1
                }
            };
            list.push((t.tag, index));
        }
        if list.is_empty() {
            return Err(bad("a font of the collection has no tables"));
        }
        members.push((font.flavor, list));
    }
    let lookup = |member: &(u32, Vec<(Tag, usize)>), tag: Tag| member.1.iter().find(|(t, _)| *t == tag).map(|(_, i)| *i);

    // The `loca` format each `head` must get, and the transforms that fit.
    let mut head_format: HashMap<usize, bool> = HashMap::new();
    type GlyfPlan = (usize, Vec<u8>, Vec<u8>, Vec<u8>);
    let mut glyf_plan: HashMap<usize, GlyfPlan> = HashMap::new();
    if options.transform_glyf {
        let glyfs: Vec<usize> = distinct.iter().enumerate().filter(|(_, d)| d.tag == GLYF).map(|(i, _)| i).collect();
        for g in glyfs {
            let users: Vec<&(u32, Vec<(Tag, usize)>)> = members.iter().filter(|m| lookup(m, GLYF) == Some(g)).collect();
            let Some(first) = users.first() else { continue };
            let (Some(l), Some(h)) = (lookup(first, LOCA), lookup(first, HEAD)) else { continue };
            let read_format = |h: usize| distinct.get(h).and_then(|d| u16_at(&d.data, HEAD_LOCA_FORMAT_AT));
            let Some(format) = read_format(h) else { continue };
            let consistent = users.iter().all(|m| m.0 != OTTO && lookup(m, LOCA) == Some(l) && lookup(m, HEAD).and_then(read_format) == Some(format));
            let loca_alone = members.iter().filter(|m| lookup(m, LOCA) == Some(l)).all(|m| lookup(m, GLYF) == Some(g));
            if !consistent || !loca_alone {
                continue;
            }
            let (Some(glyf_data), Some(loca_data)) = (distinct.get(g), distinct.get(l)) else { continue };
            let Some(glyphs) = glyf::glyph_slices(&glyf_data.data, &loca_data.data, format != 0) else { continue };
            let Some((new_glyf, new_loca, long)) = glyf::normalise(&glyphs) else { continue };
            let heads: Vec<usize> = users.iter().filter_map(|m| lookup(m, HEAD)).collect();
            if heads.iter().any(|h| head_format.get(h).is_some_and(|&f| f != long)) {
                continue;
            }
            let Some(stream) = glyf::transform(&glyphs, u16::from(long)) else { continue };
            for h in heads {
                head_format.insert(h, long);
            }
            glyf_plan.insert(g, (l, new_glyf, new_loca, stream));
        }
    }
    let mut hmtx_plan: HashMap<usize, Vec<u8>> = HashMap::new();
    if options.transform_hmtx {
        let hmtxs: Vec<usize> = distinct.iter().enumerate().filter(|(_, d)| d.tag == HMTX).map(|(i, _)| i).collect();
        for h in hmtxs {
            let users: Vec<&(u32, Vec<(Tag, usize)>)> = members.iter().filter(|m| lookup(m, HMTX) == Some(h)).collect();
            let Some(first) = users.first() else { continue };
            let key = |m: &(u32, Vec<(Tag, usize)>)| [GLYF, LOCA, HEAD, HHEA, MAXP].map(|t| lookup(m, t));
            if users.iter().any(|m| key(m) != key(first)) {
                continue;
            }
            let [Some(g), Some(l), Some(hd), Some(hh), Some(mx)] = key(first) else { continue };
            let table = |i: usize| distinct.get(i).map(|d| d.data.as_ref());
            let (Some(glyf_data), Some(loca_data), Some(head), Some(hhea), Some(maxp), Some(hmtx_data)) =
                (table(g), table(l), table(hd), table(hh), table(mx), table(h))
            else {
                continue;
            };
            let Some(format) = u16_at(head, HEAD_LOCA_FORMAT_AT) else { continue };
            let Some(glyphs) = glyf::glyph_slices(glyf_data, loca_data, format != 0) else { continue };
            let (Some(metrics), Some(count)) = (u16_at(hhea, HHEA_METRICS_AT), u16_at(maxp, MAXP_GLYPHS_AT)) else { continue };
            if usize::from(count) != glyphs.len() {
                continue;
            }
            let transformed = glyf_plan.contains_key(&g);
            let Some(x_mins) = glyphs.iter().map(|gl| glyf::x_min(gl, transformed)).collect::<Option<Vec<i16>>>() else { continue };
            if let Some(stream) = hmtx::transform(hmtx_data, metrics, &x_mins) {
                hmtx_plan.insert(h, stream);
            }
        }
    }

    // Apply the plans: every head gets bit 11 (and its loca format), glyf and loca their new form.
    for (i, d) in distinct.iter_mut().enumerate() {
        if d.tag == HEAD {
            let flags = u16_at(&d.data, HEAD_FLAGS_AT).ok_or_else(|| bad("the head table is too short"))?;
            let mut copy = d.data.to_vec();
            put_u16(&mut copy, HEAD_FLAGS_AT, flags | LOSSLESS_TRANSFORM);
            if let Some(&long) = head_format.get(&i) {
                put_u16(&mut copy, HEAD_LOCA_FORMAT_AT, u16::from(long));
            }
            d.data = Cow::Owned(copy);
        }
        if let Some(stream) = hmtx_plan.remove(&i) {
            d.transformed = Some(stream);
        }
    }
    for (g, (l, new_glyf, new_loca, stream)) in glyf_plan {
        if let Some(d) = distinct.get_mut(g) {
            d.data = Cow::Owned(new_glyf);
            d.transformed = Some(stream);
        }
        if let Some(d) = distinct.get_mut(l) {
            d.data = Cow::Owned(new_loca);
            d.transformed = Some(Vec::new());
        }
    }

    // Directory order: by tag, each font's loca right after its glyf.
    let mut pairs: HashMap<usize, usize> = HashMap::new();
    for m in &members {
        if let (Some(g), Some(l)) = (lookup(m, GLYF), lookup(m, LOCA)) {
            pairs.entry(g).or_insert(l);
        }
    }
    let paired: std::collections::HashSet<usize> = pairs.values().copied().collect();
    let mut sorted: Vec<usize> = (0..distinct.len()).collect();
    sorted.sort_by_key(|&i| (distinct.get(i).map(|d| d.tag), i));
    let mut order: Vec<usize> = Vec::with_capacity(distinct.len());
    let mut placed = vec![false; distinct.len()];
    let mut place = |i: usize, order: &mut Vec<usize>| {
        if let Some(p) = placed.get_mut(i)
            && !*p
        {
            *p = true;
            order.push(i);
        }
    };
    for &i in &sorted {
        if !paired.contains(&i) {
            place(i, &mut order);
            if let Some(&l) = pairs.get(&i) {
                place(l, &mut order);
            }
        }
    }
    for &i in &sorted {
        place(i, &mut order);
    }
    let mut position = vec![0usize; distinct.len()];
    for (at, &i) in order.iter().enumerate() {
        if let Some(slot) = position.get_mut(i) {
            *slot = at;
        }
    }
    if order.len() != distinct.len() {
        return Err(bad("the collection's tables could not be ordered"));
    }

    let mut stored = Vec::with_capacity(order.len());
    let mut total_sfnt = 12 + 4 * members.len() + if header.version == 2 { 12 } else { 0 };
    for &i in &order {
        let d = distinct.get(i).ok_or_else(|| bad("a table is missing"))?;
        let version = match (d.tag, d.transformed.is_some()) {
            (GLYF | LOCA, false) => 3,
            (HMTX, true) => 1,
            _ => 0,
        };
        total_sfnt = total_sfnt.saturating_add(pad4(d.data.len()).unwrap_or(usize::MAX));
        stored.push(Stored {
            tag: d.tag,
            version,
            orig_length: d.data.len(),
            transformed: d.transformed.is_some(),
            data: d.transformed.as_deref().unwrap_or(d.data.as_ref()),
        });
    }
    let mut directory = Vec::new();
    directory.extend_from_slice(&(if header.version == 2 { VERSION_2 } else { VERSION_1 }).to_be_bytes());
    push_255u16(&mut directory, u16::try_from(members.len()).map_err(|_| bad("too many fonts"))?);
    for (flavor, list) in &members {
        total_sfnt = total_sfnt.saturating_add(12 + 16 * list.len());
        let mut indices: Vec<(Tag, usize)> = list.iter().map(|(tag, i)| (*tag, position.get(*i).copied().unwrap_or(0))).collect();
        indices.sort_unstable();
        push_255u16(&mut directory, u16::try_from(indices.len()).map_err(|_| bad("too many tables"))?);
        directory.extend_from_slice(&flavor.to_be_bytes());
        for (_, at) in indices {
            push_255u16(&mut directory, u16::try_from(at).map_err(|_| bad("too many tables"))?);
        }
    }
    let first_head = members.first().and_then(|m| lookup(m, HEAD)).and_then(|h| distinct.get(h)).map(|d| d.data.as_ref());
    let version = options.extras.version.unwrap_or_else(|| crate::woff::revision(first_head));
    write_file(sfnt::TTCF, &stored, &directory, total_sfnt, &options.extras, version)
}
