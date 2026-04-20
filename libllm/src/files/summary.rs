//! File-summary types, lookup traits, and the `FileSummarizer` orchestrator.
//!
//! Holds the data structures and background scheduling logic for the
//! file-summary cache feature: status enum, row snapshot, scheduling input,
//! ready-event payload, lookup traits consumed by `Summarizer::format_prompt`,
//! and `FileSummarizer` which drives background LLM summarisation.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use anyhow::Result;
use rusqlite::Connection;
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use crate::client::ApiClient;
use crate::db::file_summaries;
use crate::sampling::SamplingParams;

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

const SUMMARY_STOP_TOKENS: &[&str] = &["\nUser:", "\nAssistant:", "\nSystem:"];
const DEFAULT_POLL_INTERVAL: Duration = Duration::from_millis(100);
const DEFAULT_PER_FILE_TIMEOUT: Duration = Duration::from_secs(60);
const SUMMARY_API_RETRIES: u32 = 2;

/// Orchestrates background LLM summarisation of attached file snapshots,
/// writing results to the `file_summaries` SQLite table via a dedicated
/// `rusqlite::Connection` (isolated from `App.db`) and broadcasting
/// `ReadyEvent`s on state transitions.
pub struct FileSummarizer {
    pub(crate) conn: Arc<Mutex<Connection>>,
    client: ApiClient,
    prompt: String,
    pub(crate) poll_interval: Duration,
    pub(crate) per_file_timeout: Duration,
    pub(crate) ready_tx: mpsc::UnboundedSender<ReadyEvent>,
}

impl FileSummarizer {
    pub fn new(
        conn: Arc<Mutex<Connection>>,
        client: ApiClient,
        prompt: String,
        ready_tx: mpsc::UnboundedSender<ReadyEvent>,
    ) -> Self {
        Self {
            conn,
            client,
            prompt,
            poll_interval: DEFAULT_POLL_INTERVAL,
            per_file_timeout: DEFAULT_PER_FILE_TIMEOUT,
            ready_tx,
        }
    }

    /// Schedules summarisation for one file. Idempotent: if a row already
    /// exists for `(session_id, content_hash)`, no task is spawned.
    pub fn schedule(&self, session_id: &str, file: &FileToSummarize) {
        let inserted = {
            let guard = match self.conn.lock() {
                Ok(g) => g,
                Err(err) => {
                    tracing::error!(
                        result = "error",
                        session_id = %session_id,
                        content_hash = %file.content_hash,
                        error = %err,
                        "files.summary.schedule_lock"
                    );
                    return;
                }
            };
            match file_summaries::insert_pending(
                &guard,
                session_id,
                &file.content_hash,
                &file.basename,
            ) {
                Ok(v) => v,
                Err(err) => {
                    tracing::error!(
                        result = "error",
                        session_id = %session_id,
                        content_hash = %file.content_hash,
                        error = %err,
                        "files.summary.schedule"
                    );
                    return;
                }
            }
        };
        if !inserted {
            return;
        }
        tracing::info!(
            result = "scheduled",
            session_id = %session_id,
            content_hash = %file.content_hash,
            basename = %file.basename,
            body_bytes = file.body.len(),
            "files.summary.schedule"
        );

        let conn = Arc::clone(&self.conn);
        let client = self.client.clone();
        let prompt_instruction = self.prompt.clone();
        let ready_tx = self.ready_tx.clone();
        let session_id = session_id.to_owned();
        let content_hash = file.content_hash.clone();
        let body = file.body.clone();
        tokio::spawn(async move {
            let outcome = run_summary_task(&client, &prompt_instruction, &body).await;
            let status = match &outcome {
                Ok(text) => {
                    let guard = conn.lock().expect("summarizer conn poisoned");
                    if let Err(err) =
                        file_summaries::set_done(&guard, &session_id, &content_hash, text)
                    {
                        tracing::error!(
                            result = "error",
                            error = %err,
                            session_id = %session_id,
                            content_hash = %content_hash,
                            "files.summary.persist_done"
                        );
                    }
                    FileSummaryStatus::Done
                }
                Err(err) => {
                    tracing::warn!(
                        result = "failed",
                        error = %err,
                        session_id = %session_id,
                        content_hash = %content_hash,
                        "files.summary.api"
                    );
                    let guard = conn.lock().expect("summarizer conn poisoned");
                    if let Err(set_err) =
                        file_summaries::set_failed(&guard, &session_id, &content_hash)
                    {
                        tracing::error!(
                            result = "error",
                            error = %set_err,
                            session_id = %session_id,
                            content_hash = %content_hash,
                            "files.summary.persist_failed"
                        );
                    }
                    FileSummaryStatus::Failed
                }
            };
            let _ = ready_tx.send(ReadyEvent {
                session_id,
                content_hash,
                status,
            });
        });
    }

