//! Accurate token counting against llama.cpp `/tokenize` or KoboldCPP
//! `/api/extra/tokencount`, with a heuristic fallback.

use std::num::NonZeroUsize;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use lru::LruCache;
use tokio::sync::mpsc;
use tokio::time::timeout;

use crate::client::ApiClient;

const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// Identifies which backend a `TokenCounter` is using. Exposed for UI prefix logic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TokenizerKind {
    Server,
    Heuristic,
}

/// Which server-side tokenize endpoint a `ServerTokenizer` talks to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerTokenizeFlavor {
    /// llama.cpp `POST /tokenize` with `{content, add_special}` returning `{tokens: [...]}`.
    LlamaCpp,
    /// KoboldCPP `POST /api/extra/tokencount` with `{prompt}` returning `{value, ids}`.
    KoboldCpp,
}

impl ServerTokenizeFlavor {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::LlamaCpp => "llama_cpp",
            Self::KoboldCpp => "kobold_cpp",
        }
    }
}

/// Authoritative tokenizer that delegates to one of the supported server tokenize endpoints.
#[derive(Clone)]
pub struct ServerTokenizer {
    client: ApiClient,
    flavor: ServerTokenizeFlavor,
}

impl ServerTokenizer {
    pub fn new(client: ApiClient, flavor: ServerTokenizeFlavor) -> Self {
        Self { client, flavor }
    }

    pub fn flavor(&self) -> ServerTokenizeFlavor {
        self.flavor
    }

    pub async fn count(&self, text: &str) -> Result<usize> {
        match self.flavor {
            ServerTokenizeFlavor::LlamaCpp => self.client.tokenize(text, true).await,
            ServerTokenizeFlavor::KoboldCpp => self.client.tokenize_kobold(text).await,
        }
    }
}

/// Offline estimator using a fixed-point tokens-per-char ratio plus a per-message overhead.
/// `chars_numerator / chars_denominator` is a tokens-per-char multiplier
/// (e.g. 10/33 ≈ 0.303 tokens/char, equivalent to 3.3 chars/token).
/// The formula is `ceil(text.len() * chars_numerator / chars_denominator) + overhead_per_message * message_count`.
#[derive(Clone)]
pub struct HeuristicTokenizer {
    chars_numerator: u32,
    chars_denominator: u32,
    overhead_per_message: usize,
}

impl HeuristicTokenizer {
    pub fn new(chars_numerator: u32, chars_denominator: u32, overhead_per_message: usize) -> Self {
        assert!(chars_denominator > 0, "chars_denominator must be non-zero");
        Self {
            chars_numerator,
            chars_denominator,
            overhead_per_message,
        }
    }

    /// 3.3 chars per token + 2 per-message overhead, ceiling-divided in integer arithmetic.
    pub fn standard() -> Self {
        Self::new(10, 33, 2)
    }

