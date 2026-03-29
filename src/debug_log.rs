use std::fmt;

pub struct Field<'a> {
    key: &'a str,
    value: String,
}

impl<'a> Field<'a> {
    pub fn new(key: &'a str, value: impl fmt::Display) -> Self {
        Self {
            key,
            value: value.to_string(),
        }
    }
}

pub fn field<'a>(key: &'a str, value: impl fmt::Display) -> Field<'a> {
    Field::new(key, value)
}

#[cfg(debug_assertions)]
mod debug_impl {
    use std::fmt;
    use std::io::Write;
    use std::sync::OnceLock;
    use std::time::Instant;

    use super::Field;

    static DEBUG_FILE: OnceLock<std::sync::Mutex<std::fs::File>> = OnceLock::new();
    static START_TIME: OnceLock<Instant> = OnceLock::new();

    pub fn init(path: &str) {
        let file = std::fs::File::create(path)
            .unwrap_or_else(|e| panic!("failed to create debug log at {path}: {e}"));
        DEBUG_FILE.set(std::sync::Mutex::new(file)).ok();
        START_TIME.set(Instant::now()).ok();
    }

    pub fn enabled() -> bool {
        DEBUG_FILE.get().is_some()
    }

    pub fn log(category: &str, message: &str) {
        let Some(mutex) = DEBUG_FILE.get() else {
            return;
        };
        let elapsed = START_TIME
            .get()
            .map(|s| s.elapsed().as_secs_f64())
            .unwrap_or(0.0);
        if let Ok(mut file) = mutex.lock() {
            let _ = writeln!(file, "[{elapsed:.3}s] {category}: {message}");
        }
    }

    fn needs_quotes(value: &str) -> bool {
        value.is_empty()
            || value.bytes().any(|byte| {
                !matches!(byte, b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'_' | b'-' | b'.' | b'/')
            })
    }

    fn append_field(output: &mut String, key: &str, value: &str) {
        if !output.is_empty() {
            output.push(' ');
        }
        output.push_str(key);
        output.push('=');
        if needs_quotes(value) {
            output.push('"');
            for ch in value.chars() {
                match ch {
                    '\\' => output.push_str("\\\\"),
                    '"' => output.push_str("\\\""),
                    '\n' => output.push_str("\\n"),
                    '\r' => output.push_str("\\r"),
                    '\t' => output.push_str("\\t"),
                    _ => output.push(ch),
                }
            }
            output.push('"');
        } else {
            output.push_str(value);
        }
    }

    fn build_message(fields: &[Field<'_>], elapsed_ms: Option<f64>) -> String {
        let mut message = String::new();
        for field in fields {
            append_field(&mut message, field.key, &field.value);
        }
        if let Some(elapsed_ms) = elapsed_ms {
            append_field(&mut message, "elapsed_ms", &format!("{elapsed_ms:.3}"));
        }
        message
    }

    pub fn log_kv(category: &str, fields: &[Field<'_>]) {
        log(category, &build_message(fields, None));
    }

    #[allow(dead_code)]
    pub fn timed<T>(category: &str, label: &str, f: impl FnOnce() -> T) -> T {
        if !enabled() {
            return f();
        }
        let start = Instant::now();
        let result = f();
        let elapsed_us = start.elapsed().as_micros();
        let elapsed_ms = elapsed_us as f64 / 1000.0;
        log_kv(
            category,
            &[
                Field::new("label", label),
                Field::new("elapsed_ms", format!("{elapsed_ms:.3}")),
            ],
        );
        result
    }

    pub fn timed_kv<T>(category: &str, fields: &[Field<'_>], f: impl FnOnce() -> T) -> T {
        if !enabled() {
            return f();
        }
        let start = Instant::now();
        let result = f();
        let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
        log(category, &build_message(fields, Some(elapsed_ms)));
        result
    }

    pub fn timed_result<T, E>(
        category: &str,
        fields: &[Field<'_>],
        f: impl FnOnce() -> Result<T, E>,
    ) -> Result<T, E>
    where
        E: fmt::Display,
    {
        if !enabled() {
            return f();
        }
        let start = Instant::now();
        let result = f();
        let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
        let mut message = build_message(fields, Some(elapsed_ms));
        match &result {
            Ok(_) => append_field(&mut message, "result", "ok"),
            Err(err) => {
                append_field(&mut message, "result", "error");
                append_field(&mut message, "error", &err.to_string());
            }
        }
        log(category, &message);
        result
    }
}

#[cfg(debug_assertions)]
pub use debug_impl::*;

#[cfg(not(debug_assertions))]
#[inline(always)]
pub fn log(_: &str, _: &str) {}

#[cfg(not(debug_assertions))]
#[inline(always)]
pub fn log_kv(_: &str, _: &[Field<'_>]) {}

#[cfg(not(debug_assertions))]
#[inline(always)]
#[allow(dead_code)]
pub fn timed<T>(_: &str, _: &str, f: impl FnOnce() -> T) -> T {
    f()
}

#[cfg(not(debug_assertions))]
#[inline(always)]
pub fn timed_kv<T>(_: &str, _: &[Field<'_>], f: impl FnOnce() -> T) -> T {
    f()
}

#[cfg(not(debug_assertions))]
#[inline(always)]
pub fn timed_result<T, E>(
    _: &str,
    _: &[Field<'_>],
    f: impl FnOnce() -> Result<T, E>,
) -> Result<T, E>
where
    E: fmt::Display,
{
    f()
}
