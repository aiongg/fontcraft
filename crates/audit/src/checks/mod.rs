//! The check runners of M12, one file per group. The M3 outline checks live in the crate root's
//! `contour`, `contours`, `handles`, `metrics` and `points` modules.

pub(crate) mod anchors;
pub(crate) mod color;
pub(crate) mod compat;
pub(crate) mod components;
pub(crate) mod family;
pub(crate) mod files;
pub(crate) mod glyphset;
pub(crate) mod hinting;
pub(crate) mod kerning;
pub(crate) mod layout;
pub(crate) mod metrics_font;
pub(crate) mod metrics_widths;
pub(crate) mod names;
pub(crate) mod sources;
pub(crate) mod tables;
pub(crate) mod variations_binary;
