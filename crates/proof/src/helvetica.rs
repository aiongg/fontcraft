//! Advance widths of the standard Helvetica font, which labels are set in: the PDF writer needs
//! them to end a label at a given x, and the compare layout to size its name column.

/// Widths of the printable ASCII characters (space to `~`), in thousandths of an em, from the
/// font's published metrics.
const ASCII: [u16; 95] = [
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278, // space to /
    556, 556, 556, 556, 556, 556, 556, 556, 556, 556, // 0 to 9
    278, 278, 584, 584, 584, 556, 1015, // : to @
    667, 667, 722, 722, 667, 611, 778, 722, 278, 500, 667, 556, 833, 722, 778, 667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, // A to Z
    278, 278, 278, 469, 556, 333, // [ to `
    556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500, 222, 833, 556, 556, 556, 556, 333, 500, 278, 556, 500, 722, 500, 500, 500, // a to z
    334, 260, 334, 584, // { to ~
];
/// The width taken for any other character.
const OTHER: u16 = 556;

/// The width of `text` set in Helvetica at `size` points.
pub(crate) fn width(text: &str, size: f64) -> f64 {
    let units: u64 = text
        .chars()
        .map(|c| {
            let w = u32::from(c).checked_sub(0x20).and_then(|i| ASCII.get(i as usize)).copied().unwrap_or(OTHER);
            u64::from(w)
        })
        .sum();
    units as f64 * size / 1000.0
}

#[cfg(test)]
mod tests {
    #[test]
    fn widths_follow_the_published_metrics() {
        assert_eq!(super::width("Hi", 10.0), (722.0 + 222.0) / 100.0);
        assert_eq!(super::width("~", 1000.0), 584.0);
        assert_eq!(super::width("\u{e9}", 1000.0), 556.0);
    }
}
