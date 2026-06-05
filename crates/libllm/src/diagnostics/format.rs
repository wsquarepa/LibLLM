//! Custom `tracing_subscriber::Layer` that writes aligned event lines to the debug log file.

use std::fmt::Write as _;
use std::fs::File;
use std::io::Write as _;
use std::sync::Mutex;
use std::time::Instant;

use tracing::field::{Field, Visit};
use tracing::{Event, Subscriber};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context;

const TARGET_WIDTH: usize = 28;
const STRIP_PREFIXES: &[&str] = &["libllm::", "client::"];

pub(super) struct FileLayer {
    start: Instant,
    file: Mutex<File>,
}

impl FileLayer {
    pub(super) fn new(start: Instant, file: File) -> Self {
        Self {
            start,
            file: Mutex::new(file),
        }
    }

    fn write_line(&self, line: &str) {
        let Ok(mut file) = self.file.lock() else {
            return;
        };
        let _ = file.write_all(line.as_bytes());
        let _ = file.write_all(b"\n");
    }
}

impl<S: Subscriber> Layer<S> for FileLayer {
    fn on_event(&self, event: &Event<'_>, _: Context<'_, S>) {
        let elapsed = self.start.elapsed();
        let offset = format_offset(elapsed.as_secs(), elapsed.subsec_millis());
        let level = format_level(event.metadata().level());
        let target = format_target(
            event
                .metadata()
                .module_path()
                .unwrap_or(event.metadata().target()),
        );

        let mut visitor = FieldVisitor::default();
        event.record(&mut visitor);
        let message = visitor.message;
        let fields = visitor.fields;

        let mut line = String::with_capacity(128);
        let _ = write!(&mut line, "[{offset}] {level} {target}  ");
        if !message.is_empty() {
            let _ = write!(&mut line, "msg={}", quote_if_needed(&message));
        }
        for (k, v) in &fields {
            if !line.ends_with("  ") {
                line.push(' ');
            }
            let _ = write!(&mut line, "{}={}", k, quote_if_needed(v));
        }
        if line.ends_with("  ") {
            line.truncate(line.len() - 1);
        }
        self.write_line(&line);
    }
}

fn format_offset(secs: u64, millis: u32) -> String {
    let hours = secs / 3600;
    let minutes = (secs % 3600) / 60;
    let seconds = secs % 60;
    format!("+{:02}:{:02}:{:02}.{:03}", hours, minutes, seconds, millis)
}

fn format_level(level: &tracing::Level) -> &'static str {
    match *level {
        tracing::Level::TRACE => "TRACE",
        tracing::Level::DEBUG => "DEBUG",
        tracing::Level::INFO => "INFO ",
        tracing::Level::WARN => "WARN ",
        tracing::Level::ERROR => "ERROR",
    }
}

fn format_target(raw: &str) -> String {
    let stripped = STRIP_PREFIXES
        .iter()
        .find_map(|p| raw.strip_prefix(p))
        .unwrap_or(raw);
    let stripped_len = stripped.chars().count();
    if stripped_len <= TARGET_WIDTH {
        let mut out = String::with_capacity(TARGET_WIDTH);
        out.push_str(stripped);
        out.extend(std::iter::repeat_n(' ', TARGET_WIDTH - stripped_len));
        out
    } else {
        let mut out: String = stripped.chars().take(TARGET_WIDTH - 1).collect();
        out.push('…');
        out
    }
}

fn quote_if_needed(value: &str) -> String {
    let needs = value.is_empty()
        || value.bytes().any(
            |b| !matches!(b, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'-' | b'.' | b'/'),
        );
    if !needs {
        return value.to_owned();
    }
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(ch),
        }
    }
    out.push('"');
    out
}

#[derive(Default)]
struct FieldVisitor {
    message: String,
    fields: Vec<(String, String)>,
}

impl Visit for FieldVisitor {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.push(field.name(), value.to_owned());
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.push(field.name(), format!("{value:?}"));
    }

    fn record_i64(&mut self, field: &Field, value: i64) {
        self.push(field.name(), value.to_string());
    }

    fn record_u64(&mut self, field: &Field, value: u64) {
        self.push(field.name(), value.to_string());
    }

    fn record_bool(&mut self, field: &Field, value: bool) {
        self.push(field.name(), value.to_string());
    }

    fn record_f64(&mut self, field: &Field, value: f64) {
        self.push(field.name(), value.to_string());
    }
}

impl FieldVisitor {
    fn push(&mut self, name: &str, value: String) {
        if name == "message" {
            self.message = value;
        } else {
            self.fields.push((name.to_owned(), value));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offset_formatting_spans_hours_and_millis() {
        assert_eq!(format_offset(0, 0), "+00:00:00.000");
        assert_eq!(format_offset(59, 999), "+00:00:59.999");
        assert_eq!(format_offset(3_600, 0), "+01:00:00.000");
        assert_eq!(format_offset(36_000, 123), "+10:00:00.123");
    }

    #[test]
    fn levels_are_exactly_five_columns() {
        for level in [
            tracing::Level::TRACE,
            tracing::Level::DEBUG,
            tracing::Level::INFO,
            tracing::Level::WARN,
            tracing::Level::ERROR,
        ] {
            assert_eq!(format_level(&level).chars().count(), 5, "{level:?}");
        }
    }

    #[test]
    fn target_strips_libllm_prefix_and_pads() {
        let rendered = format_target("libllm::db::characters");
        assert_eq!(rendered.chars().count(), TARGET_WIDTH);
        assert!(rendered.starts_with("db::characters"));
    }

    #[test]
    fn target_strips_client_prefix_and_pads() {
        let rendered = format_target("client::tui::render");
        assert_eq!(rendered.chars().count(), TARGET_WIDTH);
        assert!(rendered.starts_with("tui::render"));
    }

    #[test]
    fn target_truncates_with_ellipsis_when_too_long() {
        let rendered = format_target("client::tui::dialogs::delete_confirmation");
        assert_eq!(rendered.chars().count(), TARGET_WIDTH);
        assert!(rendered.ends_with('…'));
        assert!(rendered.starts_with("tui::dialogs::delete_con"));
    }

    #[test]
    fn target_passes_unknown_prefixes_through() {
        let rendered = format_target("serde::de::Error");
        assert_eq!(rendered.chars().count(), TARGET_WIDTH);
        assert!(rendered.starts_with("serde::de::Error"));
    }

    #[test]
    fn quote_if_needed_leaves_simple_values_bare() {
        assert_eq!(quote_if_needed("hello"), "hello");
        assert_eq!(quote_if_needed("file-name.log"), "file-name.log");
        assert_eq!(quote_if_needed("/path/to/thing"), "/path/to/thing");
    }

    #[test]
    fn quote_if_needed_quotes_and_escapes() {
        assert_eq!(quote_if_needed(""), "\"\"");
        assert_eq!(quote_if_needed("with space"), "\"with space\"");
        assert_eq!(quote_if_needed("a\"b"), "\"a\\\"b\"");
        assert_eq!(quote_if_needed("line\nbreak"), "\"line\\nbreak\"");
    }
}
