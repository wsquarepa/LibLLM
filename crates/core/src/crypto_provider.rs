//! Installs the process-wide rustls crypto provider exactly once.

use std::sync::OnceLock;

static INSTALLED: OnceLock<()> = OnceLock::new();

/// Installs `ring` as the default rustls crypto provider for this process.
///
/// Safe to call from any thread any number of times; only the first call installs.
/// Subsequent calls are no-ops. Callers that construct a `reqwest::Client` with
/// `rustls-no-provider` must ensure this has run first, or the builder will panic.
pub fn install_default_crypto_provider() {
    INSTALLED.get_or_init(|| {
        rustls::crypto::ring::default_provider()
            .install_default()
            .ok();
    });
}
