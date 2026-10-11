//! The TrueType execution model in the editor: which points are touched, per direction, after
//! the first n instructions of a layer. Commands validate selections against it (Snap needs
//! untouched points, Shift a touched origin…).
//!
//! Rows: `hinting.tt.hintOrder`, `hinting.tt.touchedPoints`, `hinting.tt.iup` (the model).
//! M10.6 fills the body.

use std::collections::BTreeSet;

use crate::{Direction, TtInstruction, TtPoint};

/// The touched points in each direction.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TouchedState {
    /// Touched by instructions in the horizontal direction (moved along x).
    pub horizontal: BTreeSet<TtPoint>,
    /// Touched by instructions in the vertical direction (moved along y).
    pub vertical: BTreeSet<TtPoint>,
}

impl TouchedState {
    /// The state after the first `upto` instructions (all of them when `upto` is larger).
    pub fn replay(instructions: &[TtInstruction], upto: usize) -> Self {
        // M10.6: every type touches its target; Delta touches nothing new.
        let _ = (instructions, upto);
        Self::default()
    }

    pub fn is_touched(&self, point: &TtPoint, direction: Direction) -> bool {
        match direction {
            Direction::Horizontal => self.horizontal.contains(point),
            Direction::Vertical => self.vertical.contains(point),
        }
    }
}
