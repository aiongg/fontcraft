//! Features that substitute glyphs by their name suffix (M8.10).
//!
//! Rows: `autoFeature.smcp`, `c2sc`, `pcap`, `c2pc`, `sups`, `subs`, `sinf`, `numr`, `dnom`, `case`, `hist`, `swsh`, `titl`, `init`, `medi`, `med2`, `fina`, `fin2`, `fin3`, `fwid`, `hwid`, `pwid`, `qwid`, `twid`, `hkna`, `vkna`, `pkna`, `hojo`, `ital`, `ruby`, `vrt2`, `cvXX`, `ssXX`, `salt`, `glyphName.chainedSuffixes`.

use super::Generator;

/// This module's generators.
pub(super) fn generators() -> Vec<&'static dyn Generator> {
    // M8.10
    Vec::new()
}
