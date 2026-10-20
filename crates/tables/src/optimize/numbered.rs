//! Glyphs whose point numbers other data relies on, for the `dropImpliedOncurves` step: dropping
//! a point renumbers the points after it. Such glyphs are the components (at every level) of
//! composites that place a component by point numbers or have instructions, glyphs with a `GPOS`
//! anchor on a contour point (anchor format 2, in cursive, mark-to-base, mark-to-ligature and
//! mark-to-mark lookups), and glyphs with `GDEF` attachment points or ligature carets on contour
//! points (caret format 2). A `GPOS` or `GDEF` that cannot be walked marks every glyph.
//!
//! Limit: [`MAX_NUMBERING_WALK`] lookups, subtables, records and coverage glyphs visited.

use crate::sfnt::SfntFont;

/// Most lookups, subtables, records and coverage glyphs visited in `GPOS` and `GDEF`.
pub const MAX_NUMBERING_WALK: usize = crate::limits::MAX_LAYOUT_ITEMS;

/// Per glyph, whether its point numbers must stay. `composites` holds, per glyph, its
/// component ids and whether it numbers their points (empty and false for other glyphs).
pub(super) fn numbered_glyphs(font: &SfntFont, composites: &[(Vec<u16>, bool)]) -> Vec<bool> {
    let count = composites.len();
    let mut walk = Walk { b: &[], left: MAX_NUMBERING_WALK, roots: Vec::new() };
    for (ids, _) in composites.iter().filter(|(_, numbers)| *numbers) {
        walk.roots.extend(ids.iter().copied());
    }
    if let Some(b) = font.get(*b"GPOS") {
        walk.b = b;
        if walk.gpos().is_none() {
            return vec![true; count];
        }
    }
    if let Some(b) = font.get(*b"GDEF") {
        walk.b = b;
        if walk.gdef().is_none() {
            return vec![true; count];
        }
    }
    let mut seen = vec![false; count];
    let mut stack = walk.roots;
    while let Some(g) = stack.pop() {
        let g = usize::from(g);
        let Some(slot) = seen.get_mut(g) else { continue };
        if *slot {
            continue;
        }
        *slot = true;
        if let Some((ids, _)) = composites.get(g) {
            stack.extend(ids.iter().copied());
        }
    }
    seen
}

/// A walk over one layout table; `None` stands for a table that cannot be read.
struct Walk<'a> {
    b: &'a [u8],
    left: usize,
    roots: Vec<u16>,
}

