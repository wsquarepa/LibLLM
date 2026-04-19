//! Accurate token counting against llama.cpp `/tokenize`, with a heuristic fallback.

use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use lru::LruCache;
use tokio::sync::mpsc;

use crate::client::ApiClient;

/// Identifies which backend a `TokenCounter` is using. Exposed for UI prefix logic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenizerKind {
    Server,
    Heuristic,
}

/// Authoritative tokenizer that delegates to `ApiClient::tokenize`.
#[derive(Clone)]
pub struct ServerTokenizer {
    client: ApiClient,
}

impl ServerTokenizer {
    pub fn new(client: ApiClient) -> Self {
        Self { client }
    }

    pub async fn count(&self, text: &str) -> Result<usize> {
        self.client.tokenize(text, true).await
    }
}

/// Offline estimator matching the original 4-chars-per-token heuristic with a per-message overhead.
#[derive(Clone)]
pub struct HeuristicTokenizer {
    chars_per_token: usize,
    overhead_per_message: usize,
}

impl HeuristicTokenizer {
    pub fn new(chars_per_token: usize, overhead_per_message: usize) -> Self {
        Self {
            chars_per_token,
            overhead_per_message,
        }
    }

    /// Default is the existing 4-chars-per-token + 4-per-message overhead.
    pub fn standard() -> Self {
        Self::new(4, 4)
    }

    pub fn count(&self, text: &str) -> usize {
        text.len() / self.chars_per_token + self.overhead_per_message
    }
}

/// Internal backend dispatch. Kept non-public; `TokenCounter` is the external surface.
pub enum TokenizerBackend {
    Server(ServerTokenizer),
    Heuristic(HeuristicTokenizer),
}

impl TokenizerBackend {
    pub fn kind(&self) -> TokenizerKind {
        match self {
            Self::Server(_) => TokenizerKind::Server,
            Self::Heuristic(_) => TokenizerKind::Heuristic,
        }
    }

    pub async fn count(&self, text: &str) -> Result<usize> {
        match self {
            Self::Server(s) => s.count(text).await,
            Self::Heuristic(h) => Ok(h.count(text)),
        }
    }
}

/// A single result posted from the background refresh task.
#[derive(Debug)]
pub struct TokenCountUpdate {
    pub key: u64,
    pub result: Result<usize>,
}

/// Sync-readable token-count state for UI consumers.
#[derive(Debug, Clone, Copy)]
pub enum CountState {
    /// Cached authoritative count for the given text.
    Authoritative(usize),
    /// Previous authoritative count for a text that has since changed; refresh is in flight.
    Stale(usize),
    /// No authoritative count yet; heuristic value for this frame.
    Estimated(usize),
}

const DEFAULT_CACHE_CAPACITY: usize = 512;

/// Facade over `TokenizerBackend` that exposes a sync read API and an async refresh pipeline.
/// Lives on `App`; cloned refs are cheap (all internals are `Arc`-wrapped).
pub struct TokenCounter {
    backend: Arc<TokenizerBackend>,
    fallback: HeuristicTokenizer,
    cache: Arc<Mutex<LruCache<u64, usize>>>,
    pending: Arc<Mutex<std::collections::HashSet<u64>>>,
    refresh_tx: mpsc::Sender<TokenCountUpdate>,
}

impl TokenCounter {
    /// Construct directly from a chosen backend. Used by tests and by `TokenCounter::new` once
    /// the startup probe in Task 5 decides which backend applies.
    pub fn new_with_backend(
        backend: TokenizerBackend,
        refresh_tx: mpsc::Sender<TokenCountUpdate>,
    ) -> Self {
        let capacity = NonZeroUsize::new(DEFAULT_CACHE_CAPACITY)
            .expect("cache capacity must be non-zero");
        Self {
            backend: Arc::new(backend),
            fallback: HeuristicTokenizer::standard(),
            cache: Arc::new(Mutex::new(LruCache::new(capacity))),
            pending: Arc::new(Mutex::new(std::collections::HashSet::new())),
            refresh_tx,
        }
    }

    pub fn is_heuristic(&self) -> bool {
        self.backend.kind() == TokenizerKind::Heuristic
    }

    pub fn kind(&self) -> TokenizerKind {
        self.backend.kind()
    }

    /// Returns a heuristic count for arbitrary text without touching the cache or backend.
    /// Used by the input-box render path.
    pub fn heuristic_count(&self, text: &str) -> usize {
        self.fallback.count(text)
    }

