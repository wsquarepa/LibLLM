//! Accurate token counting against llama.cpp `/tokenize`, with a heuristic fallback.

use anyhow::Result;

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
#[expect(dead_code, reason = "TokenCounter in Task 4 will consume this type")]
pub(crate) enum TokenizerBackend {
    Server(ServerTokenizer),
    Heuristic(HeuristicTokenizer),
}

impl TokenizerBackend {
    #[expect(dead_code, reason = "TokenCounter in Task 4 will call this")]
    pub(crate) fn kind(&self) -> TokenizerKind {
        match self {
            Self::Server(_) => TokenizerKind::Server,
            Self::Heuristic(_) => TokenizerKind::Heuristic,
        }
    }

    #[expect(dead_code, reason = "TokenCounter in Task 4 will call this")]
    pub(crate) async fn count(&self, text: &str) -> Result<usize> {
        match self {
            Self::Server(s) => s.count(text).await,
            Self::Heuristic(h) => Ok(h.count(text)),
        }
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
}