impl Walk<'_> {
    fn u16(&self, at: usize) -> Option<u16> {
        Some(u16::from_be_bytes([*self.b.get(at)?, *self.b.get(at.checked_add(1)?)?]))
    }

    fn u32(&self, at: usize) -> Option<u32> {
        Some((u32::from(self.u16(at)?) << 16) | u32::from(self.u16(at.checked_add(2)?)?))
    }

    fn step(&mut self, n: usize) -> Option<()> {
        self.left = self.left.checked_sub(n)?;
        Some(())
    }

    /// The 16-bit offset at `base + field`, from `base`; `None` inside for a null offset.
    fn off(&self, base: usize, field: usize) -> Option<Option<usize>> {
        let o = self.u16(base.checked_add(field)?)?;
        Some(if o == 0 { None } else { base.checked_add(usize::from(o)) })
    }

    fn coverage(&mut self, at: Option<usize>) -> Option<Vec<u16>> {
        let at = at?;
        let n = usize::from(self.u16(at + 2)?);
        self.step(n)?;
        match self.u16(at)? {
            1 => (0..n).map(|i| self.u16(at + 4 + 2 * i)).collect(),
            2 => {
                let mut out = Vec::new();
                for i in 0..n {
                    let r = at + 4 + 6 * i;
                    let (start, end) = (self.u16(r)?, self.u16(r + 2)?);
                    if end < start {
                        return None;
                    }
                    self.step(usize::from(end - start) + 1)?;
                    out.extend(start..=end);
                }
                Some(out)
            }
            _ => None,
        }
    }

    /// Marks `glyphs[i]` when the anchor at `anchor` sits on a contour point.
    fn anchor(&mut self, anchor: Option<usize>, glyphs: &[u16], i: usize) -> Option<()> {
        if let Some(a) = anchor
            && self.u16(a)? == 2
            && let Some(g) = glyphs.get(i)
        {
            self.roots.push(*g);
        }
        Some(())
    }

    fn gpos(&mut self) -> Option<()> {
        let Some(list) = self.off(0, 8)? else { return Some(()) };
        let n = usize::from(self.u16(list)?);
        self.step(n)?;
        for i in 0..n {
            let Some(lookup) = self.off(list, 2 + 2 * i)? else { continue };
            let kind = self.u16(lookup)?;
            let m = usize::from(self.u16(lookup + 4)?);
            self.step(m)?;
            for j in 0..m {
                let Some(s) = self.off(lookup, 6 + 2 * j)? else { continue };
                self.subtable(kind, s)?;
            }
        }
        Some(())
    }

    fn subtable(&mut self, kind: u16, s: usize) -> Option<()> {
        if (3..=6).contains(&kind) && self.u16(s)? != 1 {
            return None;
        }
        match kind {
            9 => {
                if self.u16(s)? != 1 {
                    return None;
                }
                let inner = self.u16(s + 2)?;
                if inner == 9 {
                    return None;
                }
                let off = usize::try_from(self.u32(s + 4)?).ok()?;
                if off == 0 { Some(()) } else { self.subtable(inner, s.checked_add(off)?) }
            }
            3 => {
                let glyphs = self.coverage(self.off(s, 2)?)?;
                let n = usize::from(self.u16(s + 4)?);
                self.step(n)?;
                for i in 0..n {
                    for f in [0, 2] {
                        let a = self.off(s, 6 + 4 * i + f)?;
                        self.anchor(a, &glyphs, i)?;
                    }
                }
                Some(())
            }
            4..=6 => {
                let marks = self.coverage(self.off(s, 2)?)?;
                let bases = self.coverage(self.off(s, 4)?)?;
                let classes = usize::from(self.u16(s + 6)?);
                if let Some(ma) = self.off(s, 8)? {
                    let n = usize::from(self.u16(ma)?);
                    self.step(n)?;
                    for i in 0..n {
                        let a = self.off(ma, 2 + 4 * i + 2)?;
                        self.anchor(a, &marks, i)?;
                    }
                }
                let Some(array) = self.off(s, 10)? else { return Some(()) };
                let n = usize::from(self.u16(array)?);
                self.step(n)?;
                for i in 0..n {
                    // Mark-to-ligature: one attach table per ligature, a row per component.
                    let (table, cells) = if kind == 5 {
                        let Some(attach) = self.off(array, 2 + 2 * i)? else { continue };
                        (attach, usize::from(self.u16(attach)?).saturating_mul(classes))
                    } else {
                        (array, classes)
                    };
                    self.step(cells)?;
                    let first = if kind == 5 { 0 } else { i.saturating_mul(classes) };
                    for k in first..first.saturating_add(cells) {
                        let a = self.off(table, 2 + 2 * k)?;
                        self.anchor(a, &bases, i)?;
                    }
                }
                Some(())
            }
            _ => Some(()),
        }
    }

    fn gdef(&mut self) -> Option<()> {
        if let Some(attach) = self.off(0, 6)? {
            let glyphs = self.coverage(self.off(attach, 0)?)?;
            let n = usize::from(self.u16(attach + 2)?);
            self.step(n)?;
            for i in 0..n {
                if let Some(points) = self.off(attach, 4 + 2 * i)?
                    && self.u16(points)? > 0
                    && let Some(g) = glyphs.get(i)
                {
                    self.roots.push(*g);
                }
            }
        }
        if let Some(carets) = self.off(0, 8)? {
            let glyphs = self.coverage(self.off(carets, 0)?)?;
            let n = usize::from(self.u16(carets + 2)?);
            self.step(n)?;
            for i in 0..n {
                let Some(lig) = self.off(carets, 4 + 2 * i)? else { continue };
                let m = usize::from(self.u16(lig)?);
                self.step(m)?;
                for k in 0..m {
                    let a = self.off(lig, 2 + 2 * k)?;
                    self.anchor(a, &glyphs, i)?;
                }
            }
        }
        Some(())
    }
}
