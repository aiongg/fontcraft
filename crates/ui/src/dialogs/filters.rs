//! Filter dialogs. Every filter with parameters gets a dialog from its `filter.list` schema without a line here; a
//! spec here names a filter dialog's row in `spec/inventory` (and its id for `dialog.open`).

use super::spec::{DialogSpec, Fields};

pub const SPECS: &[DialogSpec] = &[
    // Filter › Round Corners: the radius and the visual corrections of `filter.roundCorners`.
    DialogSpec::new("dialog.filterRoundCorners", "Round Corners", "filter.roundCorners", Fields::Filter),
];
