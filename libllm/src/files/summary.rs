//! File-summary types and lookup trait.
//!
//! Holds the data structures that wire together the file-summary cache
//! feature: status enum, row snapshot, scheduling input, ready-event
//! payload, and the lookup traits consumed by `Summarizer::format_prompt`.
//!
//! The `FileSummarizer` struct that actually drives scheduling and polling
//! is added in a follow-up task.

use serde::{Deserialize, Serialize};

pub use crate::db::FileSummaryStatus;

/// Snapshot of one cached file summary as surfaced to consumers.
#[derive(Debug, Clone)]
pub struct FileSummary {
    pub basename: String,
    pub summary: String,
    pub status: FileSummaryStatus,
}

/// Input to `FileSummarizer::schedule` / `ensure_ready`: everything needed
/// to dedupe and, if necessary, summarise a file.
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
pub trait FileSummaryLookup {
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
/// session id and a hash. `FileSummarizer` (added later) implements this.
pub trait SessionScopedLookup {
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
}
