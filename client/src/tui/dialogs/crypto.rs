use libllm::crypto::DerivedKey;
use crate::tui::BackgroundEvent;

fn log_phase(kind: &str, phase: &str, result: &str, elapsed: std::time::Duration) {
    libllm::debug_log::log_kv(
        "unlock.phase",
        &[
            libllm::debug_log::field("kind", kind),
            libllm::debug_log::field("phase", phase),
            libllm::debug_log::field("result", result),
            libllm::debug_log::field(
                "elapsed_ms",
                format!("{:.3}", elapsed.as_secs_f64() * 1000.0),
            ),
        ],
    );
}

pub(super) fn log_phase_with_path(
    kind: &str,
    phase: &str,
    result: &str,
    elapsed: std::time::Duration,
    path: std::path::Display<'_>,
) {
    libllm::debug_log::log_kv(
        "unlock.phase",
        &[
            libllm::debug_log::field("kind", kind),
            libllm::debug_log::field("phase", phase),
            libllm::debug_log::field("result", result),
            libllm::debug_log::field(
                "elapsed_ms",
                format!("{:.3}", elapsed.as_secs_f64() * 1000.0),
            ),
            libllm::debug_log::field("path", path),
        ],
    );
}

fn log_phase_with_error(
    kind: &str,
    phase: &str,
    elapsed: std::time::Duration,
    error: &anyhow::Error,
) {
    libllm::debug_log::log_kv(
        "unlock.phase",
        &[
            libllm::debug_log::field("kind", kind),
            libllm::debug_log::field("phase", phase),
            libllm::debug_log::field("result", "error"),
            libllm::debug_log::field(
                "elapsed_ms",
                format!("{:.3}", elapsed.as_secs_f64() * 1000.0),
            ),
            libllm::debug_log::field("error", error),
        ],
    );
}

pub(in crate::tui) fn derive_key_blocking<F>(
    passkey: String,
    debug_kind: &str,
    apply: F,
) -> BackgroundEvent
where
    F: FnOnce(DerivedKey, &std::path::Path) -> BackgroundEvent,
{
    let total_start = std::time::Instant::now();
    let salt_path = libllm::config::salt_path();
    let check_path = libllm::config::key_check_path();

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

    let result = apply(derived_key, &check_path);
    log_phase(debug_kind, "blocking_total", "done", total_start.elapsed());
    result
}
