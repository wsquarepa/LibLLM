//! Structured diagnostic logging with timing aggregation for performance analysis.

use std::cmp::Ordering;
use std::fmt;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Instant;

use anyhow::{Context, Result, anyhow};
use sysinfo::{RefreshKind, System};
use time::UtcOffset;
use time::macros::format_description;

const TEMP_LOG_PREFIX: &str = "libllm-debug-";
const DEFAULT_TIMESTAMP: &str = "1970-01-01 00:00:00.000 +00:00";

/// A key-value pair for structured log entries.
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

#[derive(Clone)]
struct OwnedField {
    key: String,
    value: String,
}

impl OwnedField {
    fn new(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            value: value.into(),
        }
    }

    fn from_field(field: &Field<'_>) -> Self {
        Self::new(field.key, field.value.clone())
    }
}

/// Constructs a `Field` from a key and any `Display` value.
pub fn field<'a>(key: &'a str, value: impl fmt::Display) -> Field<'a> {
    Field::new(key, value)
}

pub struct CleanupSummary {
    pub removed: usize,
    pub failed: usize,
}

pub struct DiagnosticsGuard;

struct DiagnosticsState {
    debug_path: PathBuf,
    debug_file: Mutex<File>,
    timings: Option<Mutex<TimingCollector>>,
}

struct TimingCollector {
    path: PathBuf,
    run_mode: String,
    start_instant: Instant,
    start_wall: time::OffsetDateTime,
    samples: Vec<TimingSample>,
}

struct TimingSample {
    operation: String,
    elapsed_ms: f64,
    result: Option<String>,
    fields: Vec<OwnedField>,
}

#[derive(Default)]
struct TimingAggregate {
    count: usize,
    ok_count: usize,
    error_count: usize,
    total_ms: f64,
    max_ms: f64,
}

static DIAGNOSTICS: OnceLock<DiagnosticsState> = OnceLock::new();

impl Drop for DiagnosticsGuard {
    fn drop(&mut self) {
        if let Some(state) = DIAGNOSTICS.get() {
            flush_debug_file(state);
            if let Some(timings) = state.timings.as_ref() {
                let mut collector = lock_mutex(timings);
                if let Err(err) = collector.write_report(&state.debug_path) {
                    eprintln!("Warning: failed to write timings report: {err}");
                }
            }
        }
    }
}

impl TimingCollector {
    fn new(path: PathBuf, run_mode: &str) -> Self {
        Self {
            path,
            run_mode: run_mode.to_owned(),
            start_instant: Instant::now(),
            start_wall: local_now(),
            samples: Vec::new(),
        }
    }

    fn record(&mut self, category: &str, fields: &[OwnedField], elapsed_ms: f64) {
        self.samples.push(TimingSample {
            operation: operation_name(category, fields),
            elapsed_ms,
            result: fields
                .iter()
                .find(|field| field.key == "result")
                .map(|field| field.value.clone()),
            fields: fields.to_vec(),
        });
    }

