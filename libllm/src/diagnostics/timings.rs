//! Tracing layer that records span close times and renders the `--timings` report.

use std::cmp::Ordering;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use anyhow::{Context, Result};
use time::UtcOffset;
use time::macros::format_description;
use tracing::Subscriber;
use tracing::span::{Attributes, Id};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::Context as LayerContext;
use tracing_subscriber::registry::LookupSpan;

const DEFAULT_TIMESTAMP: &str = "1970-01-01 00:00:00.000 +00:00";

pub(crate) struct OwnedField {
    pub(crate) key: String,
    pub(crate) value: String,
}

impl OwnedField {
    pub(crate) fn new(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self { key: key.into(), value: value.into() }
    }
}

pub struct TimingCollector {
    path: PathBuf,
    run_mode: String,
    start_instant: Instant,
    start_wall: time::OffsetDateTime,
    samples: Vec<TimingSample>,
}

pub(crate) struct TimingSample {
    pub(crate) operation: String,
    pub(crate) elapsed_ms: f64,
    pub(crate) result: Option<String>,
    pub(crate) fields: Vec<OwnedField>,
}

#[derive(Default)]
struct TimingAggregate {
    count: usize,
    ok_count: usize,
    error_count: usize,
    total_ms: f64,
    max_ms: f64,
}

impl TimingCollector {
    pub fn new(path: PathBuf, run_mode: &str) -> Self {
        Self {
            path,
            run_mode: run_mode.to_owned(),
            start_instant: Instant::now(),
            start_wall: local_now(),
            samples: Vec::new(),
        }
    }

    pub(crate) fn record(&mut self, category: &str, fields: Vec<OwnedField>, elapsed_ms: f64) {
        let operation = operation_name(category, &fields);
        let result = fields.iter().find(|f| f.key == "result").map(|f| f.value.clone());
        self.samples.push(TimingSample { operation, elapsed_ms, result, fields });
    }

    pub fn write_report(&mut self, debug_path: &Path) -> Result<()> {
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

        let mut rows: Vec<(String, TimingAggregate)> = aggregates.into_iter().collect();
        rows.sort_by(|left, right| {
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
        for (operation, aggregate) in &rows {
            let avg = aggregate.total_ms / aggregate.count as f64;
            writeln!(
                file,
                "{:<44} {:>7} {:>5} {:>7} {:>12.3} {:>12.3} {:>12.3}",
                truncate(operation, 44),
                aggregate.count,
                aggregate.ok_count,
                aggregate.error_count,
                aggregate.total_ms,
                avg,
                aggregate.max_ms,
            )?;
        }

        let mut slowest: Vec<&TimingSample> = self.samples.iter().collect();
        slowest.sort_by(|l, r| {
            r.elapsed_ms.partial_cmp(&l.elapsed_ms).unwrap_or(Ordering::Equal)
        });

        writeln!(file)?;
        writeln!(file, "Slowest Samples")?;
        for (i, sample) in slowest.into_iter().take(25).enumerate() {
            writeln!(file, "{}. {:.3} ms | {}", i + 1, sample.elapsed_ms, sample.operation)?;
            let detail = build_kv(&sample.fields, &["elapsed_ms"]);
            if !detail.is_empty() {
                writeln!(file, "   {}", detail)?;
            }
        }
        Ok(())
    }
}

pub struct TimingLayer {
    collector: Arc<Mutex<TimingCollector>>,
    debug_path: PathBuf,
}

impl TimingLayer {
    pub fn new(collector: Arc<Mutex<TimingCollector>>, debug_path: PathBuf) -> Self {
        Self { collector, debug_path }
    }

    pub fn finalize(&self) -> Result<()> {
        let mut collector = self.collector.lock().unwrap_or_else(|p| p.into_inner());
        collector.write_report(&self.debug_path)
    }
}

struct SpanTiming {
    opened_at: Instant,
    category: String,
    fields: Vec<OwnedField>,
}

impl<S> Layer<S> for TimingLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(&self, attrs: &Attributes<'_>, id: &Id, ctx: LayerContext<'_, S>) {
        let Some(span) = ctx.span(id) else { return };
        let mut visitor = SpanFieldVisitor::default();
        attrs.record(&mut visitor);
        span.extensions_mut().insert(SpanTiming {
            opened_at: Instant::now(),
            category: attrs.metadata().name().to_owned(),
            fields: visitor.fields,
        });
    }

    fn on_close(&self, id: Id, ctx: LayerContext<'_, S>) {
        let Some(span) = ctx.span(&id) else { return };
        let Some(timing) = span.extensions_mut().remove::<SpanTiming>() else { return };
        let elapsed_ms = timing.opened_at.elapsed().as_secs_f64() * 1000.0;
        let Ok(mut collector) = self.collector.lock() else { return };
        collector.record(&timing.category, timing.fields, elapsed_ms);
    }
}

#[derive(Default)]
struct SpanFieldVisitor {
    fields: Vec<OwnedField>,
}

impl tracing::field::Visit for SpanFieldVisitor {
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        self.fields.push(OwnedField::new(field.name(), value));
    }
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        self.fields.push(OwnedField::new(field.name(), format!("{value:?}")));
    }
    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        self.fields.push(OwnedField::new(field.name(), value.to_string()));
    }
    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        self.fields.push(OwnedField::new(field.name(), value.to_string()));
    }
    fn record_bool(&mut self, field: &tracing::field::Field, value: bool) {
        self.fields.push(OwnedField::new(field.name(), value.to_string()));
    }
    fn record_f64(&mut self, field: &tracing::field::Field, value: f64) {
        self.fields.push(OwnedField::new(field.name(), value.to_string()));
    }
}

fn operation_name(category: &str, fields: &[OwnedField]) -> String {
    let mut selected = Vec::new();
    for key in ["phase", "name", "label", "kind", "mode"] {
        if let Some(field) = fields.iter().find(|f| f.key == key) {
            selected.push(format!("{}={}", field.key, field.value));
        }
    }
    if selected.is_empty() {
        category.to_owned()
    } else {
        format!("{category} {}", selected.join(" "))
    }
}

fn build_kv(fields: &[OwnedField], exclude_keys: &[&str]) -> String {
    let mut out = String::new();
    for field in fields {
        if exclude_keys.contains(&field.key.as_str()) {
            continue;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(&field.key);
        out.push('=');
        out.push_str(&field.value);
    }
    out
}

fn truncate(value: &str, max_len: usize) -> String {
    value.chars().take(max_len).collect()
}

fn local_now() -> time::OffsetDateTime {
    let now = time::OffsetDateTime::now_utc();
    match UtcOffset::current_local_offset() {
        Ok(offset) => now.to_offset(offset),
        Err(_) => now,
    }
}

fn format_timestamp(ts: time::OffsetDateTime) -> String {
    ts.format(format_description!(
        "[year]-[month]-[day] [hour]:[minute]:[second].[subsecond digits:3] [offset_hour sign:mandatory]:[offset_minute]"
    ))
    .unwrap_or_else(|_| DEFAULT_TIMESTAMP.to_owned())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operation_name_appends_phase_field() {
        let fields = vec![OwnedField::new("phase", "bar")];
        assert_eq!(operation_name("status", &fields), "status phase=bar");
    }

    #[test]
    fn operation_name_falls_back_to_category_when_no_selected_fields() {
        let fields = vec![OwnedField::new("message_count", "14")];
        assert_eq!(operation_name("chat", &fields), "chat");
    }
}
