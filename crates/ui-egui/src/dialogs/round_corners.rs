//! Filter › Round Corners: the radius and the visual corrections of `filter.roundCorners`, from
//! the filter's schema.

use super::{Dialog, Fields};

pub const DIALOG: Dialog = Dialog { id: "dialog.filterRoundCorners", title: "Round Corners", command: "filter.roundCorners", fields: Fields::Filter };
