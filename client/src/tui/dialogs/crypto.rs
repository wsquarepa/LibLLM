//! Background key derivation and database open/rekey for passkey dialogs.

use libllm::crypto::DerivedKey;
use crate::tui::BackgroundEvent;

fn log_phase(kind: &str, phase: &str, result: &str, elapsed: std::time::Duration) {
    let elapsed_ms = format!("{:.3}", elapsed.as_secs_f64() * 1000.0);
    tracing::info!(kind, phase, result, elapsed_ms, "unlock.phase");
}

pub(super) fn log_phase_with_path(
    kind: &str,
    phase: &str,
    result: &str,
    elapsed: std::time::Duration,
    path: std::path::Display<'_>,
) {
    let elapsed_ms = format!("{:.3}", elapsed.as_secs_f64() * 1000.0);
    let path = path.to_string();
    tracing::info!(kind, phase, result, elapsed_ms, path, "unlock.phase");
}

fn log_phase_with_error(
    kind: &str,
    phase: &str,
    elapsed: std::time::Duration,
    error: &anyhow::Error,
) {
    let elapsed_ms = format!("{:.3}", elapsed.as_secs_f64() * 1000.0);
    tracing::warn!(kind, phase, result = "error", elapsed_ms, error = %error, "unlock.phase");
}

pub(in crate::tui) fn derive_key_blocking<F>(
    passkey: String,
    debug_kind: &str,
    apply: F,
) -> BackgroundEvent
where
    F: FnOnce(DerivedKey) -> BackgroundEvent,
{
    let total_start = std::time::Instant::now();
    let salt_path = libllm::config::salt_path();

    let salt_start = std::time::Instant::now();
    let salt_result = libllm::crypto::load_or_create_salt(&salt_path);
    log_phase_with_path(
        debug_kind,
        "salt",
        if salt_result.is_ok() { "ok" } else { "error" },
        salt_start.elapsed(),
        salt_path.display(),
    );
    let salt = match salt_result {
        Ok(salt) => salt,
        Err(err) => {
            log_phase_with_error(debug_kind, "blocking_total", total_start.elapsed(), &err);
            return BackgroundEvent::KeyDeriveFailed(err.to_string());
        }
    };

    let derive_start = std::time::Instant::now();
    let derive_result = libllm::crypto::derive_key(&passkey, &salt);
    log_phase(
        debug_kind,
        "argon2",
        if derive_result.is_ok() { "ok" } else { "error" },
        derive_start.elapsed(),
    );
    let derived_key = match derive_result {
        Ok(key) => key,
        Err(err) => {
            log_phase_with_error(debug_kind, "blocking_total", total_start.elapsed(), &err);
            return BackgroundEvent::KeyDeriveFailed(err.to_string());
        }
    };

    let result = apply(derived_key);
    log_phase(debug_kind, "blocking_total", "done", total_start.elapsed());
    result
}
