//! Timestamp utilities: compact and ISO-8601 wall-clock formatters.

use chrono::{DateTime, Utc};

pub fn now_compact() -> String {
    compact_at(std::time::SystemTime::now())
}

pub fn now_iso8601() -> String {
    iso8601_at(std::time::SystemTime::now())
}

fn compact_at(at: std::time::SystemTime) -> String {
    DateTime::<Utc>::from(at)
        .format("%Y%m%d-%H%M%S")
        .to_string()
}

fn iso8601_at(at: std::time::SystemTime) -> String {
    DateTime::<Utc>::from(at)
        .format("%Y-%m-%dT%H:%M:%SZ")
        .to_string()
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
