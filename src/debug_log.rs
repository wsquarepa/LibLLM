#[cfg(debug_assertions)]
mod debug_impl {
    use std::io::Write;
    use std::sync::OnceLock;
    use std::time::Instant;

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
        let Some(mutex) = DEBUG_FILE.get() else { return };
        let elapsed = START_TIME.get().map(|s| s.elapsed().as_secs_f64()).unwrap_or(0.0);
        if let Ok(mut file) = mutex.lock() {
            let _ = writeln!(file, "[{elapsed:.3}s] {category}: {message}");
        }
    }

    pub fn timed<T>(category: &str, label: &str, f: impl FnOnce() -> T) -> T {
        if !enabled() {
            return f();
        }
        let start = Instant::now();
        let result = f();
        let elapsed_us = start.elapsed().as_micros();
        let elapsed_ms = elapsed_us as f64 / 1000.0;
        log(category, &format!("{elapsed_ms:.3}ms ({label})"));
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
pub fn timed<T>(_: &str, _: &str, f: impl FnOnce() -> T) -> T { f() }