    pub fn count(&self, text: &str, message_count: usize) -> usize {
        let chars = text.len() as u64;
        let num = self.chars_numerator as u64;
        let den = self.chars_denominator as u64;
        let content = (chars * num).div_ceil(den) as usize;
        content + self.overhead_per_message * message_count
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

    /// Counts tokens in `text` using the active backend.
    ///
    /// The heuristic branch passes `message_count = 1` because every caller of
    /// `TokenCounter::count_authoritative` hands us a single already-assembled
    /// prompt string, which we account for as one logical message for overhead
    /// purposes. Multi-message heuristic estimates happen through `count_cached`
    /// and `heuristic_count`, which take `message_count` directly.
    pub async fn count(&self, text: &str) -> Result<usize> {
        match self {
            Self::Server(s) => s.count(text).await,
            Self::Heuristic(h) => Ok(h.count(text, 1)),
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
#[derive(Clone)]
pub struct TokenCounter {
    backend: Arc<TokenizerBackend>,
    fallback: HeuristicTokenizer,
    cache: Arc<Mutex<LruCache<u64, usize>>>,
    pending: Arc<Mutex<std::collections::HashSet<u64>>>,
    refresh_tx: mpsc::Sender<TokenCountUpdate>,
    last_authoritative: Arc<Mutex<Option<usize>>>,
}

impl TokenCounter {
    /// Construct directly from a chosen backend. Used by tests and by `TokenCounter::new` once
    /// the startup probe decides which backend applies.
    pub fn new_with_backend(
        backend: TokenizerBackend,
        refresh_tx: mpsc::Sender<TokenCountUpdate>,
    ) -> Self {
        let capacity =
            NonZeroUsize::new(DEFAULT_CACHE_CAPACITY).expect("cache capacity must be non-zero");
        Self {
            backend: Arc::new(backend),
            fallback: HeuristicTokenizer::standard(),
            cache: Arc::new(Mutex::new(LruCache::new(capacity))),
            pending: Arc::new(Mutex::new(std::collections::HashSet::new())),
            refresh_tx,
            last_authoritative: Arc::new(Mutex::new(None)),
        }
    }

    /// Probes server tokenize endpoints and picks the first one that succeeds: llama.cpp
    /// `/tokenize` first, then KoboldCPP `/api/extra/tokencount`. Falls back to
    /// `HeuristicTokenizer` when neither responds. Backend is fixed for the lifetime of
    /// this counter.
    pub async fn new(client: ApiClient, refresh_tx: mpsc::Sender<TokenCountUpdate>) -> Self {
        let llama = ServerTokenizer::new(client.clone(), ServerTokenizeFlavor::LlamaCpp);
        let llama_probe = match timeout(PROBE_TIMEOUT, llama.count("probe")).await {
            Ok(result) => result,
            Err(_) => Err(anyhow::anyhow!(
                "llama.cpp /tokenize probe timed out after {PROBE_TIMEOUT:?}"
            )),
        };
        let backend = match llama_probe {
            Ok(_) => {
                tracing::info!(
                    phase = "probe",
                    result = "ok",
                    kind = "server",
                    flavor = llama.flavor().as_str(),
                    "tokenizer.init"
                );
                TokenizerBackend::Server(llama)
            }
            Err(llama_err) => {
                let kobold = ServerTokenizer::new(client, ServerTokenizeFlavor::KoboldCpp);
                let kobold_probe = match timeout(PROBE_TIMEOUT, kobold.count("probe")).await {
                    Ok(result) => result,
                    Err(_) => Err(anyhow::anyhow!(
                        "KoboldCPP /api/extra/tokencount probe timed out after {PROBE_TIMEOUT:?}"
                    )),
                };
                match kobold_probe {
                    Ok(_) => {
                        tracing::info!(
                            phase = "probe",
                            result = "ok",
                            kind = "server",
                            flavor = kobold.flavor().as_str(),
                            llama_error = %llama_err,
                            "tokenizer.init"
                        );
                        TokenizerBackend::Server(kobold)
                    }
                    Err(kobold_err) => {
                        tracing::warn!(
                            phase = "probe",
                            result = "fallback",
                            kind = "heuristic",
                            llama_error = %llama_err,
                            kobold_error = %kobold_err,
                            "tokenizer.init"
                        );
                        TokenizerBackend::Heuristic(HeuristicTokenizer::standard())
                    }
                }
            }
        };
        Self::new_with_backend(backend, refresh_tx)
    }

    pub fn is_heuristic(&self) -> bool {
        self.backend.kind() == TokenizerKind::Heuristic
    }

    pub fn kind(&self) -> TokenizerKind {
        self.backend.kind()
    }

    /// When the active backend is a `ServerTokenizer`, returns which endpoint flavor it speaks.
    /// Returns `None` for the heuristic backend.
    pub fn server_flavor(&self) -> Option<ServerTokenizeFlavor> {
        match self.backend.as_ref() {
            TokenizerBackend::Server(s) => Some(s.flavor()),
            TokenizerBackend::Heuristic(_) => None,
        }
    }

    /// Returns a heuristic count for arbitrary text without touching the cache or backend.
    /// Used by the input-box render path.
    pub fn heuristic_count(&self, text: &str, message_count: usize) -> usize {
        self.fallback.count(text, message_count)
    }

    fn hash_key(text: &str) -> u64 {
        xxhash_rust::xxh3::xxh3_64(text.as_bytes())
    }

    /// Sync read. Never awaits. Enqueues a background refresh on miss.
    pub fn count_cached(&self, text: &str, message_count: usize) -> CountState {
        let key = Self::hash_key(text);

        if let Some(&n) = self
            .cache
            .lock()
            .expect("tokenizer cache poisoned")
            .get(&key)
        {
            return CountState::Authoritative(n);
        }

        let should_dispatch = {
            let mut pending = self.pending.lock().expect("tokenizer pending poisoned");
            pending.insert(key)
        };

        if should_dispatch {
            self.spawn_refresh(key, text.to_owned());
        }

        let is_server_backend = !self.is_heuristic();
        let last = *self
            .last_authoritative
            .lock()
            .expect("tokenizer last_authoritative poisoned");
        match (is_server_backend, last) {
            (true, Some(n)) => CountState::Stale(n),
            _ => CountState::Estimated(self.fallback.count(text, message_count)),
        }
    }

    /// Async read used by pre-send. Awaits the backend directly on a miss, writes result to cache.
    pub async fn count_authoritative(&self, text: &str) -> Result<usize> {
        let key = Self::hash_key(text);
        if let Some(&n) = self
            .cache
            .lock()
            .expect("tokenizer cache poisoned")
            .get(&key)
        {
            return Ok(n);
        }
        let n = self.backend.count(text).await?;
        self.cache
            .lock()
            .expect("tokenizer cache poisoned")
            .put(key, n);
        *self
            .last_authoritative
            .lock()
            .expect("tokenizer last_authoritative poisoned") = Some(n);
        Ok(n)
    }

    /// Authoritatively tokenize N strings in parallel. Returns one result per input in
    /// the same order. Failures are per-item: one bad request does not poison the others.
    pub async fn count_many_authoritative(&self, texts: &[&str]) -> Vec<Result<usize>> {
        let futures = texts.iter().map(|t| self.count_authoritative(t));
        futures::future::join_all(futures).await
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
            *self
                .last_authoritative
                .lock()
                .expect("tokenizer last_authoritative poisoned") = Some(n);
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
            if tx.send(TokenCountUpdate { key, result }).await.is_err() {
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
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn heuristic_matches_3_3_formula() {
        let t = HeuristicTokenizer::standard();
        // ceil(0 * 10 / 33) + 2 * 0 = 0
        assert_eq!(t.count("", 0), 0);
        // ceil(0 * 10 / 33) + 2 * 1 = 2
        assert_eq!(t.count("", 1), 2);
        // ceil(3 * 10 / 33) + 2 * 0 = ceil(30/33) = 1
        assert_eq!(t.count("abc", 0), 1);
        // ceil(5 * 10 / 33) + 2 * 1 = ceil(50/33) + 2 = 2 + 2 = 4
        assert_eq!(t.count("abcde", 1), 4);
        // ceil(33 * 10 / 33) + 2 * 0 = 10
        assert_eq!(t.count(&"a".repeat(33), 0), 10);
        // ceil(34 * 10 / 33) + 2 * 0 = ceil(340/33) = 11
        assert_eq!(t.count(&"a".repeat(34), 0), 11);
    }

    #[test]
    fn heuristic_tunable_overhead() {
        let t = HeuristicTokenizer::new(10, 33, 0);
        // no overhead; ceil(33 * 10 / 33) = 10
        assert_eq!(t.count(&"a".repeat(33), 1), 10);
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
        match counter.count_cached("hello", 1) {
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
        match counter.count_cached("abcdefgh", 1) {
            CountState::Estimated(n) => {
                assert_eq!(n, HeuristicTokenizer::standard().count("abcdefgh", 1));
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

    #[tokio::test]
    async fn new_probes_server_and_selects_server_backend() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/tokenize"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "tokens": [1, 2]
            })))
            .mount(&server)
            .await;

        let base = format!("{}/v1", server.uri());
        let client = ApiClient::new(&base, false, crate::config::Auth::None);
        let (tx, _rx) = tokio::sync::mpsc::channel(8);

        let counter = TokenCounter::new(client, tx).await;
        assert_eq!(counter.kind(), TokenizerKind::Server);
        assert_eq!(
            counter.server_flavor(),
            Some(ServerTokenizeFlavor::LlamaCpp)
        );
    }

    #[tokio::test]
    async fn new_falls_through_to_kobold_when_llama_cpp_probe_fails() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/tokenize"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/extra/tokencount"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "value": 1,
                "ids": [42]
            })))
            .mount(&server)
            .await;

        let base = format!("{}/v1", server.uri());
        let client = ApiClient::new(&base, false, crate::config::Auth::None);
        let (tx, _rx) = tokio::sync::mpsc::channel(8);

        let counter = TokenCounter::new(client, tx).await;
        assert_eq!(counter.kind(), TokenizerKind::Server);
        assert_eq!(
            counter.server_flavor(),
            Some(ServerTokenizeFlavor::KoboldCpp)
        );
    }

