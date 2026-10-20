//! The never-crash limits of the binary tools (`docs/plans/M12.md` § 3, Limits). Every count or
//! size read from an input is checked against one of these before anything is allocated for it.

/// Largest font, collection or decompressed WOFF or WOFF2 accepted or produced, in bytes.
pub const MAX_SFNT: usize = fontcraft_container::MAX_SFNT;
/// Most tables in one font.
pub const MAX_TABLES: usize = fontcraft_container::MAX_TABLES;
/// Most fonts in one collection.
pub const MAX_COLLECTION_FONTS: usize = 256;
/// Largest summed size of the fonts a collection unpacks to, in bytes. Members share tables, so
/// the copies can outgrow the collection: four times [`MAX_SFNT`].
pub const MAX_UNPACKED: usize = 4 * MAX_SFNT;
/// Most fonts merged into one.
pub const MAX_MERGE_FONTS: usize = 64;
/// Most fonts treated as one family.
pub const MAX_FAMILY_FONTS: usize = 256;

/// Largest TTX input, in bytes of text.
pub const MAX_TTX_BYTES: usize = 256 << 20;
/// Deepest element nesting in a TTX input.
pub const MAX_TTX_DEPTH: usize = 64;
/// Most elements in one TTX input.
pub const MAX_TTX_ELEMENTS: usize = 50_000_000;
/// Most memory the element tree of one TTX input may take, by an estimate counting each
/// element, attribute and string with its allocation. A 256 MB dump as `ttx` writes it comes to
/// about 1.6 GB on a 64-bit target and 1 GB on a 32-bit one (wasm), whose 4 GB address space
/// needs room for the rest.
pub const MAX_TTX_TREE_BYTES: usize = if usize::BITS >= 64 { 3 << 30 } else { 3 << 29 };
/// Most files one TTX dump split into one file per table may name.
pub const MAX_TTX_PARTS: usize = MAX_TABLES;
/// Most glyph files the parts of one TTX dump split by glyph may name.
pub const MAX_TTX_GLYPH_PARTS: usize = MAX_GLYPHS;
/// Largest text dump (TTX or JSON) written for one call, in bytes: the text, plus an estimate
/// of the element tree the JSON form is built from.
pub const MAX_TTX_OUTPUT: usize = 1 << 30;
/// Most `cmap` mappings a text dump reads, summed over every subtable record (a subtable that
/// several records share counts once per record).
pub const MAX_TTX_CMAP_ENTRIES: usize = 1 << 23;

/// Most glyphs in a glyph list, rename map or GOADB.
pub const MAX_GLYPHS: usize = fontcraft_model::MAX_GLYPHS;
/// Longest line of a GOADB or glyph map file, in bytes.
pub const MAX_LIST_LINE: usize = 1_024;

/// Largest subset text file, in bytes.
pub const MAX_SUBSET_TEXT: usize = 16 << 20;
/// Most code points in one selection (every Unicode scalar value and surrogate).
pub const MAX_CODE_POINTS: usize = 1_114_112;
/// Deepest nesting of contextual lookups followed by the layout closure.
pub const MAX_LOOKUP_NESTING: usize = 64;
/// Most records, offsets and glyphs a `GSUB`, `GPOS` or `GDEF` table may expand to when a tool
/// rewrites it, counting a shared subtable once per offset to it.
pub const MAX_LAYOUT_ITEMS: usize = 10_000_000;
/// Most records the `GPOS` tables of all the masters of one layout interpolation may expand to
/// together, counted as for [`MAX_LAYOUT_ITEMS`]: every master's table is held at once.
pub const MAX_INTERPOLATED_LAYOUT_ITEMS: usize = 2 * MAX_LAYOUT_ITEMS;
/// Most index subtables a bitmap location table (`EBLC`, `CBLC`, `bloc`) may list over all its
/// strikes, counting a subtable array once per strike that points at it.
pub const MAX_BITMAP_SUBTABLES: usize = 1 << 20;

/// Deepest subroutine nesting in a charstring (the CFF specification's limit).
pub const MAX_SUBR_NESTING: usize = 10;
/// Argument stack of a CFF charstring.
pub const MAX_CFF_STACK: usize = 48;
/// Argument stack of a CFF2 charstring.
pub const MAX_CFF2_STACK: usize = 513;
/// Longest charstring, in bytes.
pub const MAX_CHARSTRING: usize = 65_535;
/// Most memory the charstrings of one `CFF ` or `CFF2` table may take once their subroutine
/// calls are expanded and they are read into operators, counted as the bytes the expanded
/// charstrings, the operator records, the operands and the mask bytes hold. A full CJK font
/// (65,535 glyphs, 4.7M operators, 20.1M operands, 30 MB expanded) takes about 150 MB to read
/// for drawing. A 32-bit target (wasm) gets half, which leaves the rest of its 4 GiB to the app.
pub const MAX_CHARSTRING_MEMORY: usize = if usize::BITS >= 64 { 1 << 30 } else { 512 << 20 };

/// What the charstrings of one table may still take under a memory limit
/// ([`MAX_CHARSTRING_MEMORY`] outside tests).
#[derive(Clone, Debug)]
pub(crate) struct CharstringMemory {
    left: usize,
    limit: usize,
}

impl CharstringMemory {
    pub(crate) fn new(limit: usize) -> Self {
        CharstringMemory { left: limit, limit }
    }

    pub(crate) fn charge(&mut self, bytes: usize) -> crate::Result<()> {
        self.left = self.left.checked_sub(bytes).ok_or(crate::TablesError::TooLarge {
            what: "the memory the charstrings of one table take, subroutine calls expanded",
            limit: self.limit,
        })?;
        #[cfg(test)]
        CHARSTRING_PEAK.with(|p| p.set(p.get().max(self.limit.saturating_sub(self.left))));
        Ok(())
    }
}

#[cfg(test)]
thread_local! {
    /// The most any [`CharstringMemory`] on this thread has had charged, for tests that measure.
    pub(crate) static CHARSTRING_PEAK: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Most axes an instancing location or limit names.
pub const MAX_AXES: usize = fontcraft_model::MAX_AXIS_VALUES;
/// Most named instances read from one `fvar` table, and most instances one instancing run
/// builds.
pub const MAX_NAMED_INSTANCES: usize = 1_024;
/// Most tuple variations for one glyph.
pub const MAX_TUPLE_VARIATIONS: usize = 65_535;

/// Largest coordinate a transformed `glyf` outline may reach; beyond it the run is refused.
pub const MAX_COORDINATE: f64 = 32_767.0;
/// Smallest units per em a font may be scaled to.
pub const MIN_UNITS_PER_EM: u16 = 16;
/// Largest units per em a font may be scaled to.
pub const MAX_UNITS_PER_EM: u16 = 16_384;

/// Most differing records a diff lists per table unless every record is asked for.
pub const MAX_DIFF_RECORDS: usize = 1_000;
/// Most hex bytes a diff prints per table unless everything is asked for.
pub const MAX_DIFF_HEX: usize = 64 << 10;

/// Largest VOLT, FontDame or feature-code input, in bytes.
pub const MAX_FEATURE_SOURCE: usize = 64 << 20;

/// Most nodes `tables show` builds for one call.
pub const MAX_SHOW_NODES: usize = 1_000_000;