    fn hash_key(text: &str) -> u64 {
        xxhash_rust::xxh3::xxh3_64(text.as_bytes())
    }

    /// Sync read. Never awaits. Enqueues a background refresh on miss.
    pub fn count_cached(&self, text: &str) -> CountState {
        let key = Self::hash_key(text);

        if let Some(&n) = self.cache.lock().expect("tokenizer cache poisoned").get(&key) {
            return CountState::Authoritative(n);
        }

        let should_dispatch = {
            let mut pending = self.pending.lock().expect("tokenizer pending poisoned");
            pending.insert(key)
        };

        if should_dispatch {
            self.spawn_refresh(key, text.to_owned());
        }

        CountState::Estimated(self.fallback.count(text))
    }

    /// Async read used by pre-send. Awaits the backend directly on a miss, writes result to cache.
    pub async fn count_authoritative(&self, text: &str) -> Result<usize> {
        let key = Self::hash_key(text);
        if let Some(&n) = self.cache.lock().expect("tokenizer cache poisoned").get(&key) {
            return Ok(n);
        }
        let n = self.backend.count(text).await?;
        self.cache
            .lock()
            .expect("tokenizer cache poisoned")
            .put(key, n);
        Ok(n)
    }

    /// Called by the main event loop when a `TokenCountUpdate` arrives from a background task.
    /// Writes successful counts into the cache. Errors are already logged by the refresh task.
    pub fn apply_update(&self, update: TokenCountUpdate) {
        self.pending
            .lock()
            .expect("tokenizer pending poisoned")
            .remove(&update.key);
        if let Ok(n) = update.result {
            self.cache
                .lock()
                .expect("tokenizer cache poisoned")
                .put(update.key, n);
        }
    }

    fn spawn_refresh(&self, key: u64, text: String) {
        let backend = Arc::clone(&self.backend);
        let pending = Arc::clone(&self.pending);
        let tx = self.refresh_tx.clone();
        tokio::spawn(async move {
            let result = backend.count(&text).await;
            if let Err(ref err) = result {
                tracing::warn!(
                    phase = "refresh",
                    result = "error",
                    text_bytes = text.len(),
                    error = %err,
                    "tokenizer.refresh"
                );
            }
            if tx
                .send(TokenCountUpdate { key, result })
                .await
                .is_err()
            {
                // Main loop is gone; clear pending so we don't leak the key.
                pending
                    .lock()
                    .expect("tokenizer pending poisoned")
                    .remove(&key);
            }
        });
    }

    #[cfg(test)]
    pub(crate) fn insert_cached_for_test(&self, text: &str, count: usize) {
        let key = Self::hash_key(text);
        self.cache
            .lock()
            .expect("tokenizer cache poisoned")
            .put(key, count);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heuristic_matches_legacy_formula() {
        let t = HeuristicTokenizer::standard();
        assert_eq!(t.count("a]bc"), 4 / 4 + 4);
        assert_eq!(t.count("12345678"), 8 / 4 + 4);
        assert_eq!(t.count(""), 4);
    }

    #[test]
    fn heuristic_tunable_overhead() {
        let t = HeuristicTokenizer::new(4, 0);
        assert_eq!(t.count("12345678"), 2);
    }

    #[tokio::test]
    async fn counter_cache_hit_returns_authoritative() {
        let (tx, _rx) = tokio::sync::mpsc::channel::<TokenCountUpdate>(8);
        let counter = TokenCounter::new_with_backend(
            TokenizerBackend::Heuristic(HeuristicTokenizer::standard()),
            tx,
        );
        // Prime the cache with a known value.
        counter.insert_cached_for_test("hello", 42);
        match counter.count_cached("hello") {
            CountState::Authoritative(n) => assert_eq!(n, 42),
            other => panic!("expected Authoritative, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn counter_cache_miss_returns_estimate_and_enqueues() {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<TokenCountUpdate>(8);
        let counter = TokenCounter::new_with_backend(
            TokenizerBackend::Heuristic(HeuristicTokenizer::standard()),
            tx,
        );
        match counter.count_cached("abcdefgh") {
            CountState::Estimated(n) => {
                assert_eq!(n, HeuristicTokenizer::standard().count("abcdefgh"));
            }
            other => panic!("expected Estimated, got {other:?}"),
        }
        // A refresh should have been posted to rx.
        let update = tokio::time::timeout(std::time::Duration::from_millis(250), rx.recv())
            .await
            .expect("timed out waiting for refresh")
            .expect("channel closed");
        assert!(update.result.is_ok());
    }
}
