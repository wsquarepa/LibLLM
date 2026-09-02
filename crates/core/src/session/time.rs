//! Timestamp utilities: compact and ISO-8601 wall-clock formatters.

pub fn now_compact() -> String {
    compact_at(std::time::SystemTime::now())
}

pub fn now_iso8601() -> String {
    iso8601_at(std::time::SystemTime::now())
}

fn compact_at(at: std::time::SystemTime) -> String {
    let (year, month, day, hours, minutes, seconds) = wall_clock_parts_at(at);
    format!("{year:04}{month:02}{day:02}-{hours:02}{minutes:02}{seconds:02}")
}

fn iso8601_at(at: std::time::SystemTime) -> String {
    let (year, month, day, hours, minutes, seconds) = wall_clock_parts_at(at);
    format!("{year:04}-{month:02}-{day:02}T{hours:02}:{minutes:02}:{seconds:02}Z")
}

fn wall_clock_parts_at(at: std::time::SystemTime) -> (u64, u64, u64, u64, u64, u64) {
    let duration = at.duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    let secs = duration.as_secs();
    let time_secs = secs % 86400;
    let hours = time_secs / 3600;
    let minutes = (time_secs % 3600) / 60;
    let seconds = time_secs % 60;
    let (year, month, day) = days_to_ymd(secs / 86400);
    (year, month, day, hours, minutes, seconds)
}

fn days_to_ymd(mut days: u64) -> (u64, u64, u64) {
    let mut year = 1970u64;
    loop {
        let days_in_year = if is_leap(year) { 366 } else { 365 };
        if days < days_in_year {
            break;
        }
        days -= days_in_year;
        year += 1;
    }
    let month_days: [u64; 12] = if is_leap(year) {
        [31, 29, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    } else {
        [31, 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31]
    };
    let mut month = 0u64;
    for (i, &md) in month_days.iter().enumerate() {
        if days < md {
            month = i as u64 + 1;
            break;
        }
        days -= md;
    }
    (year, month, days + 1)
}

fn is_leap(year: u64) -> bool {
    (year.is_multiple_of(4) && !year.is_multiple_of(100)) || year.is_multiple_of(400)
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use super::{compact_at, iso8601_at};

    fn instant(seconds: u64) -> SystemTime {
        UNIX_EPOCH + Duration::from_secs(seconds)
    }

    #[test]
    fn compact_at_pins_known_instants() {
        let cases: [(u64, &str); 4] = [
            (1_700_000_000, "20231114-221320"),
            (1_709_210_096, "20240229-123456"),
            (1_709_251_200, "20240301-000000"),
            (951_782_400, "20000229-000000"),
        ];

        for (seconds, expected) in cases {
            assert_eq!(compact_at(instant(seconds)), expected);
        }
    }

    #[test]
    fn iso8601_at_pins_known_instants() {
        let cases: [(u64, &str); 4] = [
            (1_700_000_000, "2023-11-14T22:13:20Z"),
            (1_709_210_096, "2024-02-29T12:34:56Z"),
            (1_709_251_200, "2024-03-01T00:00:00Z"),
            (951_782_400, "2000-02-29T00:00:00Z"),
        ];

        for (seconds, expected) in cases {
            assert_eq!(iso8601_at(instant(seconds)), expected);
        }
    }
}
