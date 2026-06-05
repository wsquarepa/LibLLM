//! Pure file-summary types and lookup traits consumed by `Summarizer::format_prompt`.
//!
//! The stateful orchestrator lives in `client::file_summarizer` because it
//! depends on both the database and the HTTP client.

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::files::error::FileError;

/// Lifecycle of a cached file summary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FileSummaryStatus {
    Pending,
    Done,
    Failed,
}

impl FileSummaryStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Done => "done",
            Self::Failed => "failed",
        }
    }

    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "pending" => Ok(Self::Pending),
            "done" => Ok(Self::Done),
            "failed" => Ok(Self::Failed),
            other => Err(anyhow::anyhow!("unknown file_summaries.status: {other}")),
        }
    }
}

/// Snapshot of one cached file summary as surfaced to consumers.
#[derive(Debug, Clone)]
pub struct FileSummary {
    pub basename: String,
    pub summary: String,
    pub status: FileSummaryStatus,
}

/// Input passed to the orchestrator's schedule / ensure_ready methods:
/// everything needed to dedupe and, if necessary, summarise a file.
#[derive(Debug, Clone)]
pub struct FileToSummarize {
    pub basename: String,
    pub content_hash: String,
    pub body: String,
}

/// Broadcast when a row transitions out of `pending`. Consumed by the TUI
/// to invalidate the chat cache so the new state renders on the next tick.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadyEvent {
    pub session_id: String,
    pub content_hash: String,
    pub status: FileSummaryStatus,
}

/// Look up a cached summary by `content_hash`. Implementations carry their
/// own session scope.
pub trait FileSummaryLookup: Send + Sync {
    fn lookup(&self, content_hash: &str) -> Option<FileSummary>;
}

/// `FileSummaryLookup` impl for contexts that have no cache (e.g. tests,
/// or single-run CLI paths). Always returns `None`.
pub struct NullFileSummaryLookup;

impl FileSummaryLookup for NullFileSummaryLookup {
    fn lookup(&self, _content_hash: &str) -> Option<FileSummary> {
        None
    }
}

/// Object-safe view into a store that can look up summaries given both a
/// session id and a hash.
pub trait SessionScopedLookup: Send + Sync {
    fn lookup(&self, session_id: &str, content_hash: &str) -> Option<FileSummary>;
}

/// Pairs a session id with a store that implements `SessionScopedLookup`,
/// producing an unscoped `FileSummaryLookup` usable by the summariser.
pub struct ScopedFileSummaryLookup<'a> {
    pub session_id: &'a str,
    pub resolver: &'a dyn SessionScopedLookup,
}

impl FileSummaryLookup for ScopedFileSummaryLookup<'_> {
    fn lookup(&self, content_hash: &str) -> Option<FileSummary> {
        self.resolver.lookup(self.session_id, content_hash)
    }
}

/// Matches `run_summary_task`'s `SamplingParams.max_tokens` — the reserved space for
/// the response in the file-summary completion call.
pub const MAX_SUMMARY_RESPONSE_TOKENS: usize = 512;

/// Extra headroom to absorb per-flavor tokenizer quirks (BOS/EOS, chat-template wrapping).
pub const SAFETY_PAD: usize = 32;