    #[tokio::test]
    async fn counter_returns_stale_after_first_authoritative_then_cache_miss() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/tokenize"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "tokens": vec![0u32; 9]
            })))
            .mount(&server)
            .await;
        let base = format!("{}/v1", server.uri());
        let client = ApiClient::new(&base, false, crate::config::Auth::None);
        let (tx, _rx) = tokio::sync::mpsc::channel(8);
        let counter = TokenCounter::new(client, tx).await;

        let n = counter.count_authoritative("first").await.unwrap();
        assert_eq!(n, 9);

        match counter.count_cached("different text", 1) {
            CountState::Stale(prev) => assert_eq!(prev, 9),
            other => panic!("expected Stale(9), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn new_falls_back_to_heuristic_when_probe_fails() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/tokenize"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let base = format!("{}/v1", server.uri());
        let client = ApiClient::new(&base, false, crate::config::Auth::None);
        let (tx, _rx) = tokio::sync::mpsc::channel(8);

        let counter = TokenCounter::new(client, tx).await;
        assert_eq!(counter.kind(), TokenizerKind::Heuristic);
    }

    #[tokio::test]
    async fn count_many_authoritative_preserves_input_order() {
        let (tx, _rx) = tokio::sync::mpsc::channel::<TokenCountUpdate>(8);
        let counter = TokenCounter::new_with_backend(
            TokenizerBackend::Heuristic(HeuristicTokenizer::standard()),
            tx,
        );

        // Heuristic: ceil(len * 10 / 33) + 2; distinct input lengths produce distinct counts,
        // so order preservation in the output is actually verifiable.
        let a = "a";
        let bbbb = "bbbb";
        let ccccccccc = "ccccccccc";
        let results = counter
            .count_many_authoritative(&[a, bbbb, ccccccccc])
            .await;
        assert_eq!(results.len(), 3);
        assert_eq!(
            results[0].as_ref().unwrap(),
            &HeuristicTokenizer::standard().count(a, 1)
        );
        assert_eq!(
            results[1].as_ref().unwrap(),
            &HeuristicTokenizer::standard().count(bbbb, 1)
        );
        assert_eq!(
            results[2].as_ref().unwrap(),
            &HeuristicTokenizer::standard().count(ccccccccc, 1)
        );
    }

    #[tokio::test]
    async fn count_many_authoritative_empty_slice_returns_empty() {
        let (tx, _rx) = tokio::sync::mpsc::channel::<TokenCountUpdate>(8);
        let counter = TokenCounter::new_with_backend(
            TokenizerBackend::Heuristic(HeuristicTokenizer::standard()),
            tx,
        );
        let results = counter.count_many_authoritative(&[]).await;
        assert!(results.is_empty());
    }
}
