//! The size checks of `BASE` and `MATH`, read from the bytes. Every offset counts what it
//! points to again, as the owned conversion copies it again.

use std::collections::HashMap;

use read_fonts::tables::layout::CoverageTable;
use read_fonts::tables::variations::ItemVariationStore;
use read_fonts::{FontData, FontRead};

use super::{Walk, coverage, variation_store};
use crate::Result;
use crate::limits::MAX_LAYOUT_ITEMS;
use crate::reorder::be::Data;

/// Refuses a `BASE` table whose owned form would hold more than [`MAX_LAYOUT_ITEMS`] records.
pub(crate) fn check_base(data: &[u8]) -> Result<()> {
    check_base_within(data, MAX_LAYOUT_ITEMS)
}

pub(super) fn check_base_within(data: &[u8], limit: usize) -> Result<()> {
    let mut b = Base { d: Data::new(*b"BASE", data), w: Walk::new("BASE", limit) };
    for field in [4, 6] {
        let axis = b.offset(0, field);
        if axis != 0 {
            b.axis(axis)?;
        }
    }
    if b.d.u16(2).is_ok_and(|minor| minor >= 1)
        && let Ok(store) = b.d.u32_usize(8)
        && store != 0
        && let Some(Ok(store)) = data.get(store..).map(|s| ItemVariationStore::read(FontData::new(s)))
    {
        variation_store(&mut b.w, &store)?;
    }
    Ok(())
}

struct Base<'a> {
    d: Data<'a>,
    w: Walk,
}

impl Base<'_> {
    /// The target of the 16-bit offset at `parent + field`, 0 when null or unreadable.
    fn offset(&self, parent: usize, field: usize) -> usize {
        match self.d.u16(parent + field) {
            Ok(0) | Err(_) => 0,
            Ok(off) => parent + usize::from(off),
        }
    }

    fn count(&self, at: usize) -> usize {
        self.d.u16(at).map(usize::from).unwrap_or(0)
    }

    fn axis(&mut self, at: usize) -> Result<()> {
        self.w.charge(1)?;
        let tags = self.offset(at, 0);
        if tags != 0 {
            self.w.charge(1 + self.count(tags))?;
        }
        let scripts = self.offset(at, 2);
        if scripts != 0 {
            let n = self.count(scripts);
            self.w.charge(1 + n)?;
            for i in 0..n {
                let script = self.offset(scripts, 2 + 6 * i + 4);
                if script != 0 {
                    self.script(script)?;
                }
            }
        }
        Ok(())
    }

    fn script(&mut self, at: usize) -> Result<()> {
        self.w.charge(1)?;
        let values = self.offset(at, 0);
        if values != 0 {
            let n = self.count(values + 2);
            self.w.charge(1 + n)?;
            for i in 0..n {
                self.coord(values, 4 + 2 * i)?;
            }
        }
        let default = self.offset(at, 2);
        if default != 0 {
            self.min_max(default)?;
        }
        let n = self.count(at + 4);
        self.w.charge(n)?;
        for i in 0..n {
            let m = self.offset(at, 6 + 6 * i + 4);
            if m != 0 {
                self.min_max(m)?;
            }
        }
        Ok(())
    }

    fn min_max(&mut self, at: usize) -> Result<()> {
        self.w.charge(1)?;
        self.coord(at, 0)?;
        self.coord(at, 2)?;
        let n = self.count(at + 4);
        self.w.charge(n)?;
        for i in 0..n {
            self.coord(at, 6 + 8 * i + 4)?;
            self.coord(at, 6 + 8 * i + 6)?;
        }
        Ok(())
    }

    /// The coordinate the offset at `parent + field` points to, with its device.
    fn coord(&mut self, parent: usize, field: usize) -> Result<()> {
        let at = self.offset(parent, field);
        if at == 0 {
            return Ok(());
        }
        self.w.charge(1)?;
        if self.d.u16(at).is_ok_and(|format| format == 3) {
            let device = self.offset(at, 4);
            if device != 0 {
                self.w.charge(1 + device_sizes(self.d, device))?;
            }
        }
        Ok(())
    }
}

/// The sizes a device table at `at` lists; 0 for a variation index or what cannot be read.
fn device_sizes(d: Data<'_>, at: usize) -> usize {
    match (d.u16(at), d.u16(at + 2), d.u16(at + 4)) {
        (Ok(start), Ok(end), Ok(1..=3)) => usize::from(end.saturating_sub(start)),
        _ => 0,
    }
}

/// Refuses a `MATH` table whose owned form would hold more than [`MAX_LAYOUT_ITEMS`] records.
pub(crate) fn check_math(data: &[u8]) -> Result<()> {
    check_math_within(data, MAX_LAYOUT_ITEMS)
}

pub(super) fn check_math_within(data: &[u8], limit: usize) -> Result<()> {
    let d = Data::new(*b"MATH", data);
    let mut m = Math { d, data, w: Walk::new("MATH", limit), kerns: HashMap::new(), constructions: HashMap::new() };
    let constants = usize::from(d.u16(4)?);
    if constants != 0 {
        let c = m.values(constants, constants + 8, 51);
        m.w.charge(c)?;
    }
    let info = usize::from(d.u16(6)?);
    if info != 0 {
        for field in [0, 2] {
            let off = usize::from(d.u16(info + field)?);
            if off != 0 {
                let t = info + off;
                m.coverage(t, 0)?;
                let c = m.values(t, t + 4, usize::from(d.u16(t + 2)?));
                m.w.charge(c)?;
            }
        }
        m.coverage(info, 4)?;
        let off = usize::from(d.u16(info + 6)?);
        if off != 0 {
            let t = info + off;
            m.coverage(t, 0)?;
            for i in 0..usize::from(d.u16(t + 2)?) {
                for k in 0..4 {
                    let kern = usize::from(d.u16(t + 4 + 8 * i + 2 * k)?);
                    if kern != 0 {
                        let c = m.kern(t + kern);
                        m.w.charge(c)?;
                    }
                }
            }
        }
    }
    let variants = usize::from(d.u16(8)?);
    if variants != 0 {
        m.coverage(variants, 2)?;
        m.coverage(variants, 4)?;
        let count = usize::from(d.u16(variants + 6)?) + usize::from(d.u16(variants + 8)?);
        for i in 0..count {
            let c = m.construction(variants + usize::from(d.u16(variants + 10 + 2 * i)?));
            m.w.charge(c)?;
        }
    }
    Ok(())
}

