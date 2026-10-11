//! Small text helpers the writers share: number formatting, XML escaping and Base64.

use std::fmt::Write as _;

/// A coordinate as short text: at most three decimals, no trailing zeros, no `-0`. A value that
/// is not finite prints as `0` (the layout never makes one; this keeps the output well-formed).
pub(crate) fn num(v: f64) -> String {
    if !v.is_finite() {
        return "0".into();
    }
    let rounded = (v * 1000.0).round() / 1000.0;
    if rounded == 0.0 {
        return "0".into();
    }
    let mut s = format!("{rounded:.3}");
    while s.ends_with('0') {
        s.pop();
    }
    if s.ends_with('.') {
        s.pop();
    }
    s
}

/// Text made safe for XML and HTML content and attribute values.
pub(crate) fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            // Control characters other than tab and line feed are not allowed in XML 1.0.
            c if (c as u32) < 0x20 && c != '\t' && c != '\n' => out.push('\u{FFFD}'),
            c => out.push(c),
        }
    }
    out
}

/// `U+0041` style code point.
pub(crate) fn code_point(cp: u32) -> String {
    format!("U+{cp:04X}")
}

/// Standard Base64 with padding (RFC 4648 § 4), for `data:` URLs.
pub(crate) fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let symbol = |i: u32| ALPHABET.get((i & 63) as usize).copied().map(char::from).unwrap_or('A');
    let mut out = String::with_capacity(bytes.len().div_ceil(3).saturating_mul(4));
    for chunk in bytes.chunks(3) {
        let b0 = u32::from(chunk.first().copied().unwrap_or(0));
        let b1 = u32::from(chunk.get(1).copied().unwrap_or(0));
        let b2 = u32::from(chunk.get(2).copied().unwrap_or(0));
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(symbol(n >> 18));
        out.push(symbol(n >> 12));
        out.push(if chunk.len() > 1 { symbol(n >> 6) } else { '=' });
        out.push(if chunk.len() > 2 { symbol(n) } else { '=' });
    }
    out
}

/// Appends formatted text; writing to a `String` cannot fail.
pub(crate) fn push(out: &mut String, args: std::fmt::Arguments<'_>) {
    let _ = out.write_fmt(args);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_escaping_and_base64() {
        assert_eq!(num(12.0), "12");
        assert_eq!(num(-0.0004), "0");
        assert_eq!(num(1.25), "1.25");
        assert_eq!(num(f64::NAN), "0");
        assert_eq!(escape("a<b & \"c\""), "a&lt;b &amp; &quot;c&quot;");
        // RFC 4648 § 10 test vectors.
        for (input, expected) in [("", ""), ("f", "Zg=="), ("fo", "Zm8="), ("foo", "Zm9v"), ("foobar", "Zm9vYmFy")] {
            assert_eq!(base64(input.as_bytes()), expected);
        }
    }
}