    fn write_report(&mut self, debug_path: &Path) -> Result<()> {
        let end_wall = local_now();
        let run_duration_ms = self.start_instant.elapsed().as_secs_f64() * 1000.0;
        let mut file = create_output_file(&self.path, false, true).with_context(|| {
            format!("failed to create timings report at {}", self.path.display())
        })?;

        writeln!(file, "LibLLM Timings Report")?;
        writeln!(file, "Generated: {}", format_timestamp(end_wall))?;
        writeln!(file, "Run started: {}", format_timestamp(self.start_wall))?;
        writeln!(file, "Run ended: {}", format_timestamp(end_wall))?;
        writeln!(file, "Run mode: {}", self.run_mode)?;
        writeln!(file, "Run duration ms: {:.3}", run_duration_ms)?;
        writeln!(file, "Debug log: {}", debug_path.display())?;
        writeln!(file, "Sample count: {}", self.samples.len())?;
        writeln!(file)?;

        if self.samples.is_empty() {
            writeln!(file, "No timing samples were recorded.")?;
            return Ok(());
        }

        let mut aggregates = std::collections::BTreeMap::<String, TimingAggregate>::new();
        for sample in &self.samples {
            let aggregate = aggregates.entry(sample.operation.clone()).or_default();
            aggregate.count += 1;
            aggregate.total_ms += sample.elapsed_ms;
            aggregate.max_ms = aggregate.max_ms.max(sample.elapsed_ms);
            match sample.result.as_deref() {
                Some("ok") => aggregate.ok_count += 1,
                Some("error") => aggregate.error_count += 1,
                _ => {}
            }
        }

        let mut aggregate_rows: Vec<(String, TimingAggregate)> = aggregates.into_iter().collect();
        aggregate_rows.sort_by(|left, right| {
            right
                .1
                .total_ms
                .partial_cmp(&left.1.total_ms)
                .unwrap_or(Ordering::Equal)
                .then_with(|| left.0.cmp(&right.0))
        });

        writeln!(file, "Summary")?;
        writeln!(
            file,
            "{:<44} {:>7} {:>5} {:>7} {:>12} {:>12} {:>12}",
            "operation", "count", "ok", "error", "total_ms", "avg_ms", "max_ms"
        )?;
        for (operation, aggregate) in &aggregate_rows {
            let average_ms = aggregate.total_ms / aggregate.count as f64;
            writeln!(
                file,
                "{:<44} {:>7} {:>5} {:>7} {:>12.3} {:>12.3} {:>12.3}",
                truncate(operation, 44),
                aggregate.count,
                aggregate.ok_count,
                aggregate.error_count,
                aggregate.total_ms,
                average_ms,
                aggregate.max_ms,
            )?;
        }

        let mut slowest: Vec<&TimingSample> = self.samples.iter().collect();
        slowest.sort_by(|left, right| {
            right
                .elapsed_ms
                .partial_cmp(&left.elapsed_ms)
                .unwrap_or(Ordering::Equal)
        });

        writeln!(file)?;
        writeln!(file, "Slowest Samples")?;
        for (index, sample) in slowest.into_iter().take(25).enumerate() {
            writeln!(
                file,
                "{}. {:.3} ms | {}",
                index + 1,
                sample.elapsed_ms,
                sample.operation,
            )?;
            let detail = build_message(
                &sample
                    .fields
                    .iter()
                    .filter(|field| field.key != "elapsed_ms")
                    .cloned()
                    .collect::<Vec<_>>(),
            );
            if !detail.is_empty() {
                writeln!(file, "   {}", detail)?;
            }
        }

        Ok(())
    }
}

