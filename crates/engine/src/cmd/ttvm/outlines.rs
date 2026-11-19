//! The font-wide data the interpreter reads: the font and control value programs and the `maxp`
//! sizes. Follows `Outlines::new` of skrifa 0.48.0 (`src/outline/glyf/mod.rs`), reduced to the
//! fields the interpreter uses, plus what the debugger's glyph loader needs.

use skrifa::raw::tables::gvar::Gvar;
use skrifa::raw::tables::head::Flags;
use skrifa::raw::types::Tag;
use skrifa::raw::{FontRef, TableProvider};

pub struct Outlines<'a> {
    pub font: FontRef<'a>,
    pub gvar: Option<Gvar<'a>>,
    pub fpgm: &'a [u8],
    pub prep: &'a [u8],
    pub cvt_len: u32,
    pub max_function_defs: u16,
    pub max_instruction_defs: u16,
    /// `maxp`'s twilight points plus 4, as FreeType allocates them.
    pub max_twilight_points: u16,
    /// `maxp`'s stack depth plus 32, FreeType's allowance for fonts that understate it.
    pub max_stack_elements: u16,
    pub max_storage: u16,
    pub units_per_em: u16,
    /// The typographic ascender and descender, for the vertical phantom points.
    pub os2_vmetrics: [i16; 2],
    /// Whether the font allows fractional sizes (bit 3 of `head.flags` clear).
    pub fractional_size_hinting: bool,
}

impl<'a> Outlines<'a> {
    pub fn new(font: &FontRef<'a>) -> Option<Self> {
        let head = font.head().ok()?;
        let fractional_size_hinting = !head.flags().contains(Flags::FORCE_INTEGER_PPEM);
        let maxp = font.maxp().ok()?;
        let os2_vmetrics = font.os2().map(|os2| [os2.s_typo_ascender(), os2.s_typo_descender()]).unwrap_or_default();
        let fpgm = font.data_for_tag(Tag::new(b"fpgm")).unwrap_or_default().as_bytes();
        let prep = font.data_for_tag(Tag::new(b"prep")).unwrap_or_default().as_bytes();
        let cvt_len = font.cvt().map(|cvt| u32::try_from(cvt.len()).unwrap_or(u32::MAX)).unwrap_or_default();
        Some(Self {
            font: font.clone(),
            gvar: font.gvar().ok(),
            fpgm,
            prep,
            cvt_len,
            max_function_defs: maxp.max_function_defs().unwrap_or_default(),
            max_instruction_defs: maxp.max_instruction_defs().unwrap_or_default(),
            max_twilight_points: maxp.max_twilight_points().unwrap_or_default().saturating_add(4),
            max_stack_elements: maxp.max_stack_elements().unwrap_or_default().saturating_add(32),
            max_storage: maxp.max_storage().unwrap_or_default(),
            units_per_em: head.units_per_em(),
            os2_vmetrics,
            fractional_size_hinting,
        })
    }
}
