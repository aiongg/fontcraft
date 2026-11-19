//! A TrueType bytecode interpreter that can stop before any instruction and show its whole state,
//! for the TrueType debugger and delta suggestions (`cmd::tt_debug`).
//!
//! Ported from skrifa 0.48.0 (fontations, <https://github.com/googlefonts/fontations>, MIT OR
//! Apache-2.0), `src/outline/glyf/hint`: the interpreter itself, unchanged in behaviour, so the
//! debugger's points match what the hinted preview draws. FontCraft's additions are `debug.rs`
//! (glyph loading, the stepping driver), `engine/snapshot.rs` (stopping and state capture),
//! `outlines.rs` and `target.rs` (the parts of skrifa's glyph loader and hinting options the
//! interpreter reads). Indexing was replaced by checked access.
//!
//! Every program run stops after `engine::MAX_RUN_INSTRUCTIONS` instructions, as in skrifa, and
//! loop calls and backward jumps have their own budget.

#![allow(dead_code)]

mod call_stack;
mod cow_slice;
mod cvt;
pub mod debug;
mod definition;
mod engine;
mod graphics;
mod instance;
mod math;
mod outlines;
mod program;
mod projection;
mod round;
mod storage;
mod target;
mod value_stack;
mod zone;

pub use outlines::Outlines;
pub use target::{SmoothMode, Target};

use skrifa::raw::{
    tables::glyf::PointFlags,
    types::{F2Dot14, F26Dot6, GlyphId, Point},
};

// What can go wrong reading bytecode is the font format's, so read-fonts
// states it; this interpreter reports it.
pub use instance::HintInstance;
pub use program::Program;
pub use skrifa::raw::tables::glyf::bytecode::HintError;
pub(crate) use skrifa::raw::tables::glyf::bytecode::HintErrorKind;

/// Outline data that is passed to the hinter.
pub struct HintOutline<'a> {
    pub glyph_id: GlyphId,
    pub unscaled: &'a [Point<i32>],
    pub scaled: &'a mut [Point<F26Dot6>],
    pub original_scaled: &'a mut [Point<F26Dot6>],
    pub flags: &'a mut [PointFlags],
    pub contours: &'a [u16],
    pub phantom: &'a mut [Point<F26Dot6>],
    pub bytecode: &'a [u8],
    pub stack: &'a mut [i32],
    pub cvt: &'a mut [i32],
    pub storage: &'a mut [i32],
    pub twilight_scaled: &'a mut [Point<F26Dot6>],
    pub twilight_original_scaled: &'a mut [Point<F26Dot6>],
    pub twilight_flags: &'a mut [PointFlags],
    pub is_composite: bool,
    pub coords: &'a [F2Dot14],
}
