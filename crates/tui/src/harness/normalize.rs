use std::sync::OnceLock;

use regex::Regex;

struct Patterns {
    age: Regex,
    ts_iso: Regex,
    ts_month: Regex,
    uuid: Regex,
}

fn patterns() -> &'static Patterns {
    static P: OnceLock<Patterns> = OnceLock::new();
    P.get_or_init(|| Patterns {
        age: Regex::new(r"\b\d+[smhd] ago\b").expect("age regex"),
        ts_iso: Regex::new(r"\b\d{4}-\d{2}-\d{2} \d{2}:\d{2}\b").expect("iso ts regex"),
        ts_month: Regex::new(r"\b[A-Z][a-z]{2} \d{2} \d{2}:\d{2}\b").expect("month ts regex"),
        uuid: Regex::new(
            r"\b[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}\b",
        )
        .expect("uuid regex"),
    })
}

/// Replaces volatile substrings with stable placeholders so screen assertions
/// and golden snapshots are reproducible.
///
/// The age and timestamp formats mirror `crates/cli/src/time.rs`
/// (`format_relative_core`): relative ages use units `s`, `m`, `h`, `d`
/// (e.g. `"3h ago"`); absolute timestamps use either `"Apr 03 14:32"`
/// (`%b %d %H:%M`) or `"2025-12-31 23:59"` (`%Y-%m-%d %H:%M`).
pub(crate) fn normalize(s: &str) -> String {
    let p = patterns();
    let s = p.age.replace_all(s, "<AGE>");
    let s = p.ts_iso.replace_all(&s, "<TIME>");
    let s = p.ts_month.replace_all(&s, "<TIME>");
    let s = p.uuid.replace_all(&s, "<UUID>");
    s.into_owned()
}

#[cfg(test)]
mod tests {
    use super::normalize;

    #[test]
    fn redacts_relative_ages() {
        assert_eq!(normalize("saved 12s ago"), "saved <AGE>");
        assert_eq!(normalize("3h ago / 2d ago"), "<AGE> / <AGE>");
    }

    #[test]
    fn redacts_absolute_timestamps() {
        assert_eq!(normalize("at 2025-12-31 23:59 done"), "at <TIME> done");
        assert_eq!(normalize("Apr 03 14:32"), "<TIME>");
    }

    #[test]
    fn redacts_uuids() {
        assert_eq!(
            normalize("id 550e8400-e29b-41d4-a716-446655440000 end"),
            "id <UUID> end",
        );
    }

    #[test]
    fn leaves_stable_text_untouched() {
        assert_eq!(
            normalize("model: test-model | tokens: 0"),
            "model: test-model | tokens: 0",
        );
    }
}
