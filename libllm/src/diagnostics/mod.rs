//! Diagnostics: banner, tracing subscriber, file log layer, timing aggregation.

mod banner;
mod format;
mod io_helpers;
mod subscriber;
mod sysinfo_snapshot;
mod timings;

pub use banner::BuildInfo;

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Instant;

use anyhow::{Context, Result, anyhow};
use time::macros::format_description;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use banner::{BannerContext, RuntimeInfo, render};
use format::FileLayer;
use io_helpers::{create_output_file, local_now};
use subscriber::resolve_filter;
use sysinfo_snapshot::{collect_system, collect_terminal};
use timings::{TimingCollector, TimingLayer};

const TEMP_LOG_PREFIX: &str = "libllm-debug-";

pub struct CleanupSummary {
    pub removed: usize,
    pub failed: usize,
}

pub struct DiagnosticsGuard;

struct DiagnosticsState {
    debug_path: PathBuf,
    debug_file: Mutex<File>,
    timing_layer_finalizer: Option<Box<dyn Fn() -> Result<()> + Send + Sync>>,
}

static DIAGNOSTICS: OnceLock<DiagnosticsState> = OnceLock::new();

impl Drop for DiagnosticsGuard {
    fn drop(&mut self) {
        let Some(state) = DIAGNOSTICS.get() else { return };
        if let Ok(mut file) = state.debug_file.lock() {
            let _ = file.flush();
        }
        if let Some(finalize) = state.timing_layer_finalizer.as_ref()
            && let Err(err) = finalize()
        {
            eprintln!("Warning: failed to write timings report: {err}");
        }
    }
}

pub struct InitParams<'a> {
    pub debug_override: Option<&'a Path>,
    pub timings_path: Option<&'a Path>,
    pub run_mode: &'a str,
    pub cli_args: String,
    pub build: BuildInfo,
    pub filter_flag: Option<&'a str>,
    pub filter_env: Option<&'a str>,
}

pub fn init(params: InitParams<'_>) -> Result<DiagnosticsGuard> {
    if DIAGNOSTICS.get().is_some() {
        anyhow::bail!("diagnostics already initialized");
    }

    let debug_opted_in = params.debug_override.is_some();
    let filter = resolve_filter(params.filter_flag, params.filter_env, debug_opted_in);

    let (debug_path, mut debug_file) = open_debug_file(params.debug_override)?;

    let wall_clock = format_wall_clock(local_now());
    let system = collect_system();
    let terminal = collect_terminal();
    let executable = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "unknown".to_owned());
    let working_dir = std::env::current_dir()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "unknown".to_owned());
    let runtime = RuntimeInfo {
        run_mode: params.run_mode.to_owned(),
        pid: std::process::id(),
        executable,
        working_dir,
        cli_args: params.cli_args,
        debug_log_path: debug_path.display().to_string(),
        timings_path: params
            .timings_path
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "disabled".to_owned()),
        filter_directive: filter.directive.clone(),
        filter_source: filter.source.to_owned(),
    };

    let banner_text = render(&BannerContext {
        build: &params.build,
        system: &system,
        terminal: &terminal,
        runtime: &runtime,
        wall_clock: &wall_clock,
    });
    debug_file
        .write_all(banner_text.as_bytes())
        .with_context(|| format!("failed to write banner to {}", debug_path.display()))?;
    debug_file.flush()?;

    let start = Instant::now();
    let file_layer = FileLayer::new(start, debug_file.try_clone()?);

    let (timing_layer, timing_finalizer) = match params.timings_path {
        Some(path) => {
            let collector = Arc::new(Mutex::new(TimingCollector::new(
                path.to_path_buf(),
                params.run_mode,
            )));
            let layer = TimingLayer::new(Arc::clone(&collector));
            let finalizer_path = debug_path.clone();
            let finalizer: Box<dyn Fn() -> Result<()> + Send + Sync> = Box::new(move || {
                let mut c = collector.lock().unwrap_or_else(|p| p.into_inner());
                c.write_report(&finalizer_path)
            });
            (Some(layer), Some(finalizer))
        }
        None => (None, None),
    };

    let env_filter = EnvFilter::try_new(&filter.directive)
        .with_context(|| format!("invalid filter directive: {}", filter.directive))?;

    let state = DiagnosticsState {
        debug_path: debug_path.clone(),
        debug_file: Mutex::new(debug_file),
        timing_layer_finalizer: timing_finalizer,
    };
    DIAGNOSTICS
        .set(state)
        .map_err(|_| anyhow!("diagnostics already initialized"))?;

    tracing_subscriber::registry()
        .with(env_filter)
        .with(file_layer)
        .with(timing_layer)
        .init();

    tracing::info!(
        version = params.build.version,
        mode = %params.run_mode,
        pid = std::process::id(),
        "run started"
    );

    Ok(DiagnosticsGuard)
}

