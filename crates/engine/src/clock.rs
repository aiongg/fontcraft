//! The time [`crate::Session::edit`] stamps on changed glyphs (`fontView.column.lastChanged`).
//!
//! The app gives the session a clock ([`crate::Session::set_clock`]); a session without one
//! stamps nothing. Tests use [`fixed_clock`], so their results do not depend on when they run.

use std::sync::Arc;

use fontcraft_model::{Font, Glyph, GlyphInfo, param_flag};

/// Seconds since the Unix epoch, or `None` when the time is not known.
pub type Clock = Arc<dyn Fn() -> Option<i64> + Send + Sync>;

/// The system's clock.
#[cfg(not(target_arch = "wasm32"))]
pub fn system_clock() -> Clock {
    Arc::new(|| {
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?;
        i64::try_from(now.as_secs()).ok()
    })
}

/// A clock that always gives `seconds`.
pub fn fixed_clock(seconds: i64) -> Clock {
    Arc::new(move || Some(seconds))
}

/// Last second of the year 9999: later times have no four-digit year.
const MAX_SECONDS: i64 = 253_402_300_799;

/// A time as a glyph's `lastChange` holds it: `2026-10-08 12:00:00 +0000`. `None` before 1970
/// or after 9999.
pub fn last_change_text(seconds: i64) -> Option<String> {
    if !(0..=MAX_SECONDS).contains(&seconds) {
        return None;
    }
    let (days, rest) = (seconds / 86_400, seconds % 86_400);
    let (y, m, d) = civil_from_days(days);
    Some(format!("{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02} +0000", rest / 3600, rest % 3600 / 60, rest % 60))
}

/// Year, month and day of a day count from 1970-01-01 (Howard Hinnant's `civil_from_days`, for
/// days ≥ 0).
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

/// Whether the font wants last-change times: not when its *Write lastChange* parameter is off
/// or its *Disable Last Change* parameter is on.
pub fn writes_last_change(font: &Font) -> bool {
    let params = &font.info.params;
    params.get("Write lastChange").and_then(param_flag) != Some(false) && params.get("Disable Last Change").and_then(param_flag) != Some(true)
}

/// Stamps `stamp` on every glyph of `next` whose contents differ from the glyph of that name in
/// `before` (new and renamed glyphs included). Glyphs `next` still shares with `before` are not
/// looked into.
pub(crate) fn stamp_changed(next: &mut Font, before: &Font, stamp: &str) {
    let (new, old) = (next.glyphs.as_arcs(), before.glyphs.as_arcs());
    if std::ptr::eq(new, old) {
        return;
    }
    let changed: Vec<usize> = new
        .iter()
        .enumerate()
        .filter(|(i, g)| {
            let was = match old.get(*i) {
                Some(b) if b.name == g.name => Some(b),
                _ => before.glyph(&g.name),
            };
            match was {
                Some(b) if Arc::ptr_eq(b, g) => false,
                Some(b) => !same_but_last_change(g, b),
                None => true,
            }
        })
        .map(|(i, _)| i)
        .collect();
    for i in changed {
        if let Some(mut g) = next.glyphs.get_mut_at(i)
            && g.info.last_change.as_deref() != Some(stamp)
        {
            g.info.last_change = Some(stamp.to_string());
        }
    }
}

/// Whether two glyphs are equal apart from their last-change times.
fn same_but_last_change(a: &Glyph, b: &Glyph) -> bool {
    // Destructured, so a new glyph field cannot be forgotten here.
    let Glyph { name, unicodes, export, kern_groups, info, layers, file_name, extra } = a;
    let info_same = if info.last_change == b.info.last_change {
        *info == b.info
    } else {
        let mut probe = GlyphInfo::clone(info);
        probe.last_change = b.info.last_change.clone();
        probe == *b.info
    };
    *name == b.name
        && *unicodes == b.unicodes
        && *export == b.export
        && *kern_groups == b.kern_groups
        && info_same
        && *layers == b.layers
        && *file_name == b.file_name
        && *extra == b.extra
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates() {
        assert_eq!(last_change_text(0).as_deref(), Some("1970-01-01 00:00:00 +0000"));
        assert_eq!(last_change_text(1_791_460_800).as_deref(), Some("2026-10-08 12:00:00 +0000"));
        assert_eq!(last_change_text(951_782_400).as_deref(), Some("2000-02-29 00:00:00 +0000"));
        assert_eq!(last_change_text(MAX_SECONDS).as_deref(), Some("9999-12-31 23:59:59 +0000"));
        assert_eq!(last_change_text(-1), None);
        assert_eq!(last_change_text(i64::MAX), None);
    }
}