/// Returns `Ok(())` if the resolved file can be summarized under `context_size` tokens.
/// Returns `Err(FileError::TooLargeForSummary { .. })` if the file is too large to fit,
/// or `Err(FileError::SummaryTokenize { .. })` if the tokenize call itself fails.
///
/// Tokenizes the exact prompt `run_summary_task` would send, so the check is
/// self-consistent with the real call and cache-warms the counter.
pub async fn check_file_fits(
    counter: &crate::tokenizer::TokenCounter,
    file: &crate::files::ResolvedFile,
    instruction: &str,
    context_size: usize,
) -> Result<(), FileError> {
    let prompt = format!(
        "--- FILE ---\n{}\n--- END FILE ---\n\n{}\n\nSummary:",
        file.body, instruction,
    );
    let prompt_tokens = counter
        .count_authoritative(&prompt)
        .await
        .map_err(|source| FileError::SummaryTokenize {
            path: file.canonical_path.clone(),
            source,
        })?;
    let limit = context_size.saturating_sub(MAX_SUMMARY_RESPONSE_TOKENS + SAFETY_PAD);
    if prompt_tokens > limit {
        return Err(FileError::TooLargeForSummary {
            path: file.canonical_path.clone(),
            tokens: prompt_tokens,
            limit,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FakeResolver;
    impl SessionScopedLookup for FakeResolver {
        fn lookup(&self, session_id: &str, hash: &str) -> Option<FileSummary> {
            if session_id == "s1" && hash == "h1" {
                Some(FileSummary {
                    basename: "a.md".to_owned(),
                    summary: "S".to_owned(),
                    status: FileSummaryStatus::Done,
                })
            } else {
                None
            }
        }
    }

    #[test]
    fn scoped_lookup_forwards_to_resolver() {
        let resolver = FakeResolver;
        let scoped = ScopedFileSummaryLookup {
            session_id: "s1",
            resolver: &resolver,
        };
        assert!(scoped.lookup("h1").is_some());
        assert!(scoped.lookup("nope").is_none());
    }

    #[test]
    fn null_lookup_always_returns_none() {
        let null = NullFileSummaryLookup;
        assert!(null.lookup("anything").is_none());
    }

    #[test]
    fn scoped_lookup_other_session_returns_none() {
        let resolver = FakeResolver;
        let scoped = ScopedFileSummaryLookup {
            session_id: "s2",
            resolver: &resolver,
        };
        assert!(scoped.lookup("h1").is_none());
    }

    use crate::files::ResolvedFile;

    fn heuristic_token_counter() -> (
        crate::tokenizer::TokenCounter,
        tokio::sync::mpsc::Receiver<crate::tokenizer::TokenCountUpdate>,
    ) {
        let (tx, rx) = tokio::sync::mpsc::channel(8);
        let counter = crate::tokenizer::TokenCounter::new_with_backend(
            crate::tokenizer::TokenizerBackend::Heuristic(
                crate::tokenizer::HeuristicTokenizer::standard(),
            ),
            tx,
        );
        (counter, rx)
    }

    fn small_file(body: &str) -> ResolvedFile {
        ResolvedFile {
            raw_token: "@notes.md".to_owned(),
            canonical_path: std::path::PathBuf::from("/tmp/notes.md"),
            basename: "notes.md".to_owned(),
            body: body.to_owned(),
            byte_size: body.len(),
        }
    }

    #[tokio::test]
    async fn check_file_fits_accepts_small_file() {
        let (counter, _rx) = heuristic_token_counter();
        let file = small_file("hello world");
        let result = check_file_fits(&counter, &file, "Summarize this file.", 4096).await;
        assert!(result.is_ok(), "expected small file to fit, got {result:?}");
    }

    #[tokio::test]
    async fn check_file_fits_rejects_when_prompt_exceeds_limit() {
        let (counter, _rx) = heuristic_token_counter();
        // 3.3 chars/token heuristic: 100_000 chars ≈ 30_304 tokens + overhead + response reserve.
        let file = small_file(&"a".repeat(100_000));
        let result = check_file_fits(&counter, &file, "Summarize this file.", 4096).await;
        let err = result.expect_err("expected TooLargeForSummary");
        match err {
            FileError::TooLargeForSummary { tokens, limit, .. } => {
                assert!(
                    tokens > limit,
                    "tokens {tokens} should exceed limit {limit}"
                );
            }
            other => panic!("expected TooLargeForSummary, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn check_file_fits_uses_instruction_length_dynamically() {
        let (counter, _rx) = heuristic_token_counter();
        // 3.3 chars/token heuristic: 12_000-char body ≈ 3_639 prompt tokens,
        // which exceeds the 3_552-token limit (4096 - 512 response - 32 pad).
        let file = small_file(&"a".repeat(12_000));
        let short = check_file_fits(&counter, &file, "Summarize.", 4096).await;
        assert!(
            short.is_err(),
            "12k-char body must not fit in 4096-token context"
        );

        // Same file, larger context — should fit.
        let ok = check_file_fits(&counter, &file, "Summarize.", 131_072).await;
        assert!(ok.is_ok());
    }

    #[tokio::test]
    async fn check_file_fits_rejects_when_context_size_below_response_reserve() {
        // saturating_sub guarantees `limit = 0` when context_size <
        // MAX_SUMMARY_RESPONSE_TOKENS + SAFETY_PAD. Any file — including an
        // empty one — must reject; future refactors that replace saturating_sub
        // with `-` would panic on release/debug or silently underflow.
        let (counter, _rx) = heuristic_token_counter();
        let file = small_file("tiny");
        let result = check_file_fits(&counter, &file, "Summarize.", 100).await;
        match result {
            Err(FileError::TooLargeForSummary { limit, .. }) => assert_eq!(limit, 0),
            other => panic!("expected TooLargeForSummary with limit=0, got {other:?}"),
        }
    }
}
