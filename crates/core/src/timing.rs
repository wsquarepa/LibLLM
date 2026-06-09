//! Span-timing macro used across the workspace. `#[macro_export]` places
//! `timed_result!` at the crate root (`libllm_core::timed_result!`).

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
