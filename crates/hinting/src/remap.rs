//! Node-reference upkeep: after a command changes the node structure of a layer with hints, the
//! references of that layer are remapped in the same undo step.
//!
//! Contract (`docs/plans/M10.md` § 3 › Node references): nodes are matched old to new by path,
//! position and type; a PostScript edge whose node is gone becomes a number at the node's last
//! position; a TrueType instruction whose node is gone is removed and reported. Layers whose
//! node structure did not change, and layers without hints, are left alone (pointer-equal).
//!
//! The engine hook: `Session::realign_after` in `crates/engine/src/lib.rs` already runs the
//! component re-alignment after every font-changing command (undo and redo excluded); M10.3 adds
//! one call to [`remap_after_edit`] there, after the re-alignment, so both land in the command's
//! undo step. Rows: `hinting.ps.manual.linkOnDrop` (links survive edits),
//! `hinting.ps.masters.transferByNodeLink`, `hinting.tt.export.overlapRemovalMovesHints` (the
//! editor half). M10.3 fills the bodies marked `// M10.3:`.

use fontcraft_model::{Font, Layer};

use crate::HintingError;

/// What remapping did to one font.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct RemapReport {
    /// Messages for removed TrueType instructions, one per instruction: glyph, layer, what.
    pub removed: Vec<String>,
    /// PostScript edges turned into numbers.
    pub unlinked: usize,
}

/// `new` with the hint references of every changed layer remapped against `old`. Returns `new`
/// unchanged (the same `Arc`s) when no layer with hints changed its node structure.
pub fn remap_after_edit(old: &Font, new: Font) -> Result<(Font, RemapReport), HintingError> {
    // M10.3: find changed layers with hints, remap_layer each.
    let _ = old;
    Ok((new, RemapReport::default()))
}

/// The hints of `new` remapped against `old` (the same layer before the edit); `None` when
/// nothing needs to change.
pub fn remap_layer(old: &Layer, new: &Layer, report: &mut RemapReport) -> Result<Option<Layer>, HintingError> {
    // M10.3: match nodes, rewrite references.
    let _ = (old, new, report);
    Ok(None)
}
