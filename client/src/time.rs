use chrono::{DateTime, Datelike, Utc};

pub const TIME_COLUMN_WIDTH: usize = 16;

/// Formats a UTC timestamp for the interactive backup list.
///
/// - `< 14 days` from `now`: `"3h ago"`, `"2d ago"`, etc.
/// - Same calendar year as `now` but outside the relative bucket: `"Apr 03 14:32"`.
/// - Different year: `"2025-12-31 23:59"`.
///
/// Output is left-aligned and padded to [`TIME_COLUMN_WIDTH`] characters.
pub fn format_relative(now: DateTime<Utc>, then: DateTime<Utc>) -> String {
    let core = format_relative_core(now, then);
    let pad_needed = TIME_COLUMN_WIDTH.saturating_sub(core.chars().count());
    let mut out = core;
    for _ in 0..pad_needed {
        out.push(' ');
    }
    out
}

fn format_relative_core(now: DateTime<Utc>, then: DateTime<Utc>) -> String {
    let delta = now.signed_duration_since(then);
    let secs = delta.num_seconds().max(0);
    if secs < 60 {
        return format!("{secs}s ago");
    }
    let mins = secs / 60;
    if mins < 60 {
        return format!("{mins}m ago");
    }
    let hours = mins / 60;
    if hours < 24 {
        return format!("{hours}h ago");
    }
    let days = hours / 24;
    if days < 14 {
        return format!("{days}d ago");
    }
    if then.year() == now.year() {
        return then.format("%b %d %H:%M").to_string();
    }
    then.format("%Y-%m-%d %H:%M").to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn utc(y: i32, mo: u32, d: u32, h: u32, mi: u32, s: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, mo, d, h, mi, s).unwrap()
    }

    #[test]
    fn seconds_bucket_boundary() {
        let now = utc(2026, 4, 21, 12, 0, 59);
        assert!(format_relative_core(now, utc(2026, 4, 21, 12, 0, 0)).starts_with("59s"));
        assert!(format_relative_core(now, utc(2026, 4, 21, 11, 59, 59)).starts_with("1m"));
    }

    #[test]
    fn minutes_bucket_boundary() {
        let now = utc(2026, 4, 21, 12, 0, 0);
        assert!(format_relative_core(now, utc(2026, 4, 21, 11, 1, 0)).starts_with("59m"));
        assert!(format_relative_core(now, utc(2026, 4, 21, 11, 0, 0)).starts_with("1h"));
    }

    #[test]
    fn hours_bucket_boundary() {
        let now = utc(2026, 4, 21, 12, 0, 0);
        assert!(format_relative_core(now, utc(2026, 4, 20, 13, 0, 0)).starts_with("23h"));
        assert!(format_relative_core(now, utc(2026, 4, 20, 12, 0, 0)).starts_with("1d"));
    }

    #[test]
    fn days_bucket_boundary_transitions_to_absolute() {
        let now = utc(2026, 4, 21, 12, 0, 0);
        let last_relative = utc(2026, 4, 8, 12, 0, 0);
        let first_absolute = utc(2026, 4, 7, 12, 0, 0);
        assert!(format_relative_core(now, last_relative).starts_with("13d"));
        assert_eq!(
            format_relative_core(now, first_absolute),
            "Apr 07 12:00"
        );
    }

    #[test]
    fn same_day_entries_get_distinct_hhmm() {
        let now = utc(2026, 5, 10, 12, 0, 0);
        let a = utc(2026, 4, 3, 14, 32, 0);
        let b = utc(2026, 4, 3, 9, 18, 0);
        assert_eq!(format_relative_core(now, a), "Apr 03 14:32");
        assert_eq!(format_relative_core(now, b), "Apr 03 09:18");
    }

    #[test]
    fn cross_year_uses_iso_prefix() {
        let now = utc(2026, 1, 15, 0, 0, 0);
        let then = utc(2025, 12, 31, 23, 59, 0);
        assert_eq!(format_relative_core(now, then), "2025-12-31 23:59");
    }

    #[test]
    fn output_is_padded_to_column_width() {
        let now = utc(2026, 4, 21, 12, 0, 0);
        let short = format_relative(now, utc(2026, 4, 21, 11, 0, 0));
        let long = format_relative(now, utc(2025, 1, 1, 0, 0, 0));
        assert_eq!(short.len(), TIME_COLUMN_WIDTH);
        assert_eq!(long.len(), TIME_COLUMN_WIDTH);
    }
}
