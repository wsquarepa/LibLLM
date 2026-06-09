//! LibLLM's diagnostics infrastructure: tracing-subscriber setup, the diagnostics
//! global state, log-file management, the `--timings` report, and the startup
//! banner's sysinfo collection.

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

/// Errors from diagnostics initialization, log cleanup, log copying, and report writing.
#[derive(Debug, thiserror::Error)]
pub enum DiagnosticsError {
    #[error("{context}: {source}")]
    Io {
        context: String,
        #[source]
        source: std::io::Error,
    },

    #[error("diagnostics already initialized")]
    AlreadyInitialized,

    #[error("diagnostics are not initialized")]
    NotInitialized,

    #[error("no debug log file is active (run with --debug or --timings to enable)")]
    NoDebugLog,

    #[error("invalid filter directive: {directive}: {source}")]
    InvalidFilter {
        directive: String,
        #[source]
        source: tracing_subscriber::filter::ParseError,
    },

    /// Wraps an I/O error from writing a timings report or log file.
    #[error("write failed: {0}")]
    Write(#[from] std::io::Error),
}

pub struct CleanupSummary {
    pub removed: usize,
    pub failed: usize,
}

pub struct DiagnosticsGuard;

struct DiagnosticsState {
    debug_path: Option<PathBuf>,
    debug_file: Option<Mutex<File>>,
    timing_layer_finalizer: Option<Box<dyn Fn() -> Result<(), DiagnosticsError> + Send + Sync>>,
}

static DIAGNOSTICS: OnceLock<DiagnosticsState> = OnceLock::new();

impl Drop for DiagnosticsGuard {
    fn drop(&mut self) {
        let Some(state) = DIAGNOSTICS.get() else {
            return;
        };
        if let Some(file) = state.debug_file.as_ref()
            && let Ok(mut file) = file.lock()
        {
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

pub fn init(params: InitParams<'_>) -> Result<DiagnosticsGuard, DiagnosticsError> {
    if DIAGNOSTICS.get().is_some() {
        return Err(DiagnosticsError::AlreadyInitialized);
    }

    let debug_opted_in = params.debug_override.is_some();
    let filter = resolve_filter(params.filter_flag, params.filter_env, debug_opted_in);

    let needs_log_file = params.debug_override.is_some() || params.timings_path.is_some();
    let log_file_result = if needs_log_file {
        Some(open_debug_file(params.debug_override)?)
    } else {
        None
    };

    let debug_log_path_display = log_file_result
        .as_ref()
        .map(|(p, _)| p.display().to_string())
        .unwrap_or_else(|| "disabled".to_owned());

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
        debug_log_path: debug_log_path_display,
        timings_path: params
            .timings_path
            .map(|p| p.display().to_string())
            .unwrap_or_else(|| "disabled".to_owned()),
        filter_directive: filter.directive.clone(),
        filter_source: filter.source.to_owned(),
    };

    let start = Instant::now();
    let (debug_path, debug_file, file_layer) = match log_file_result {
        Some((path, mut file)) => {
            let banner_text = render(&BannerContext {
                build: &params.build,
                system: &system,
                terminal: &terminal,
                runtime: &runtime,
                wall_clock: &wall_clock,
            });
            file.write_all(banner_text.as_bytes())
                .map_err(|e| DiagnosticsError::Io {
                    context: format!("failed to write banner to {}", path.display()),
                    source: e,
                })?;
            file.flush().map_err(|e| DiagnosticsError::Io {
                context: "failed to flush banner".to_owned(),
                source: e,
            })?;
            let layer = FileLayer::new(
                start,
                file.try_clone().map_err(|e| DiagnosticsError::Io {
                    context: "failed to clone log file handle".to_owned(),
                    source: e,
                })?,
            );
            (Some(path), Some(file), Some(layer))
        }
        None => (None, None, None),
    };

    let (timing_layer, timing_finalizer) = match params.timings_path {
        Some(path) => {
            let collector = Arc::new(Mutex::new(TimingCollector::new(
                path.to_path_buf(),
                params.run_mode,
            )));
            let layer = TimingLayer::new(Arc::clone(&collector));
            let finalizer_path = debug_path.clone().unwrap_or_default();
            let finalizer: Box<dyn Fn() -> Result<(), DiagnosticsError> + Send + Sync> =
                Box::new(move || {
                    let mut c = collector.lock().unwrap_or_else(|p| p.into_inner());
                    c.write_report(&finalizer_path)
                });
            (Some(layer), Some(finalizer))
        }
        None => (None, None),
    };

    let env_filter = EnvFilter::try_new(&filter.directive).map_err(|source| {
        DiagnosticsError::InvalidFilter {
            directive: filter.directive.clone(),
            source,
        }
    })?;

    let state = DiagnosticsState {
        debug_path,
        debug_file: debug_file.map(Mutex::new),
        timing_layer_finalizer: timing_finalizer,
    };
    DIAGNOSTICS
        .set(state)
        .map_err(|_| DiagnosticsError::AlreadyInitialized)?;

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

pub fn cleanup_temp_logs() -> Result<CleanupSummary, DiagnosticsError> {
    let temp_dir = std::env::temp_dir();
    let entries = std::fs::read_dir(&temp_dir).map_err(|e| DiagnosticsError::Io {
        context: format!("failed to read temp directory: {}", temp_dir.display()),
        source: e,
    })?;
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

pub fn copy_current_log_to(path: &Path) -> Result<(), DiagnosticsError> {
    let Some(state) = DIAGNOSTICS.get() else {
        return Err(DiagnosticsError::NotInitialized);
    };
    let Some(ref debug_path) = state.debug_path else {
        return Err(DiagnosticsError::NoDebugLog);
    };
    if let Some(ref file) = state.debug_file
        && let Ok(mut file) = file.lock()
    {
        let _ = file.flush();
    }
    let mut source = File::open(debug_path).map_err(|e| DiagnosticsError::Io {
        context: format!(
            "failed to open active debug log at {}",
            debug_path.display()
        ),
        source: e,
    })?;
    let mut destination = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .open(path)
        .map_err(|e| DiagnosticsError::Io {
            context: format!("failed to create {}", path.display()),
            source: e,
        })?;
    std::io::copy(&mut source, &mut destination).map_err(|e| DiagnosticsError::Io {
        context: format!("failed to copy debug log to {}", path.display()),
        source: e,
    })?;
    destination.flush().map_err(|e| DiagnosticsError::Io {
        context: "failed to flush destination log file".to_owned(),
        source: e,
    })?;
    libllm_core::crypto::chmod_0600(path).map_err(|source| DiagnosticsError::Io {
        context: format!("failed to set permissions on {}", path.display()),
        source,
    })?;
    Ok(())
}

fn open_debug_file(debug_override: Option<&Path>) -> Result<(PathBuf, File), DiagnosticsError> {
    match debug_override {
        Some(path) => {
            let file = create_output_file(path, false, true).map_err(|e| DiagnosticsError::Io {
                context: format!("failed to create debug log at {}", path.display()),
                source: e,
            })?;
            Ok((path.to_path_buf(), file))
        }
        None => {
            let path = std::env::temp_dir().join(format!(
                "{TEMP_LOG_PREFIX}{}-{}.log",
                std::process::id(),
                uuid::Uuid::new_v4()
            ));
            let file =
                create_output_file(&path, true, false).map_err(|e| DiagnosticsError::Io {
                    context: format!("failed to create debug log at {}", path.display()),
                    source: e,
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
