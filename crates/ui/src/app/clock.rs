//! The person's clock, for the names the app gives by date and time. The engine's own defaults are in UTC.

#[cfg(not(target_arch = "wasm32"))]
use craft::i18n::{DateTime, TimeZone};

/// The local date and time as `2026-10-10 14:05`; `None` on the web, where the engine's default stands.
pub fn local_minute() -> Option<String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        Some(minute(DateTime::from_system(std::time::SystemTime::now(), TimeZone::Local)))
    }
    #[cfg(target_arch = "wasm32")]
    {
        None
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn minute(at: DateTime) -> String {
    format!("{:04}-{:02}-{:02} {:02}:{:02}", at.date.year, at.date.month, at.date.day, at.hour, at.minute)
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    #[test]
    fn a_minute_reads_as_the_engines_default_name() {
        assert_eq!(minute(DateTime::from_unix(1_791_633_900, TimeZone::Utc)), "2026-10-10 12:05");
    }
}