/// What the MATH conversion builds. A kern or construction's size is counted once and then
/// charged at every reference: computing it costs about what it charges.
struct Math<'a> {
    d: Data<'a>,
    data: &'a [u8],
    w: Walk,
    kerns: HashMap<usize, usize>,
    constructions: HashMap<usize, usize>,
}

impl Math<'_> {
    /// The coverage the offset at `parent + field` points to, glyph by glyph (the subsetter and
    /// the merger walk every glyph of it).
    fn coverage(&mut self, parent: usize, field: usize) -> Result<()> {
        let off = self.d.u16(parent + field).map(usize::from).unwrap_or(0);
        if off == 0 {
            return Ok(());
        }
        match self.data.get(parent + off..).map(|s| CoverageTable::read(FontData::new(s))) {
            Some(Ok(c)) => coverage(&mut self.w, &c),
            _ => self.w.charge(1),
        }
    }

    /// What one value record at `at` builds, its device resolved from `parent`.
    fn value(&self, parent: usize, at: usize) -> usize {
        let off = self.d.u16(at + 2).map(usize::from).unwrap_or(0);
        if off == 0 {
            return 1;
        }
        2 + device_sizes(self.d, parent + off)
    }

    fn values(&self, parent: usize, at: usize, count: usize) -> usize {
        (0..count).map(|i| self.value(parent, at + 4 * i)).sum()
    }

    fn kern(&mut self, at: usize) -> usize {
        if let Some(c) = self.kerns.get(&at) {
            return *c;
        }
        let h = self.d.u16(at).map(usize::from).unwrap_or(0);
        let c = 1 + self.values(at, at + 2, 2 * h + 1);
        self.kerns.insert(at, c);
        c
    }

    fn construction(&mut self, at: usize) -> usize {
        if let Some(c) = self.constructions.get(&at) {
            return *c;
        }
        let assembly = self.d.u16(at).map(usize::from).unwrap_or(0);
        let mut c = 1 + self.d.u16(at + 2).map(usize::from).unwrap_or(0);
        if assembly != 0 {
            let a = at + assembly;
            c += self.value(a, a) + self.d.u16(a + 4).map(usize::from).unwrap_or(0);
        }
        self.constructions.insert(at, c);
        c
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;

    fn be16(out: &mut Vec<u8>, v: u16) {
        out.extend_from_slice(&v.to_be_bytes());
    }

    /// A `BASE` table whose horizontal axis lists `scripts` records that all point to one base
    /// script, whose `langs` language records all point to one min/max with a feature record.
    fn shared_base(scripts: u16, langs: u16) -> Vec<u8> {
        let mut t = Vec::new();
        for v in [1, 0, 8, 0] {
            be16(&mut t, v);
        }
        // Axis at 8: no tag list, the script list right after it.
        be16(&mut t, 0);
        be16(&mut t, 4);
        be16(&mut t, scripts);
        let script_at = 2 + 6 * scripts;
        for _ in 0..scripts {
            t.extend_from_slice(b"latn");
            be16(&mut t, script_at);
        }
        // Base script: no values, no default min/max.
        be16(&mut t, 0);
        be16(&mut t, 0);
        be16(&mut t, langs);
        let min_max_at = 6 + 6 * langs;
        for _ in 0..langs {
            t.extend_from_slice(b"dflt");
            be16(&mut t, min_max_at);
        }
        // Min/max: a min coordinate, no max, one feature record with both; coordinates after.
        be16(&mut t, 14);
        be16(&mut t, 0);
        be16(&mut t, 1);
        t.extend_from_slice(b"kern");
        be16(&mut t, 14);
        be16(&mut t, 14);
        be16(&mut t, 1);
        be16(&mut t, 0xFF00);
        t
    }

    #[test]
    fn base_records_that_share_offsets_count_once_per_reference() {
        let small = shared_base(2, 3);
        check_base(&small).unwrap();
        assert!(read_fonts::tables::base::Base::read(FontData::new(&small)).is_ok(), "the test table is well formed");
        assert!(check_base_within(&small, 10).is_err(), "a small budget refuses even the small table");
        // 10,000 scripts × 10,000 languages share one min/max: 1e8 copies from 120 KB.
        let err = check_base(&shared_base(10_000, 10_000)).unwrap_err().to_string();
        assert!(err.contains("too large"), "{err}");
    }

    #[test]
    fn math_coverages_count_every_glyph() {
        // MATH with only a glyph info whose extended shape coverage is one range of 60,000
        // glyphs, repeated: under a small budget it is refused, by glyph count.
        let mut t = Vec::new();
        for v in [1, 0, 0, 10, 0] {
            be16(&mut t, v);
        }
        // Glyph info at 10: extended shape coverage at 8 from it.
        for v in [0, 0, 8, 0] {
            be16(&mut t, v);
        }
        for v in [2, 1, 0, 59_999, 0] {
            be16(&mut t, v);
        }
        check_math(&t).unwrap();
        assert!(check_math_within(&t, 50_000).is_err());
    }
}