/// Initializes the global diagnostics system: opens the debug log file and optionally
/// sets up a timing collector for performance reporting.
///
/// Must be called exactly once per process. Returns a `DiagnosticsGuard` whose `Drop`
/// impl flushes logs and writes the timing report.
pub fn init(
    debug_override: Option<&Path>,
    timings_path: Option<&Path>,
    run_mode: &str,
    run_fields: &[Field<'_>],
) -> Result<DiagnosticsGuard> {
    let (debug_path, debug_file, temp_debug_log) = open_debug_file(debug_override)?;
    let timings =
        timings_path.map(|path| Mutex::new(TimingCollector::new(path.to_path_buf(), run_mode)));

    let state = DiagnosticsState {
        debug_path: debug_path.clone(),
        debug_file: Mutex::new(debug_file),
        timings,
    };

    DIAGNOSTICS
        .set(state)
        .map_err(|_| anyhow!("diagnostics already initialized"))?;

    log_kv(
        "run.start",
        &[
            field("mode", run_mode),
            field("version", env!("CARGO_PKG_VERSION")),
            field("pid", std::process::id()),
        ],
    );
    log_kv(
        "diagnostics",
        &[
            field("debug_log", debug_path.display()),
            field(
                "debug_log_kind",
                if temp_debug_log { "temp" } else { "custom" },
            ),
            field(
                "timings_report",
                timings_path
                    .map(|path| path.display().to_string())
                    .unwrap_or_else(|| "disabled".to_owned()),
            ),
        ],
    );
    log_kv(
        "build.info",
        &[
            field(
                "profile",
                if cfg!(debug_assertions) {
                    "debug"
                } else {
                    "release"
                },
            ),
            field("target_os", std::env::consts::OS),
            field("arch", std::env::consts::ARCH),
            field("family", std::env::consts::FAMILY),
        ],
    );

    if let Ok(exe) = std::env::current_exe() {
        log_kv("runtime.info", &[field("exe", exe.display())]);
    }
    if let Ok(cwd) = std::env::current_dir() {
        log_kv("runtime.info", &[field("cwd", cwd.display())]);
    }

    let mut system = System::new_with_specifics(RefreshKind::everything());
    system.refresh_all();
    let cpu_brand = system
        .cpus()
        .first()
        .map(|cpu| cpu.brand().to_owned())
        .unwrap_or_else(|| "unknown".to_owned());
    log_kv(
        "system.info",
        &[
            field(
                "host",
                System::host_name().unwrap_or_else(|| "unknown".to_owned()),
            ),
            field(
                "os_name",
                System::name().unwrap_or_else(|| "unknown".to_owned()),
            ),
            field(
                "os_version",
                System::os_version().unwrap_or_else(|| "unknown".to_owned()),
            ),
            field(
                "kernel",
                System::kernel_version().unwrap_or_else(|| "unknown".to_owned()),
            ),
            field("cpu_brand", cpu_brand),
            field("logical_cpus", system.cpus().len()),
            field("total_memory_bytes", system.total_memory()),
        ],
    );

    if !run_fields.is_empty() {
        log_kv("run.args", run_fields);
    }

    Ok(DiagnosticsGuard)
}

pub fn cleanup_temp_logs() -> Result<CleanupSummary> {
    let temp_dir = std::env::temp_dir();
    let entries = std::fs::read_dir(&temp_dir)
        .with_context(|| format!("failed to read temp directory: {}", temp_dir.display()))?;
    let mut removed = 0usize;
    let mut failed = 0usize;

    for entry in entries {
        let entry = match entry {
            Ok(entry) => entry,
            Err(_) => {
                failed += 1;
                continue;
            }
        };

        let file_name = entry.file_name();
        let file_name = file_name.to_string_lossy();
        if !file_name.starts_with(TEMP_LOG_PREFIX) || !file_name.ends_with(".log") {
            continue;
        }

        match std::fs::remove_file(entry.path()) {
            Ok(()) => removed += 1,
            Err(_) => failed += 1,
        }
    }

    Ok(CleanupSummary { removed, failed })
}

pub fn copy_current_log_to(path: &Path) -> Result<()> {
    let Some(state) = DIAGNOSTICS.get() else {
        anyhow::bail!("debug logging is not initialized")
    };

    flush_debug_file(state);

    let mut source = File::open(&state.debug_path).with_context(|| {
        format!(
            "failed to open active debug log at {}",
            state.debug_path.display()
        )
    })?;
    let mut destination = create_output_file(path, true, false)
        .with_context(|| format!("failed to create {}", path.display()))?;
    std::io::copy(&mut source, &mut destination)
        .with_context(|| format!("failed to copy debug log to {}", path.display()))?;
    destination.flush()?;
    Ok(())
}

/// Writes a structured log line and records timing data if an `elapsed_ms` field is present.
pub fn log_kv(category: &str, fields: &[Field<'_>]) {
    let owned = own_fields(fields);
    if let Some(elapsed_ms) = primary_elapsed_ms(&owned) {
        record_timing_sample(category, &owned, elapsed_ms);
    }
    let debug_fields = strip_debug_timing_fields(&owned);
    if !debug_fields.is_empty() {
        write_debug_fields(category, &debug_fields);
    }
}

/// Executes `f`, measures its wall-clock duration, and records a timing sample with the given fields.
pub fn timed_kv<T>(category: &str, fields: &[Field<'_>], f: impl FnOnce() -> T) -> T {
    let start = Instant::now();
    let result = f();
    let owned = own_fields(fields);
    let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
    record_timing_sample(category, &owned, elapsed_ms);
    if !owned.is_empty() {
        write_debug_fields(category, &owned);
    }
    result
}

/// Like `timed_kv`, but also appends `result=ok` or `result=error` based on the `Result` outcome.
pub fn timed_result<T, E>(
    category: &str,
    fields: &[Field<'_>],
    f: impl FnOnce() -> Result<T, E>,
) -> Result<T, E>
where
    E: fmt::Display,
{
    let start = Instant::now();
    let result = f();
    let mut owned = own_fields(fields);
    match &result {
        Ok(_) => owned.push(OwnedField::new("result", "ok")),
        Err(err) => {
            owned.push(OwnedField::new("result", "error"));
            owned.push(OwnedField::new("error", err.to_string()));
        }
    }
    let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
    record_timing_sample(category, &owned, elapsed_ms);
    write_debug_fields(category, &owned);
    result
}

fn own_fields(fields: &[Field<'_>]) -> Vec<OwnedField> {
    fields.iter().map(OwnedField::from_field).collect()
}

fn open_debug_file(debug_override: Option<&Path>) -> Result<(PathBuf, File, bool)> {
    match debug_override {
        Some(path) => {
            let file = create_output_file(path, false, true)
                .with_context(|| format!("failed to create debug log at {}", path.display()))?;
            Ok((path.to_path_buf(), file, false))
        }
        None => {
            let path = std::env::temp_dir().join(format!(
                "{TEMP_LOG_PREFIX}{}-{}.log",
                std::process::id(),
                uuid::Uuid::new_v4()
            ));
            let file = create_output_file(&path, true, false)
                .with_context(|| format!("failed to create debug log at {}", path.display()))?;
            Ok((path, file, true))
        }
    }
}

fn create_output_file(path: &Path, create_new: bool, truncate: bool) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options.write(true).create(true);
    if create_new {
        options.create_new(true);
    }
    if truncate {
        options.truncate(true);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

fn flush_debug_file(state: &DiagnosticsState) {
    let mut file = lock_mutex(&state.debug_file);
    let _ = file.flush();
}

fn write_log_line(state: &DiagnosticsState, category: &str, message: &str) {
    let mut file = lock_mutex(&state.debug_file);
    let timestamp = format_timestamp(local_now());
    let line = if message.is_empty() {
        format!("[{timestamp}] {category}")
    } else {
        format!("[{timestamp}] {category}: {message}")
    };
    let _ = writeln!(file, "{line}");
}

fn write_debug_fields(category: &str, fields: &[OwnedField]) {
    let Some(state) = DIAGNOSTICS.get() else {
        return;
    };
    let message = build_message(fields);
    write_log_line(state, category, &message);
}

fn build_message(fields: &[OwnedField]) -> String {
    let mut message = String::new();
    for field in fields {
        append_field(&mut message, &field.key, &field.value);
    }
    message
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

fn record_timing_sample(category: &str, fields: &[OwnedField], elapsed_ms: f64) {
    let Some(state) = DIAGNOSTICS.get() else {
        return;
    };
    let Some(timings) = state.timings.as_ref() else {
        return;
    };
    let mut collector = lock_mutex(timings);
    collector.record(category, fields, elapsed_ms);
}

fn primary_elapsed_ms(fields: &[OwnedField]) -> Option<f64> {
    fields
        .iter()
        .find(|field| field.key == "elapsed_ms")
        .and_then(|field| field.value.parse::<f64>().ok())
}

fn strip_debug_timing_fields(fields: &[OwnedField]) -> Vec<OwnedField> {
    fields
        .iter()
        .filter(|field| !field.key.ends_with("_elapsed_ms") && field.key != "elapsed_ms")
        .cloned()
        .collect()
}

fn operation_name(category: &str, fields: &[OwnedField]) -> String {
    let mut selected = Vec::new();
    for key in ["phase", "name", "label", "kind", "mode"] {
        if let Some(field) = fields.iter().find(|field| field.key == key) {
            selected.push(field.clone());
        }
    }
    if selected.is_empty() {
        category.to_owned()
    } else {
        format!("{category} {}", build_message(&selected))
    }
}

fn lock_mutex<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn local_now() -> time::OffsetDateTime {
    let now = time::OffsetDateTime::now_utc();
    match UtcOffset::current_local_offset() {
        Ok(offset) => now.to_offset(offset),
        Err(_) => now,
    }
}

fn format_timestamp(timestamp: time::OffsetDateTime) -> String {
    timestamp
        .format(format_description!(
            "[year]-[month]-[day] [hour]:[minute]:[second].[subsecond digits:3] [offset_hour sign:mandatory]:[offset_minute]"
        ))
        .unwrap_or_else(|_| DEFAULT_TIMESTAMP.to_owned())
}

fn truncate(value: &str, max_len: usize) -> String {
    let mut truncated = String::new();
    for ch in value.chars().take(max_len) {
        truncated.push(ch);
    }
    truncated
}