    /// Synchronous DB lookup through the dedicated connection.
    pub fn lookup(&self, session_id: &str, content_hash: &str) -> Option<FileSummary> {
        let guard = self.conn.lock().ok()?;
        match file_summaries::lookup(&guard, session_id, content_hash) {
            Ok(Some(row)) => Some(FileSummary {
                basename: row.basename,
                summary: row.summary,
                status: row.status,
            }),
            Ok(None) => None,
            Err(err) => {
                tracing::error!(
                    result = "error",
                    session_id = %session_id,
                    content_hash = %content_hash,
                    error = %err,
                    "files.summary.lookup"
                );
                None
            }
        }
    }
}

impl SessionScopedLookup for FileSummarizer {
    fn lookup(&self, session_id: &str, content_hash: &str) -> Option<FileSummary> {
        Self::lookup(self, session_id, content_hash)
    }
}

async fn run_summary_task(
    client: &ApiClient,
    instruction: &str,
    body: &str,
) -> Result<String> {
    let prompt = format!("--- FILE ---\n{body}\n--- END FILE ---\n\n{instruction}\n\nSummary:");
    let sampling = SamplingParams {
        temperature: 0.3,
        max_tokens: 512,
        ..SamplingParams::default()
    };

    let mut last_err: Option<anyhow::Error> = None;
    for attempt in 0..=SUMMARY_API_RETRIES {
        match client.complete(&prompt, SUMMARY_STOP_TOKENS, &sampling).await {
            Ok(text) => return Ok(text.trim().to_owned()),
            Err(err) => {
                tracing::warn!(
                    result = "retry",
                    attempt = attempt,
                    error = %err,
                    "files.summary.api"
                );
                last_err = Some(err);
            }
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("summary API call failed")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::ApiClient;
    use rusqlite::Connection;
    use std::sync::{Arc, Mutex};
    use tokio::sync::mpsc;

    fn summarizer_conn() -> Arc<Mutex<Connection>> {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys = ON;").unwrap();
        crate::db::schema::run_migrations(&conn).unwrap();
        conn.execute(
            "INSERT INTO sessions (id, created_at, updated_at) VALUES ('s1', 'now', 'now')",
            [],
        )
        .unwrap();
        Arc::new(Mutex::new(conn))
    }

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

    #[tokio::test]
    async fn summarizer_schedule_inserts_pending_row() {
        let conn = summarizer_conn();
        let (tx, _rx) = mpsc::unbounded_channel();
        let summarizer = FileSummarizer::new(
            Arc::clone(&conn),
            ApiClient::new("http://127.0.0.1:1", true, crate::config::Auth::None),
            "summarize this".to_owned(),
            tx,
        );
        let file = FileToSummarize {
            basename: "a.md".to_owned(),
            content_hash: "h1".to_owned(),
            body: "hello world".to_owned(),
        };
        summarizer.schedule("s1", &file);

        let row = crate::db::file_summaries::lookup(&conn.lock().unwrap(), "s1", "h1")
            .unwrap()
            .unwrap();
        assert_eq!(row.status, FileSummaryStatus::Pending);
    }

    #[tokio::test]
    async fn summarizer_schedule_is_idempotent() {
        let conn = summarizer_conn();
        let (tx, _rx) = mpsc::unbounded_channel();
        let summarizer = FileSummarizer::new(
            Arc::clone(&conn),
            ApiClient::new("http://127.0.0.1:1", true, crate::config::Auth::None),
            "summarize this".to_owned(),
            tx,
        );
        let file = FileToSummarize {
            basename: "a.md".to_owned(),
            content_hash: "h1".to_owned(),
            body: "hello world".to_owned(),
        };
        summarizer.schedule("s1", &file);
        summarizer.schedule("s1", &file);

        let count: i64 = conn
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM file_summaries", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn summarizer_lookup_returns_pending_row() {
        let conn = summarizer_conn();
        let (tx, _rx) = mpsc::unbounded_channel();
        let summarizer = FileSummarizer::new(
            Arc::clone(&conn),
            ApiClient::new("http://127.0.0.1:1", true, crate::config::Auth::None),
            "summarize this".to_owned(),
            tx,
        );
        let file = FileToSummarize {
            basename: "a.md".to_owned(),
            content_hash: "h1".to_owned(),
            body: "hello world".to_owned(),
        };
        summarizer.schedule("s1", &file);

        let got = summarizer.lookup("s1", "h1").unwrap();
        assert_eq!(got.status, FileSummaryStatus::Pending);
        assert_eq!(got.basename, "a.md");
    }
}
