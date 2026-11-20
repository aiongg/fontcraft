//! Framework pieces FontCraft needs and the craft framework does not have yet, staged here until craft-libs takes
//! them (docs/decisions/0009-framework-gaps-staged-here.md).
//!
//! The modules sit where the pieces belong in craft-libs: `craft_ext::ui::list` beside `craft::ui::list`, and so on.
//! Each module's doc names the craft crate and file the piece is meant for and the craft piece it extends or stands
//! in for. Nothing here depends on FontCraft; when a piece moves into craft, its call sites change their import path
//! and nothing else.

pub mod ui;