pub fn cleanup_temp_logs() -> Result<CleanupSummary> {
    let temp_dir = std::env::temp_dir();
    let entries = std::fs::read_dir(&temp_dir)
        .with_context(|| format!("failed to read temp directory: {}", temp_dir.display()))?;
    let mut removed = 0usize;
    let mut failed = 0usize;
    for entry in entries {
        let Ok(entry) = entry else {
            failed += 1;
            continue;
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
        anyhow::bail!("diagnostics are not initialized");
    };
    if let Ok(mut file) = state.debug_file.lock() {
        let _ = file.flush();
    }
    let mut source = File::open(&state.debug_path).with_context(|| {
        format!(
            "failed to open active debug log at {}",
            state.debug_path.display()
        )
    })?;
    let mut destination = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
        .with_context(|| format!("failed to create {}", path.display()))?;
    std::io::copy(&mut source, &mut destination)
        .with_context(|| format!("failed to copy debug log to {}", path.display()))?;
    destination.flush()?;
    Ok(())
}

fn open_debug_file(debug_override: Option<&Path>) -> Result<(PathBuf, File)> {
    match debug_override {
        Some(path) => {
            let file = create_output_file(path, false, true).with_context(|| {
                format!("failed to create debug log at {}", path.display())
            })?;
            Ok((path.to_path_buf(), file))
        }
        None => {
            let path = std::env::temp_dir().join(format!(
                "{TEMP_LOG_PREFIX}{}-{}.log",
                std::process::id(),
                uuid::Uuid::new_v4()
            ));
            let file = create_output_file(&path, true, false).with_context(|| {
                format!("failed to create debug log at {}", path.display())
            })?;
            Ok((path, file))
        }
    }
}

fn format_wall_clock(ts: time::OffsetDateTime) -> String {
    ts.format(format_description!(
        "[year]-[month]-[day] [hour]:[minute]:[second]"
    ))
    .unwrap_or_else(|_| "1970-01-01 00:00:00".to_owned())
}

/// Wraps a block in a span at the given level, recording `elapsed_ms` and
/// `result=ok|error` on completion.
#[macro_export]
macro_rules! timed_result {
    ($level:expr, $name:expr, $($field_key:ident = $field_value:expr),* ; $body:block) => {{
        let __span = tracing::span!($level, $name, $($field_key = $field_value),*);
        let __start = std::time::Instant::now();
        let __result = __span.in_scope(|| $body);
        let __elapsed_ms = __start.elapsed().as_secs_f64() * 1000.0;
        match &__result {
            Ok(_) => tracing::event!(
                parent: &__span,
                $level,
                elapsed_ms = __elapsed_ms,
                result = "ok",
                "completed"
            ),
            Err(err) => tracing::event!(
                parent: &__span,
                $level,
                elapsed_ms = __elapsed_ms,
                result = "error",
                error = %err,
                "failed"
            ),
        }
        __result
    }};
}
